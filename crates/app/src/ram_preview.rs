use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use uuid::Uuid;
use ve_audio::WaveformPeaks;
use ve_core::{Project, ProjectFolder, RationalTime, SceneId};
use ve_media::HwDecodeBackend;
use ve_render::{LinearImage, RenderGraph};

/// RAM プレビューキャッシュの上限フレーム数（`Rgba16Float` 8 B/px 格納により従来の 16 B/px 比でメモリ消費を 50% 削減：640x360×96 枚 ≈ 170 MiB）
pub const MAX_RAM_PREVIEW_FRAMES: usize = 96;
/// 再生ヘッドの前方に先読みするフレーム数
const PREFETCH_AHEAD_FRAMES: i64 = 45;
/// 再生ヘッドの直前も数フレーム先読みしておく（J逆再生や微調整用）
const PREFETCH_BEHIND_FRAMES: i64 = 6;

/// `Rgba16Float` パック済みの省メモリ RAM プレビューフレーム（8 B/px: `LinearImage` 比 50% 省メモリ＋GPU 直接転送対応）
#[derive(Debug, Clone)]
pub struct PackedRamFrame {
    pub width: u32,
    pub height: u32,
    pub rgba16f: Vec<u8>,
}

impl PackedRamFrame {
    pub fn from_linear_image(img: &LinearImage) -> Self {
        Self {
            width: img.width,
            height: img.height,
            rgba16f: img.to_rgba16f_bytes(),
        }
    }

    pub fn to_linear_image(&self) -> Option<LinearImage> {
        LinearImage::from_rgba16f_bytes(self.width, self.height, &self.rgba16f)
    }

    pub fn byte_len(&self) -> usize {
        self.rgba16f.len()
    }
}

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
    proxies: Arc<HashMap<Uuid, PathBuf>>,
    waveforms: Arc<BTreeMap<Uuid, WaveformPeaks>>,
    use_proxies: bool,
    hw_decode: HwDecodeBackend,
    disk_cache_bytes: u64,
    revision: u64,
    scene_id: SceneId,
    center_frame: i64,
    width: u32,
    height: u32,
    enabled: bool,
}

struct SharedState {
    target: Option<PrefetchTarget>,
    frames: HashMap<RamFrameKey, (u64, Arc<PackedRamFrame>)>,
    lru_tick: u64,
    shutdown: bool,
}

type TargetMetaSig = (u64, SceneId, u32, u32, bool, usize, usize, Option<PathBuf>);

/// AE 風のバックグラウンド RAM プレビュー先読みキャッシュマネージャ。
/// 専用ワーカースレッドが `RenderGraph`（常駐 `VideoStreamDecoder`＋SSD 永続フレームキャッシュ内蔵）を保持し、
/// 再生ヘッド周辺のフレームを 8 B/px `Rgba16Float` で省メモリ先読みしてフレーム落ちを防止する。
pub struct RamPreviewManager {
    pub enabled: bool,
    pub hw_decode: HwDecodeBackend,
    pub disk_cache_max_bytes: u64,
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    worker: Option<JoinHandle<()>>,
    last_submitted: Option<(u64, SceneId, i64, u32, u32, bool)>,
    last_meta_sig: Option<TargetMetaSig>,
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
            hw_decode: HwDecodeBackend::Auto,
            disk_cache_max_bytes: 512 * 1024 * 1024,
            shared,
            worker,
            last_submitted: None,
            last_meta_sig: None,
        }
    }

    /// 呼び出し側で `Project` / `proxy_paths` のクローンを行う前に、ターゲット更新が必要かを O(1) 判定する
    pub fn needs_target_update(
        &self,
        revision: u64,
        scene_id: SceneId,
        center_frame: i64,
        width: u32,
        height: u32,
    ) -> bool {
        let sig = (
            revision,
            scene_id,
            center_frame,
            width,
            height,
            self.enabled,
        );
        self.last_submitted != Some(sig)
    }

    /// 編集リビジョン・プロキシ・波形・フォルダが変わらず `center_frame`（再生・スクラブ位置）だけ進んだ場合のゼロクローン高速更新。
    /// 更新に成功した場合は `true`（呼び出し側は `Project` クローン不要）、フル更新が必要な場合は `false` を返す。
    #[allow(clippy::too_many_arguments)]
    pub fn try_update_center_frame_fast(
        &mut self,
        folder: Option<&ProjectFolder>,
        use_proxies: bool,
        proxy_count: usize,
        waveform_count: usize,
        revision: u64,
        scene_id: SceneId,
        center_frame: i64,
        width: u32,
        height: u32,
    ) -> bool {
        let sig = (
            revision,
            scene_id,
            center_frame,
            width,
            height,
            self.enabled,
        );
        if self.last_submitted == Some(sig) {
            return true;
        }
        let meta_sig = (
            revision,
            scene_id,
            width,
            height,
            use_proxies,
            proxy_count,
            waveform_count,
            folder.map(|f| f.root().to_path_buf()),
        );
        if self.last_meta_sig.as_ref() != Some(&meta_sig) {
            return false;
        }

        let (lock, cvar) = &*self.shared;
        let Ok(mut st) = lock.lock() else {
            return false;
        };
        let Some(target) = st.target.as_mut() else {
            return false;
        };
        target.center_frame = center_frame.max(0);
        target.hw_decode = self.hw_decode;
        target.disk_cache_bytes = self.disk_cache_max_bytes;
        target.enabled = self.enabled;
        self.last_submitted = Some(sig);
        cvar.notify_one();
        true
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
        if self.try_update_center_frame_fast(
            folder,
            use_proxies,
            proxies.len(),
            waveforms.len(),
            revision,
            scene_id,
            center_frame,
            width,
            height,
        ) {
            return;
        }

        let sig = (
            revision,
            scene_id,
            center_frame,
            width,
            height,
            self.enabled,
        );
        self.last_submitted = Some(sig);
        self.last_meta_sig = Some((
            revision,
            scene_id,
            width,
            height,
            use_proxies,
            proxies.len(),
            waveforms.len(),
            folder.map(|f| f.root().to_path_buf()),
        ));

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
            let reuse_project = st
                .target
                .as_ref()
                .filter(|t| t.revision == revision)
                .map(|t| Arc::clone(&t.project));
            let reuse_proxies = st
                .target
                .as_ref()
                .filter(|t| t.proxies.len() == proxies.len() && *t.proxies == *proxies)
                .map(|t| Arc::clone(&t.proxies));
            let reuse_waveforms = st
                .target
                .as_ref()
                .filter(|t| t.waveforms.len() == waveforms.len())
                .map(|t| Arc::clone(&t.waveforms));
            st.target = Some(PrefetchTarget {
                project: reuse_project.unwrap_or_else(|| Arc::new(project.clone())),
                folder: folder.cloned(),
                proxies: reuse_proxies.unwrap_or_else(|| Arc::new(proxies.clone())),
                waveforms: reuse_waveforms.unwrap_or_else(|| Arc::new(waveforms.clone())),
                use_proxies,
                hw_decode: self.hw_decode,
                disk_cache_bytes: self.disk_cache_max_bytes,
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

    /// 先読み済みキャッシュから `Rgba16Float` フレームを O(1) で取得する
    pub fn get_frame(&self, key: RamFrameKey) -> Option<Arc<PackedRamFrame>> {
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

    /// メインスレッド側で同期レンダリングしたフレームも 8 B/px パック済みで RAM キャッシュへ格納して再利用する
    pub fn insert_frame(&self, key: RamFrameKey, img: Arc<PackedRamFrame>) {
        let (lock, _) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            insert_lru(&mut st, key, img);
        }
    }

    pub fn clear(&mut self) {
        self.last_submitted = None;
        self.last_meta_sig = None;
        let (lock, _) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            st.frames.clear();
        }
    }

    pub fn cached_bytes(&self) -> usize {
        let (lock, _) = &*self.shared;
        let Ok(st) = lock.lock() else {
            return 0;
        };
        st.frames.values().map(|(_, f)| f.byte_len()).sum()
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

fn insert_lru(st: &mut SharedState, key: RamFrameKey, img: Arc<PackedRamFrame>) {
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
    let mut disk_cache_dir: Option<PathBuf> = None;
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
        rg.active_revision = target.revision.max(1);
        rg.use_proxies = target.use_proxies;
        rg.set_hw_decode(target.hw_decode);
        rg.set_project_folder(target.folder.clone());
        if let Some(ref folder) = target.folder {
            let frames_dir = folder.cache_frames_dir();
            if disk_cache_dir.as_ref() != Some(&frames_dir) {
                let _ = rg.enable_disk_frame_cache(frames_dir.clone(), target.disk_cache_bytes);
                disk_cache_dir = Some(frames_dir);
            }
        }
        for (aid, p) in target.proxies.iter() {
            rg.register_proxy(*aid, p.clone());
        }
        for (aid, wf) in target.waveforms.iter() {
            if !rg.asset_waveforms.contains_key(aid) {
                rg.register_waveform(*aid, wf.clone());
            }
        }

        let Some(fps) = target.project.scene(target.scene_id).map(|s| s.fps) else {
            continue;
        };
        let t = RationalTime::from_frame(frame_idx, fps);
        let img = if disk_cache_dir.is_some() {
            match rg.render_project_scene_cached_rgba8(
                &target.project,
                target.scene_id,
                t,
                target.width,
                target.height,
                false,
            ) {
                Some(rgba) => LinearImage::from_rgba8_srgb(target.width, target.height, &rgba),
                None => continue,
            }
        } else {
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
            img
        };

        let packed = Arc::new(PackedRamFrame::from_linear_image(&img));
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
                insert_lru(&mut st, key, packed);
            }
        }
    }
}

fn pick_next_uncached_frame(
    target: &PrefetchTarget,
    frames: &HashMap<RamFrameKey, (u64, Arc<PackedRamFrame>)>,
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
        assert!(mgr.needs_target_update(1, proj.active_scene_id, 0, 64, 36));
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
        assert!(!mgr.needs_target_update(1, proj.active_scene_id, 0, 64, 36));
        // 再生ヘッド移動のみの高速パスは Project クローンなしで true を返す
        assert!(mgr.try_update_center_frame_fast(
            None,
            true,
            0,
            0,
            1,
            proj.active_scene_id,
            1,
            64,
            36,
        ));

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && mgr.cached_count(1, proj.active_scene_id, 64, 36) < 3 {
            thread::sleep(Duration::from_millis(15));
        }
        let cnt = mgr.cached_count(1, proj.active_scene_id, 64, 36);
        assert!(cnt >= 3, "expected background worker to prefetch frames");
        // 8 B/px Rgba16Float 格納（従来の 16 B/px の半分）を検証
        assert_eq!(mgr.cached_bytes(), cnt * 64 * 36 * 8);
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
