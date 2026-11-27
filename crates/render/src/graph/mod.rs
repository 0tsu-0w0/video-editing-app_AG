//! Render Graph コア評価器（v10.0.0 モジュール分割: `mod.rs` / `effects.rs` / `tests.rs`）

mod effects;
#[cfg(test)]
mod tests;

use crate::color::{LinearImage, LinearPremul};
use crate::color_grading::{CubeLut3D, apply_scene_color_grade, apply_scene_color_management};
use crate::hot_shader::ShaderHotReloader;
use crate::psd_sprite::PsdCharacterAsset;
use crate::sprite::{load_png_linear, render_placeholder_character, scale_linear_image};
use crate::text::TextRenderer;
use crate::v09_fx::{
    apply_3d_lighting_tint, build_drop_shadow_image, render_particle_emitter, render_vector_shape,
};
use effects::{apply_effect_chain, apply_group_transform, apply_transition, resolve_scope_layers};
pub(crate) use effects::{apply_named_effect, box_blur_separable, rotate_linear_image};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use ve_audio::WaveformPeaks;
use ve_core::{
    AssetResolution, AudioReactiveBand, BlendMode, CharacterSpriteConfig, ColorPalette, EffectNode,
    FilterTargetScope, ItemKind, LipSyncMode, MouthShape, PersistentFrameCache,
    PrecompCacheManager, Project, ProjectFolder, RationalTime, RgbaColor, Scene, SceneId,
    ShapeKind, TimelineItem, Transform2D, TransitionKind, content_hash_hex, evaluate_item_property,
    evaluate_item_transform, evaluate_vowel_mouth, item_expressions_use_audio, resolve_mouth_png,
    rms_to_mouth,
};
use ve_media::{FfmpegCli, HwDecodeBackend, VideoStreamDecoder};
use ve_plugin_api::PluginRegistry;

/// 組み込み WGSL エフェクトチェーンシェーダーソース
pub const EFFECTS_WGSL_SOURCE: &str = include_str!("../shaders/effects.wgsl");

/// インメモリ Precomp キャッシュの上限フレーム数（スクラブ時の RAM 肥大化を防止）
pub const MAX_MEM_PRECOMP_FRAMES: usize = 32;
const MAX_PNG_CACHE_ENTRIES: usize = 64;
const MAX_PSD_CACHE_ENTRIES: usize = 16;
const MAX_VIDEO_FRAME_CACHE: usize = 64;
const MAX_VIDEO_STREAMS: usize = 4;

type VideoFrameCacheKey = (Uuid, i64, u32, u32, bool);
type VideoStreamKey = (Uuid, u32, u32, bool);
type SceneSigCacheKey = (SceneId, u64, bool, bool, bool);

/// 仕様書 2.2 / 4.2 の評価順序に従う Render Graph 評価器:
/// 1. 各オブジェクトのキーフレーム／エクスプレッション評価・実動画/画像デコード・パーティクル・ベクターシェイプ・立ち絵口パク・3Dカメラ透視投影/照明/影/DoF・個別エフェクト＆プラグイン適用
/// 2. トラックマット・グループ制御・AviUtl2風フィルタオブジェクト（指定レイヤー範囲）・シーンチェンジ適用
/// 3. レイヤー合成（Linear sRGB + Premultiplied Alpha + BlendMode + 3D Zソート）＋ シャッター角モーションブラー ＋ シーンカラーグレーディング＆`.cube` 3D LUT
pub struct RenderGraph {
    text_renderer: TextRenderer,
    mem_precomp_cache: HashMap<String, (u64, LinearImage)>,
    mem_cache_tick: u64,
    disk_precomp_cache: Option<PrecompCacheManager>,
    /// v1.0 #18: アプリ再起動後も再利用できる SSD 永続フレームキャッシュ（`cache/frames/`）
    disk_frame_cache: Option<PersistentFrameCache>,
    project_folder: Option<ProjectFolder>,
    ffmpeg: FfmpegCli,
    /// v1.0 #18: FFmpeg ハードウェアデコード設定（自動検出＋安全 Software フォールバック）
    pub hw_decode: HwDecodeBackend,
    resolved_hw_decode: Option<HwDecodeBackend>,
    /// true のときプレビュー用に軽量プロキシ (`cache/proxy/`) を優先使用（書き出し時は false で原本参照）
    pub use_proxies: bool,
    /// v0.9 #16: true のとき書き出し用高品質サブフレーム分割数でモーションブラーを合成（プレビュー時は false で軽量）
    pub high_quality_motion_blur: bool,
    proxy_paths: HashMap<Uuid, PathBuf>,
    video_frame_cache: HashMap<VideoFrameCacheKey, (u64, LinearImage)>,
    video_frame_tick: u64,
    video_streams: HashMap<VideoStreamKey, VideoStreamDecoder>,
    png_cache: HashMap<PathBuf, LinearImage>,
    psd_cache: HashMap<PathBuf, PsdCharacterAsset>,
    /// v0.8 #13: `.cube` 3D LUT メモリキャッシュ（`Arc` 共有で毎フレームの 430KB クローンをゼロ化）
    lut_cache: HashMap<String, Option<Arc<CubeLut3D>>>,
    /// SSD 永続フレームキャッシュ向けのシーンハッシュ署名キャッシュ（リビジョン不変時の毎フレーム JSON 化を排除）
    cached_scene_sig: Option<(SceneSigCacheKey, String)>,
    /// 音声アセットIDごとの事前解析波形（立ち絵の音量連動口パク参照用）
    pub asset_waveforms: HashMap<Uuid, WaveformPeaks>,
    /// Phase 4: OSS プラグインレジストリ（ビネット・セピア・外部プラグイン）
    pub plugins: PluginRegistry,
    /// Phase 4: カスタム WGSL シェーダーのホットリロード管理
    pub hot_shaders: ShaderHotReloader,
    /// v7.0 #47: 4K / 8K レイヤー合成キャンバスのゼロ再確保プール（LRU Eviction 対応）
    pub frame_pool: crate::gpu_v70::ResidentFramePool,
    /// v7.0 #48: 10,000+ クリップシーン向け $O(\log N)$ 区間木キャッシュ
    pub interval_index: ve_core::CachedSceneIntervalIndex,
    /// `CommandBus::revision()` を同期すると 10,000+ クリップでも毎フレーム再構築ゼロで区間木を再利用する
    pub active_revision: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

impl Default for RenderGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderGraph {
    pub fn new() -> Self {
        Self {
            text_renderer: TextRenderer::new(),
            mem_precomp_cache: HashMap::new(),
            mem_cache_tick: 0,
            disk_precomp_cache: None,
            disk_frame_cache: None,
            project_folder: None,
            ffmpeg: FfmpegCli::default(),
            hw_decode: HwDecodeBackend::Auto,
            resolved_hw_decode: None,
            use_proxies: true,
            high_quality_motion_blur: false,
            proxy_paths: HashMap::new(),
            video_frame_cache: HashMap::new(),
            video_frame_tick: 0,
            video_streams: HashMap::new(),
            png_cache: HashMap::new(),
            psd_cache: HashMap::new(),
            lut_cache: HashMap::new(),
            cached_scene_sig: None,
            asset_waveforms: HashMap::new(),
            plugins: PluginRegistry::with_builtins(),
            hot_shaders: ShaderHotReloader::new(),
            frame_pool: crate::gpu_v70::ResidentFramePool::default(),
            interval_index: ve_core::CachedSceneIntervalIndex::default(),
            active_revision: 0,
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    /// v1.0 #18: HW デコード設定を変更し、既存の常駐デコーダを再初期化する
    pub fn set_hw_decode(&mut self, backend: HwDecodeBackend) {
        if self.hw_decode != backend {
            self.hw_decode = backend;
            self.resolved_hw_decode = None;
            self.video_streams.clear();
        }
    }

    /// 実際に使用する HW デコードバックエンドを解決する（未解決時は初回自動判定）
    pub fn effective_hw_decode(&mut self) -> HwDecodeBackend {
        if let Some(resolved) = self.resolved_hw_decode {
            return resolved;
        }
        let resolved = self.ffmpeg.resolve_hw_decoder(self.hw_decode);
        self.resolved_hw_decode = Some(resolved);
        resolved
    }

    /// v1.0 #18: SSD 永続フレームキャッシュ（`cache/frames/`）を有効化する
    pub fn enable_disk_frame_cache(&mut self, cache_dir: PathBuf, max_bytes: u64) -> bool {
        match PersistentFrameCache::open(cache_dir, max_bytes) {
            Ok(fc) => {
                self.disk_frame_cache = Some(fc);
                true
            }
            Err(_) => false,
        }
    }

    /// 永続フレームキャッシュの統計情報 `(entries, total_bytes, hits, misses)`
    pub fn disk_frame_cache_stats(&self) -> Option<(usize, u64, u64, u64)> {
        self.disk_frame_cache
            .as_ref()
            .map(|c| (c.entry_count(), c.total_bytes(), c.hits(), c.misses()))
    }

    pub fn text_renderer(&self) -> &TextRenderer {
        &self.text_renderer
    }

    pub fn text_renderer_mut(&mut self) -> &mut TextRenderer {
        &mut self.text_renderer
    }

    pub fn set_project_folder(&mut self, folder: Option<ProjectFolder>) {
        let same_root = match (&self.project_folder, &folder) {
            (Some(a), Some(b)) => a.root() == b.root(),
            (None, None) => true,
            _ => false,
        };
        if !same_root {
            self.project_folder = folder;
            self.lut_cache.clear();
            self.cached_scene_sig = None;
        }
    }

    pub fn register_proxy(&mut self, asset_id: Uuid, proxy_path: PathBuf) {
        if self.proxy_paths.get(&asset_id) == Some(&proxy_path) {
            return;
        }
        self.proxy_paths.insert(asset_id, proxy_path);
        // プロキシ切替時に古いデコード済みフレームと開いているストリームを破棄
        self.video_frame_cache
            .retain(|(aid, _, _, _, _), _| *aid != asset_id);
        self.video_streams
            .retain(|(aid, _, _, _), _| *aid != asset_id);
    }

    pub fn proxy_count(&self) -> usize {
        self.proxy_paths.len()
    }

    pub fn proxy_paths_snapshot(&self) -> HashMap<Uuid, PathBuf> {
        self.proxy_paths.clone()
    }

    pub fn clear_media_caches(&mut self) {
        self.mem_precomp_cache.clear();
        self.video_frame_cache.clear();
        self.video_streams.clear();
        self.png_cache.clear();
        self.psd_cache.clear();
        self.lut_cache.clear();
        self.cached_scene_sig = None;
        self.text_renderer.clear_text_cache();
        self.proxy_paths.clear();
        self.frame_pool.clear();
        self.interval_index.invalidate();
    }

    /// `.cube` 3D LUT を組み込みプリセット / プロジェクト `luts/` / 絶対パスから解決して `Arc` キャッシュする
    fn resolve_lut(&mut self, spec: &str) -> Option<Arc<CubeLut3D>> {
        let key = spec.trim();
        if key.is_empty() {
            return None;
        }
        if let Some(cached) = self.lut_cache.get(key) {
            return cached.as_ref().map(Arc::clone);
        }
        let loaded = if key.starts_with("builtin:") {
            CubeLut3D::builtin_preset(key).map(Arc::new)
        } else {
            let raw_path = PathBuf::from(key);
            let candidate = if raw_path.is_absolute() && raw_path.exists() {
                Some(raw_path)
            } else if let Some(folder) = self.project_folder.as_ref() {
                let p1 = folder.root().join(key);
                let p2 = folder.luts_dir().join(key);
                if p1.exists() {
                    Some(p1)
                } else if p2.exists() {
                    Some(p2)
                } else if raw_path.exists() {
                    Some(raw_path)
                } else {
                    None
                }
            } else if raw_path.exists() {
                Some(raw_path)
            } else {
                None
            };
            candidate.and_then(|p| CubeLut3D::load_file(&p).ok().map(Arc::new))
        };
        if self.lut_cache.len() >= 16 {
            self.lut_cache.clear();
        }
        self.lut_cache.insert(key.to_string(), loaded.clone());
        loaded
    }

    pub fn mem_precomp_cache_len(&self) -> usize {
        self.mem_precomp_cache.len()
    }

    pub fn video_frame_cache_len(&self) -> usize {
        self.video_frame_cache.len()
    }

    fn insert_mem_precomp(&mut self, key: String, img: LinearImage) {
        if !self.mem_precomp_cache.contains_key(&key)
            && self.mem_precomp_cache.len() >= MAX_MEM_PRECOMP_FRAMES
            && let Some(oldest_key) = self
                .mem_precomp_cache
                .iter()
                .min_by_key(|(_, (tick, _))| *tick)
                .map(|(k, _)| k.clone())
        {
            self.mem_precomp_cache.remove(&oldest_key);
        }
        self.mem_cache_tick = self.mem_cache_tick.wrapping_add(1);
        self.mem_precomp_cache
            .insert(key, (self.mem_cache_tick, img));
    }

    fn insert_video_frame(&mut self, key: VideoFrameCacheKey, img: LinearImage) {
        if !self.video_frame_cache.contains_key(&key)
            && self.video_frame_cache.len() >= MAX_VIDEO_FRAME_CACHE
            && let Some(oldest_key) = self
                .video_frame_cache
                .iter()
                .min_by_key(|(_, (tick, _))| *tick)
                .map(|(k, _)| *k)
        {
            self.video_frame_cache.remove(&oldest_key);
        }
        self.video_frame_tick = self.video_frame_tick.wrapping_add(1);
        self.video_frame_cache
            .insert(key, (self.video_frame_tick, img));
    }

    fn resolve_asset_path(
        &self,
        project: &Project,
        asset_id: Uuid,
        allow_proxy: bool,
    ) -> Option<(PathBuf, bool)> {
        if allow_proxy
            && self.use_proxies
            && let Some(proxy) = self.proxy_paths.get(&asset_id)
            && proxy.exists()
        {
            return Some((proxy.clone(), true));
        }
        let asset = project.asset(asset_id)?;
        let resolved = if let Some(folder) = &self.project_folder {
            match folder.resolve_asset(asset) {
                AssetResolution::Found(p) => Some(p),
                AssetResolution::Offline => None,
            }
        } else {
            asset
                .fallback_absolute_path
                .as_ref()
                .filter(|p| p.exists())
                .cloned()
        };
        resolved.map(|p| (p, false))
    }

    pub fn with_disk_cache(mut self, manager: PrecompCacheManager) -> Self {
        self.disk_precomp_cache = Some(manager);
        self
    }

    pub fn register_waveform(&mut self, asset_id: Uuid, peaks: WaveformPeaks) {
        self.asset_waveforms.insert(asset_id, peaks);
    }

    pub fn render_scene(&mut self, scene: &Scene, time: RationalTime) -> LinearImage {
        self.render_scene_inner(None, scene, time, false, 0)
    }

    /// 透過書き出し（WebM VP9+Alpha / PNG連番）向け: 背景を透明にしたままレンダリング
    pub fn render_scene_transparent(&mut self, scene: &Scene, time: RationalTime) -> LinearImage {
        self.render_scene_inner(None, scene, time, true, 0)
    }

    /// プロジェクト全体のシーン参照（Smart Precompose 再帰評価＋ハッシュキャッシュ）付きレンダリング
    pub fn render_project_scene(
        &mut self,
        project: &Project,
        scene_id: SceneId,
        time: RationalTime,
        transparent_bg: bool,
    ) -> Option<LinearImage> {
        let scene = project.scene(scene_id)?;
        Some(self.render_scene_inner(Some(project), scene, time, transparent_bg, 0))
    }

    /// v1.0 #18: SSD 永続フレームキャッシュ（`cache/frames/`）を優先参照して RGBA8 フレームを取得する。
    /// 同一シーン状態・同一時刻であればアプリ再起動後もデコード＆合成をスキップして即座に返す。
    pub fn render_project_scene_cached_rgba8(
        &mut self,
        project: &Project,
        scene_id: SceneId,
        time: RationalTime,
        target_w: u32,
        target_h: u32,
        transparent_bg: bool,
    ) -> Option<Vec<u8>> {
        let scene = project.scene(scene_id)?;
        let tw = target_w.max(1);
        let th = target_h.max(1);
        let frame_idx = time.to_frame_floor(scene.fps);

        let cache_key = if self.disk_frame_cache.is_some() {
            let sig = if self.active_revision > 0 {
                let sig_key: SceneSigCacheKey = (
                    scene_id,
                    self.active_revision,
                    transparent_bg,
                    self.use_proxies,
                    self.high_quality_motion_blur,
                );
                if let Some((cached_k, cached_s)) = self.cached_scene_sig.as_ref()
                    && *cached_k == sig_key
                {
                    cached_s.clone()
                } else {
                    let scene_json = serde_json::to_vec(scene).unwrap_or_default();
                    let computed = format!(
                        "{}|t={transparent_bg}|p={}|mb={}",
                        content_hash_hex(&scene_json),
                        self.use_proxies,
                        self.high_quality_motion_blur
                    );
                    self.cached_scene_sig = Some((sig_key, computed.clone()));
                    computed
                }
            } else {
                let scene_json = serde_json::to_vec(scene).unwrap_or_default();
                format!(
                    "{}|t={transparent_bg}|p={}|mb={}",
                    content_hash_hex(&scene_json),
                    self.use_proxies,
                    self.high_quality_motion_blur
                )
            };
            Some(PersistentFrameCache::frame_key(&sig, tw, th, frame_idx))
        } else {
            None
        };

        if let (Some(key), Some(fc)) = (cache_key.as_deref(), self.disk_frame_cache.as_mut())
            && let Ok(Some((cw, ch, rgba))) = fc.get_rgba8(key)
            && cw == tw
            && ch == th
        {
            self.cache_hits += 1;
            return Some(rgba);
        }

        let img =
            self.render_project_scene_scaled(project, scene_id, time, tw, th, transparent_bg)?;
        let rgba = img.to_rgba8_srgb();
        if let (Some(key), Some(fc)) = (cache_key.as_deref(), self.disk_frame_cache.as_mut()) {
            self.cache_misses += 1;
            let _ = fc.put_rgba8(key, tw, th, &rgba);
        }
        Some(rgba)
    }

    /// プレビュー用の任意解像度 `(target_w, target_h)` へシーン全体の座標・文字サイズ・図形サイズを
    /// 正確にスケーリングしてレンダリングする（ギズモ座標と描画ピクセルを 1:1 一致させる）。
    pub fn render_project_scene_scaled(
        &mut self,
        project: &Project,
        scene_id: SceneId,
        time: RationalTime,
        target_w: u32,
        target_h: u32,
        transparent_bg: bool,
    ) -> Option<LinearImage> {
        let orig_scene = project.scene(scene_id)?;
        let tw = target_w.max(1);
        let th = target_h.max(1);
        if orig_scene.width == tw && orig_scene.height == th {
            return Some(self.render_scene_inner(
                Some(project),
                orig_scene,
                time,
                transparent_bg,
                0,
            ));
        }

        let mut scaled = orig_scene.clone();
        let sx = tw as f32 / orig_scene.width.max(1) as f32;
        let sy = th as f32 / orig_scene.height.max(1) as f32;
        let uniform = sx.min(sy);
        scaled.width = tw;
        scaled.height = th;
        for layer in &mut scaled.layers {
            layer.offset_x *= sx;
            layer.offset_y *= sy;
            for item in &mut layer.items {
                item.transform.x *= sx;
                item.transform.y *= sy;
                if let Some(kfs) = item.keyframes.get_mut("x") {
                    for kf in kfs {
                        kf.value *= sx;
                    }
                }
                if let Some(kfs) = item.keyframes.get_mut("y") {
                    for kf in kfs {
                        kf.value *= sy;
                    }
                }
                item.audio_reactive.y_bounce_px *= sy;
                for m in &mut item.masks {
                    m.feather_px *= uniform;
                }
                match &mut item.kind {
                    ItemKind::Shape { width, height, .. }
                    | ItemKind::CharacterSprite { width, height, .. }
                    | ItemKind::AudioSpectrum { width, height, .. } => {
                        *width = (*width * sx).max(1.0);
                        *height = (*height * sy).max(1.0);
                    }
                    ItemKind::VectorShape { config } => {
                        config.width = (config.width * sx).max(4.0);
                        config.height = (config.height * sy).max(4.0);
                        config.stroke_width *= uniform;
                        config.dash_length *= uniform;
                        config.dash_gap *= uniform;
                    }
                    ItemKind::Mesh3D { config } => {
                        config.width = (config.width * sx).max(16.0);
                        config.height = (config.height * sy).max(16.0);
                    }
                    ItemKind::GenerativeBg { config } => {
                        config.width = (config.width * sx).max(16.0);
                        config.height = (config.height * sy).max(16.0);
                    }
                    ItemKind::RiggedPuppet { config } => {
                        config.width = (config.width * sx).max(32.0);
                        config.height = (config.height * sy).max(32.0);
                    }
                    ItemKind::DataChart { config } => {
                        config.width = (config.width * sx).max(64.0);
                        config.height = (config.height * sy).max(48.0);
                    }
                    ItemKind::SvgMorph { config } => {
                        config.width = (config.width * sx).max(16.0);
                        config.height = (config.height * sy).max(16.0);
                        config.stroke_width *= uniform;
                    }
                    ItemKind::VtuberAvatar { config } => {
                        config.width = (config.width * sx).max(32.0);
                        config.height = (config.height * sy).max(32.0);
                    }
                    ItemKind::FluidSim { config } => {
                        config.width = (config.width * sx).max(32.0);
                        config.height = (config.height * sy).max(32.0);
                    }
                    ItemKind::ClothSim { config } => {
                        config.width = (config.width * sx).max(32.0);
                        config.height = (config.height * sy).max(32.0);
                    }
                    ItemKind::Text { font_size, .. } => {
                        *font_size = (*font_size * uniform).max(4.0);
                        item.text_style.outline_width *= uniform;
                        item.text_style.shadow_offset[0] *= sx;
                        item.text_style.shadow_offset[1] *= sy;
                        item.text_style.shadow_blur *= uniform;
                        item.text_style.box_padding[0] *= sx;
                        item.text_style.box_padding[1] *= sy;
                        item.text_style.box_radius *= uniform;
                        if item.text_on_path.enabled {
                            item.text_on_path.letter_spacing_px *= uniform;
                            item.text_on_path.vertical_offset_px *= uniform;
                        }
                    }
                    _ => {}
                }
                if item.vfx_paint.enabled {
                    for stroke in &mut item.vfx_paint.strokes {
                        stroke.radius_px = (stroke.radius_px * uniform).max(1.0);
                    }
                }
                if item.roto_brush.enabled {
                    for kf in &mut item.roto_brush.keyframes {
                        kf.feather_px *= uniform;
                        kf.expansion_px *= uniform;
                    }
                }
                if item.warp_displace.enabled {
                    item.warp_displace.luma_displace_px *= uniform;
                }
            }
        }
        if scaled.optical_vfx.bokeh_enabled {
            scaled.optical_vfx.bokeh_radius_px =
                (scaled.optical_vfx.bokeh_radius_px * uniform).max(0.5);
        }
        Some(self.render_scene_inner(Some(project), &scaled, time, transparent_bg, 0))
    }

    fn render_scene_inner(
        &mut self,
        project: Option<&Project>,
        scene: &Scene,
        time: RationalTime,
        transparent_bg: bool,
        depth: u8,
    ) -> LinearImage {
        let mut canvas =
            if depth == 0 && scene.motion_blur.enabled && scene.motion_blur.shutter_angle_deg > 0.5
            {
                let samples = if self.high_quality_motion_blur {
                    scene.motion_blur.export_samples
                } else {
                    scene.motion_blur.preview_samples
                }
                .clamp(1, 16);
                if samples > 1 {
                    let frame_dur = 1.0 / scene.fps.as_f64().max(1.0);
                    let shutter_dur =
                        frame_dur * (f64::from(scene.motion_blur.shutter_angle_deg) / 360.0);
                    let phase_offset =
                        frame_dur * (f64::from(scene.motion_blur.shutter_phase_deg) / 360.0);
                    let base_sec = time.as_seconds();
                    let mut acc: Option<LinearImage> = None;
                    let inv = 1.0 / samples as f32;
                    for s in 0..samples {
                        let frac = (s as f64 + 0.5) / samples as f64;
                        let sub_sec = (base_sec + phase_offset + shutter_dur * frac).max(0.0);
                        let sub_time = RationalTime::from_seconds(sub_sec);
                        let sub_img = self.render_scene_single_frame(
                            project,
                            scene,
                            sub_time,
                            transparent_bg,
                            depth,
                        );
                        match acc.as_mut() {
                            None => {
                                let mut first = sub_img;
                                for p in &mut first.pixels {
                                    p.r *= inv;
                                    p.g *= inv;
                                    p.b *= inv;
                                    p.a *= inv;
                                }
                                acc = Some(first);
                            }
                            Some(dst) => {
                                for (dp, sp) in dst.pixels.iter_mut().zip(sub_img.pixels.iter()) {
                                    dp.r += sp.r * inv;
                                    dp.g += sp.g * inv;
                                    dp.b += sp.b * inv;
                                    dp.a += sp.a * inv;
                                }
                            }
                        }
                    }
                    acc.unwrap_or_else(|| {
                        self.render_scene_single_frame(project, scene, time, transparent_bg, depth)
                    })
                } else {
                    self.render_scene_single_frame(project, scene, time, transparent_bg, depth)
                }
            } else {
                self.render_scene_single_frame(project, scene, time, transparent_bg, depth)
            };

        // v0.8 #13: シーン全体の非破壊カラーグレーディング（3-Way + .cube 3D LUT）
        if scene.color_grade.enabled {
            let lut = scene
                .color_grade
                .lut_path
                .as_deref()
                .and_then(|p| self.resolve_lut(p));
            apply_scene_color_grade(&mut canvas, &scene.color_grade, lut.as_deref());
        }

        // v4.0 #33: ACEScg / OpenColorIO プロカラー管理（IDT → Working → Gamut Compress → Tone Map → ODT）
        if scene.color_management.enabled {
            apply_scene_color_management(&mut canvas, &scene.color_management);
        }

        // v8.0 #51: シネマ光学 VFX（擬似深度ボケ Rack Focus・ゴッドレイ・Brown-Conrady レンズ歪み＆物理色収差）
        if scene.optical_vfx.is_active() {
            crate::v80_render::apply_cinema_optical_vfx(&mut canvas, &scene.optical_vfx);
        }

        // v5.0 #40: シネマフィニッシング（35mm ハレーション・アナモルフィックフレア・グレイン・レターボックス・バーンイン）
        if scene.cinema_finishing.enabled {
            crate::finishing_render::apply_cinema_finishing(
                &mut canvas,
                &scene.cinema_finishing,
                time,
                scene.fps,
            );
        }

        // v8.0 #52: ADR カウント収録 3-2-1 映像ストリーマ
        if depth == 0
            && let Some(adr_state) = scene.adr_session.evaluate_countdown_at(time)
        {
            crate::v80_render::draw_adr_countdown_streamer(&mut canvas, &adr_state);
        }

        // v9.0 #53: ライブスイッチャー PGM/PVW タリー＆REC HUD
        if depth == 0 && scene.live_switcher.enabled && scene.live_switcher.show_tally_hud {
            crate::v90_render::draw_live_tally_hud(&mut canvas, &scene.live_switcher);
        }

        // v10.0 #57: 360° 全天球投影（OverCapture / Tiny Planet / 水平ロック）＆ ステレオスコピック空間 3D パス
        if depth == 0 && scene.spatial_video.enabled {
            canvas = crate::v100_render::render_spatial_360_and_stereo_pass(
                &canvas,
                &scene.spatial_video,
                time.as_seconds() as f32,
            );
        }

        canvas
    }

    fn render_scene_single_frame(
        &mut self,
        project: Option<&Project>,
        scene: &Scene,
        time: RationalTime,
        transparent_bg: bool,
        depth: u8,
    ) -> LinearImage {
        let w = scene.width.max(1);
        let h = scene.height.max(1);
        let mut layer_outputs: Vec<Option<(LinearImage, BlendMode, f32)>> =
            vec![None; scene.layers.len()];
        let mut filter_passes: Vec<(usize, String, FilterTargetScope, BTreeMap<String, f32>)> =
            Vec::new();
        let mut transition_passes: Vec<(usize, TransitionKind, FilterTargetScope, f32, f32)> =
            Vec::new();
        let mut group_passes: Vec<(usize, usize, Transform2D)> = Vec::new();

        // v7.0 #48: 大規模タイムライン（64+ クリップ）では $O(\log N)$ 区間木で現在時刻のアクティブクリップのみを抽出
        let total_scene_items: usize = scene.layers.iter().map(|l| l.items.len()).sum();
        let prefiltered_active_per_layer: Option<Vec<Vec<usize>>> =
            if depth == 0 && total_scene_items >= 64 && self.active_revision > 0 {
                let idx = self
                    .interval_index
                    .get_or_build(scene, self.active_revision);
                Some(
                    (0..scene.layers.len())
                        .map(|l_i| idx.query_layer_indices_at(l_i, time))
                        .collect(),
                )
            } else {
                None
            };

        // Step 1: 各レイヤーのアクティブオブジェクトを描画し、個別エフェクト・3Dカメラ透視投影/DoFを適用
        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            if !layer.visible {
                continue;
            }
            let mut layer_buf: Option<LinearImage> = None;
            let mut layer_blend = BlendMode::Normal;
            let mut layer_depth = 1000.0_f32;

            let active_indices: Vec<usize> = match prefiltered_active_per_layer.as_ref() {
                Some(per_layer) => per_layer[layer_idx].clone(),
                None => layer
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, it)| it.contains(time).then_some(idx))
                    .collect(),
            };

            for item_idx in active_indices {
                let item = &layer.items[item_idx];
                let expr_audio_bands = if item_expressions_use_audio(item) {
                    let audio_layer = if item.audio_reactive.enabled {
                        item.audio_reactive.linked_audio_layer
                    } else {
                        scene
                            .layers
                            .iter()
                            .enumerate()
                            .find_map(|(idx, l)| {
                                l.items
                                    .iter()
                                    .any(|it| matches!(it.kind, ItemKind::Audio { .. }))
                                    .then_some(idx)
                            })
                            .unwrap_or(0)
                    };
                    let (_, low, mid, high, rms) =
                        self.evaluate_audio_reactive_bands(scene, audio_layer, time, 16);
                    [low, mid, high, rms]
                } else {
                    [0.0; 4]
                };

                let (mut tf, tf3d) =
                    evaluate_item_transform(scene, layer_idx, item, time, expr_audio_bands);
                if item.audio_reactive.enabled {
                    let (_, low, mid, high, rms) = self.evaluate_audio_reactive_bands(
                        scene,
                        item.audio_reactive.linked_audio_layer,
                        time,
                        16,
                    );
                    let raw_energy = match item.audio_reactive.band {
                        AudioReactiveBand::Rms => rms,
                        AudioReactiveBand::Low => low,
                        AudioReactiveBand::Mid => mid,
                        AudioReactiveBand::High => high,
                    };
                    let energy =
                        (raw_energy * item.audio_reactive.sensitivity.max(0.0)).clamp(0.0, 1.5);
                    tf.scale_x =
                        (tf.scale_x + item.audio_reactive.scale_amount * energy).clamp(0.05, 8.0);
                    tf.scale_y =
                        (tf.scale_y + item.audio_reactive.scale_amount * energy).clamp(0.05, 8.0);
                    tf.y += item.audio_reactive.y_bounce_px * energy;
                    tf.opacity =
                        (tf.opacity + item.audio_reactive.opacity_amount * energy).clamp(0.0, 1.0);
                }
                match &item.kind {
                    ItemKind::FilterObject {
                        effect_type,
                        scope,
                        params,
                    } => {
                        filter_passes.push((
                            layer_idx,
                            effect_type.clone(),
                            *scope,
                            params.clone(),
                        ));
                    }
                    ItemKind::Transition {
                        transition,
                        scope,
                        softness,
                    } => {
                        let dur_s = item.duration.as_seconds().max(1e-4);
                        let progress =
                            ((time - item.start).as_seconds() / dur_s).clamp(0.0, 1.0) as f32;
                        transition_passes.push((
                            layer_idx,
                            *transition,
                            *scope,
                            progress,
                            *softness,
                        ));
                    }
                    ItemKind::GroupControl { target_layer_count } => {
                        group_passes.push((layer_idx, *target_layer_count, tf));
                    }
                    _ => {
                        if let Some(mut item_img) = self.render_item(
                            project,
                            scene,
                            layer_idx,
                            item,
                            &tf,
                            expr_audio_bands,
                            time,
                            w,
                            h,
                            depth,
                        ) {
                            apply_effect_chain(
                                &mut item_img,
                                &item.effect_chain,
                                time.as_seconds(),
                                &self.plugins,
                            );
                            if !item.masks.is_empty() {
                                crate::mask_tracker::apply_bezier_masks(&mut item_img, &item.masks);
                            }
                            if let Some(roto_mask) =
                                item.roto_brush.evaluate_mask_at(time - item.start)
                            {
                                crate::mask_tracker::apply_bezier_masks(
                                    &mut item_img,
                                    std::slice::from_ref(&roto_mask),
                                );
                            }
                            if item.vfx_paint.enabled {
                                let dur_s = item.duration.as_seconds().max(1e-4);
                                let progress = ((time - item.start).as_seconds() / dur_s)
                                    .clamp(0.0, 1.0)
                                    as f32;
                                crate::v60_render::apply_vfx_paint(
                                    &mut item_img,
                                    &item.vfx_paint,
                                    progress,
                                );
                            }
                            if item.warp_displace.enabled {
                                item_img = crate::v80_render::apply_warp_and_displacement(
                                    &item_img,
                                    &item.warp_displace,
                                    (time - item.start).as_seconds() as f32,
                                );
                            }
                            let eff_corner_pin = item.eval_corner_pin(time);
                            if eff_corner_pin.enabled {
                                item_img =
                                    crate::vfx_track::apply_corner_pin(&item_img, &eff_corner_pin);
                            }

                            let (mut draw_x, mut draw_y) =
                                (tf.x + layer.offset_x, tf.y + layer.offset_y);
                            if tf.rotation_deg.abs() > 0.05 {
                                let old_w = item_img.width as f32;
                                let old_h = item_img.height as f32;
                                item_img = rotate_linear_image(&item_img, tf.rotation_deg);
                                draw_x += (old_w - item_img.width as f32) * 0.5;
                                draw_y += (old_h - item_img.height as f32) * 0.5;
                            }
                            let mut shadow_to_draw: Option<(LinearImage, f32, f32)> = None;
                            if scene.camera.enabled && tf3d.enable_3d {
                                let cx = w as f32 * 0.5;
                                let cy = h as f32 * 0.5;
                                let item_cx = draw_x + item_img.width as f32 * 0.5 - cx;
                                let item_cy = draw_y + item_img.height as f32 * 0.5 - cy;
                                let Some(proj) = scene.camera.project(item_cx, item_cy, tf3d.z, h)
                                else {
                                    continue;
                                };
                                layer_depth = proj.camera_depth;
                                let yaw_scale =
                                    tf3d.rotation_y_deg.to_radians().cos().abs().max(0.05);
                                let pitch_scale =
                                    tf3d.rotation_x_deg.to_radians().cos().abs().max(0.05);
                                let scaled_w =
                                    ((item_img.width as f32) * proj.perspective_scale * yaw_scale)
                                        .round()
                                        .clamp(1.0, (w * 4) as f32)
                                        as u32;
                                let scaled_h = ((item_img.height as f32)
                                    * proj.perspective_scale
                                    * pitch_scale)
                                    .round()
                                    .clamp(1.0, (h * 4) as f32)
                                    as u32;
                                item_img = scale_linear_image(&item_img, scaled_w, scaled_h);
                                if proj.dof_blur_radius > 0 {
                                    box_blur_separable(&mut item_img, proj.dof_blur_radius);
                                }
                                draw_x = cx + proj.screen_offset_x - item_img.width as f32 * 0.5;
                                draw_y = cy + proj.screen_offset_y - item_img.height as f32 * 0.5;

                                if scene.camera.light.enabled {
                                    let (light_tint, shadow_off) =
                                        scene.camera.compute_layer_lighting(
                                            [item_cx, item_cy, tf3d.z],
                                            tf3d.rotation_x_deg,
                                            tf3d.rotation_y_deg,
                                        );
                                    if scene.camera.light.cast_shadow {
                                        let blur_r = (scene.camera.light.shadow_softness
                                            * proj.perspective_scale)
                                            .round()
                                            .clamp(0.0, 24.0)
                                            as i32;
                                        let shadow_img = build_drop_shadow_image(
                                            &item_img,
                                            scene.camera.light.shadow_opacity,
                                            blur_r,
                                        );
                                        shadow_to_draw = Some((
                                            shadow_img,
                                            draw_x + shadow_off[0] * proj.perspective_scale,
                                            draw_y + shadow_off[1] * proj.perspective_scale,
                                        ));
                                    }
                                    apply_3d_lighting_tint(&mut item_img, light_tint);
                                }
                            }

                            let dst = layer_buf.get_or_insert_with(|| {
                                self.frame_pool.acquire_linear_canvas(
                                    w,
                                    h,
                                    LinearPremul::TRANSPARENT,
                                )
                            });
                            if let Some((shadow_img, sx_pos, sy_pos)) = shadow_to_draw {
                                dst.blend_with_mode(
                                    &shadow_img,
                                    sx_pos.round() as i32,
                                    sy_pos.round() as i32,
                                    tf.opacity,
                                    BlendMode::Normal,
                                );
                            }
                            dst.blend_with_mode(
                                &item_img,
                                draw_x.round() as i32,
                                draw_y.round() as i32,
                                tf.opacity,
                                item.blend_mode,
                            );
                            layer_blend = item.blend_mode;
                        }
                    }
                }
            }

            if let Some(buf) = layer_buf {
                layer_outputs[layer_idx] = Some((buf, layer_blend, layer_depth));
            }
        }

        // v2.0 #21: ノード合成空間（NodeCompositeGraph）が有効な場合はトポロジカル順にノード評価を実行
        if scene.node_graph.enabled && !scene.node_graph.nodes.is_empty() {
            let packets: Vec<Option<crate::node_eval::LayerRenderPacket>> = layer_outputs
                .iter()
                .map(|opt| {
                    opt.as_ref()
                        .map(|(img, _, depth)| crate::node_eval::LayerRenderPacket {
                            image: img.clone(),
                            depth: *depth,
                        })
                })
                .collect();
            if let Some(node_canvas) = crate::node_eval::evaluate_node_composite_graph(
                &scene.node_graph,
                &packets,
                w,
                h,
                time.as_seconds(),
                &self.plugins,
                transparent_bg,
            ) {
                for (buf, _, _) in layer_outputs.into_iter().flatten() {
                    self.frame_pool.release_linear_canvas(buf);
                }
                return node_canvas;
            }
        }

        // トラックマット適用（split_at_mut でフルフレームクローンを回避）
        let mut consumed_matte_layers = vec![false; scene.layers.len()];
        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            if let Some(matte) = layer.track_matte
                && matte.source_layer_index < scene.layers.len()
                && matte.source_layer_index != layer_idx
            {
                let src_idx = matte.source_layer_index;
                if src_idx < layer_idx {
                    let (left, right) = layer_outputs.split_at_mut(layer_idx);
                    if let (Some((matte_img, _, _)), Some((target_img, _, _))) =
                        (left[src_idx].as_ref(), right[0].as_mut())
                    {
                        target_img.apply_track_matte(matte_img, matte.mode);
                        consumed_matte_layers[src_idx] = true;
                    }
                } else {
                    let (left, right) = layer_outputs.split_at_mut(src_idx);
                    if let (Some((target_img, _, _)), Some((matte_img, _, _))) =
                        (left[layer_idx].as_mut(), right[0].as_ref())
                    {
                        target_img.apply_track_matte(matte_img, matte.mode);
                        consumed_matte_layers[src_idx] = true;
                    }
                }
            }
        }

        // グループ制御適用
        for (group_layer_idx, target_count, tf) in &group_passes {
            let start = group_layer_idx.saturating_sub(*target_count);
            for target in &mut layer_outputs[start..*group_layer_idx] {
                if let Some((img, _, _)) = target.as_mut() {
                    let transformed = apply_group_transform(img, *tf);
                    *img = transformed;
                }
            }
        }

        // AviUtl2風フィルタオブジェクト（指定レイヤー範囲へ適用）
        for (filter_layer_idx, effect_type, scope, params) in &filter_passes {
            for target_idx in resolve_scope_layers(*filter_layer_idx, *scope) {
                if let Some((img, _, _)) = layer_outputs[target_idx].as_mut() {
                    apply_named_effect(img, effect_type, params, time.as_seconds(), &self.plugins);
                }
            }
        }

        // AviUtl2風シーンチェンジ（トランジションオブジェクト）適用
        for &(tr_layer_idx, transition, scope, progress, softness) in &transition_passes {
            for target_idx in resolve_scope_layers(tr_layer_idx, scope) {
                if let Some((img, _, _)) = layer_outputs[target_idx].as_mut() {
                    apply_transition(img, transition, progress, softness);
                }
            }
        }

        // 最終レイヤー合成（3Dカメラ有効時は奥から手前へ Z ソート）＋ 使用済みレイヤーバッファのプール返却
        let bg = if transparent_bg {
            LinearPremul::TRANSPARENT
        } else {
            LinearPremul::from_srgb_straight(RgbaColor::BLACK)
        };
        let mut canvas = LinearImage::new(w, h, bg);
        let mut draw_list: Vec<(usize, LinearImage, BlendMode, f32)> = Vec::new();
        for (idx, opt) in layer_outputs.into_iter().enumerate() {
            if let Some((img, bm, d)) = opt {
                if consumed_matte_layers[idx] {
                    self.frame_pool.release_linear_canvas(img);
                } else {
                    draw_list.push((idx, img, bm, d));
                }
            }
        }

        if scene.camera.enabled {
            // 奥（camera_depth が大きい）から手前（小さい）の順に描画
            draw_list.sort_by(|a, b| {
                b.3.partial_cmp(&a.3)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });
        }

        for (_, layer_img, blend_mode, _) in draw_list {
            canvas.blend_with_mode(&layer_img, 0, 0, 1.0, blend_mode);
            self.frame_pool.release_linear_canvas(layer_img);
        }

        canvas
    }

    #[allow(clippy::too_many_arguments)]
    fn render_item(
        &mut self,
        project: Option<&Project>,
        scene: &Scene,
        layer_idx: usize,
        item: &TimelineItem,
        tf: &Transform2D,
        expr_audio_bands: [f32; 4],
        global_time: RationalTime,
        canvas_w: u32,
        canvas_h: u32,
        depth: u8,
    ) -> Option<LinearImage> {
        let local_time = global_time - item.start;
        match &item.kind {
            ItemKind::Shape {
                shape,
                width,
                height,
                color,
            } => {
                let sw = (*width * tf.scale_x).round().max(1.0) as u32;
                let sh = (*height * tf.scale_y).round().max(1.0) as u32;
                let fill = LinearPremul::from_srgb_straight(*color);
                let mut img = LinearImage::new(sw, sh, LinearPremul::TRANSPARENT);
                match shape {
                    ShapeKind::Rectangle => {
                        img.pixels.fill(fill);
                    }
                    ShapeKind::RoundedRectangle => {
                        let radius = (sw.min(sh) as f32 * 0.14).clamp(4.0, 48.0);
                        let cx = sw as f32 * 0.5;
                        let cy = sh as f32 * 0.5;
                        let hx = (cx - radius).max(0.0);
                        let hy = (cy - radius).max(0.0);
                        for y in 0..sh {
                            let fy = y as f32 + 0.5;
                            for x in 0..sw {
                                let fx = x as f32 + 0.5;
                                let qx = (fx - cx).abs() - hx;
                                let qy = (fy - cy).abs() - hy;
                                let dist =
                                    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
                                let cov = (0.5 - dist).clamp(0.0, 1.0);
                                if cov > 0.0 {
                                    img.pixels[(y * sw + x) as usize] = LinearPremul {
                                        r: fill.r * cov,
                                        g: fill.g * cov,
                                        b: fill.b * cov,
                                        a: fill.a * cov,
                                    };
                                }
                            }
                        }
                    }
                    ShapeKind::Ellipse => {
                        let rx = sw as f32 * 0.5;
                        let ry = sh as f32 * 0.5;
                        for y in 0..sh {
                            for x in 0..sw {
                                let nx = (x as f32 + 0.5 - rx) / rx;
                                let ny = (y as f32 + 0.5 - ry) / ry;
                                if nx * nx + ny * ny <= 1.0 {
                                    img.pixels[(y * sw + x) as usize] = fill;
                                }
                            }
                        }
                    }
                }
                Some(img)
            }
            ItemKind::Text {
                content,
                font_family,
                font_size,
                color,
                animator,
            } => {
                let default_palette = ColorPalette::default();
                let palette = project
                    .map(|p| &p.color_palette)
                    .unwrap_or(&default_palette);
                let scaled_font_size = *font_size * tf.scale_y.max(0.1);
                if item.text_on_path.enabled {
                    return Some(crate::v80_render::render_text_on_path(
                        content,
                        font_family,
                        scaled_font_size,
                        *color,
                        &item.text_on_path,
                        &item.text_style,
                        palette,
                        &mut self.text_renderer,
                        canvas_w,
                        canvas_h,
                        local_time.as_seconds() as f32,
                    ));
                }
                let kinetic_buf;
                let effective_content: &str = if item.kinetic_subtitle.enabled {
                    let dur_s = item.duration.as_seconds().max(1e-4);
                    let progress = (local_time.as_seconds() / dur_s).clamp(0.0, 1.0) as f32;
                    kinetic_buf = ve_core::generate_kinetic_inline_markup(
                        content,
                        progress,
                        scaled_font_size,
                        &item.kinetic_subtitle,
                    );
                    &kinetic_buf
                } else {
                    content
                };
                Some(self.text_renderer.render_text_styled(
                    effective_content,
                    font_family,
                    scaled_font_size,
                    *color,
                    animator,
                    &item.text_style,
                    palette,
                    local_time.as_seconds() as f32,
                    canvas_w,
                    canvas_h,
                ))
            }
            ItemKind::CharacterSprite {
                config,
                width,
                height,
            } => {
                let mouth = self.evaluate_character_mouth(scene, config, global_time);
                let sw = (*width * tf.scale_x).round().max(16.0) as u32;
                let sh = (*height * tf.scale_y).round().max(16.0) as u32;

                // Phase 4: PSD ネイティブ立ち絵（目パチ・口パク差分レイヤー直接合成）
                if let Some(psd_cfg) = &config.psd_config
                    && !psd_cfg.psd_path.as_os_str().is_empty()
                {
                    if !self.psd_cache.contains_key(&psd_cfg.psd_path)
                        && let Ok(loaded) = PsdCharacterAsset::from_file(&psd_cfg.psd_path)
                    {
                        if self.psd_cache.len() >= MAX_PSD_CACHE_ENTRIES {
                            self.psd_cache.clear();
                        }
                        self.psd_cache.insert(psd_cfg.psd_path.clone(), loaded);
                    }
                    if let Some(psd_asset) = self.psd_cache.get(&psd_cfg.psd_path) {
                        let eye_closed = psd_cfg.is_eye_closed_at(global_time.as_seconds());
                        let composited = psd_asset.composite(psd_cfg, mouth, eye_closed);
                        return Some(scale_linear_image(&composited, sw, sh));
                    }
                }

                // Phase 3: PNG 差分方式
                if let Some(png_path) = resolve_mouth_png(&config.png_paths, mouth) {
                    if !self.png_cache.contains_key(png_path)
                        && let Some(loaded) = load_png_linear(png_path)
                    {
                        if self.png_cache.len() >= MAX_PNG_CACHE_ENTRIES {
                            self.png_cache.clear();
                        }
                        self.png_cache.insert(png_path.clone(), loaded);
                    }
                    if let Some(cached) = self.png_cache.get(png_path) {
                        return Some(scale_linear_image(cached, sw, sh));
                    }
                }
                Some(render_placeholder_character(sw, sh, mouth))
            }
            ItemKind::Precomp { scene_id } => {
                if depth >= 4 {
                    return None;
                }
                let proj = project?;
                let nested = proj.scene(*scene_id)?;
                let mut scaled_nested = nested.clone();
                scaled_nested.width = canvas_w;
                scaled_nested.height = canvas_h;

                let local_frame = local_time.to_frame_floor(scaled_nested.fps);
                let key_payload = format!(
                    "{}|{}x{}|f{}",
                    serde_json::to_string(&scaled_nested).unwrap_or_default(),
                    canvas_w,
                    canvas_h,
                    local_frame
                );
                let cache_key = content_hash_hex(key_payload.as_bytes());

                if let Some((tick, cached)) = self.mem_precomp_cache.get_mut(&cache_key) {
                    self.mem_cache_tick = self.mem_cache_tick.wrapping_add(1);
                    *tick = self.mem_cache_tick;
                    self.cache_hits += 1;
                    return Some(cached.clone());
                }
                if let Some(disk) = self.disk_precomp_cache.as_mut()
                    && let Ok(Some(bytes)) = disk.get(&cache_key)
                    && let Some(img) = LinearImage::from_rgba16f_bytes(canvas_w, canvas_h, &bytes)
                {
                    self.cache_hits += 1;
                    self.insert_mem_precomp(cache_key, img.clone());
                    return Some(img);
                }

                self.cache_misses += 1;
                let rendered =
                    self.render_scene_inner(project, &scaled_nested, local_time, true, depth + 1);
                if let Some(disk) = self.disk_precomp_cache.as_mut() {
                    let _ = disk.put(&cache_key, &rendered.to_rgba16f_bytes());
                }
                self.insert_mem_precomp(cache_key, rendered.clone());
                Some(rendered)
            }
            ItemKind::Image { asset_id } => {
                let proj = project?;
                let (path, _) = self.resolve_asset_path(proj, *asset_id, false)?;
                if !self.png_cache.contains_key(&path) {
                    let loaded = load_png_linear(&path).or_else(|| {
                        let (w, h, rgba) = self
                            .ffmpeg
                            .decode_image_rgba8(&path, canvas_w.max(1920), canvas_h.max(1080))
                            .ok()?;
                        Some(LinearImage::from_rgba8_srgb(w, h, &rgba))
                    });
                    if let Some(img) = loaded {
                        if self.png_cache.len() >= MAX_PNG_CACHE_ENTRIES {
                            self.png_cache.clear();
                        }
                        self.png_cache.insert(path.clone(), img);
                    }
                }
                let cached = self.png_cache.get(&path)?;
                let fit = (canvas_w as f32 / cached.width.max(1) as f32)
                    .min(canvas_h as f32 / cached.height.max(1) as f32)
                    .min(1.0);
                let sw = ((cached.width as f32) * fit * tf.scale_x.max(0.01))
                    .round()
                    .clamp(1.0, (canvas_w * 4) as f32) as u32;
                let sh = ((cached.height as f32) * fit * tf.scale_y.max(0.01))
                    .round()
                    .clamp(1.0, (canvas_h * 4) as f32) as u32;
                Some(scale_linear_image(cached, sw, sh))
            }
            ItemKind::Video {
                asset_id,
                source_offset,
                ..
            } => {
                let proj = project?;
                let (path, is_proxy) = self.resolve_asset_path(proj, *asset_id, true)?;
                let src_sec = item.evaluate_source_time_sec(local_time, *source_offset, None);
                let src_time = RationalTime::from_seconds(src_sec).max(RationalTime::ZERO);
                let tw = ((canvas_w as f32) * tf.scale_x.max(0.01))
                    .round()
                    .clamp(1.0, 4096.0) as u32;
                let th = ((canvas_h as f32) * tf.scale_y.max(0.01))
                    .round()
                    .clamp(1.0, 4096.0) as u32;

                // v2.0 #22: スロー再生時のオプティカルフロー中間フレーム補間
                if item.playback.optical_flow_interp && item.playback.speed.abs() < 0.999 {
                    let exact_f = src_time.as_seconds() * scene.fps.as_f64().max(1.0);
                    let f0 = exact_f.floor().max(0.0) as i64;
                    let frac = (exact_f - f0 as f64) as f32;
                    if frac > 0.04
                        && frac < 0.96
                        && let (Some(img_a), Some(img_b)) = (
                            self.decode_video_frame_at(
                                *asset_id, &path, is_proxy, scene.fps, f0, tw, th,
                            ),
                            self.decode_video_frame_at(
                                *asset_id,
                                &path,
                                is_proxy,
                                scene.fps,
                                f0 + 1,
                                tw,
                                th,
                            ),
                        )
                    {
                        return Some(crate::ai_vision::interpolate_frame_optical_flow(
                            &img_a,
                            &img_b,
                            frac,
                            &crate::ai_vision::OpticalFlowConfig::default(),
                        ));
                    }
                }

                let frame_idx = src_time.to_frame_floor(scene.fps).max(0);
                self.decode_video_frame_at(*asset_id, &path, is_proxy, scene.fps, frame_idx, tw, th)
            }
            ItemKind::AudioSpectrum {
                linked_audio_layer,
                width,
                height,
                bar_count,
                color,
                peak_color,
                sensitivity,
            } => {
                let sw = (*width * tf.scale_x).round().clamp(16.0, 4096.0) as u32;
                let sh = (*height * tf.scale_y).round().clamp(16.0, 4096.0) as u32;
                let bars = (*bar_count).clamp(4, 64);
                let (band_vals, _, _, _, _) = self.evaluate_audio_reactive_bands(
                    scene,
                    *linked_audio_layer,
                    global_time,
                    bars,
                );
                let sens = (*sensitivity).clamp(0.1, 8.0);
                let mut img = LinearImage::new(sw, sh, LinearPremul::TRANSPARENT);
                let base_lin = LinearPremul::from_srgb_straight(*color);
                let peak_lin = LinearPremul::from_srgb_straight(*peak_color);
                let step = sw as f32 / bars as f32;
                let gap = (step * 0.22).clamp(1.0, 8.0);
                let bar_w = (step - gap).max(1.0);

                for (i, &v) in band_vals.iter().enumerate() {
                    let level = (v * sens).clamp(0.0, 1.0);
                    let bar_h = ((sh as f32) * (0.04 + level * 0.94))
                        .round()
                        .clamp(2.0, sh as f32) as u32;
                    let x0 = (i as f32 * step + gap * 0.5).floor().max(0.0) as u32;
                    let x1 = ((x0 as f32 + bar_w).ceil() as u32).min(sw);
                    let y_top = sh.saturating_sub(bar_h);
                    let cap_h = (sh / 24).clamp(2, 6);

                    for y in y_top..sh {
                        let frac_up = 1.0 - (y - y_top) as f32 / bar_h.max(1) as f32;
                        let is_cap = y < y_top + cap_h && level > 0.08;
                        let px = if is_cap {
                            peak_lin
                        } else {
                            let mix = (frac_up * level).clamp(0.0, 1.0);
                            LinearPremul {
                                r: base_lin.r + (peak_lin.r - base_lin.r) * mix,
                                g: base_lin.g + (peak_lin.g - base_lin.g) * mix,
                                b: base_lin.b + (peak_lin.b - base_lin.b) * mix,
                                a: base_lin.a + (peak_lin.a - base_lin.a) * mix,
                            }
                        };
                        let row = y * sw;
                        for x in x0..x1 {
                            img.pixels[(row + x) as usize] = px;
                        }
                    }
                }
                Some(img)
            }
            ItemKind::ParticleEmitter { config } => Some(render_particle_emitter(
                config,
                local_time.as_seconds() as f32,
                canvas_w,
                canvas_h,
            )),
            ItemKind::VectorShape { config } => {
                let trim_start = evaluate_item_property(
                    scene,
                    layer_idx,
                    item,
                    "trim_start",
                    global_time,
                    expr_audio_bands,
                );
                let trim_end = evaluate_item_property(
                    scene,
                    layer_idx,
                    item,
                    "trim_end",
                    global_time,
                    expr_audio_bands,
                );
                let trim_offset = evaluate_item_property(
                    scene,
                    layer_idx,
                    item,
                    "trim_offset",
                    global_time,
                    expr_audio_bands,
                );
                Some(render_vector_shape(
                    config,
                    trim_start,
                    trim_end,
                    trim_offset,
                    tf.scale_x,
                    tf.scale_y,
                ))
            }
            ItemKind::Mesh3D { config } => {
                let sw = (config.width * tf.scale_x).round().clamp(16.0, 2048.0) as u32;
                let sh = (config.height * tf.scale_y).round().clamp(16.0, 2048.0) as u32;
                let base_dir = self.project_folder.as_ref().map(|f| f.root());
                Some(crate::mesh3d_render::render_mesh3d_item_with_adv(
                    config,
                    base_dir,
                    &scene.camera.light,
                    &scene.lighting_3d_adv,
                    local_time.as_seconds() as f32,
                    sw,
                    sh,
                ))
            }
            ItemKind::GenerativeBg { config } => {
                let sw = (config.width * tf.scale_x).round().clamp(16.0, 4096.0) as u32;
                let sh = (config.height * tf.scale_y).round().clamp(16.0, 4096.0) as u32;
                let t_sec = if item.speed_ramp.enabled {
                    ve_core::evaluate_speed_ramp_source_time(
                        local_time.as_seconds(),
                        item.duration.as_seconds(),
                        &item.speed_ramp,
                    ) as f32
                } else {
                    local_time.as_seconds() as f32
                };
                Some(crate::generative_render::render_generative_bg(
                    config, t_sec, sw, sh,
                ))
            }
            ItemKind::RiggedPuppet { config } => {
                let sw = (config.width * tf.scale_x).round().clamp(32.0, 4096.0) as u32;
                let sh = (config.height * tf.scale_y).round().clamp(32.0, 4096.0) as u32;
                let t_sec = if item.speed_ramp.enabled {
                    ve_core::evaluate_speed_ramp_source_time(
                        local_time.as_seconds(),
                        item.duration.as_seconds(),
                        &item.speed_ramp,
                    ) as f32
                } else {
                    local_time.as_seconds() as f32
                };
                Some(crate::rig_render::render_rigged_puppet(
                    config, t_sec, sw, sh,
                ))
            }
            ItemKind::DataChart { config } => {
                let sw = (config.width * tf.scale_x).round().clamp(64.0, 4096.0) as u32;
                let sh = (config.height * tf.scale_y).round().clamp(48.0, 4096.0) as u32;
                let t_sec = if item.speed_ramp.enabled {
                    ve_core::evaluate_speed_ramp_source_time(
                        local_time.as_seconds(),
                        item.duration.as_seconds(),
                        &item.speed_ramp,
                    ) as f32
                } else {
                    local_time.as_seconds() as f32
                };
                Some(crate::v60_render::render_data_chart(config, t_sec, sw, sh))
            }
            ItemKind::SvgMorph { config } => {
                let sw = (config.width * tf.scale_x).round().clamp(16.0, 4096.0) as u32;
                let sh = (config.height * tf.scale_y).round().clamp(16.0, 4096.0) as u32;
                let morph_override = item
                    .keyframes
                    .get("morph")
                    .filter(|k| !k.is_empty())
                    .map(|k| ve_core::evaluate_track(config.morph_progress, k, local_time));
                Some(crate::v80_render::render_svg_morph(
                    config,
                    morph_override,
                    local_time.as_seconds() as f32,
                    sw,
                    sh,
                ))
            }
            ItemKind::VtuberAvatar { config } => {
                let mut scaled_cfg = (**config).clone();
                scaled_cfg.width = (config.width * tf.scale_x).clamp(32.0, 1600.0);
                scaled_cfg.height = (config.height * tf.scale_y).clamp(32.0, 1600.0);
                let t_sec = local_time.as_seconds() as f32;
                let mut shapes = ve_core::evaluate_vtuber_shapes(&scaled_cfg, t_sec);
                if let Some(k) = item.keyframes.get("vt_yaw")
                    && !k.is_empty()
                {
                    shapes.head_yaw_deg =
                        ve_core::evaluate_track(shapes.head_yaw_deg, k, local_time)
                            .clamp(-35.0, 35.0);
                }
                if let Some(k) = item.keyframes.get("vt_pitch")
                    && !k.is_empty()
                {
                    shapes.head_pitch_deg =
                        ve_core::evaluate_track(shapes.head_pitch_deg, k, local_time)
                            .clamp(-25.0, 25.0);
                }
                if let Some(k) = item.keyframes.get("vt_roll")
                    && !k.is_empty()
                {
                    shapes.head_roll_deg =
                        ve_core::evaluate_track(shapes.head_roll_deg, k, local_time)
                            .clamp(-25.0, 25.0);
                }
                if let Some(k) = item.keyframes.get("vt_mouth")
                    && !k.is_empty()
                {
                    shapes.mouth_open =
                        ve_core::evaluate_track(shapes.mouth_open, k, local_time).clamp(0.0, 1.0);
                }
                if let Some(k) = item.keyframes.get("vt_blink_l")
                    && !k.is_empty()
                {
                    shapes.blink_l =
                        ve_core::evaluate_track(shapes.blink_l, k, local_time).clamp(0.0, 1.0);
                }
                if let Some(k) = item.keyframes.get("vt_blink_r")
                    && !k.is_empty()
                {
                    shapes.blink_r =
                        ve_core::evaluate_track(shapes.blink_r, k, local_time).clamp(0.0, 1.0);
                }
                let custom_path = scaled_cfg.asset_id.and_then(|aid| {
                    let proj = project?;
                    self.resolve_asset_path(proj, aid, false).map(|(p, _)| p)
                });
                if let Some(ref p) = custom_path
                    && !self.png_cache.contains_key(p)
                    && let Some(loaded) = load_png_linear(p)
                {
                    if self.png_cache.len() >= MAX_PNG_CACHE_ENTRIES {
                        self.png_cache.clear();
                    }
                    self.png_cache.insert(p.clone(), loaded);
                }
                let custom_tex = custom_path.as_ref().and_then(|p| self.png_cache.get(p));
                Some(crate::v90_render::render_vtuber_avatar(
                    &scaled_cfg,
                    custom_tex,
                    t_sec,
                    &shapes,
                ))
            }
            ItemKind::FluidSim { config } => {
                let sw = (config.width * tf.scale_x).round().clamp(32.0, 2048.0) as u32;
                let sh = (config.height * tf.scale_y).round().clamp(32.0, 2048.0) as u32;
                Some(crate::v100_render::render_fluid_sim_item(
                    config,
                    local_time.as_seconds() as f32,
                    sw,
                    sh,
                ))
            }
            ItemKind::ClothSim { config } => {
                let sw = (config.width * tf.scale_x).round().clamp(32.0, 2048.0) as u32;
                let sh = (config.height * tf.scale_y).round().clamp(32.0, 2048.0) as u32;
                Some(crate::v100_render::render_cloth_sim_item(
                    config,
                    local_time.as_seconds() as f32,
                    sw,
                    sh,
                ))
            }
            ItemKind::Audio { .. }
            | ItemKind::FilterObject { .. }
            | ItemKind::Transition { .. }
            | ItemKind::GroupControl { .. } => None,
        }
    }

    /// v0.6 #4: 指定音声レイヤーの事前解析波形から、スペクトラムバー配列と Low / Mid / High / RMS を高速算出する
    fn evaluate_audio_reactive_bands(
        &self,
        scene: &Scene,
        linked_audio_layer: usize,
        global_time: RationalTime,
        bar_count: usize,
    ) -> (Vec<f32>, f32, f32, f32, f32) {
        let count = bar_count.clamp(1, 64);
        let Some(layer) = scene.layers.get(linked_audio_layer) else {
            return (vec![0.0; count], 0.0, 0.0, 0.0, 0.0);
        };
        if layer.muted || layer.volume <= 0.0 {
            return (vec![0.0; count], 0.0, 0.0, 0.0, 0.0);
        }

        let mut bands = vec![0.0f32; count];
        let mut total_rms = 0.0f32;
        let mut total_low = 0.0f32;
        let mut total_mid = 0.0f32;
        let mut total_high = 0.0f32;

        for audio_item in &layer.items {
            if !audio_item.contains(global_time) {
                continue;
            }
            if let ItemKind::Audio {
                asset_id,
                source_offset,
                volume,
            }
            | ItemKind::Video {
                asset_id,
                source_offset,
                volume,
            } = &audio_item.kind
                && let Some(peaks) = self.asset_waveforms.get(asset_id)
                && !peaks.buckets.is_empty()
                && peaks.sample_rate > 0
            {
                let gain = (*volume).max(0.0) * layer.volume.max(0.0);
                let local_time = global_time - audio_item.start;
                let src_sec = audio_item.evaluate_source_time_sec(local_time, *source_offset, None);
                let sample_idx = RationalTime::from_seconds(src_sec)
                    .to_audio_samples(peaks.sample_rate)
                    .max(0) as usize;
                let center_bucket =
                    (sample_idx / peaks.samples_per_bucket.max(1)).min(peaks.buckets.len() - 1);

                let cur = peaks.buckets[center_bucket];
                let cur_rms = (cur.rms * gain).clamp(0.0, 1.5);
                let cur_peak = (cur.max.abs().max(cur.min.abs()) * gain).clamp(0.0, 1.5);
                // 低域＝近傍平均エンベロープ、中域＝瞬間RMS、高域＝トランジェント(ピーク−RMS)
                let lo_start = center_bucket.saturating_sub(3);
                let lo_end = (center_bucket + 4).min(peaks.buckets.len());
                let lo_avg = peaks.buckets[lo_start..lo_end]
                    .iter()
                    .map(|b| b.rms * gain)
                    .sum::<f32>()
                    / (lo_end - lo_start).max(1) as f32;
                let hi_trans = ((cur_peak - cur_rms * 0.75).max(0.0) * 1.45).min(1.5);

                total_rms = total_rms.max(cur_rms);
                total_low = total_low.max(lo_avg);
                total_mid = total_mid.max(cur_rms);
                total_high = total_high.max(hi_trans);

                let half = count as isize / 2;
                let phase = global_time.as_seconds() as f32;
                for (b, slot) in bands.iter_mut().enumerate() {
                    let frac = if count > 1 {
                        b as f32 / (count - 1) as f32
                    } else {
                        0.5
                    };
                    let offset = b as isize - half;
                    let idx = (center_bucket as isize + offset)
                        .clamp(0, (peaks.buckets.len() - 1) as isize)
                        as usize;
                    let bk = peaks.buckets[idx];
                    let bk_rms = bk.rms * gain;
                    let bk_peak = bk.max.abs().max(bk.min.abs()) * gain;
                    let band_base = if frac < 0.35 {
                        lo_avg * 0.6 + bk_rms * 0.4
                    } else if frac < 0.70 {
                        bk_rms * 0.65 + cur_rms * 0.35
                    } else {
                        bk_peak * 0.55 + hi_trans * 0.45
                    };
                    // 帯域ごとの自然な揺らぎ（無音時は band_base == 0 なので完全ゼロ）
                    let ripple = 0.80 + 0.20 * ((b as f32 * 1.37 + phase * 11.0).sin().abs());
                    *slot = (*slot).max((band_base * ripple).clamp(0.0, 1.0));
                }
            }
        }

        (bands, total_low, total_mid, total_high, total_rms)
    }

    /// 立ち絵の口形状を連動音声レイヤー（音量RMS または 音素母音）から自動決定する
    fn evaluate_character_mouth(
        &self,
        scene: &Scene,
        config: &CharacterSpriteConfig,
        global_time: RationalTime,
    ) -> MouthShape {
        match config.mode {
            LipSyncMode::VolumeRms {
                closed_threshold,
                open_threshold,
            } => {
                let Some(layer_idx) = config.linked_audio_layer else {
                    return MouthShape::Closed;
                };
                let Some(layer) = scene.layers.get(layer_idx) else {
                    return MouthShape::Closed;
                };
                if layer.muted {
                    return MouthShape::Closed;
                }
                let mut max_rms = 0.0f32;
                for audio_item in &layer.items {
                    if !audio_item.contains(global_time) {
                        continue;
                    }
                    if let ItemKind::Audio {
                        asset_id,
                        source_offset,
                        volume,
                    }
                    | ItemKind::Video {
                        asset_id,
                        source_offset,
                        volume,
                    } = &audio_item.kind
                        && let Some(peaks) = self.asset_waveforms.get(asset_id)
                        && !peaks.buckets.is_empty()
                        && peaks.sample_rate > 0
                    {
                        let src_t = (global_time - audio_item.start) + *source_offset;
                        let sample_idx = src_t.to_audio_samples(peaks.sample_rate).max(0) as usize;
                        let bucket_idx = (sample_idx / peaks.samples_per_bucket.max(1))
                            .min(peaks.buckets.len() - 1);
                        let rms = peaks.buckets[bucket_idx].rms * volume.max(0.0);
                        max_rms = max_rms.max(rms);
                    }
                }
                rms_to_mouth(max_rms, closed_threshold, open_threshold)
            }
            LipSyncMode::VowelPhoneme => {
                if !config.phoneme_override.is_empty() {
                    return evaluate_vowel_mouth(&config.phoneme_override, global_time);
                }
                let Some(layer_idx) = config.linked_audio_layer else {
                    return MouthShape::Closed;
                };
                let Some(layer) = scene.layers.get(layer_idx) else {
                    return MouthShape::Closed;
                };
                for item in &layer.items {
                    if item.contains(global_time) && !item.phoneme_segments.is_empty() {
                        let local = global_time - item.start;
                        return evaluate_vowel_mouth(&item.phoneme_segments, local);
                    }
                }
                MouthShape::Closed
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_video_frame_at(
        &mut self,
        asset_id: Uuid,
        path: &std::path::Path,
        is_proxy: bool,
        fps: ve_core::FrameRate,
        frame_idx: i64,
        tw: u32,
        th: u32,
    ) -> Option<LinearImage> {
        let frame_idx = frame_idx.max(0);
        let key = (asset_id, frame_idx, tw, th, is_proxy);

        if let Some((tick, cached)) = self.video_frame_cache.get_mut(&key) {
            self.video_frame_tick = self.video_frame_tick.wrapping_add(1);
            *tick = self.video_frame_tick;
            return Some(cached.clone());
        }

        // 常駐 FFmpeg ストリームデコーダ経由で高速連続フレーム取得（毎フレーム起動を回避）
        let stream_key = (asset_id, tw, th, is_proxy);
        let mut rgba_opt = self
            .video_streams
            .get_mut(&stream_key)
            .filter(|dec| dec.path == path && dec.fps == fps)
            .and_then(|dec| dec.read_frame_at(frame_idx));

        if rgba_opt.is_none() {
            let hw = self.effective_hw_decode();
            let opened = self
                .ffmpeg
                .open_video_stream_with_hwaccel(path, frame_idx, fps, tw, th, hw);
            let mut stream_opt = match opened {
                Ok(mut st) => {
                    let first = st.read_frame_at(frame_idx);
                    if first.is_some() {
                        rgba_opt = first;
                        Some(st)
                    } else if hw != HwDecodeBackend::Software {
                        self.resolved_hw_decode = Some(HwDecodeBackend::Software);
                        if let Ok(mut sw_st) = self.ffmpeg.open_video_stream_with_hwaccel(
                            path,
                            frame_idx,
                            fps,
                            tw,
                            th,
                            HwDecodeBackend::Software,
                        ) {
                            rgba_opt = sw_st.read_frame_at(frame_idx);
                            Some(sw_st)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                Err(_) if hw != HwDecodeBackend::Software => {
                    self.resolved_hw_decode = Some(HwDecodeBackend::Software);
                    if let Ok(mut sw_st) = self.ffmpeg.open_video_stream_with_hwaccel(
                        path,
                        frame_idx,
                        fps,
                        tw,
                        th,
                        HwDecodeBackend::Software,
                    ) {
                        rgba_opt = sw_st.read_frame_at(frame_idx);
                        Some(sw_st)
                    } else {
                        None
                    }
                }
                Err(_) => None,
            };
            if rgba_opt.is_some()
                && let Some(new_stream) = stream_opt.take()
            {
                if self.video_streams.len() >= MAX_VIDEO_STREAMS
                    && !self.video_streams.contains_key(&stream_key)
                {
                    self.video_streams.clear();
                }
                self.video_streams.insert(stream_key, new_stream);
            }
        }

        let fallback_sec = RationalTime::from_frame(frame_idx, fps).as_seconds();
        let rgba = rgba_opt.or_else(|| {
            self.ffmpeg
                .decode_frame_rgba8(path, fallback_sec, tw, th)
                .ok()
        })?;
        let decoded = LinearImage::from_rgba8_srgb(tw, th, &rgba);
        self.insert_video_frame(key, decoded.clone());
        Some(decoded)
    }
}

/// v1.0 #18: マルチスレッド並列フレームバッチレンダリング（`std::thread::scope`）。
/// 連続フレーム区間を各ワーカースレッドへ均等割り当てすることで、
/// ワーカースレッドごとの常駐 FFmpeg デコーダ逐次読み出し性能とマルチコア並列化を両立する。
#[allow(clippy::too_many_arguments)]
pub fn render_frame_batch_parallel(
    project: &Project,
    scene_id: SceneId,
    folder: Option<&ProjectFolder>,
    frame_times: &[RationalTime],
    transparent_bg: bool,
    use_proxies: bool,
    high_quality_motion_blur: bool,
    hw_decode: HwDecodeBackend,
    proxy_paths: &HashMap<Uuid, PathBuf>,
    waveforms: &HashMap<Uuid, WaveformPeaks>,
    worker_threads: usize,
) -> Vec<Vec<u8>> {
    if frame_times.is_empty() {
        return Vec::new();
    }
    let avail = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let threads = worker_threads.clamp(1, avail.max(1)).min(frame_times.len());

    let make_graph = || {
        let mut g = RenderGraph::new();
        g.active_revision = 1;
        g.set_project_folder(folder.cloned());
        g.use_proxies = use_proxies;
        g.high_quality_motion_blur = high_quality_motion_blur;
        g.set_hw_decode(hw_decode);
        for (&aid, p) in proxy_paths {
            g.register_proxy(aid, p.clone());
        }
        for (&aid, wf) in waveforms {
            g.register_waveform(aid, wf.clone());
        }
        g
    };

    if threads <= 1 {
        let mut g = make_graph();
        return frame_times
            .iter()
            .map(|&t| {
                g.render_project_scene(project, scene_id, t, transparent_bg)
                    .map(|img| img.to_rgba8_srgb())
                    .unwrap_or_default()
            })
            .collect();
    }

    let chunk_size = frame_times.len().div_ceil(threads);
    let mut chunk_outputs: Vec<Vec<Vec<u8>>> = Vec::with_capacity(threads);
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(threads);
        for chunk in frame_times.chunks(chunk_size) {
            handles.push(scope.spawn(move || {
                let mut g = make_graph();
                let mut out = Vec::with_capacity(chunk.len());
                for &t in chunk {
                    let rgba = g
                        .render_project_scene(project, scene_id, t, transparent_bg)
                        .map(|img| img.to_rgba8_srgb())
                        .unwrap_or_default();
                    out.push(rgba);
                }
                out
            }));
        }
        for h in handles {
            chunk_outputs.push(h.join().unwrap_or_default());
        }
    });

    let mut flat = Vec::with_capacity(frame_times.len());
    for mut part in chunk_outputs {
        flat.append(&mut part);
    }
    flat
}
