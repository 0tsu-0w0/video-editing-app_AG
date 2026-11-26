//! v9.0 #53〜#54 レンダリング実装:
//! - #53: ライブスイッチャー スティンガー遷移コンポジタ (`render_live_stinger_transition`) ＆
//!   放送タリー / ON-AIR HUD オーバーレイ (`draw_live_tally_hud`)
//! - #54: VTuber 2D 三角メッシュ アフィンテクスチャラスタライザ (`render_vtuber_avatar`:
//!   ブレンドシェイプ変形・Verlet 物理揺れ・プロシージャル／カスタム立ち絵テクスチャ・ワイヤーフレーム可視化)
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロ。

use crate::color::{LinearImage, LinearPremul, srgb_u8_to_linear};
use ve_core::{
    LiveSwitcherConfig, RgbaColor, StingerConfig, StingerKind, VtuberAvatarPreset,
    VtuberBlendShapes, VtuberMeshConfig, deform_vtuber_mesh,
};

#[inline]
fn new_transparent(w: u32, h: u32) -> LinearImage {
    LinearImage::new(w, h, LinearPremul::TRANSPARENT)
}

#[inline]
fn get_px(img: &LinearImage, x: u32, y: u32) -> LinearPremul {
    img.pixels[(y * img.width + x) as usize]
}

#[inline]
fn set_px(img: &mut LinearImage, x: u32, y: u32, px: LinearPremul) {
    img.pixels[(y * img.width + x) as usize] = px;
}

fn rgba_to_premul(c: RgbaColor) -> LinearPremul {
    let a = c.a.clamp(0.0, 1.0);
    let r = srgb_u8_to_linear((c.r.clamp(0.0, 1.0) * 255.0).round() as u8);
    let g = srgb_u8_to_linear((c.g.clamp(0.0, 1.0) * 255.0).round() as u8);
    let b = srgb_u8_to_linear((c.b.clamp(0.0, 1.0) * 255.0).round() as u8);
    LinearPremul {
        r: r * a,
        g: g * a,
        b: b * a,
        a,
    }
}

fn sample_bilinear(img: &LinearImage, u: f32, v: f32) -> LinearPremul {
    if img.width == 0 || img.height == 0 {
        return LinearPremul::TRANSPARENT;
    }
    let fx = (u * (img.width as f32 - 1.0)).clamp(0.0, (img.width - 1) as f32);
    let fy = (v * (img.height as f32 - 1.0)).clamp(0.0, (img.height - 1) as f32);
    let x0 = fx.floor() as u32;
    let y0 = fy.floor() as u32;
    let x1 = (x0 + 1).min(img.width - 1);
    let y1 = (y0 + 1).min(img.height - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;

    let p00 = get_px(img, x0, y0);
    let p10 = get_px(img, x1, y0);
    let p01 = get_px(img, x0, y1);
    let p11 = get_px(img, x1, y1);
    let w00 = (1.0 - tx) * (1.0 - ty);
    let w10 = tx * (1.0 - ty);
    let w01 = (1.0 - tx) * ty;
    let w11 = tx * ty;
    LinearPremul {
        r: p00.r * w00 + p10.r * w10 + p01.r * w01 + p11.r * w11,
        g: p00.g * w00 + p10.g * w10 + p01.g * w01 + p11.g * w11,
        b: p00.b * w00 + p10.b * w10 + p01.b * w01 + p11.b * w11,
        a: p00.a * w00 + p10.a * w10 + p01.a * w01 + p11.a * w11,
    }
}

fn blend_over(dst: LinearPremul, src: LinearPremul) -> LinearPremul {
    let inv = (1.0 - src.a).clamp(0.0, 1.0);
    LinearPremul {
        r: src.r + dst.r * inv,
        g: src.g + dst.g * inv,
        b: src.b + dst.b * inv,
        a: (src.a + dst.a * inv).clamp(0.0, 1.0),
    }
}

fn lerp_premul(a: LinearPremul, b: LinearPremul, t: f32) -> LinearPremul {
    let tc = t.clamp(0.0, 1.0);
    let inv = 1.0 - tc;
    LinearPremul {
        r: a.r * inv + b.r * tc,
        g: a.g * inv + b.g * tc,
        b: a.b * inv + b.b * tc,
        a: a.a * inv + b.a * tc,
    }
}

// ============================================================================
// #53: ライブスイッチャー アルファ付きスティンガー遷移 ＆ タリー HUD
// ============================================================================

/// PGM (`from`) から PVW (`to`) へスティンガートランジション (`progress: 0..=1`) を決定論的に合成する
pub fn render_live_stinger_transition(
    from: &LinearImage,
    to: &LinearImage,
    stinger: &StingerConfig,
    progress: f32,
) -> LinearImage {
    let w = from.width.max(1);
    let h = from.height.max(1);
    let mut out = new_transparent(w, h);
    let p = progress.clamp(0.0, 1.0);
    let cut = stinger.cut_point_ratio.clamp(0.2, 0.8);
    let accent = rgba_to_premul(stinger.accent_color);
    let white = LinearPremul {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    for y in 0..h {
        let ny = y as f32 / (h.max(1) as f32);
        for x in 0..w {
            let nx = x as f32 / (w.max(1) as f32);
            let a = get_px(from, x, y);
            let b = sample_bilinear(to, nx, ny);

            let pix = match stinger.kind {
                StingerKind::Cut => {
                    if p < cut {
                        a
                    } else {
                        b
                    }
                }
                StingerKind::CrossDissolve => lerp_premul(a, b, p),
                StingerKind::DiagonalWipe => {
                    let diag = nx * 0.78 + ny * 0.22;
                    let sweep = p * 1.45 - 0.22;
                    let base = if diag < sweep { b } else { a };
                    // 光るスティンガー帯グラフィック
                    let band_d = (diag - sweep).abs();
                    if band_d < 0.085 {
                        let edge = (1.0 - band_d / 0.085).clamp(0.0, 1.0);
                        let ov = LinearPremul {
                            r: (accent.r * 0.85 + white.r * 0.35 * edge) * edge,
                            g: (accent.g * 0.85 + white.g * 0.35 * edge) * edge,
                            b: (accent.b * 0.85 + white.b * 0.35 * edge) * edge,
                            a: (edge * 0.92).clamp(0.0, 1.0),
                        };
                        blend_over(base, ov)
                    } else {
                        base
                    }
                }
                StingerKind::SlashFlash => {
                    let base = if p < cut { a } else { b };
                    let flash = (1.0 - ((p - cut).abs() / 0.35)).clamp(0.0, 1.0);
                    let f2 = flash * flash;
                    let ov = LinearPremul {
                        r: (white.r * 0.7 + accent.r * 0.3) * f2,
                        g: (white.g * 0.7 + accent.g * 0.3) * f2,
                        b: (white.b * 0.7 + accent.b * 0.3) * f2,
                        a: f2,
                    };
                    blend_over(base, ov)
                }
                StingerKind::LumaSwipe => {
                    let stripe = ((ny * 10.0).sin() * 0.06) + nx;
                    let edge = p * 1.25 - 0.12;
                    let blend = ((edge - stripe) * 8.0 + 0.5).clamp(0.0, 1.0);
                    let base = lerp_premul(a, b, blend);
                    let glow = (1.0 - ((edge - stripe).abs() / 0.07)).clamp(0.0, 1.0) * 0.65;
                    blend_over(
                        base,
                        LinearPremul {
                            r: accent.r * glow,
                            g: accent.g * glow,
                            b: accent.b * glow,
                            a: glow,
                        },
                    )
                }
                StingerKind::ZoomBurst => {
                    let intensity = (1.0 - ((p - cut).abs() / 0.5)).clamp(0.0, 1.0);
                    let zoom = 1.0 - intensity * 0.22;
                    let zu = (0.5 + (nx - 0.5) * zoom).clamp(0.0, 1.0);
                    let zv = (0.5 + (ny - 0.5) * zoom).clamp(0.0, 1.0);
                    let src_img = if p < cut { from } else { to };
                    let zpix = sample_bilinear(src_img, zu, zv);
                    let halo = (intensity * 0.32).clamp(0.0, 0.5);
                    blend_over(
                        zpix,
                        LinearPremul {
                            r: accent.r * halo,
                            g: accent.g * halo,
                            b: accent.b * halo,
                            a: halo,
                        },
                    )
                }
            };
            set_px(&mut out, x, y, pix);
        }
    }
    out
}

/// 放送用 PGM 赤タリー／PVW 緑タリー＆REC 枠 HUD をフレームへ描画する
pub fn draw_live_tally_hud(canvas: &mut LinearImage, cfg: &LiveSwitcherConfig) {
    if !cfg.enabled || !cfg.show_tally_hud || canvas.width < 48 || canvas.height < 32 {
        return;
    }
    let w = canvas.width;
    let h = canvas.height;
    let red = LinearPremul {
        r: 0.92,
        g: 0.05,
        b: 0.06,
        a: 0.92,
    };
    let green = LinearPremul {
        r: 0.06,
        g: 0.78,
        b: 0.18,
        a: 0.88,
    };
    let bg = LinearPremul {
        r: 0.02,
        g: 0.02,
        b: 0.04,
        a: 0.72,
    };

    // 同時収録中なら 3px 赤タリーフレームを外周に描画
    if cfg.recording_active {
        let bw = 3u32.min(w / 16).max(1);
        for y in 0..h {
            for x in 0..w {
                if x < bw || x + bw >= w || y < bw || y + bw >= h {
                    let cur = get_px(canvas, x, y);
                    set_px(canvas, x, y, blend_over(cur, red));
                }
            }
        }
    }

    // 右上にコンパクトな PGM / PVW タリーインジケータバーを描画
    let bar_w = (w / 5).clamp(44, 150);
    let bar_h = (h / 16).clamp(12, 26);
    let x0 = w.saturating_sub(bar_w + 8);
    let y0 = 8u32.min(h.saturating_sub(bar_h + 1));

    for y in y0..(y0 + bar_h).min(h) {
        for x in x0..(x0 + bar_w).min(w) {
            let cur = get_px(canvas, x, y);
            let rel_x = x - x0;
            let ov = if rel_x < bar_w / 2 {
                if rel_x < bar_h / 2 { red } else { bg }
            } else if rel_x < bar_w / 2 + bar_h / 2 {
                green
            } else {
                bg
            };
            set_px(canvas, x, y, blend_over(cur, ov));
        }
    }
}

// ============================================================================
// #54: VTuber 2D 三角メッシュ アフィンテクスチャラスタライザ
// ============================================================================

/// 内蔵 VTuber アバター用のプロシージャル高精細 2D ポートレートテクスチャを生成する
fn generate_procedural_vtuber_texture(
    preset: VtuberAvatarPreset,
    shapes: &VtuberBlendShapes,
    tex_w: u32,
    tex_h: u32,
) -> LinearImage {
    let mut img = new_transparent(tex_w, tex_h);
    let (outfit_c, hair_c, eye_c) = preset.palette();
    let outfit = rgba_to_premul(outfit_c);
    let hair = rgba_to_premul(hair_c);
    let hair_dark = LinearPremul {
        r: hair.r * 0.58,
        g: hair.g * 0.58,
        b: hair.b * 0.65,
        a: 1.0,
    };
    let eye = rgba_to_premul(eye_c);
    let skin = rgba_to_premul(RgbaColor {
        r: 0.99,
        g: 0.87,
        b: 0.80,
        a: 1.0,
    });
    let skin_shade = rgba_to_premul(RgbaColor {
        r: 0.92,
        g: 0.72,
        b: 0.66,
        a: 1.0,
    });
    let blush = LinearPremul {
        r: 0.95 * 0.35,
        g: 0.22 * 0.35,
        b: 0.32 * 0.35,
        a: 0.35,
    };
    let white = LinearPremul {
        r: 0.98,
        g: 0.98,
        b: 0.99,
        a: 1.0,
    };
    let dark_line = LinearPremul {
        r: 0.04,
        g: 0.03,
        b: 0.07,
        a: 1.0,
    };
    let mouth_inner = rgba_to_premul(RgbaColor {
        r: 0.82,
        g: 0.22,
        b: 0.30,
        a: 1.0,
    });

    for y in 0..tex_h {
        let v = (y as f32 + 0.5) / tex_h as f32;
        for x in 0..tex_w {
            let u = (x as f32 + 0.5) / tex_w as f32;
            let mut px = LinearPremul::TRANSPARENT;

            // 1. 後ろ髪シルエット（ロングヘア＋左右サイドテール）
            let back_hair_d = ((u - 0.5) / 0.36).hypot((v - 0.48) / 0.42);
            let side_l = ((u - 0.24) / 0.12).hypot((v - 0.68) / 0.27);
            let side_r = ((u - 0.76) / 0.12).hypot((v - 0.68) / 0.27);
            if back_hair_d < 1.0 || side_l < 1.0 || side_r < 1.0 {
                let grad = (0.75 + 0.25 * (1.0 - v)).clamp(0.5, 1.0);
                px = LinearPremul {
                    r: hair_dark.r * grad,
                    g: hair_dark.g * grad,
                    b: hair_dark.b * grad,
                    a: 1.0,
                };
            }

            // プリセット固有の耳／アンテナ
            if matches!(
                preset,
                VtuberAvatarPreset::CyberFoxViolet | VtuberAvatarPreset::MascotCatGold
            ) {
                let ear_l = (u - 0.27).abs() * 2.4 + (0.24 - v).abs() * 1.4;
                let ear_r = (u - 0.73).abs() * 2.4 + (0.24 - v).abs() * 1.4;
                if (ear_l < 0.16 && v < 0.26) || (ear_r < 0.16 && v < 0.26) {
                    px = hair;
                }
            }

            // 2. 肩・衣装ボディ（下部 0.68..0.98）
            let body_d = ((u - 0.5) / 0.32).hypot((v - 0.92) / 0.24);
            if body_d < 1.0 && v > 0.69 {
                let shade = 0.8 + 0.2 * (1.0 - (u - 0.5).abs() * 2.0).clamp(0.0, 1.0);
                px = LinearPremul {
                    r: outfit.r * shade,
                    g: outfit.g * shade,
                    b: outfit.b * shade,
                    a: 1.0,
                };
                // 襟Vライン
                if (u - 0.5).abs() < (0.86 - v).max(0.0) * 0.55 && v < 0.84 {
                    px = white;
                }
            }

            // 3. 首
            if (u - 0.5).abs() < 0.065 && (0.64..=0.76).contains(&v) {
                px = skin_shade;
            }

            // 4. 顔輪郭（頬の丸み＋顎ライン）
            let face_d = ((u - 0.5) / 0.22).hypot((v - 0.47) / 0.23);
            if face_d < 1.0 {
                let rim = if face_d > 0.92 { 0.82 } else { 1.0 };
                px = LinearPremul {
                    r: skin.r * rim,
                    g: skin.g * rim,
                    b: skin.b * rim,
                    a: 1.0,
                };
                // チーク
                let blush_l = ((u - 0.36) / 0.06).hypot((v - 0.53) / 0.032);
                let blush_r = ((u - 0.64) / 0.06).hypot((v - 0.53) / 0.032);
                if blush_l < 1.0 || blush_r < 1.0 {
                    px = blend_over(px, blush);
                }
            }

            // 5. 左右の目（まばたき・視線・ハイライト反映）
            for (cx, blink) in [(0.40_f32, shapes.blink_l), (0.60_f32, shapes.blink_r)] {
                let open_h = (0.055 * (1.0 - blink * 0.88)).max(0.008);
                let ey = 0.45;
                let ed = ((u - cx) / 0.056).hypot((v - ey) / open_h);
                if ed < 1.0 {
                    if blink > 0.78 {
                        px = dark_line;
                    } else {
                        px = white;
                        let px_c = cx + shapes.eye_look_x * 0.018;
                        let py_c = ey + shapes.eye_look_y * 0.014;
                        let iris_d = ((u - px_c) / 0.034).hypot((v - py_c) / (open_h * 0.82));
                        if iris_d < 1.0 {
                            px = eye;
                            let pupil_d = ((u - px_c) / 0.016).hypot((v - py_c) / (open_h * 0.42));
                            if pupil_d < 1.0 {
                                px = dark_line;
                            }
                            let spec_d = ((u - (px_c - 0.012)) / 0.011)
                                .hypot((v - (py_c - open_h * 0.28)) / (open_h * 0.24));
                            if spec_d < 1.0 {
                                px = white;
                            }
                        }
                        // 上まつ毛
                        if v < ey - open_h * 0.55 {
                            px = dark_line;
                        }
                    }
                }
                // 眉毛
                let brow_y = 0.365 - shapes.brow_y * 0.025;
                if (u - cx).abs() < 0.048 && (v - brow_y).abs() < 0.008 {
                    px = hair_dark;
                }
            }

            // 6. 口（開き＆スマイル反映）
            let my = 0.585;
            let mouth_w = 0.046 + shapes.mouth_smile * 0.022;
            let mouth_h = 0.008 + shapes.mouth_open * 0.042;
            let smile_curve = -shapes.mouth_smile * ((u - 0.5) / mouth_w).powi(2) * 0.018;
            let md = ((u - 0.5) / mouth_w).hypot((v - (my + smile_curve)) / mouth_h);
            if md < 1.0 {
                px = if shapes.mouth_open > 0.15 {
                    mouth_inner
                } else {
                    dark_line
                };
            }

            // 7. 前髪（バング）＋ハイライト天使の輪
            let bang_wave = 0.34 + 0.045 * ((u - 0.5) * 18.0).sin().abs();
            let top_hair_d = ((u - 0.5) / 0.28).hypot((v - 0.30) / 0.22);
            if top_hair_d < 1.0 && v < bang_wave {
                let halo = if (v - 0.25).abs() < 0.022 && (u - 0.5).abs() < 0.18 {
                    1.28
                } else {
                    1.0
                };
                px = LinearPremul {
                    r: (hair.r * halo).min(1.0),
                    g: (hair.g * halo).min(1.0),
                    b: (hair.b * halo).min(1.0),
                    a: 1.0,
                };
            }

            set_px(&mut img, x, y, px);
        }
    }
    img
}

#[inline]
fn edge_fn(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

fn draw_line_premul(img: &mut LinearImage, x0: f32, y0: f32, x1: f32, y1: f32, col: LinearPremul) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
    let w = img.width as i32;
    let h = img.height as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (x0 + (x1 - x0) * t).round() as i32;
        let y = (y0 + (y1 - y0) * t).round() as i32;
        if x >= 0 && x < w && y >= 0 && y < h {
            let cur = get_px(img, x as u32, y as u32);
            set_px(img, x as u32, y as u32, blend_over(cur, col));
        }
    }
}

/// VTuber 2D 三角メッシュ（ブレンドシェイプ＋Verlet 物理揺れ）をアフィンテクスチャマッピング描画する
pub fn render_vtuber_avatar(
    cfg: &VtuberMeshConfig,
    custom_tex: Option<&LinearImage>,
    local_sec: f32,
    eval_shapes: &VtuberBlendShapes,
) -> LinearImage {
    let w = (cfg.width.round() as u32).clamp(32, 1600);
    let h = (cfg.height.round() as u32).clamp(32, 1600);
    let mut out = new_transparent(w, h);

    let owned_tex;
    let tex = if let Some(t) = custom_tex {
        t
    } else {
        let tw = w.clamp(64, 384);
        let th = h.clamp(64, 384);
        owned_tex = generate_procedural_vtuber_texture(cfg.preset, eval_shapes, tw, th);
        &owned_tex
    };

    let (verts, tris) = deform_vtuber_mesh(cfg, eval_shapes, local_sec);
    let wf = w as f32;
    let hf = h as f32;

    for &[i0, i1, i2] in &tris {
        let v0 = verts[i0];
        let v1 = verts[i1];
        let v2 = verts[i2];

        let ax = v0.pos[0] * wf;
        let ay = v0.pos[1] * hf;
        let bx = v1.pos[0] * wf;
        let by = v1.pos[1] * hf;
        let cx = v2.pos[0] * wf;
        let cy = v2.pos[1] * hf;

        let area = edge_fn(ax, ay, bx, by, cx, cy);
        if area.abs() < 1e-4 {
            continue;
        }
        let inv_area = 1.0 / area;

        let min_x = ax.min(bx).min(cx).floor().max(0.0) as u32;
        let max_x = ax.max(bx).max(cx).ceil().min(wf - 1.0).max(0.0) as u32;
        let min_y = ay.min(by).min(cy).floor().max(0.0) as u32;
        let max_y = ay.max(by).max(cy).ceil().min(hf - 1.0).max(0.0) as u32;

        for y in min_y..=max_y {
            let py = y as f32 + 0.5;
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let w0 = edge_fn(bx, by, cx, cy, px, py) * inv_area;
                let w1 = edge_fn(cx, cy, ax, ay, px, py) * inv_area;
                let w2 = edge_fn(ax, ay, bx, by, px, py) * inv_area;
                if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                    let u = (w0 * v0.uv[0] + w1 * v1.uv[0] + w2 * v2.uv[0]).clamp(0.0, 1.0);
                    let v = (w0 * v0.uv[1] + w1 * v1.uv[1] + w2 * v2.uv[1]).clamp(0.0, 1.0);
                    let sample = sample_bilinear(tex, u, v);
                    if sample.a > 1e-4 {
                        set_px(&mut out, x, y, sample);
                    }
                }
            }
        }
    }

    if cfg.show_wireframe {
        let wire = LinearPremul {
            r: 0.1 * 0.55,
            g: 0.9 * 0.55,
            b: 1.0 * 0.55,
            a: 0.55,
        };
        for &[i0, i1, i2] in &tris {
            let p0 = [verts[i0].pos[0] * wf, verts[i0].pos[1] * hf];
            let p1 = [verts[i1].pos[0] * wf, verts[i1].pos[1] * hf];
            let p2 = [verts[i2].pos[0] * wf, verts[i2].pos[1] * hf];
            draw_line_premul(&mut out, p0[0], p0[1], p1[0], p1[1], wire);
            draw_line_premul(&mut out, p1[0], p1[1], p2[0], p2[1], wire);
            draw_line_premul(&mut out, p2[0], p2[1], p0[0], p0[1], wire);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_stinger_all_six_modes_transition_deterministically_and_draw_tally_hud() {
        let pgm = LinearImage::new(
            96,
            54,
            LinearPremul {
                r: 0.8,
                g: 0.1,
                b: 0.1,
                a: 1.0,
            },
        );
        let pvw = LinearImage::new(
            96,
            54,
            LinearPremul {
                r: 0.1,
                g: 0.2,
                b: 0.85,
                a: 1.0,
            },
        );

        for &kind in StingerKind::ALL {
            let st = StingerConfig {
                kind,
                ..StingerConfig::default()
            };
            let m1 = render_live_stinger_transition(&pgm, &pvw, &st, 0.5);
            let m2 = render_live_stinger_transition(&pgm, &pvw, &st, 0.5);
            assert_eq!(m1.pixels, m2.pixels);
            assert!(m1.pixels.iter().all(|p| p.r.is_finite() && p.a.is_finite()));
        }

        let mut hud_img = pgm.clone();
        let sw = LiveSwitcherConfig {
            enabled: true,
            recording_active: true,
            show_tally_hud: true,
            ..LiveSwitcherConfig::default()
        };
        draw_live_tally_hud(&mut hud_img, &sw);
        assert_ne!(hud_img.pixels, pgm.pixels);
    }

    #[test]
    fn vtuber_2d_triangle_mesh_rasterizes_all_presets_and_responds_to_blendshapes() {
        for &preset in VtuberAvatarPreset::ALL {
            let cfg = VtuberMeshConfig {
                width: 128.0,
                height: 128.0,
                preset,
                show_wireframe: false,
                ..VtuberMeshConfig::default()
            };
            let s_rest = VtuberBlendShapes::default();
            let s_turn = VtuberBlendShapes {
                head_yaw_deg: 28.0,
                mouth_open: 0.9,
                blink_l: 1.0,
                ..VtuberBlendShapes::default()
            };
            let img_a = render_vtuber_avatar(&cfg, None, 0.5, &s_rest);
            let img_a2 = render_vtuber_avatar(&cfg, None, 0.5, &s_rest);
            let img_b = render_vtuber_avatar(&cfg, None, 1.2, &s_turn);
            assert_eq!(img_a.pixels, img_a2.pixels);
            let visible = img_a.pixels.iter().filter(|p| p.a > 0.5).count();
            assert!(visible > 1500, "avatar should have visible pixels");
            assert_ne!(img_a.pixels, img_b.pixels);
        }
    }
}
