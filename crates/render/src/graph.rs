use crate::color::{LinearImage, LinearPremul};
use crate::text::TextRenderer;
use std::collections::{BTreeMap, HashMap};
use ve_core::{
    BlendMode, EffectNode, FilterTargetScope, ItemKind, PrecompCacheManager, Project, RationalTime,
    RgbaColor, Scene, SceneId, ShapeKind, TimelineItem, Transform2D, content_hash_hex,
};

/// 組み込み WGSL エフェクトチェーンシェーダーソース
pub const EFFECTS_WGSL_SOURCE: &str = include_str!("shaders/effects.wgsl");

/// 仕様書 2.2 / 4.2 の評価順序に従う Render Graph 評価器:
/// 1. 各オブジェクトのキーフレーム評価・個別エフェクトチェーン適用（Precomp はハッシュキャッシュ利用）
/// 2. グループ制御・トラックマット・AviUtl2風フィルタオブジェクト（指定レイヤー範囲）適用
/// 3. レイヤー合成（Linear sRGB + Premultiplied Alpha + BlendMode）
pub struct RenderGraph {
    text_renderer: TextRenderer,
    mem_precomp_cache: HashMap<String, LinearImage>,
    disk_precomp_cache: Option<PrecompCacheManager>,
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
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    pub fn with_disk_cache(mut self, manager: PrecompCacheManager) -> Self {
        self.disk_precomp_cache = Some(manager);
        self
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
        let mut layer_outputs: Vec<Option<(LinearImage, BlendMode)>> =
            vec![None; scene.layers.len()];
        let mut filter_passes: Vec<(usize, String, FilterTargetScope, BTreeMap<String, f32>)> =
            Vec::new();
        let mut group_passes: Vec<(usize, usize, Transform2D)> = Vec::new();

        // Step 1: 各レイヤーのアクティブオブジェクトを描画し、オブジェクト個別エフェクトチェーンを適用
        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            if !layer.visible {
                continue;
            }
            let mut layer_buf: Option<LinearImage> = None;
            let mut layer_blend = BlendMode::Normal;

            for item in &layer.items {
                if !item.contains(time) {
                    continue;
                }
                let tf = item.eval_transform(time);
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
                            self.render_item(project, item, &tf, time, w, h, depth)
                        {
                            apply_effect_chain(&mut item_img, &item.effect_chain);
                            let dst = layer_buf.get_or_insert_with(|| {
                                LinearImage::new(w, h, LinearPremul::TRANSPARENT)
                            });
                            dst.blend_with_mode(
                                &item_img,
                                tf.x.round() as i32,
                                tf.y.round() as i32,
                                tf.opacity,
                                item.blend_mode,
                            );
                            layer_blend = item.blend_mode;
                        }
                    }
                }
            }

            if let Some(buf) = layer_buf {
                layer_outputs[layer_idx] = Some((buf, layer_blend));
            }
        }

        // トラックマット適用（マット元レイヤーは合成対象から非表示化し、対象レイヤーを切り抜く）
        let mut consumed_matte_layers = vec![false; scene.layers.len()];
        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            if let Some(matte) = layer.track_matte
                && matte.source_layer_index < scene.layers.len()
                && matte.source_layer_index != layer_idx
            {
                let matte_img_opt = layer_outputs[matte.source_layer_index]
                    .as_ref()
                    .map(|(img, _)| img.clone());
                if let (Some(matte_img), Some((target_img, _))) =
                    (matte_img_opt, layer_outputs[layer_idx].as_mut())
                {
                    target_img.apply_track_matte(&matte_img, matte.mode);
                    consumed_matte_layers[matte.source_layer_index] = true;
                }
            }
        }

        // グループ制御適用（指定レイヤー数分の位置・拡大率・不透明度を一括変換）
        for (group_layer_idx, target_count, tf) in &group_passes {
            let start = group_layer_idx.saturating_sub(*target_count);
            for target in &mut layer_outputs[start..*group_layer_idx] {
                if let Some((img, _)) = target.as_mut() {
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
                if let Some((img, _)) = layer_outputs[target_idx].as_mut() {
                    apply_named_effect(img, effect_type, params);
                }
            }
        }

        // 最終レイヤー合成（Linear sRGB + Premultiplied Alpha）
        let bg = if transparent_bg {
            LinearPremul::TRANSPARENT
        } else {
            LinearPremul::from_srgb_straight(RgbaColor::BLACK)
        };
        let mut canvas = LinearImage::new(w, h, bg);
        for (idx, layer_out) in layer_outputs.into_iter().enumerate() {
            if consumed_matte_layers[idx] {
                continue;
            }
            if let Some((layer_img, blend_mode)) = layer_out {
                canvas.blend_with_mode(&layer_img, 0, 0, 1.0, blend_mode);
            }
        }

        canvas
    }

    #[allow(clippy::too_many_arguments)]
    fn render_item(
        &mut self,
        project: Option<&Project>,
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
            ItemKind::Precomp { scene_id } => {
                if depth >= 4 {
                    return None;
                }
                let proj = project?;
                let nested = proj.scene(*scene_id)?;
                let mut scaled_nested = nested.clone();
                scaled_nested.width = canvas_w;
                scaled_nested.height = canvas_h;

                // コンテンツハッシュキー生成（シーン定義＋解像度＋ローカルフレーム番号）
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

fn apply_effect_chain(image: &mut LinearImage, chain: &[EffectNode]) {
    for node in chain {
        if node.enabled {
            apply_named_effect(image, &node.effect_type, &node.params);
        }
    }
}

fn apply_named_effect(image: &mut LinearImage, effect_type: &str, params: &BTreeMap<String, f32>) {
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
        _ => {}
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

    // 水平パス
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

    // 垂直パス
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
    use tempfile::tempdir;
    use uuid::Uuid;
    use ve_core::{
        CommandBus, CommandSource, EditCommand, FrameRate, ProjectSettings, TrackMatte,
        TrackMatteMode,
    };

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
        // Layer 0: 全面白
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
        // Layer 1: 左半分 (8x16) だけ不透明なマット
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
        // 左半分 (x=2) は白、右半分 (x=12) はマットで切り抜かれて黒
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
        // 元の (0,0) は黒、グループ移動後の (8,0) は白
        assert!(frame.pixels[0].r < 0.05);
        assert!(frame.pixels[8].r > 0.9);
    }

    #[test]
    fn precomp_uses_hash_cache_and_invalidates_on_edit() {
        let tmp = tempdir().unwrap();
        let cache = PrecompCacheManager::new(tmp.path().join("precomp"), 1024 * 1024).unwrap();
        let mut settings = ProjectSettings {
            width: 16,
            height: 16,
            ..Default::default()
        };
        settings.fps = FrameRate::FPS_60;
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

        // 1回目: キャッシュミス -> 計算して保存
        let f1 = graph
            .render_project_scene(&project, main_id, RationalTime::ZERO, false)
            .unwrap();
        assert!(f1.pixels[0].r > 0.9);
        assert_eq!(graph.cache_misses, 1);
        assert_eq!(graph.cache_hits, 0);

        // 2回目: 同じフレーム -> ハッシュ一致でキャッシュヒット
        let _f2 = graph
            .render_project_scene(&project, main_id, RationalTime::ZERO, false)
            .unwrap();
        assert_eq!(graph.cache_hits, 1);

        // Precomp 内アイテムの透明度を変更 -> ハッシュが変わり自動無効化（再計算）
        project.scenes[1].layers[0].items[0].transform.opacity = 0.5;
        let f3 = graph
            .render_project_scene(&project, main_id, RationalTime::ZERO, false)
            .unwrap();
        assert_eq!(graph.cache_misses, 2);
        assert!((f3.pixels[0].r - 0.5).abs() < 0.05);
    }
}
