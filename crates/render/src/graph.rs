use crate::color::{LinearImage, LinearPremul};
use crate::hot_shader::ShaderHotReloader;
use crate::psd_sprite::PsdCharacterAsset;
use crate::sprite::{load_png_linear, render_placeholder_character, scale_linear_image};
use crate::text::TextRenderer;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use uuid::Uuid;
use ve_audio::WaveformPeaks;
use ve_core::{
    AssetResolution, BlendMode, CharacterSpriteConfig, ColorPalette, EffectNode, FilterTargetScope,
    ItemKind, LipSyncMode, MouthShape, PrecompCacheManager, Project, ProjectFolder, RationalTime,
    RgbaColor, Scene, SceneId, ShapeKind, TimelineItem, Transform2D, TransitionKind,
    content_hash_hex, evaluate_vowel_mouth, resolve_mouth_png, rms_to_mouth,
};
use ve_media::{FfmpegCli, VideoStreamDecoder};
use ve_plugin_api::PluginRegistry;

/// 組み込み WGSL エフェクトチェーンシェーダーソース
pub const EFFECTS_WGSL_SOURCE: &str = include_str!("shaders/effects.wgsl");

/// インメモリ Precomp キャッシュの上限フレーム数（スクラブ時の RAM 肥大化を防止）
pub const MAX_MEM_PRECOMP_FRAMES: usize = 32;
const MAX_PNG_CACHE_ENTRIES: usize = 64;
const MAX_PSD_CACHE_ENTRIES: usize = 16;
const MAX_VIDEO_FRAME_CACHE: usize = 64;
const MAX_VIDEO_STREAMS: usize = 4;

type VideoFrameCacheKey = (Uuid, i64, u32, u32, bool);
type VideoStreamKey = (Uuid, u32, u32, bool);

/// 仕様書 2.2 / 4.2 の評価順序に従う Render Graph 評価器:
/// 1. 各オブジェクトのキーフレーム評価・実動画/画像デコード・立ち絵口パク(PNG/PSD)・3Dカメラ透視投影/DoF・個別エフェクト＆プラグイン適用
/// 2. トラックマット・グループ制御・AviUtl2風フィルタオブジェクト（指定レイヤー範囲）・シーンチェンジ適用
/// 3. レイヤー合成（Linear sRGB + Premultiplied Alpha + BlendMode + 3D Zソート）
pub struct RenderGraph {
    text_renderer: TextRenderer,
    mem_precomp_cache: HashMap<String, (u64, LinearImage)>,
    mem_cache_tick: u64,
    disk_precomp_cache: Option<PrecompCacheManager>,
    project_folder: Option<ProjectFolder>,
    ffmpeg: FfmpegCli,
    /// true のときプレビュー用に軽量プロキシ (`cache/proxy/`) を優先使用（書き出し時は false で原本参照）
    pub use_proxies: bool,
    proxy_paths: HashMap<Uuid, PathBuf>,
    video_frame_cache: HashMap<VideoFrameCacheKey, (u64, LinearImage)>,
    video_frame_tick: u64,
    video_streams: HashMap<VideoStreamKey, VideoStreamDecoder>,
    png_cache: HashMap<PathBuf, LinearImage>,
    psd_cache: HashMap<PathBuf, PsdCharacterAsset>,
    /// 音声アセットIDごとの事前解析波形（立ち絵の音量連動口パク参照用）
    pub asset_waveforms: HashMap<Uuid, WaveformPeaks>,
    /// Phase 4: OSS プラグインレジストリ（ビネット・セピア・外部プラグイン）
    pub plugins: PluginRegistry,
    /// Phase 4: カスタム WGSL シェーダーのホットリロード管理
    pub hot_shaders: ShaderHotReloader,
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
            project_folder: None,
            ffmpeg: FfmpegCli::default(),
            use_proxies: true,
            proxy_paths: HashMap::new(),
            video_frame_cache: HashMap::new(),
            video_frame_tick: 0,
            video_streams: HashMap::new(),
            png_cache: HashMap::new(),
            psd_cache: HashMap::new(),
            asset_waveforms: HashMap::new(),
            plugins: PluginRegistry::with_builtins(),
            hot_shaders: ShaderHotReloader::new(),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    pub fn text_renderer(&self) -> &TextRenderer {
        &self.text_renderer
    }

    pub fn text_renderer_mut(&mut self) -> &mut TextRenderer {
        &mut self.text_renderer
    }

    pub fn set_project_folder(&mut self, folder: Option<ProjectFolder>) {
        self.project_folder = folder;
    }

    pub fn register_proxy(&mut self, asset_id: Uuid, proxy_path: PathBuf) {
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
        self.proxy_paths.clear();
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
                match &mut item.kind {
                    ItemKind::Shape { width, height, .. }
                    | ItemKind::CharacterSprite { width, height, .. } => {
                        *width = (*width * sx).max(1.0);
                        *height = (*height * sy).max(1.0);
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
                    }
                    _ => {}
                }
            }
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
        let w = scene.width.max(1);
        let h = scene.height.max(1);
        let mut layer_outputs: Vec<Option<(LinearImage, BlendMode, f32)>> =
            vec![None; scene.layers.len()];
        let mut filter_passes: Vec<(usize, String, FilterTargetScope, BTreeMap<String, f32>)> =
            Vec::new();
        let mut transition_passes: Vec<(usize, TransitionKind, FilterTargetScope, f32, f32)> =
            Vec::new();
        let mut group_passes: Vec<(usize, usize, Transform2D)> = Vec::new();

        // Step 1: 各レイヤーのアクティブオブジェクトを描画し、個別エフェクト・3Dカメラ透視投影/DoFを適用
        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            if !layer.visible {
                continue;
            }
            let mut layer_buf: Option<LinearImage> = None;
            let mut layer_blend = BlendMode::Normal;
            let mut layer_depth = 1000.0_f32;

            for item in &layer.items {
                if !item.contains(time) {
                    continue;
                }
                let tf = item.eval_transform(time);
                let tf3d = item.eval_transform_3d(time);
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
                        if let Some(mut item_img) =
                            self.render_item(project, scene, item, &tf, time, w, h, depth)
                        {
                            apply_effect_chain(
                                &mut item_img,
                                &item.effect_chain,
                                time.as_seconds(),
                                &self.plugins,
                            );

                            let (mut draw_x, mut draw_y) =
                                (tf.x + layer.offset_x, tf.y + layer.offset_y);
                            if tf.rotation_deg.abs() > 0.05 {
                                let old_w = item_img.width as f32;
                                let old_h = item_img.height as f32;
                                item_img = rotate_linear_image(&item_img, tf.rotation_deg);
                                draw_x += (old_w - item_img.width as f32) * 0.5;
                                draw_y += (old_h - item_img.height as f32) * 0.5;
                            }
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
                            }

                            let dst = layer_buf.get_or_insert_with(|| {
                                LinearImage::new(w, h, LinearPremul::TRANSPARENT)
                            });
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

        // 最終レイヤー合成（3Dカメラ有効時は奥から手前へ Z ソート）
        let bg = if transparent_bg {
            LinearPremul::TRANSPARENT
        } else {
            LinearPremul::from_srgb_straight(RgbaColor::BLACK)
        };
        let mut canvas = LinearImage::new(w, h, bg);
        let mut draw_list: Vec<(usize, LinearImage, BlendMode, f32)> = layer_outputs
            .into_iter()
            .enumerate()
            .filter_map(|(idx, opt)| {
                if consumed_matte_layers[idx] {
                    None
                } else {
                    opt.map(|(img, bm, d)| (idx, img, bm, d))
                }
            })
            .collect();

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
        }

        canvas
    }

    #[allow(clippy::too_many_arguments)]
    fn render_item(
        &mut self,
        project: Option<&Project>,
        scene: &Scene,
        item: &TimelineItem,
        tf: &Transform2D,
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
                Some(self.text_renderer.render_text_styled(
                    content,
                    font_family,
                    *font_size * tf.scale_y.max(0.1),
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
                let frame_idx = src_time.to_frame_floor(scene.fps).max(0);
                let tw = ((canvas_w as f32) * tf.scale_x.max(0.01))
                    .round()
                    .clamp(1.0, 4096.0) as u32;
                let th = ((canvas_h as f32) * tf.scale_y.max(0.01))
                    .round()
                    .clamp(1.0, 4096.0) as u32;
                let key = (*asset_id, frame_idx, tw, th, is_proxy);

                if let Some((tick, cached)) = self.video_frame_cache.get_mut(&key) {
                    self.video_frame_tick = self.video_frame_tick.wrapping_add(1);
                    *tick = self.video_frame_tick;
                    return Some(cached.clone());
                }

                // 常駐 FFmpeg ストリームデコーダ経由で高速連続フレーム取得（毎フレーム起動を回避）
                let stream_key = (*asset_id, tw, th, is_proxy);
                let mut rgba_opt = self
                    .video_streams
                    .get_mut(&stream_key)
                    .filter(|dec| dec.path == path && dec.fps == scene.fps)
                    .and_then(|dec| dec.read_frame_at(frame_idx));

                if rgba_opt.is_none()
                    && let Ok(mut new_stream) = self
                        .ffmpeg
                        .open_video_stream(&path, frame_idx, scene.fps, tw, th)
                {
                    rgba_opt = new_stream.read_frame_at(frame_idx);
                    if rgba_opt.is_some() {
                        if self.video_streams.len() >= MAX_VIDEO_STREAMS
                            && !self.video_streams.contains_key(&stream_key)
                        {
                            self.video_streams.clear();
                        }
                        self.video_streams.insert(stream_key, new_stream);
                    }
                }

                let rgba = rgba_opt.or_else(|| {
                    self.ffmpeg
                        .decode_frame_rgba8(&path, src_time.as_seconds(), tw, th)
                        .ok()
                })?;
                let decoded = LinearImage::from_rgba8_srgb(tw, th, &rgba);
                self.insert_video_frame(key, decoded.clone());
                Some(decoded)
            }
            ItemKind::Audio { .. }
            | ItemKind::FilterObject { .. }
            | ItemKind::Transition { .. }
            | ItemKind::GroupControl { .. } => None,
        }
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
}

fn apply_group_transform(src: &LinearImage, tf: Transform2D) -> LinearImage {
    let mut dst = LinearImage::new(src.width, src.height, LinearPremul::TRANSPARENT);
    let ox = tf.x.round() as i32;
    let oy = tf.y.round() as i32;
    let sx = tf.scale_x.max(0.01);
    let sy = tf.scale_y.max(0.01);
    let opacity = tf.opacity.clamp(0.0, 1.0);

    if (sx - 1.0).abs() < 1e-4 && (sy - 1.0).abs() < 1e-4 {
        dst.blend_over(src, ox, oy, opacity);
        return dst;
    }

    let cx = src.width as f32 * 0.5;
    let cy = src.height as f32 * 0.5;
    for y in 0..dst.height {
        for x in 0..dst.width {
            let src_x = ((x as f32 - tf.x - cx) / sx + cx).round() as i32;
            let src_y = ((y as f32 - tf.y - cy) / sy + cy).round() as i32;
            if src_x >= 0 && src_x < src.width as i32 && src_y >= 0 && src_y < src.height as i32 {
                let mut p = src.pixels[(src_y as u32 * src.width + src_x as u32) as usize];
                p.r *= opacity;
                p.g *= opacity;
                p.b *= opacity;
                p.a *= opacity;
                dst.pixels[(y * dst.width + x) as usize] = p;
            }
        }
    }
    dst
}

fn apply_effect_chain(
    image: &mut LinearImage,
    chain: &[EffectNode],
    time_sec: f64,
    plugins: &PluginRegistry,
) {
    for node in chain {
        if node.enabled {
            apply_named_effect(image, &node.effect_type, &node.params, time_sec, plugins);
        }
    }
}

fn apply_named_effect(
    image: &mut LinearImage,
    effect_type: &str,
    params: &BTreeMap<String, f32>,
    time_sec: f64,
    plugins: &PluginRegistry,
) {
    match effect_type {
        "brightness" => {
            let gain = params.get("gain").copied().unwrap_or(1.0).max(0.0);
            for px in &mut image.pixels {
                px.r *= gain;
                px.g *= gain;
                px.b *= gain;
            }
        }
        "invert" => {
            for px in &mut image.pixels {
                if px.a > 0.0 {
                    px.r = (px.a - px.r).max(0.0);
                    px.g = (px.a - px.g).max(0.0);
                    px.b = (px.a - px.b).max(0.0);
                }
            }
        }
        "color_grade" => {
            let exposure = params.get("exposure").copied().unwrap_or(0.0);
            let contrast = params.get("contrast").copied().unwrap_or(1.0).max(0.0);
            let saturation = params.get("saturation").copied().unwrap_or(1.0).max(0.0);
            let exp_mul = 2.0_f32.powf(exposure);
            for px in &mut image.pixels {
                if px.a <= 1e-6 {
                    continue;
                }
                let mut r = (px.r / px.a) * exp_mul;
                let mut g = (px.g / px.a) * exp_mul;
                let mut b = (px.b / px.a) * exp_mul;
                r = (r - 0.5) * contrast + 0.5;
                g = (g - 0.5) * contrast + 0.5;
                b = (b - 0.5) * contrast + 0.5;
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = (luma + (r - luma) * saturation).max(0.0);
                g = (luma + (g - luma) * saturation).max(0.0);
                b = (luma + (b - luma) * saturation).max(0.0);
                px.r = r * px.a;
                px.g = g * px.a;
                px.b = b * px.a;
            }
        }
        "blur" => {
            let radius = params.get("radius").copied().unwrap_or(2.0).round() as i32;
            if radius > 0 {
                box_blur_separable(image, radius.clamp(1, 16));
            }
        }
        "glow" => {
            let radius = params.get("radius").copied().unwrap_or(3.0).round() as i32;
            let threshold = params.get("threshold").copied().unwrap_or(0.5);
            let intensity = params.get("intensity").copied().unwrap_or(1.0).max(0.0);
            let mut bright = LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
            for (dst, src) in bright.pixels.iter_mut().zip(image.pixels.iter()) {
                let luma = src.luma();
                if luma > threshold {
                    let excess = (luma - threshold).max(0.0);
                    dst.r = src.r * excess;
                    dst.g = src.g * excess;
                    dst.b = src.b * excess;
                    dst.a = src.a;
                }
            }
            box_blur_separable(&mut bright, radius.clamp(1, 16));
            for (dst, b) in image.pixels.iter_mut().zip(bright.pixels.iter()) {
                dst.r += b.r * intensity;
                dst.g += b.g * intensity;
                dst.b += b.b * intensity;
            }
        }
        "clip_rect" => {
            let left = params.get("left").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let right = params.get("right").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let top = params.get("top").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let bottom = params.get("bottom").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let w = image.width.max(1) as f32;
            let h = image.height.max(1) as f32;
            for y in 0..image.height {
                let v = (y as f32 + 0.5) / h;
                for x in 0..image.width {
                    let u = (x as f32 + 0.5) / w;
                    if u < left || u > (1.0 - right) || v < top || v > (1.0 - bottom) {
                        image.pixels[(y * image.width + x) as usize] = LinearPremul::TRANSPARENT;
                    }
                }
            }
        }
        "chroma_key" => {
            // グリーンバック / ブルーバック / 任意色クロマキー＋スピル抑制
            let kr = params.get("key_r").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let kg = params.get("key_g").copied().unwrap_or(1.0).clamp(0.0, 1.0);
            let kb = params.get("key_b").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let tolerance = params
                .get("tolerance")
                .copied()
                .unwrap_or(0.35)
                .clamp(0.01, 1.5);
            let softness = params
                .get("softness")
                .copied()
                .unwrap_or(0.15)
                .clamp(0.01, 1.0);
            let spill = params
                .get("spill_suppress")
                .copied()
                .unwrap_or(0.4)
                .clamp(0.0, 1.0);
            let key_lin = LinearPremul::from_srgb_straight(RgbaColor {
                r: kr,
                g: kg,
                b: kb,
                a: 1.0,
            });
            for px in &mut image.pixels {
                if px.a <= 1e-6 {
                    continue;
                }
                let inv_a = 1.0 / px.a;
                let r = px.r * inv_a;
                let mut g = px.g * inv_a;
                let b = px.b * inv_a;
                let dr = r - key_lin.r;
                let dg = g - key_lin.g;
                let db = b - key_lin.b;
                let dist = (dr * dr + dg * dg + db * db).sqrt();
                let mask = ((dist - tolerance) / softness).clamp(0.0, 1.0);
                if spill > 0.0 && kg >= kr && kg >= kb {
                    let max_rb = r.max(b);
                    if g > max_rb {
                        g -= (g - max_rb) * spill;
                    }
                }
                let new_a = px.a * mask;
                px.r = r * new_a;
                px.g = g * new_a;
                px.b = b * new_a;
                px.a = new_a;
            }
        }
        "luma_key" => {
            // 黒背景・白背景エフェクト素材透過用のルミナンスキー
            let threshold = params
                .get("threshold")
                .copied()
                .unwrap_or(0.12)
                .clamp(0.0, 1.0);
            let softness = params
                .get("softness")
                .copied()
                .unwrap_or(0.08)
                .clamp(0.005, 1.0);
            let invert = params.get("invert").copied().unwrap_or(0.0) >= 0.5;
            for px in &mut image.pixels {
                if px.a <= 1e-6 {
                    continue;
                }
                let luma = px.luma() / px.a.max(1e-5);
                let mut mask = ((luma - threshold) / softness).clamp(0.0, 1.0);
                if invert {
                    mask = 1.0 - mask;
                }
                px.r *= mask;
                px.g *= mask;
                px.b *= mask;
                px.a *= mask;
            }
        }
        "mosaic" => {
            // ブロック平均モザイク / ピクセレート（個人情報隠し・ネタ演出）
            let block = params
                .get("block_size")
                .copied()
                .unwrap_or(12.0)
                .round()
                .clamp(2.0, 128.0) as u32;
            let w = image.width;
            let h = image.height;
            for by in (0..h).step_by(block as usize) {
                let y_end = (by + block).min(h);
                for bx in (0..w).step_by(block as usize) {
                    let x_end = (bx + block).min(w);
                    let mut sum = [0.0f64; 4];
                    let mut cnt = 0.0f64;
                    for y in by..y_end {
                        for x in bx..x_end {
                            let p = image.pixels[(y * w + x) as usize];
                            sum[0] += f64::from(p.r);
                            sum[1] += f64::from(p.g);
                            sum[2] += f64::from(p.b);
                            sum[3] += f64::from(p.a);
                            cnt += 1.0;
                        }
                    }
                    if cnt > 0.0 {
                        let inv = 1.0 / cnt;
                        let avg = LinearPremul {
                            r: (sum[0] * inv) as f32,
                            g: (sum[1] * inv) as f32,
                            b: (sum[2] * inv) as f32,
                            a: (sum[3] * inv) as f32,
                        };
                        for y in by..y_end {
                            for x in bx..x_end {
                                image.pixels[(y * w + x) as usize] = avg;
                            }
                        }
                    }
                }
            }
        }
        "rounded_clip" => {
            // 角丸クリッピング（ワイプ枠・小窓演出用）
            let w = image.width as f32;
            let h = image.height as f32;
            let margin = params
                .get("margin")
                .copied()
                .unwrap_or(0.0)
                .clamp(0.0, w.min(h) * 0.45);
            let max_r = ((w - margin * 2.0).min(h - margin * 2.0) * 0.5).max(0.0);
            let radius = params
                .get("radius")
                .copied()
                .unwrap_or(28.0)
                .clamp(0.0, max_r);
            let cx = w * 0.5;
            let cy = h * 0.5;
            let hx = (w * 0.5 - margin - radius).max(0.0);
            let hy = (h * 0.5 - margin - radius).max(0.0);
            for y in 0..image.height {
                let fy = y as f32 + 0.5;
                for x in 0..image.width {
                    let fx = x as f32 + 0.5;
                    let qx = (fx - cx).abs() - hx;
                    let qy = (fy - cy).abs() - hy;
                    let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
                    let cov = (0.5 - dist).clamp(0.0, 1.0);
                    if cov < 1.0 {
                        let p = &mut image.pixels[(y * image.width + x) as usize];
                        p.r *= cov;
                        p.g *= cov;
                        p.b *= cov;
                        p.a *= cov;
                    }
                }
            }
        }
        "diagonal_clip" => {
            // AviUtl 定番「斜めクリッピング」（画面分割・スラッシュ演出）
            let angle_rad = params
                .get("angle_deg")
                .copied()
                .unwrap_or(30.0)
                .to_radians();
            let center_x = params
                .get("center_x")
                .copied()
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let center_y = params
                .get("center_y")
                .copied()
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let half_width = params.get("width").copied().unwrap_or(0.0).max(0.0) * 0.5;
            let feather = params.get("feather").copied().unwrap_or(2.0).max(0.5);
            let nx = angle_rad.cos();
            let ny = angle_rad.sin();
            let cx = image.width as f32 * center_x;
            let cy = image.height as f32 * center_y;
            for y in 0..image.height {
                let dy = y as f32 + 0.5 - cy;
                for x in 0..image.width {
                    let dx = x as f32 + 0.5 - cx;
                    let signed = dx * nx + dy * ny;
                    let cov = if half_width > 0.0 {
                        // バンド幅指定時は中心帯を残す
                        let d = half_width - signed.abs();
                        (d / feather + 0.5).clamp(0.0, 1.0)
                    } else {
                        // 片側クリップ
                        (signed / feather + 0.5).clamp(0.0, 1.0)
                    };
                    if cov < 1.0 {
                        let p = &mut image.pixels[(y * image.width + x) as usize];
                        p.r *= cov;
                        p.g *= cov;
                        p.b *= cov;
                        p.a *= cov;
                    }
                }
            }
        }
        "shake" => {
            // AviUtl 定番「振動」（手ブレ・ツッコミ衝撃シェイク）
            let amp_x = params
                .get("amp_x")
                .copied()
                .unwrap_or(14.0)
                .clamp(0.0, 200.0);
            let amp_y = params
                .get("amp_y")
                .copied()
                .unwrap_or(10.0)
                .clamp(0.0, 200.0);
            let freq = params
                .get("freq")
                .copied()
                .unwrap_or(24.0)
                .clamp(0.5, 120.0);
            let seed = params.get("seed").copied().unwrap_or(1.0);
            let t = time_sec as f32 * freq;
            let ox = ((t * 1.7 + seed * 3.1).sin() * 0.65 + (t * 3.3 + seed).cos() * 0.35) * amp_x;
            let oy = ((t * 2.1 + seed * 5.7).cos() * 0.65 + (t * 4.1 + seed).sin() * 0.35) * amp_y;
            let dx = ox.round() as i32;
            let dy = oy.round() as i32;
            if dx != 0 || dy != 0 {
                let mut shifted =
                    LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
                shifted.blend_over(image, dx, dy, 1.0);
                *image = shifted;
            }
        }
        "outline" => {
            // 任意画像・立ち絵・字幕へ適用できる縁取りフィルタ
            let width = params.get("width").copied().unwrap_or(3.0).clamp(1.0, 14.0);
            let oc = LinearPremul::from_srgb_straight(RgbaColor {
                r: params.get("r").copied().unwrap_or(0.0),
                g: params.get("g").copied().unwrap_or(0.0),
                b: params.get("b").copied().unwrap_or(0.0),
                a: params.get("a").copied().unwrap_or(1.0),
            });
            let rad = width.ceil() as i32;
            let rad_sq = width * width + 0.5;
            let mut offsets = Vec::new();
            for dy in -rad..=rad {
                for dx in -rad..=rad {
                    if (dx * dx + dy * dy) as f32 <= rad_sq {
                        offsets.push((dx, dy));
                    }
                }
            }
            let w = image.width as i32;
            let h = image.height as i32;
            let mut out = LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
            for y in 0..h {
                for x in 0..w {
                    let mut max_a = 0.0f32;
                    for &(dx, dy) in &offsets {
                        let sx = x + dx;
                        let sy = y + dy;
                        if sx >= 0 && sx < w && sy >= 0 && sy < h {
                            let a = image.pixels[(sy as u32 * image.width + sx as u32) as usize].a;
                            if a > max_a {
                                max_a = a;
                                if max_a >= 0.999 {
                                    break;
                                }
                            }
                        }
                    }
                    let idx = (y as u32 * image.width + x as u32) as usize;
                    let border = LinearPremul {
                        r: oc.r * max_a,
                        g: oc.g * max_a,
                        b: oc.b * max_a,
                        a: oc.a * max_a,
                    };
                    out.pixels[idx] = image.pixels[idx].over(border);
                }
            }
            *image = out;
        }
        "drop_shadow" => {
            // 任意画像・立ち絵・字幕へ適用できるドロップシャドウフィルタ
            let ox = params.get("offset_x").copied().unwrap_or(6.0).round() as i32;
            let oy = params.get("offset_y").copied().unwrap_or(6.0).round() as i32;
            let blur = params
                .get("blur")
                .copied()
                .unwrap_or(4.0)
                .round()
                .clamp(0.0, 14.0) as i32;
            let opacity = params
                .get("opacity")
                .copied()
                .unwrap_or(0.6)
                .clamp(0.0, 1.0);
            let mut shadow = LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
            let w = image.width as i32;
            let h = image.height as i32;
            for y in 0..h {
                let sy = y - oy;
                if sy < 0 || sy >= h {
                    continue;
                }
                for x in 0..w {
                    let sx = x - ox;
                    if sx < 0 || sx >= w {
                        continue;
                    }
                    let a =
                        image.pixels[(sy as u32 * image.width + sx as u32) as usize].a * opacity;
                    shadow.pixels[(y as u32 * image.width + x as u32) as usize] = LinearPremul {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a,
                    };
                }
            }
            if blur > 0 {
                box_blur_separable(&mut shadow, blur);
            }
            for (dst, &src) in shadow.pixels.iter_mut().zip(image.pixels.iter()) {
                *dst = src.over(*dst);
            }
            *image = shadow;
        }
        other => {
            // Phase 4: プラグイン API 経由のエフェクトディスパッチ
            if plugins.get_effect(other).is_some() {
                let mut raw: Vec<[f32; 4]> =
                    image.pixels.iter().map(|p| [p.r, p.g, p.b, p.a]).collect();
                if plugins
                    .apply_effect(other, &mut raw, image.width, image.height, time_sec, params)
                    .is_ok()
                {
                    for (dst, src) in image.pixels.iter_mut().zip(raw) {
                        dst.r = src[0];
                        dst.g = src[1];
                        dst.b = src[2];
                        dst.a = src[3];
                    }
                }
            }
        }
    }
}

fn resolve_scope_layers(owner_layer_idx: usize, scope: FilterTargetScope) -> Vec<usize> {
    match scope {
        FilterTargetScope::ImmediateLayer => {
            if owner_layer_idx > 0 {
                vec![owner_layer_idx - 1]
            } else {
                vec![]
            }
        }
        FilterTargetScope::LayerRange { count } => {
            let start = owner_layer_idx.saturating_sub(count);
            (start..owner_layer_idx).collect()
        }
        FilterTargetScope::AllRemaining => (0..owner_layer_idx).collect(),
    }
}

/// AviUtl2 風シーンチェンジ（トランジションオブジェクト）を対象レイヤーへ適用する
fn apply_transition(
    image: &mut LinearImage,
    transition: TransitionKind,
    progress: f32,
    softness: f32,
) {
    let p = progress.clamp(0.0, 1.0);
    let soft = softness.clamp(0.005, 0.5);
    let w = image.width.max(1) as f32;
    let h = image.height.max(1) as f32;

    match transition {
        TransitionKind::CrossFade => {
            for px in &mut image.pixels {
                px.r *= p;
                px.g *= p;
                px.b *= p;
                px.a *= p;
            }
        }
        TransitionKind::WipeLeft
        | TransitionKind::WipeRight
        | TransitionKind::WipeUp
        | TransitionKind::WipeDown => {
            let edge = p * (1.0 + soft) - soft * 0.5;
            for y in 0..image.height {
                let v = (y as f32 + 0.5) / h;
                for x in 0..image.width {
                    let u = (x as f32 + 0.5) / w;
                    let coord = match transition {
                        TransitionKind::WipeLeft => u,
                        TransitionKind::WipeRight => 1.0 - u,
                        TransitionKind::WipeUp => v,
                        TransitionKind::WipeDown => 1.0 - v,
                        _ => u,
                    };
                    let mask = ((edge - coord) / soft + 0.5).clamp(0.0, 1.0);
                    if mask < 1.0 {
                        let px = &mut image.pixels[(y * image.width + x) as usize];
                        px.r *= mask;
                        px.g *= mask;
                        px.b *= mask;
                        px.a *= mask;
                    }
                }
            }
        }
        TransitionKind::SlideLeft | TransitionKind::SlideRight => {
            let dir = if matches!(transition, TransitionKind::SlideLeft) {
                -1.0
            } else {
                1.0
            };
            let shift_x = (dir * (1.0 - p) * w).round() as i32;
            let mut moved = LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
            moved.blend_over(image, shift_x, 0, 1.0);
            *image = moved;
        }
        TransitionKind::Flash => {
            // 中央 (p=0.5) でピークに達する発光トランジション
            let bell = (1.0 - (p * 2.0 - 1.0).abs()).clamp(0.0, 1.0);
            let boost = bell * bell * 2.5;
            for px in &mut image.pixels {
                if px.a > 0.0 {
                    px.r += boost * px.a;
                    px.g += boost * px.a;
                    px.b += boost * px.a;
                }
            }
        }
        TransitionKind::RandomLines => {
            // 決定論的ハッシュによる水平スキャンライン出現（AviUtl2「ランダムライン」相当）
            let bands = 32u32;
            let band_h = (image.height / bands).max(1);
            for y in 0..image.height {
                let band_idx = y / band_h;
                let hash = ((band_idx.wrapping_mul(2_654_435_761)) >> 16) as f32 / 65535.0;
                if hash > p {
                    let row_start = (y * image.width) as usize;
                    image.pixels[row_start..row_start + image.width as usize]
                        .fill(LinearPremul::TRANSPARENT);
                }
            }
        }
    }
}

/// O(W * H) スライディングウィンドウ分離ボックスブラー（半径 R に依存しない一定計算量＋行優先メモリアクセス）
fn box_blur_separable(image: &mut LinearImage, radius: i32) {
    let w = image.width as i32;
    let h = image.height as i32;
    if w == 0 || h == 0 || radius <= 0 {
        return;
    }
    let wu = w as usize;
    let hu = h as usize;
    let mut temp = vec![LinearPremul::TRANSPARENT; image.pixels.len()];
    let inv_denom = 1.0_f64 / f64::from(radius * 2 + 1);

    // 水平パス（行ごとにスライディングウィンドウ加算）
    for y in 0..hu {
        let src_row = &image.pixels[y * wu..(y + 1) * wu];
        let dst_row = &mut temp[y * wu..(y + 1) * wu];

        let mut acc = [0.0f64; 4];
        for dx in -radius..=radius {
            let sx = dx.clamp(0, w - 1) as usize;
            let p = src_row[sx];
            acc[0] += f64::from(p.r);
            acc[1] += f64::from(p.g);
            acc[2] += f64::from(p.b);
            acc[3] += f64::from(p.a);
        }
        dst_row[0] = LinearPremul {
            r: (acc[0] * inv_denom).max(0.0) as f32,
            g: (acc[1] * inv_denom).max(0.0) as f32,
            b: (acc[2] * inv_denom).max(0.0) as f32,
            a: (acc[3] * inv_denom).max(0.0) as f32,
        };

        for x in 1..w {
            let sub_x = (x - 1 - radius).clamp(0, w - 1) as usize;
            let add_x = (x + radius).clamp(0, w - 1) as usize;
            let p_sub = src_row[sub_x];
            let p_add = src_row[add_x];
            acc[0] += f64::from(p_add.r) - f64::from(p_sub.r);
            acc[1] += f64::from(p_add.g) - f64::from(p_sub.g);
            acc[2] += f64::from(p_add.b) - f64::from(p_sub.b);
            acc[3] += f64::from(p_add.a) - f64::from(p_sub.a);
            dst_row[x as usize] = LinearPremul {
                r: (acc[0] * inv_denom).max(0.0) as f32,
                g: (acc[1] * inv_denom).max(0.0) as f32,
                b: (acc[2] * inv_denom).max(0.0) as f32,
                a: (acc[3] * inv_denom).max(0.0) as f32,
            };
        }
    }

    // 垂直パス（幅 W の行アキュムレータで常に連続ストライド1のキャッシュフレンドリー走査）
    let mut col_acc = vec![[0.0f64; 4]; wu];
    for dy in -radius..=radius {
        let sy = dy.clamp(0, h - 1) as usize;
        let row = &temp[sy * wu..(sy + 1) * wu];
        for (acc, &p) in col_acc.iter_mut().zip(row.iter()) {
            acc[0] += f64::from(p.r);
            acc[1] += f64::from(p.g);
            acc[2] += f64::from(p.b);
            acc[3] += f64::from(p.a);
        }
    }
    for (dst, acc) in image.pixels[..wu].iter_mut().zip(col_acc.iter()) {
        *dst = LinearPremul {
            r: (acc[0] * inv_denom).max(0.0) as f32,
            g: (acc[1] * inv_denom).max(0.0) as f32,
            b: (acc[2] * inv_denom).max(0.0) as f32,
            a: (acc[3] * inv_denom).max(0.0) as f32,
        };
    }

    for y in 1..h {
        let sub_y = (y - 1 - radius).clamp(0, h - 1) as usize;
        let add_y = (y + radius).clamp(0, h - 1) as usize;
        let sub_row = &temp[sub_y * wu..(sub_y + 1) * wu];
        let add_row = &temp[add_y * wu..(add_y + 1) * wu];
        let dst_row = &mut image.pixels[(y as usize) * wu..(y as usize + 1) * wu];
        for ((acc, (&p_add, &p_sub)), dst) in col_acc
            .iter_mut()
            .zip(add_row.iter().zip(sub_row.iter()))
            .zip(dst_row.iter_mut())
        {
            acc[0] += f64::from(p_add.r) - f64::from(p_sub.r);
            acc[1] += f64::from(p_add.g) - f64::from(p_sub.g);
            acc[2] += f64::from(p_add.b) - f64::from(p_sub.b);
            acc[3] += f64::from(p_add.a) - f64::from(p_sub.a);
            *dst = LinearPremul {
                r: (acc[0] * inv_denom).max(0.0) as f32,
                g: (acc[1] * inv_denom).max(0.0) as f32,
                b: (acc[2] * inv_denom).max(0.0) as f32,
                a: (acc[3] * inv_denom).max(0.0) as f32,
            };
        }
    }
}

/// Linear Premultiplied 画像を中心軸まわりに指定角度（度）でバイリニア回転する
fn rotate_linear_image(src: &LinearImage, angle_deg: f32) -> LinearImage {
    let rad = angle_deg.to_radians();
    let cos = rad.cos();
    let sin = rad.sin();
    let sw = src.width as f32;
    let sh = src.height as f32;
    let out_w = (sw * cos.abs() + sh * sin.abs()).ceil().clamp(1.0, 4096.0) as u32;
    let out_h = (sw * sin.abs() + sh * cos.abs()).ceil().clamp(1.0, 4096.0) as u32;

    let mut dst = LinearImage::new(out_w, out_h, LinearPremul::TRANSPARENT);
    let src_cx = sw * 0.5;
    let src_cy = sh * 0.5;
    let dst_cx = out_w as f32 * 0.5;
    let dst_cy = out_h as f32 * 0.5;
    let iw = src.width as i32;
    let ih = src.height as i32;

    let sample = |x: i32, y: i32| -> LinearPremul {
        if x < 0 || y < 0 || x >= iw || y >= ih {
            LinearPremul::TRANSPARENT
        } else {
            src.pixels[(y as u32 * src.width + x as u32) as usize]
        }
    };

    for y in 0..out_h {
        let dy = y as f32 + 0.5 - dst_cy;
        for x in 0..out_w {
            let dx = x as f32 + 0.5 - dst_cx;
            // 逆回転でソース座標を算出
            let sx = dx * cos + dy * sin + src_cx - 0.5;
            let sy = -dx * sin + dy * cos + src_cy - 0.5;
            if sx < -1.0 || sy < -1.0 || sx >= sw || sy >= sh {
                continue;
            }
            let x0 = sx.floor() as i32;
            let y0 = sy.floor() as i32;
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let p00 = sample(x0, y0);
            let p10 = sample(x0 + 1, y0);
            let p01 = sample(x0, y0 + 1);
            let p11 = sample(x0 + 1, y0 + 1);
            let w00 = (1.0 - fx) * (1.0 - fy);
            let w10 = fx * (1.0 - fy);
            let w01 = (1.0 - fx) * fy;
            let w11 = fx * fy;
            dst.pixels[(y * out_w + x) as usize] = LinearPremul {
                r: p00.r * w00 + p10.r * w10 + p01.r * w01 + p11.r * w11,
                g: p00.g * w00 + p10.g * w10 + p01.g * w01 + p11.g * w11,
                b: p00.b * w00 + p10.b * w10 + p01.b * w01 + p11.b * w11,
                a: p00.a * w00 + p10.a * w10 + p01.a * w01 + p11.a * w11,
            };
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;
    use ve_audio::WaveformBucket;
    use ve_core::{
        Camera3D, CommandBus, CommandSource, EditCommand, FrameRate, PhonemeSegment,
        ProjectSettings, TrackMatte, TrackMatteMode, Transform3D,
    };

    fn write_solid_png(path: &std::path::Path, rgba: [u8; 4]) {
        let file = File::create(path).unwrap();
        let mut enc = png::Encoder::new(file, 8, 8);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        let data: Vec<u8> = rgba.into_iter().cycle().take(8 * 8 * 4).collect();
        w.write_image_data(&data).unwrap();
    }

    #[test]
    fn filter_object_applies_to_immediate_layer_distinct_from_effect_chain() {
        let mut scene = Scene::new("Test", 16, 16, FrameRate::FPS_60);
        scene.layers[0].items.push(TimelineItem::new(
            "Rect",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 16.0,
                height: 16.0,
                color: RgbaColor::WHITE,
            },
        ));
        let mut params = BTreeMap::new();
        params.insert("gain".to_string(), 0.5);
        scene.layers[1].items.push(TimelineItem::new(
            "DimFilter",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::FilterObject {
                effect_type: "brightness".into(),
                scope: FilterTargetScope::ImmediateLayer,
                params,
            },
        ));

        let mut graph = RenderGraph::new();
        let frame = graph.render_scene(&scene, RationalTime::ZERO);
        assert!((frame.pixels[0].r - 0.5).abs() < 1e-4);
    }

    #[test]
    fn track_matte_alpha_masks_target_layer() {
        let mut scene = Scene::new("MatteTest", 16, 16, FrameRate::FPS_60);
        scene.layers[0].items.push(TimelineItem::new(
            "FullWhite",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 16.0,
                height: 16.0,
                color: RgbaColor::WHITE,
            },
        ));
        scene.layers[1].items.push(TimelineItem::new(
            "LeftMatte",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 8.0,
                height: 16.0,
                color: RgbaColor::WHITE,
            },
        ));
        scene.layers[0].track_matte = Some(TrackMatte {
            mode: TrackMatteMode::Alpha,
            source_layer_index: 1,
        });

        let mut graph = RenderGraph::new();
        let frame = graph.render_scene(&scene, RationalTime::ZERO);
        assert!(frame.pixels[2].r > 0.9);
        assert!(frame.pixels[12].r < 0.05);
    }

    #[test]
    fn group_control_offsets_targeted_layer() {
        let mut scene = Scene::new("GroupTest", 16, 16, FrameRate::FPS_60);
        scene.layers[0].items.push(TimelineItem::new(
            "Box",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 4.0,
                height: 4.0,
                color: RgbaColor::WHITE,
            },
        ));
        let mut grp = TimelineItem::new(
            "Group1",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::GroupControl {
                target_layer_count: 1,
            },
        );
        grp.transform.x = 8.0;
        scene.layers[1].items.push(grp);

        let mut graph = RenderGraph::new();
        let frame = graph.render_scene(&scene, RationalTime::ZERO);
        assert!(frame.pixels[0].r < 0.05);
        assert!(frame.pixels[8].r > 0.9);
    }

    #[test]
    fn precomp_uses_hash_cache_and_invalidates_on_edit() {
        let tmp = tempdir().unwrap();
        let cache = PrecompCacheManager::new(tmp.path().join("precomp"), 1024 * 1024).unwrap();
        let settings = ProjectSettings {
            width: 16,
            height: 16,
            fps: FrameRate::FPS_60,
            ..Default::default()
        };
        let mut project = Project::new("CacheTest", settings);
        let mut bus = CommandBus::new();

        let rect = TimelineItem::new(
            "Rect",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 16.0,
                height: 16.0,
                color: RgbaColor::WHITE,
            },
        );
        let rect_id = rect.id;
        let precomp_scene_id = Uuid::new_v4();
        let precomp_item_id = Uuid::new_v4();

        bus.execute(
            &mut project,
            EditCommand::AddObject {
                layer_index: 0,
                item: rect,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();
        bus.execute(
            &mut project,
            EditCommand::PrecomposeSelection {
                item_ids: vec![rect_id],
                new_scene_id: precomp_scene_id,
                precomp_name: "Comp1".into(),
                precomp_item_id,
                target_layer_index: 0,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        let main_id = project.active_scene_id;
        let mut graph = RenderGraph::new().with_disk_cache(cache);

        let f1 = graph
            .render_project_scene(&project, main_id, RationalTime::ZERO, false)
            .unwrap();
        assert!(f1.pixels[0].r > 0.9);
        assert_eq!(graph.cache_misses, 1);
        assert_eq!(graph.cache_hits, 0);

        let _f2 = graph
            .render_project_scene(&project, main_id, RationalTime::ZERO, false)
            .unwrap();
        assert_eq!(graph.cache_hits, 1);

        project.scenes[1].layers[0].items[0].transform.opacity = 0.5;
        let f3 = graph
            .render_project_scene(&project, main_id, RationalTime::ZERO, false)
            .unwrap();
        assert_eq!(graph.cache_misses, 2);
        assert!((f3.pixels[0].r - 0.5).abs() < 0.05);
    }

    #[test]
    fn character_sprite_switches_png_by_volume_and_vowel_lipsync() {
        let tmp = tempdir().unwrap();
        let closed_png = tmp.path().join("closed.png");
        let open_png = tmp.path().join("open.png");
        let a_png = tmp.path().join("a.png");
        // Closed = 青, Open = 赤, A = 緑
        write_solid_png(&closed_png, [0, 0, 255, 255]);
        write_solid_png(&open_png, [255, 0, 0, 255]);
        write_solid_png(&a_png, [0, 255, 0, 255]);

        let mut png_paths = BTreeMap::new();
        png_paths.insert(MouthShape::Closed, closed_png);
        png_paths.insert(MouthShape::Open, open_png);
        png_paths.insert(MouthShape::A, a_png);

        let audio_asset = Uuid::new_v4();
        let mut scene = Scene::new("LipSync", 32, 32, FrameRate::FPS_60);
        let mut audio_item = TimelineItem::new(
            "Voice",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Audio {
                asset_id: audio_asset,
                source_offset: RationalTime::ZERO,
                volume: 1.0,
            },
        );
        audio_item.phoneme_segments = vec![PhonemeSegment {
            start: RationalTime::from_seconds(0.5),
            end: RationalTime::from_seconds(1.5),
            phoneme: "a".into(),
            mouth: MouthShape::A,
        }];
        scene.layers[3].items.push(audio_item);

        // 1) VolumeRms モード検証: 0.0s..1.0s は無音 (rms=0.0)、1.0s..2.0s は発話 (rms=0.5)
        let mut graph = RenderGraph::new();
        graph.register_waveform(
            audio_asset,
            WaveformPeaks {
                sample_rate: 48_000,
                samples_per_bucket: 48_000,
                buckets: vec![
                    WaveformBucket {
                        min: 0.0,
                        max: 0.0,
                        rms: 0.0,
                    },
                    WaveformBucket {
                        min: -0.5,
                        max: 0.5,
                        rms: 0.5,
                    },
                ],
            },
        );

        let sprite_rms = TimelineItem::new(
            "Tachie",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::CharacterSprite {
                config: CharacterSpriteConfig {
                    linked_audio_layer: Some(3),
                    mode: LipSyncMode::VolumeRms {
                        closed_threshold: 0.05,
                        open_threshold: 0.2,
                    },
                    png_paths: png_paths.clone(),
                    psd_config: None,
                    phoneme_override: vec![],
                },
                width: 32.0,
                height: 32.0,
            },
        );
        scene.layers[0].items.push(sprite_rms);

        let silent_frame = graph.render_scene(&scene, RationalTime::from_seconds(0.2));
        assert!(silent_frame.pixels[0].b > 0.9); // Closed (青)
        let loud_frame = graph.render_scene(&scene, RationalTime::from_seconds(1.2));
        assert!(loud_frame.pixels[0].r > 0.9); // Open (赤)

        // 2) VowelPhoneme モード検証: 1.0s は母音 "a" (緑)
        if let ItemKind::CharacterSprite { config, .. } = &mut scene.layers[0].items[0].kind {
            config.mode = LipSyncMode::VowelPhoneme;
        }
        let vowel_frame = graph.render_scene(&scene, RationalTime::from_seconds(1.0));
        assert!(vowel_frame.pixels[0].g > 0.9); // A (緑)
    }

    #[test]
    fn camera_3d_depth_sorts_closer_layer_in_front() {
        let mut scene = Scene::new("Cam3D", 32, 32, FrameRate::FPS_60);
        scene.camera = Camera3D {
            enabled: true,
            position: [0.0, 0.0, -1000.0],
            target: [0.0, 0.0, 0.0],
            fov_deg: 50.0,
            dof_enabled: false,
            focus_distance: 1000.0,
            aperture: 0.0,
        };

        // Layer 0 (下レイヤー) だが 3D 空間では手前 (z = -200) にある赤い四角
        let mut near_red = TimelineItem::new(
            "NearRed",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 32.0,
                height: 32.0,
                color: RgbaColor {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
            },
        );
        near_red.transform_3d = Transform3D {
            enable_3d: true,
            z: -200.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
        };
        scene.layers[0].items.push(near_red);

        // Layer 1 (上レイヤー) だが 3D 空間では奥 (z = +200) にある青い四角
        let mut far_blue = TimelineItem::new(
            "FarBlue",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 32.0,
                height: 32.0,
                color: RgbaColor {
                    r: 0.0,
                    g: 0.0,
                    b: 1.0,
                    a: 1.0,
                },
            },
        );
        far_blue.transform_3d = Transform3D {
            enable_3d: true,
            z: 200.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
        };
        scene.layers[1].items.push(far_blue);

        let mut graph = RenderGraph::new();
        let frame = graph.render_scene(&scene, RationalTime::ZERO);
        let center = frame.pixels[16 * 32 + 16];
        // Zソートにより手前の赤が奥の青より前に描画される
        assert!(center.r > 0.9 && center.b < 0.1, "got {center:?}");
    }

    #[test]
    fn plugin_effect_executes_via_render_graph() {
        let mut scene = Scene::new("PluginTest", 16, 16, FrameRate::FPS_60);
        let mut item = TimelineItem::new(
            "White",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 16.0,
                height: 16.0,
                color: RgbaColor::WHITE,
            },
        );
        let mut params = BTreeMap::new();
        params.insert("strength".to_string(), 1.0);
        item.effect_chain.push(EffectNode {
            id: Uuid::new_v4(),
            effect_type: "plugin.vignette".to_string(),
            enabled: true,
            params,
        });
        scene.layers[0].items.push(item);

        let mut graph = RenderGraph::new();
        let frame = graph.render_scene(&scene, RationalTime::ZERO);
        assert!(frame.pixels[8 * 16 + 8].r > frame.pixels[0].r);
    }

    #[test]
    fn sliding_window_box_blur_matches_naive_reference() {
        let (w, h) = (13u32, 11u32);
        let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
        for y in 0..h {
            for x in 0..w {
                img.pixels[(y * w + x) as usize] = LinearPremul {
                    r: ((x * 7 + y * 3) % 11) as f32 / 10.0,
                    g: ((x * 5 + y * 13) % 17) as f32 / 16.0,
                    b: ((x + y * 2) % 9) as f32 / 8.0,
                    a: 1.0,
                };
            }
        }
        let orig = img.clone();

        for radius in [1, 3, 6] {
            let mut fast = orig.clone();
            box_blur_separable(&mut fast, radius);

            // ナイーブ参照実装との厳密一致検証
            let denom = (radius * 2 + 1) as f64;
            let mut tmp = vec![LinearPremul::TRANSPARENT; orig.pixels.len()];
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
                    for dx in -radius..=radius {
                        let p =
                            orig.pixels[(y * w as i32 + (x + dx).clamp(0, w as i32 - 1)) as usize];
                        r += f64::from(p.r);
                        g += f64::from(p.g);
                        b += f64::from(p.b);
                        a += f64::from(p.a);
                    }
                    tmp[(y * w as i32 + x) as usize] = LinearPremul {
                        r: (r / denom) as f32,
                        g: (g / denom) as f32,
                        b: (b / denom) as f32,
                        a: (a / denom) as f32,
                    };
                }
            }
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
                    for dy in -radius..=radius {
                        let p = tmp[((y + dy).clamp(0, h as i32 - 1) * w as i32 + x) as usize];
                        r += f64::from(p.r);
                        g += f64::from(p.g);
                        b += f64::from(p.b);
                        a += f64::from(p.a);
                    }
                    let got = fast.pixels[(y * w as i32 + x) as usize];
                    assert!((got.r - (r / denom) as f32).abs() < 1e-5);
                    assert!((got.g - (g / denom) as f32).abs() < 1e-5);
                    assert!((got.b - (b / denom) as f32).abs() < 1e-5);
                    assert!((got.a - (a / denom) as f32).abs() < 1e-5);
                }
            }
        }
    }

    #[test]
    fn mem_precomp_cache_is_bounded_by_lru_cap() {
        let mut graph = RenderGraph::new();
        for i in 0..(MAX_MEM_PRECOMP_FRAMES + 10) {
            graph.insert_mem_precomp(
                format!("key_{i:04}"),
                LinearImage::new(2, 2, LinearPremul::TRANSPARENT),
            );
        }
        assert_eq!(graph.mem_precomp_cache_len(), MAX_MEM_PRECOMP_FRAMES);
    }

    #[test]
    fn renders_image_and_video_assets_with_proxy_and_frame_cache() {
        use ve_core::{AssetRef, MediaKind};
        use ve_media::{ExportConfig, ExportFormat, VideoEncoder};

        let tmp = tempdir().unwrap();
        let png_path = tmp.path().join("overlay_red.png");
        write_solid_png(&png_path, [255, 0, 0, 255]);

        let cli = FfmpegCli::default();
        let write_solid_mp4 = |path: &std::path::Path, rgba: [u8; 4]| {
            let cfg = ExportConfig {
                width: 32,
                height: 32,
                fps: FrameRate::FPS_30,
                format: ExportFormat::Mp4H264,
                crf: 18,
                encoder: VideoEncoder::Software,
            };
            let mut session = cli.start_export(path, &cfg, None).unwrap();
            let frame: Vec<u8> = rgba.into_iter().cycle().take(32 * 32 * 4).collect();
            for _ in 0..10 {
                session.write_rgba8_frame(&frame).unwrap();
            }
            session.finish().unwrap();
        };

        let orig_mp4 = tmp.path().join("orig_blue.mp4");
        let proxy_mp4 = tmp.path().join("proxy_green.mp4");
        write_solid_mp4(&orig_mp4, [0, 0, 255, 255]);
        write_solid_mp4(&proxy_mp4, [0, 255, 0, 255]);

        let settings = ProjectSettings {
            width: 32,
            height: 32,
            fps: FrameRate::FPS_30,
            ..Default::default()
        };
        let mut project = Project::new("MediaRender", settings);
        let vid_id = Uuid::new_v4();
        let img_id = Uuid::new_v4();
        project.assets.push(AssetRef {
            id: vid_id,
            name: "orig_blue.mp4".into(),
            kind: MediaKind::Video,
            relative_path: None,
            fallback_absolute_path: Some(orig_mp4),
            collected: false,
        });
        project.assets.push(AssetRef {
            id: img_id,
            name: "overlay_red.png".into(),
            kind: MediaKind::Image,
            relative_path: None,
            fallback_absolute_path: Some(png_path),
            collected: false,
        });

        // Layer 0: フルサイズ動画、Layer 1: 左上 8x8 の赤い画像
        project.active_scene_mut().unwrap().layers[0]
            .items
            .push(TimelineItem::new(
                "VideoClip",
                RationalTime::ZERO,
                RationalTime::from_seconds(1.0),
                ItemKind::Video {
                    asset_id: vid_id,
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            ));
        project.active_scene_mut().unwrap().layers[1]
            .items
            .push(TimelineItem::new(
                "ImageClip",
                RationalTime::ZERO,
                RationalTime::from_seconds(1.0),
                ItemKind::Image { asset_id: img_id },
            ));

        let main_id = project.active_scene_id;
        let mut graph = RenderGraph::new();

        // 1) 原本レンダリング: 左上(2,2)は赤いPNG、右下(24,24)は青い動画
        let f1 = graph
            .render_project_scene(&project, main_id, RationalTime::from_seconds(0.1), false)
            .unwrap();
        assert!(f1.pixels[2 * 32 + 2].r > 0.8);
        assert!(f1.pixels[24 * 32 + 24].b > 0.6);
        assert_eq!(graph.video_frame_cache_len(), 1);

        // 2) プロキシ登録＋プレビューモード (use_proxies = true) -> 緑のプロキシから描画
        graph.register_proxy(vid_id, proxy_mp4);
        let f_proxy = graph
            .render_project_scene(&project, main_id, RationalTime::from_seconds(0.1), false)
            .unwrap();
        assert!(f_proxy.pixels[24 * 32 + 24].g > 0.6);

        // 3) 書き出しモード (use_proxies = false) -> 青い原本へ自動切替
        graph.use_proxies = false;
        let f_export = graph
            .render_project_scene(&project, main_id, RationalTime::from_seconds(0.1), false)
            .unwrap();
        assert!(f_export.pixels[24 * 32 + 24].b > 0.6);
    }

    #[test]
    fn scene_change_wipe_and_layer_offset_work() {
        let mut scene = Scene::new("TransTest", 32, 16, FrameRate::FPS_60);
        scene.layers[0].offset_x = 4.0;
        scene.layers[0].items.push(TimelineItem::new(
            "White",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 28.0,
                height: 16.0,
                color: RgbaColor::WHITE,
            },
        ));
        scene.layers[1].items.push(TimelineItem::new(
            "WipeLeft",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Transition {
                transition: TransitionKind::WipeLeft,
                scope: FilterTargetScope::ImmediateLayer,
                softness: 0.02,
            },
        ));

        let mut graph = RenderGraph::new();
        // t=1.0s => progress=0.5: x=0..4 is transparent due to layer.offset_x=4, x=6 is visible, x=28 is wiped out
        let frame =
            graph.render_scene_inner(None, &scene, RationalTime::from_seconds(1.0), true, 0);
        assert!(frame.pixels[2].a < 0.05);
        assert!(frame.pixels[6].r > 0.8);
        assert!(frame.pixels[28].a < 0.05);
    }

    #[test]
    fn chroma_key_removes_green_and_preserves_red() {
        let mut img = LinearImage::new(8, 4, LinearPremul::TRANSPARENT);
        let green = LinearPremul::from_srgb_straight(RgbaColor {
            r: 0.0,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        });
        let red = LinearPremul::from_srgb_straight(RgbaColor {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        });
        for y in 0..4 {
            for x in 0..8 {
                img.pixels[(y * 8 + x) as usize] = if x < 4 { green } else { red };
            }
        }
        let plugins = PluginRegistry::new();
        let params = BTreeMap::new();
        apply_named_effect(&mut img, "chroma_key", &params, 0.0, &plugins);
        assert!(img.pixels[1].a < 0.05, "green should be keyed out");
        assert!(img.pixels[6].a > 0.95 && img.pixels[6].r > 0.9);
    }

    #[test]
    fn mosaic_and_rounded_clip_modify_pixels_cleanly() {
        let mut img = LinearImage::new(16, 16, LinearPremul::from_srgb_straight(RgbaColor::WHITE));
        let plugins = PluginRegistry::new();
        let mut rparams = BTreeMap::new();
        rparams.insert("radius".into(), 6.0);
        apply_named_effect(&mut img, "rounded_clip", &rparams, 0.0, &plugins);
        // Corner pixel (0,0) clipped transparent, center (8,8) remains opaque white
        assert!(img.pixels[0].a < 0.05);
        assert!(img.pixels[8 * 16 + 8].a > 0.95);

        let mut mparams = BTreeMap::new();
        mparams.insert("block_size".into(), 4.0);
        apply_named_effect(&mut img, "mosaic", &mparams, 0.0, &plugins);
        // All pixels inside top-left 4x4 block become identical
        assert_eq!(img.pixels[0], img.pixels[3 * 16 + 3]);
    }

    #[test]
    fn rotate_linear_image_rotates_around_center_cleanly() {
        let mut img = LinearImage::new(20, 10, LinearPremul::TRANSPARENT);
        img.pixels.fill(LinearPremul::from_srgb_straight(RgbaColor {
            r: 1.0,
            g: 0.2,
            b: 0.1,
            a: 1.0,
        }));
        let rotated = rotate_linear_image(&img, 90.0);
        assert!(rotated.width >= 10 && rotated.height >= 20);
        let center =
            rotated.pixels[((rotated.height / 2) * rotated.width + rotated.width / 2) as usize];
        assert!(center.a > 0.9 && center.r > 0.9);
    }

    #[test]
    fn scaled_project_scene_render_matches_full_composition_coords() {
        let mut proj = Project::new(
            "ScaledPreview",
            ve_core::ProjectSettings {
                width: 200,
                height: 100,
                ..Default::default()
            },
        );
        // 右側領域 (x=140..180, y=40..80) に赤い矩形を配置
        let mut box_item = TimelineItem::new(
            "RightBox",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 40.0,
                height: 40.0,
                color: RgbaColor {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
            },
        );
        box_item.transform.x = 140.0;
        box_item.transform.y = 40.0;
        proj.active_scene_mut().unwrap().layers[0]
            .items
            .push(box_item);

        let mut rg = RenderGraph::new();
        // 1/2 スケール (100x50) で描画しても右側ボックスが (70..90, 20..40) に正しく収まる
        let scaled = rg
            .render_project_scene_scaled(
                &proj,
                proj.active_scene_id,
                RationalTime::ZERO,
                100,
                50,
                false,
            )
            .unwrap();
        assert_eq!((scaled.width, scaled.height), (100, 50));
        let px = scaled.pixels[(30 * 100 + 80) as usize];
        assert!(
            px.r > 0.9,
            "expected red pixel at scaled (80,30), got {px:?}"
        );
    }
}
