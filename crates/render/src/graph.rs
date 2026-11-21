use crate::color::{LinearImage, LinearPremul};
use crate::text::TextRenderer;
use std::collections::BTreeMap;
use ve_core::{
    EffectNode, FilterTargetScope, ItemKind, RationalTime, RgbaColor, Scene, ShapeKind,
    TimelineItem,
};

/// 仕様書 2.2 / 4.2 の評価順序に従う Render Graph 評価器:
/// 1. 各オブジェクトの個別エフェクトチェーン適用
/// 2. レイヤー合成（Linear sRGB + Premultiplied Alpha）
/// 3. タイムライン上の AviUtl2風フィルタオブジェクト（指定レイヤー範囲）適用
pub struct RenderGraph {
    text_renderer: TextRenderer,
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
        }
    }

    pub fn render_scene(&mut self, scene: &Scene, time: RationalTime) -> LinearImage {
        let w = scene.width;
        let h = scene.height;
        let mut layer_outputs: Vec<Option<LinearImage>> = vec![None; scene.layers.len()];
        let mut filter_passes: Vec<(usize, String, FilterTargetScope, BTreeMap<String, f32>)> =
            Vec::new();

        // Step 1: 各レイヤーのアクティブオブジェクトを描画し、オブジェクト個別エフェクトチェーンを適用
        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            if !layer.visible {
                continue;
            }
            let mut layer_buf: Option<LinearImage> = None;

            for item in &layer.items {
                if !item.contains(time) {
                    continue;
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
                    _ => {
                        if let Some(mut item_img) = self.render_item(item, w, h) {
                            apply_effect_chain(&mut item_img, &item.effect_chain);
                            let dst = layer_buf
                                .get_or_insert_with(|| LinearImage::new(w, h, LinearPremul::TRANSPARENT));
                            dst.blend_over(
                                &item_img,
                                item.transform.x.round() as i32,
                                item.transform.y.round() as i32,
                                item.transform.opacity,
                            );
                        }
                    }
                }
            }

            layer_outputs[layer_idx] = layer_buf;
        }

        // Step 3 (範囲指定フィルタオブジェクト): 各フィルタが対象とする直上/範囲レイヤーへ適用
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
                if let Some(img) = layer_outputs[target_idx].as_mut() {
                    apply_named_effect(img, effect_type, params);
                }
            }
        }

        // Step 2: 背景（黒・不透明）の上に全レイヤーを Linear Premultiplied Alpha で合成
        let mut canvas = LinearImage::new(w, h, LinearPremul::from_srgb_straight(RgbaColor::BLACK));
        for layer_img in layer_outputs.into_iter().flatten() {
            canvas.blend_over(&layer_img, 0, 0, 1.0);
        }

        canvas
    }

    fn render_item(&mut self, item: &TimelineItem, canvas_w: u32, canvas_h: u32) -> Option<LinearImage> {
        match &item.kind {
            ItemKind::Shape {
                shape,
                width,
                height,
                color,
            } => {
                let sw = (*width * item.transform.scale_x).round().max(1.0) as u32;
                let sh = (*height * item.transform.scale_y).round().max(1.0) as u32;
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
            } => Some(self.text_renderer.render_text(
                content,
                font_family,
                *font_size * item.transform.scale_y.max(0.1),
                *color,
                canvas_w,
                canvas_h,
            )),
            ItemKind::Video { .. }
            | ItemKind::Audio { .. }
            | ItemKind::Image { .. }
            | ItemKind::Precomp { .. }
            | ItemKind::FilterObject { .. } => None,
        }
    }
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
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use ve_core::{FrameRate, Transform2D};

    #[test]
    fn filter_object_applies_to_immediate_layer_distinct_from_effect_chain() {
        let mut scene = Scene::new("Test", 16, 16, FrameRate::FPS_60);
        // Layer 0: 白い矩形（個別エフェクトなし）
        scene.layers[0].items.push(TimelineItem {
            id: Uuid::new_v4(),
            name: "Rect".into(),
            start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            transform: Transform2D::default(),
            effect_chain: vec![],
            kind: ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 16.0,
                height: 16.0,
                color: RgbaColor::WHITE,
            },
        });
        // Layer 1: AviUtl2風フィルタオブジェクト（直上Layer 0へ brightness=0.5 適用）
        let mut params = BTreeMap::new();
        params.insert("gain".to_string(), 0.5);
        scene.layers[1].items.push(TimelineItem {
            id: Uuid::new_v4(),
            name: "DimFilter".into(),
            start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            transform: Transform2D::default(),
            effect_chain: vec![],
            kind: ItemKind::FilterObject {
                effect_type: "brightness".into(),
                scope: FilterTargetScope::ImmediateLayer,
                params,
            },
        });

        let mut graph = RenderGraph::new();
        let frame = graph.render_scene(&scene, RationalTime::ZERO);
        // Linear 空間で 1.0 * 0.5 = 0.5 になっていること
        assert!((frame.pixels[0].r - 0.5).abs() < 1e-4);
    }
}
