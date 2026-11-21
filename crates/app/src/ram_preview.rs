use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use uuid::Uuid;
use ve_audio::WaveformPeaks;
use ve_core::{Project, ProjectFolder, RationalTime, SceneId};
use ve_render::{LinearImage, RenderGraph};

/// RAM プレビューキャッシュの上限フレーム数（640x360 Linear f32 で ~3.6MB/枚 → 上限 96 枚 ≈ 340MB）
pub const MAX_RAM_PREVIEW_FRAMES: usize = 96;
/// 再生ヘッドの前方に先読みするフレーム数
const PREFETCH_AHEAD_FRAMES: i64 = 45;
/// 再生ヘッドの直前も数フレーム先読みしておく（J逆再生や微調整用）
const PREFETCH_BEHIND_FRAMES: i64 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RamFrameKey {
    pub revision: u64,
    pub scene_id: SceneId,
    pub frame_idx: i64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
struct PrefetchTarget {
    project: Arc<Project>,
    folder: Option<ProjectFolder>,
    proxies: HashMap<Uuid, PathBuf>,
    waveforms: BTreeMap<Uuid, WaveformPeaks>,
    use_proxies: bool,
    revision: u64,
    scene_id: SceneId,
    center_frame: i64,
    width: u32,
    height: u32,
    enabled: bool,
}

struct SharedState {
    target: Option<PrefetchTarget>,
    frames: HashMap<RamFrameKey, (u64, Arc<LinearImage>)>,
    lru_tick: u64,
    shutdown: bool,
}

/// AE 風のバックグラウンド RAM プレビュー先読みキャッシュマネージャ。
/// 専用ワーカースレッドが `RenderGraph`（常駐 `VideoStreamDecoder` 内蔵）を保持し、
/// 再生ヘッド周辺のフレームを先読みして再生・スクラブ時のフレーム落ちを防止する。
pub struct RamPreviewManager {
    pub enabled: bool,
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    worker: Option<JoinHandle<()>>,
    last_submitted: Option<(u64, SceneId, i64, u32, u32, bool)>,
}

impl Default for RamPreviewManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RamPreviewManager {
    pub fn new() -> Self {
        let shared = Arc::new((
            Mutex::new(SharedState {
                target: None,
                frames: HashMap::new(),
                lru_tick: 0,
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let bg = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("ve-ram-preview".to_string())
            .spawn(move || ram_preview_worker_loop(bg))
            .ok();

        Self {
            enabled: true,
            shared,
            worker,
            last_submitted: None,
        }
    }

    /// 再生ヘッド位置・編集リビジョン・プレビュー解像度をワーカーへ通知する
    #[allow(clippy::too_many_arguments)]
    pub fn update_target(
        &mut self,
        project: &Project,
        folder: Option<&ProjectFolder>,
        proxies: &HashMap<Uuid, PathBuf>,
        waveforms: &BTreeMap<Uuid, WaveformPeaks>,
        use_proxies: bool,
        revision: u64,
        scene_id: SceneId,
        center_frame: i64,
        width: u32,
        height: u32,
    ) {
        let sig = (
            revision,
            scene_id,
            center_frame,
            width,
            height,
            self.enabled,
        );
        if self.last_submitted == Some(sig) {
            return;
        }
        self.last_submitted = Some(sig);

        let (lock, cvar) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            // リビジョン・解像度が変わったら古いキャッシュを即座に破棄
            if st
                .target
                .as_ref()
                .is_some_and(|t| t.revision != revision || t.width != width || t.height != height)
            {
                st.frames.clear();
            }
            st.target = Some(PrefetchTarget {
                project: Arc::new(project.clone()),
                folder: folder.cloned(),
                proxies: proxies.clone(),
                waveforms: waveforms.clone(),
                use_proxies,
                revision,
                scene_id,
                center_frame: center_frame.max(0),
                width: width.max(1),
                height: height.max(1),
                enabled: self.enabled,
            });
            cvar.notify_one();
        }
    }

    /// 先読み済みキャッシュからフレームを O(1) で取得する
    pub fn get_frame(&self, key: RamFrameKey) -> Option<Arc<LinearImage>> {
        let (lock, _) = &*self.shared;
        let mut st = lock.lock().ok()?;
        let next_tick = st.lru_tick.wrapping_add(1);
        if let Some((tick, img)) = st.frames.get_mut(&key) {
            *tick = next_tick;
            let cloned = Arc::clone(img);
            st.lru_tick = next_tick;
            Some(cloned)
        } else {
            None
        }
    }

    /// メインスレッド側で同期レンダリングしたフレームも RAM キャッシュへ格納して再利用する
    pub fn insert_frame(&self, key: RamFrameKey, img: Arc<LinearImage>) {
        let (lock, _) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            insert_lru(&mut st, key, img);
        }
    }

    pub fn clear(&mut self) {
        self.last_submitted = None;
        let (lock, _) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            st.frames.clear();
        }
    }

    pub fn cached_count(&self, revision: u64, scene_id: SceneId, width: u32, height: u32) -> usize {
        let (lock, _) = &*self.shared;
        let Ok(st) = lock.lock() else {
            return 0;
        };
        st.frames
            .keys()
            .filter(|k| {
                k.revision == revision
                    && k.scene_id == scene_id
                    && k.width == width
                    && k.height == height
            })
            .count()
    }

    /// タイムライン上の緑色キャッシュ済みバー描画用に、連続するキャッシュ済みフレーム区間 `(start_f, end_f_inclusive)` を返す
    pub fn cached_frame_ranges(
        &self,
        revision: u64,
        scene_id: SceneId,
        width: u32,
        height: u32,
    ) -> Vec<(i64, i64)> {
        let (lock, _) = &*self.shared;
        let Ok(st) = lock.lock() else {
            return Vec::new();
        };
        let mut frames: Vec<i64> = st
            .frames
            .keys()
            .filter(|k| {
                k.revision == revision
                    && k.scene_id == scene_id
                    && k.width == width
                    && k.height == height
            })
            .map(|k| k.frame_idx)
            .collect();
        if frames.is_empty() {
            return Vec::new();
        }
        frames.sort_unstable();
        let mut ranges = Vec::new();
        let mut start = frames[0];
        let mut prev = frames[0];
        for &f in &frames[1..] {
            if f <= prev + 1 {
                prev = f;
            } else {
                ranges.push((start, prev));
                start = f;
                prev = f;
            }
        }
        ranges.push((start, prev));
        ranges
    }
}

impl Drop for RamPreviewManager {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            st.shutdown = true;
            cvar.notify_all();
        }
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

fn insert_lru(st: &mut SharedState, key: RamFrameKey, img: Arc<LinearImage>) {
    if !st.frames.contains_key(&key)
        && st.frames.len() >= MAX_RAM_PREVIEW_FRAMES
        && let Some(oldest) = st
            .frames
            .iter()
            .min_by_key(|(_, (tick, _))| *tick)
            .map(|(k, _)| *k)
    {
        st.frames.remove(&oldest);
    }
    st.lru_tick = st.lru_tick.wrapping_add(1);
    let tick = st.lru_tick;
    st.frames.insert(key, (tick, img));
}

fn ram_preview_worker_loop(shared: Arc<(Mutex<SharedState>, Condvar)>) {
    let mut rg = RenderGraph::new();
    let (lock, cvar) = &*shared;

    loop {
        let work = {
            let mut st = match lock.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            loop {
                if st.shutdown {
                    return;
                }
                if let Some(ref target) = st.target
                    && target.enabled
                    && let Some(next_f) = pick_next_uncached_frame(target, &st.frames)
                {
                    break (target.clone(), next_f);
                }
                st = match cvar.wait(st) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
        };

        let (target, frame_idx) = work;
        rg.use_proxies = target.use_proxies;
        rg.set_project_folder(target.folder.clone());
        for (aid, p) in &target.proxies {
            rg.register_proxy(*aid, p.clone());
        }
        for (aid, wf) in &target.waveforms {
            if !rg.asset_waveforms.contains_key(aid) {
                rg.register_waveform(*aid, wf.clone());
            }
        }

        let Some(fps) = target.project.scene(target.scene_id).map(|s| s.fps) else {
            continue;
        };
        let t = RationalTime::from_frame(frame_idx, fps);
        let Some(img) = rg.render_project_scene_scaled(
            &target.project,
            target.scene_id,
            t,
            target.width,
            target.height,
            false,
        ) else {
            continue;
        };

        let key = RamFrameKey {
            revision: target.revision,
            scene_id: target.scene_id,
            frame_idx,
            width: target.width,
            height: target.height,
        };
        if let Ok(mut st) = lock.lock() {
            // レンダリング中にリビジョンが進んでいたら破棄して最新ターゲットへ即切り替え
            let still_current = st.target.as_ref().is_some_and(|t| {
                t.revision == target.revision
                    && t.scene_id == target.scene_id
                    && t.width == target.width
                    && t.height == target.height
            });
            if still_current {
                insert_lru(&mut st, key, Arc::new(img));
            }
        }
    }
}

fn pick_next_uncached_frame(
    target: &PrefetchTarget,
    frames: &HashMap<RamFrameKey, (u64, Arc<LinearImage>)>,
) -> Option<i64> {
    let scene = target.project.scene(target.scene_id)?;
    let max_frame =
        ((scene.total_duration().as_seconds().max(1.0) * scene.fps.as_f64()).ceil() as i64).max(1);
    let c = target.center_frame.clamp(0, max_frame);

    // 1. 再生ヘッドから前方へ優先的に連続先読み（常駐 VideoStreamDecoder の効率を最大化）
    let ahead_end = (c + PREFETCH_AHEAD_FRAMES).min(max_frame);
    for f in c..=ahead_end {
        let key = RamFrameKey {
            revision: target.revision,
            scene_id: target.scene_id,
            frame_idx: f,
            width: target.width,
            height: target.height,
        };
        if !frames.contains_key(&key) {
            return Some(f);
        }
    }

    // 2. 直前の数フレームも先読み
    let behind_start = (c - PREFETCH_BEHIND_FRAMES).max(0);
    for f in behind_start..c {
        let key = RamFrameKey {
            revision: target.revision,
            scene_id: target.scene_id,
            frame_idx: f,
            width: target.width,
            height: target.height,
        };
        if !frames.contains_key(&key) {
            return Some(f);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use ve_core::{ItemKind, ProjectSettings, RgbaColor, ShapeKind, TimelineItem};

    #[test]
    fn ram_preview_prefetches_frames_and_invalidates_on_revision_change() {
        let mut proj = Project::new(
            "RamPreviewTest",
            ProjectSettings {
                width: 64,
                height: 36,
                ..Default::default()
            },
        );
        proj.active_scene_mut().unwrap().layers[0]
            .items
            .push(TimelineItem::new(
                "Bg",
                RationalTime::ZERO,
                RationalTime::from_seconds(0.5),
                ItemKind::Shape {
                    shape: ShapeKind::Rectangle,
                    width: 64.0,
                    height: 36.0,
                    color: RgbaColor::WHITE,
                },
            ));

        let mut mgr = RamPreviewManager::new();
        let proxies = HashMap::new();
        let waveforms = BTreeMap::new();
        mgr.update_target(
            &proj,
            None,
            &proxies,
            &waveforms,
            true,
            1,
            proj.active_scene_id,
            0,
            64,
            36,
        );

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && mgr.cached_count(1, proj.active_scene_id, 64, 36) < 3 {
            thread::sleep(Duration::from_millis(15));
        }
        assert!(
            mgr.cached_count(1, proj.active_scene_id, 64, 36) >= 3,
            "expected background worker to prefetch frames"
        );
        let ranges = mgr.cached_frame_ranges(1, proj.active_scene_id, 64, 36);
        assert!(!ranges.is_empty());

        // リビジョン更新で古いキャッシュが自動破棄される
        mgr.update_target(
            &proj,
            None,
            &proxies,
            &waveforms,
            true,
            2,
            proj.active_scene_id,
            0,
            64,
            36,
        );
        assert_eq!(mgr.cached_count(1, proj.active_scene_id, 64, 36), 0);
    }
}
