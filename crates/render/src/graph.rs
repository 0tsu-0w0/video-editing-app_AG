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
    BlendMode, CharacterSpriteConfig, EffectNode, FilterTargetScope, ItemKind, LipSyncMode,
    MouthShape, PrecompCacheManager, Project, RationalTime, RgbaColor, Scene, SceneId, ShapeKind,
    TimelineItem, Transform2D, content_hash_hex, evaluate_vowel_mouth, resolve_mouth_png,
    rms_to_mouth,
};
use ve_plugin_api::PluginRegistry;

/// 組み込み WGSL エフェクトチェーンシェーダーソース
pub const EFFECTS_WGSL_SOURCE: &str = include_str!("shaders/effects.wgsl");

/// 仕様書 2.2 / 4.2 の評価順序に従う Render Graph 評価器:
/// 1. 各オブジェクトのキーフレーム評価・立ち絵口パク(PNG/PSD)・3Dカメラ透視投影/DoF・個別エフェクト＆プラグイン適用
/// 2. トラックマット・グループ制御・AviUtl2風フィルタオブジェクト（指定レイヤー範囲）適用
/// 3. レイヤー合成（Linear sRGB + Premultiplied Alpha + BlendMode + 3D Zソート）
pub struct RenderGraph {
    text_renderer: TextRenderer,
    mem_precomp_cache: HashMap<String, LinearImage>,
    disk_precomp_cache: Option<PrecompCacheManager>,
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
            disk_precomp_cache: None,
            png_cache: HashMap::new(),
            psd_cache: HashMap::new(),
            asset_waveforms: HashMap::new(),
            plugins: PluginRegistry::with_builtins(),
            hot_shaders: ShaderHotReloader::new(),
            cache_hits: 0,
            cache_misses: 0,
        }
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

                            let (mut draw_x, mut draw_y) = (tf.x, tf.y);
                            if scene.camera.enabled && tf3d.enable_3d {
                                let cx = w as f32 * 0.5;
                                let cy = h as f32 * 0.5;
                                let item_cx = tf.x + item_img.width as f32 * 0.5 - cx;
                                let item_cy = tf.y + item_img.height as f32 * 0.5 - cy;
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

        // トラックマット適用
        let mut consumed_matte_layers = vec![false; scene.layers.len()];
        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            if let Some(matte) = layer.track_matte
                && matte.source_layer_index < scene.layers.len()
                && matte.source_layer_index != layer_idx
            {
                let matte_img_opt = layer_outputs[matte.source_layer_index]
                    .as_ref()
                    .map(|(img, _, _)| img.clone());
                if let (Some(matte_img), Some((target_img, _, _))) =
                    (matte_img_opt, layer_outputs[layer_idx].as_mut())
                {
                    target_img.apply_track_matte(&matte_img, matte.mode);
                    consumed_matte_layers[matte.source_layer_index] = true;
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
            let target_indices: Vec<usize> = match *scope {
                FilterTargetScope::ImmediateLayer => {
                    if *filter_layer_idx > 0 {
                        vec![filter_layer_idx - 1]
                    } else {
                        vec![]
                    }
                }
                FilterTargetScope::LayerRange { count } => {
                    let start = filter_layer_idx.saturating_sub(count);
                    (start..*filter_layer_idx).collect()
                }
                FilterTargetScope::AllRemaining => (0..*filter_layer_idx).collect(),
            };

            for target_idx in target_indices {
                if let Some((img, _, _)) = layer_outputs[target_idx].as_mut() {
                    apply_named_effect(img, effect_type, params, time.as_seconds(), &self.plugins);
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
            } => Some(self.text_renderer.render_text_animated(
                content,
                font_family,
                *font_size * tf.scale_y.max(0.1),
                *color,
                animator,
                local_time.as_seconds() as f32,
                canvas_w,
                canvas_h,
            )),
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

                if let Some(cached) = self.mem_precomp_cache.get(&cache_key) {
                    self.cache_hits += 1;
                    return Some(cached.clone());
                }
                if let Some(disk) = self.disk_precomp_cache.as_mut()
                    && let Ok(Some(bytes)) = disk.get(&cache_key)
                    && let Some(img) = LinearImage::from_rgba16f_bytes(canvas_w, canvas_h, &bytes)
                {
                    self.cache_hits += 1;
                    self.mem_precomp_cache.insert(cache_key, img.clone());
                    return Some(img);
                }

                self.cache_misses += 1;
                let rendered =
                    self.render_scene_inner(project, &scaled_nested, local_time, true, depth + 1);
                if let Some(disk) = self.disk_precomp_cache.as_mut() {
                    let _ = disk.put(&cache_key, &rendered.to_rgba16f_bytes());
                }
                self.mem_precomp_cache.insert(cache_key, rendered.clone());
                Some(rendered)
            }
            ItemKind::Video { .. }
            | ItemKind::Audio { .. }
            | ItemKind::Image { .. }
            | ItemKind::FilterObject { .. }
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

fn box_blur_separable(image: &mut LinearImage, radius: i32) {
    let w = image.width as i32;
    let h = image.height as i32;
    if w == 0 || h == 0 {
        return;
    }
    let mut temp = vec![LinearPremul::TRANSPARENT; image.pixels.len()];
    let denom = (radius * 2 + 1) as f32;

    for y in 0..h {
        for x in 0..w {
            let mut acc = LinearPremul::TRANSPARENT;
            for dx in -radius..=radius {
                let sx = (x + dx).clamp(0, w - 1);
                let p = image.pixels[(y * w + sx) as usize];
                acc.r += p.r;
                acc.g += p.g;
                acc.b += p.b;
                acc.a += p.a;
            }
            temp[(y * w + x) as usize] = LinearPremul {
                r: acc.r / denom,
                g: acc.g / denom,
                b: acc.b / denom,
                a: acc.a / denom,
            };
        }
    }

    for y in 0..h {
        for x in 0..w {
            let mut acc = LinearPremul::TRANSPARENT;
            for dy in -radius..=radius {
                let sy = (y + dy).clamp(0, h - 1);
                let p = temp[(sy * w + x) as usize];
                acc.r += p.r;
                acc.g += p.g;
                acc.b += p.b;
                acc.a += p.a;
            }
            image.pixels[(y * w + x) as usize] = LinearPremul {
                r: acc.r / denom,
                g: acc.g / denom,
                b: acc.b / denom,
                a: acc.a / denom,
            };
        }
    }
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
}
