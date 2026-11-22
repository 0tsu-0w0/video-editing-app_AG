use crate::color::{LinearImage, LinearPremul};
use crate::graph::{apply_named_effect, rotate_linear_image};
use crate::sprite::scale_linear_image;
use std::collections::HashMap;
use ve_core::{
    BlendMode, ChannelSelector, CompositeNodeKind, NodeCompositeGraph, NodeId, RgbaColor,
};
use ve_plugin_api::PluginRegistry;

/// レイヤー描画結果と 3D 深度をまとめてノードグラフへ受け渡すパケット
#[derive(Debug, Clone)]
pub struct LayerRenderPacket {
    pub image: LinearImage,
    /// カメラ空間深度（2D レイヤー既定 1000.0、3D レイヤーはカメラからの距離）
    pub depth: f32,
}

impl LayerRenderPacket {
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            image: LinearImage::new(width.max(1), height.max(1), LinearPremul::TRANSPARENT),
            depth: 1000.0,
        }
    }
}

/// v2.0 #21: Fusion / Nuke スタイルの `NodeCompositeGraph` をトポロジカル順に評価し、最終合成フレームを生成する
pub fn evaluate_node_composite_graph(
    graph: &NodeCompositeGraph,
    layer_packets: &[Option<LayerRenderPacket>],
    width: u32,
    height: u32,
    time_sec: f64,
    plugins: &PluginRegistry,
    transparent_bg: bool,
) -> Option<LinearImage> {
    let order = graph.topological_order().ok()?;
    let output_id = graph.output_node_id()?;

    let mut cache: HashMap<NodeId, LayerRenderPacket> = HashMap::with_capacity(order.len());

    for nid in order {
        let Some(node) = graph.node(nid) else {
            continue;
        };

        // バイパス時: 主入力（"in" または "bg"）をそのままパススルー
        if node.bypassed {
            let passthrough = graph
                .upstream_for(nid, "in")
                .or_else(|| graph.upstream_for(nid, "bg"))
                .and_then(|up| cache.get(&up).cloned())
                .unwrap_or_else(|| LayerRenderPacket::empty(width, height));
            cache.insert(nid, passthrough);
            continue;
        }

        let packet = match &node.kind {
            CompositeNodeKind::LayerInput { layer_index } => layer_packets
                .get(*layer_index)
                .and_then(|o| o.clone())
                .unwrap_or_else(|| LayerRenderPacket::empty(width, height)),

            CompositeNodeKind::SolidColor { color } => LayerRenderPacket {
                image: LinearImage::new(
                    width.max(1),
                    height.max(1),
                    LinearPremul::from_srgb_straight(*color),
                ),
                depth: 1000.0,
            },

            CompositeNodeKind::Effect {
                effect_type,
                params,
            } => {
                let mut pkt = upstream_packet(graph, &cache, nid, "in", width, height);
                apply_named_effect(&mut pkt.image, effect_type, params, time_sec, plugins);
                pkt
            }

            CompositeNodeKind::ChannelSplit { channel } => {
                let src = upstream_packet(graph, &cache, nid, "in", width, height);
                LayerRenderPacket {
                    image: extract_channel(&src, *channel),
                    depth: src.depth,
                }
            }

            CompositeNodeKind::ChannelMerge => {
                let r_pkt = graph.upstream_for(nid, "r").and_then(|id| cache.get(&id));
                let g_pkt = graph.upstream_for(nid, "g").and_then(|id| cache.get(&id));
                let b_pkt = graph.upstream_for(nid, "b").and_then(|id| cache.get(&id));
                let a_pkt = graph.upstream_for(nid, "a").and_then(|id| cache.get(&id));
                let depth = r_pkt
                    .or(g_pkt)
                    .or(b_pkt)
                    .or(a_pkt)
                    .map(|p| p.depth)
                    .unwrap_or(1000.0);
                LayerRenderPacket {
                    image: merge_channels(
                        r_pkt.map(|p| &p.image),
                        g_pkt.map(|p| &p.image),
                        b_pkt.map(|p| &p.image),
                        a_pkt.map(|p| &p.image),
                        width,
                        height,
                    ),
                    depth,
                }
            }

            CompositeNodeKind::Transform {
                x,
                y,
                scale_x,
                scale_y,
                rotation_deg,
                opacity,
            } => {
                let src = upstream_packet(graph, &cache, nid, "in", width, height);
                let transformed = transform_packet_image(
                    &src.image,
                    width,
                    height,
                    *x,
                    *y,
                    *scale_x,
                    *scale_y,
                    *rotation_deg,
                    *opacity,
                );
                LayerRenderPacket {
                    image: transformed,
                    depth: src.depth,
                }
            }

            CompositeNodeKind::Merge {
                blend_mode,
                opacity,
            } => {
                let mut bg = upstream_packet(graph, &cache, nid, "bg", width, height);
                let fg_opt = graph.upstream_for(nid, "fg").and_then(|id| cache.get(&id));
                let mask_opt = graph
                    .upstream_for(nid, "mask")
                    .and_then(|id| cache.get(&id));
                if let Some(fg) = fg_opt {
                    let op = opacity.clamp(0.0, 1.0);
                    let fg_img = if let Some(mask) = mask_opt {
                        let mut masked = fg.image.clone();
                        for (p, m) in masked.pixels.iter_mut().zip(mask.image.pixels.iter()) {
                            let mv = m.a.max(m.luma()).clamp(0.0, 1.0);
                            p.r *= mv;
                            p.g *= mv;
                            p.b *= mv;
                            p.a *= mv;
                        }
                        masked
                    } else {
                        fg.image.clone()
                    };
                    bg.image.blend_with_mode(&fg_img, 0, 0, op, *blend_mode);
                    bg.depth = bg.depth.min(fg.depth);
                }
                bg
            }

            CompositeNodeKind::MaskCombine { mode } => {
                let mut src = upstream_packet(graph, &cache, nid, "in", width, height);
                if let Some(mask_id) = graph.upstream_for(nid, "mask")
                    && let Some(mask_pkt) = cache.get(&mask_id)
                {
                    src.image.apply_track_matte(&mask_pkt.image, *mode);
                }
                src
            }

            CompositeNodeKind::ColorCorrect {
                exposure,
                contrast,
                saturation,
            } => {
                let mut pkt = upstream_packet(graph, &cache, nid, "in", width, height);
                let exp_mul = 2.0_f32.powf(exposure.clamp(-4.0, 4.0));
                let con = contrast.clamp(0.1, 3.0);
                let sat = saturation.clamp(0.0, 3.0);
                for px in &mut pkt.image.pixels {
                    if px.a <= 1e-6 {
                        continue;
                    }
                    let inv = 1.0 / px.a;
                    let mut r = (px.r * inv) * exp_mul;
                    let mut g = (px.g * inv) * exp_mul;
                    let mut b = (px.b * inv) * exp_mul;
                    r = (r - 0.5) * con + 0.5;
                    g = (g - 0.5) * con + 0.5;
                    b = (b - 0.5) * con + 0.5;
                    let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    r = (l + (r - l) * sat).max(0.0);
                    g = (l + (g - l) * sat).max(0.0);
                    b = (l + (b - l) * sat).max(0.0);
                    px.r = r * px.a;
                    px.g = g * px.a;
                    px.b = b * px.a;
                }
                pkt
            }

            CompositeNodeKind::DepthFog {
                near,
                far,
                fog_color,
                strength,
            } => {
                let mut pkt = upstream_packet(graph, &cache, nid, "in", width, height);
                let span = (far - near).abs().max(1.0);
                let factor = (((pkt.depth - *near) / span).clamp(0.0, 1.0)
                    * strength.clamp(0.0, 1.0))
                .clamp(0.0, 1.0);
                if factor > 1e-4 {
                    let fog = LinearPremul::from_srgb_straight(*fog_color);
                    for px in &mut pkt.image.pixels {
                        if px.a <= 1e-6 {
                            continue;
                        }
                        let inv = 1.0 - factor;
                        px.r = px.r * inv + fog.r * px.a * factor;
                        px.g = px.g * inv + fog.g * px.a * factor;
                        px.b = px.b * inv + fog.b * px.a * factor;
                    }
                }
                pkt
            }

            CompositeNodeKind::Output => upstream_packet(graph, &cache, nid, "in", width, height),
        };

        cache.insert(nid, packet);
    }

    let final_pkt = cache.remove(&output_id)?;
    if transparent_bg {
        Some(final_pkt.image)
    } else {
        let mut bg = LinearImage::new(
            width.max(1),
            height.max(1),
            LinearPremul::from_srgb_straight(RgbaColor::BLACK),
        );
        bg.blend_with_mode(&final_pkt.image, 0, 0, 1.0, BlendMode::Normal);
        Some(bg)
    }
}

fn upstream_packet(
    graph: &NodeCompositeGraph,
    cache: &HashMap<NodeId, LayerRenderPacket>,
    node_id: NodeId,
    port: &str,
    width: u32,
    height: u32,
) -> LayerRenderPacket {
    graph
        .upstream_for(node_id, port)
        .and_then(|up| cache.get(&up).cloned())
        .unwrap_or_else(|| LayerRenderPacket::empty(width, height))
}

fn extract_channel(pkt: &LayerRenderPacket, channel: ChannelSelector) -> LinearImage {
    let src = &pkt.image;
    let mut out = LinearImage::new(src.width, src.height, LinearPremul::TRANSPARENT);
    let depth_norm = (pkt.depth / 2000.0).clamp(0.0, 1.0);
    for (dst, px) in out.pixels.iter_mut().zip(src.pixels.iter()) {
        match channel {
            ChannelSelector::Rgba => *dst = *px,
            ChannelSelector::Red => {
                let v = px.r.clamp(0.0, 1.0);
                *dst = LinearPremul {
                    r: v,
                    g: v,
                    b: v,
                    a: px.a,
                };
            }
            ChannelSelector::Green => {
                let v = px.g.clamp(0.0, 1.0);
                *dst = LinearPremul {
                    r: v,
                    g: v,
                    b: v,
                    a: px.a,
                };
            }
            ChannelSelector::Blue => {
                let v = px.b.clamp(0.0, 1.0);
                *dst = LinearPremul {
                    r: v,
                    g: v,
                    b: v,
                    a: px.a,
                };
            }
            ChannelSelector::Alpha => {
                let v = px.a.clamp(0.0, 1.0);
                *dst = LinearPremul {
                    r: v,
                    g: v,
                    b: v,
                    a: 1.0,
                };
            }
            ChannelSelector::Luma => {
                let v = px.luma();
                *dst = LinearPremul {
                    r: v,
                    g: v,
                    b: v,
                    a: px.a,
                };
            }
            ChannelSelector::Depth => {
                let v = if px.a > 1e-4 { depth_norm } else { 0.0 };
                *dst = LinearPremul {
                    r: v,
                    g: v,
                    b: v,
                    a: px.a,
                };
            }
        }
    }
    out
}

fn merge_channels(
    r_img: Option<&LinearImage>,
    g_img: Option<&LinearImage>,
    b_img: Option<&LinearImage>,
    a_img: Option<&LinearImage>,
    width: u32,
    height: u32,
) -> LinearImage {
    let w = width.max(1);
    let h = height.max(1);
    let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let len = (w * h) as usize;
    for i in 0..len {
        let r = r_img
            .and_then(|im| im.pixels.get(i))
            .map(|p| p.r)
            .unwrap_or(0.0);
        let g = g_img
            .and_then(|im| im.pixels.get(i))
            .map(|p| p.g)
            .unwrap_or(0.0);
        let b = b_img
            .and_then(|im| im.pixels.get(i))
            .map(|p| p.b)
            .unwrap_or(0.0);
        let a = a_img
            .and_then(|im| im.pixels.get(i))
            .map(|p| p.luma().max(p.a))
            .unwrap_or_else(|| {
                r_img
                    .or(g_img)
                    .or(b_img)
                    .and_then(|im| im.pixels.get(i))
                    .map(|p| p.a)
                    .unwrap_or(1.0)
            })
            .clamp(0.0, 1.0);
        out.pixels[i] = LinearPremul {
            r: r * a,
            g: g * a,
            b: b * a,
            a,
        };
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn transform_packet_image(
    src: &LinearImage,
    canvas_w: u32,
    canvas_h: u32,
    x: f32,
    y: f32,
    scale_x: f32,
    scale_y: f32,
    rotation_deg: f32,
    opacity: f32,
) -> LinearImage {
    let sx = scale_x.clamp(0.05, 8.0);
    let sy = scale_y.clamp(0.05, 8.0);
    let mut work = if (sx - 1.0).abs() > 1e-3 || (sy - 1.0).abs() > 1e-3 {
        let nw = ((src.width as f32) * sx).round().clamp(1.0, 4096.0) as u32;
        let nh = ((src.height as f32) * sy).round().clamp(1.0, 4096.0) as u32;
        scale_linear_image(src, nw, nh)
    } else {
        src.clone()
    };

    let mut dx = x + (canvas_w as f32 - work.width as f32) * 0.5;
    let mut dy = y + (canvas_h as f32 - work.height as f32) * 0.5;

    if rotation_deg.abs() > 0.05 {
        let ow = work.width as f32;
        let oh = work.height as f32;
        work = rotate_linear_image(&work, rotation_deg);
        dx += (ow - work.width as f32) * 0.5;
        dy += (oh - work.height as f32) * 0.5;
    }

    let mut dst = LinearImage::new(canvas_w.max(1), canvas_h.max(1), LinearPremul::TRANSPARENT);
    dst.blend_with_mode(
        &work,
        dx.round() as i32,
        dy.round() as i32,
        opacity.clamp(0.0, 1.0),
        BlendMode::Normal,
    );
    dst
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::{CompositeNode, TrackMatteMode};

    #[test]
    fn node_graph_channel_split_merge_and_depth_fog_evaluate_correctly() {
        let w = 16;
        let h = 16;
        let red_pkt = LayerRenderPacket {
            image: LinearImage::new(
                w,
                h,
                LinearPremul::from_srgb_straight(RgbaColor {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                }),
            ),
            depth: 1500.0,
        };
        let blue_pkt = LayerRenderPacket {
            image: LinearImage::new(
                w,
                h,
                LinearPremul::from_srgb_straight(RgbaColor {
                    r: 0.0,
                    g: 0.0,
                    b: 1.0,
                    a: 1.0,
                }),
            ),
            depth: 500.0,
        };

        let mut graph = NodeCompositeGraph {
            enabled: true,
            ..Default::default()
        };
        let l0 = graph.add_node(CompositeNode::new(
            "L0",
            [0.0, 0.0],
            CompositeNodeKind::LayerInput { layer_index: 0 },
        ));
        let l1 = graph.add_node(CompositeNode::new(
            "L1",
            [0.0, 100.0],
            CompositeNodeKind::LayerInput { layer_index: 1 },
        ));
        let split_r = graph.add_node(CompositeNode::new(
            "SplitR",
            [180.0, 0.0],
            CompositeNodeKind::ChannelSplit {
                channel: ChannelSelector::Red,
            },
        ));
        let fog = graph.add_node(CompositeNode::new(
            "Fog",
            [360.0, 0.0],
            CompositeNodeKind::DepthFog {
                near: 500.0,
                far: 2000.0,
                fog_color: RgbaColor::WHITE,
                strength: 0.5,
            },
        ));
        let merge = graph.add_node(CompositeNode::new(
            "Merge",
            [540.0, 50.0],
            CompositeNodeKind::Merge {
                blend_mode: BlendMode::Add,
                opacity: 0.5,
            },
        ));
        let out = graph.add_node(CompositeNode::new(
            "Out",
            [720.0, 50.0],
            CompositeNodeKind::Output,
        ));

        graph.connect(l0, "out", split_r, "in").unwrap();
        graph.connect(split_r, "out", fog, "in").unwrap();
        graph.connect(fog, "out", merge, "bg").unwrap();
        graph.connect(l1, "out", merge, "fg").unwrap();
        graph.connect(merge, "out", out, "in").unwrap();

        let plugins = PluginRegistry::with_builtins();
        let img = evaluate_node_composite_graph(
            &graph,
            &[Some(red_pkt), Some(blue_pkt)],
            w,
            h,
            0.0,
            &plugins,
            false,
        )
        .expect("graph rendered");
        let center = img.pixels[(8 * w + 8) as usize];
        // SplitR turns red into white grayscale (1,1,1), Add blue makes blue channel > red channel
        assert!(
            center.b > center.r + 0.3,
            "expected blue boost, got {center:?}"
        );

        // MaskCombine テスト
        let mut mask_graph = NodeCompositeGraph {
            enabled: true,
            ..Default::default()
        };
        let s = mask_graph.add_node(CompositeNode::new(
            "White",
            [0.0, 0.0],
            CompositeNodeKind::SolidColor {
                color: RgbaColor::WHITE,
            },
        ));
        let m = mask_graph.add_node(CompositeNode::new(
            "BlackMask",
            [0.0, 100.0],
            CompositeNodeKind::SolidColor {
                color: RgbaColor::BLACK,
            },
        ));
        let mc = mask_graph.add_node(CompositeNode::new(
            "Mask",
            [200.0, 50.0],
            CompositeNodeKind::MaskCombine {
                mode: TrackMatteMode::Luma,
            },
        ));
        let mo = mask_graph.add_node(CompositeNode::new(
            "Out",
            [400.0, 50.0],
            CompositeNodeKind::Output,
        ));
        mask_graph.connect(s, "out", mc, "in").unwrap();
        mask_graph.connect(m, "out", mc, "mask").unwrap();
        mask_graph.connect(mc, "out", mo, "in").unwrap();
        let masked = evaluate_node_composite_graph(&mask_graph, &[], w, h, 0.0, &plugins, true)
            .expect("masked");
        assert!(masked.pixels[0].a < 0.01);
    }
}
