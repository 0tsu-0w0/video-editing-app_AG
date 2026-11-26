//! v8.0 #50: SVG `<path>` インポート ＆ 等弧長パスモーフィング ・ Text-on-Path ・ ディスプレイスメント / グリッドワープ
//!
//! Pure Rust・ゼロ `unsafe`・外部クレート追加ゼロで以下を完結する:
//! - SVG `<path d="...">` 完全パーサ（`M/m`, `L/l`, `H/h`, `V/v`, `C/c`, `S/s`, `Q/q`, `T/t`, `A/a` 楕円弧→3次ベジェ厳密変換, `Z/z`）
//! - 頂点数の異なる任意 2 パス間の等弧長リサンプリング＆モーフィング (`morph_paths_equal_arclength`)
//! - 曲線接線角度追従 Text-on-Path (`TextOnPathConfig` / `sample_polyline_at_fraction`)
//! - 双三次グリッドワープ＆輝度/プロシージャルディスプレイスメント設定 (`WarpDisplaceConfig`)

use crate::project::{MaskVertex, RgbaColor, VectorFillMode};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// ワンクリック組み込み SVG パスプリセット（モーフィング・Text-on-Path 即時検証用）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SvgShapePreset {
    LightningBolt,
    Heart,
    PlayBadge,
    Crown,
    WaveRibbon,
    ArchCurve,
}

impl SvgShapePreset {
    pub const ALL: &'static [Self] = &[
        Self::LightningBolt,
        Self::Heart,
        Self::PlayBadge,
        Self::Crown,
        Self::WaveRibbon,
        Self::ArchCurve,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::LightningBolt => "⚡ イナズマ (閉パス)",
            Self::Heart => "❤ ハート (3次ベジェ)",
            Self::PlayBadge => "▶ 再生バッジ (丸角)",
            Self::Crown => "👑 クラウン (王冠)",
            Self::WaveRibbon => "〰 ウェーブ曲線 (開パス)",
            Self::ArchCurve => "🌈 アーチ曲線 (Text-on-Path)",
        }
    }

    pub fn svg_path_d(self) -> &'static str {
        match self {
            Self::LightningBolt => "M 56 4 L 14 54 L 46 54 L 38 96 L 86 42 L 54 42 Z",
            Self::Heart => {
                "M 50 88 C 20 68 8 48 8 30 C 8 15 20 6 33 6 C 42 6 47 12 50 18 C 53 12 58 6 67 6 C 80 6 92 15 92 30 C 92 48 80 68 50 88 Z"
            }
            Self::PlayBadge => {
                "M 24 14 Q 16 14 16 22 L 16 78 Q 16 86 24 86 L 84 54 Q 90 50 84 46 Z"
            }
            Self::Crown => "M 10 80 L 14 28 L 36 52 L 50 16 L 64 52 L 86 28 L 90 80 Z",
            Self::WaveRibbon => "M 5 55 C 25 15 45 15 50 50 S 75 85 95 45",
            Self::ArchCurve => "M 8 78 A 48 48 0 0 1 92 78",
        }
    }
}

/// XML を含む `.svg` 文字列または生の `d` 属性文字列から `<path d="...">` のパス列を抽出する
pub fn extract_svg_path_data(svg_or_d: &str) -> Vec<String> {
    let trimmed = svg_or_d.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if !trimmed.contains('<') {
        return vec![trimmed.to_string()];
    }

    let mut out = Vec::new();
    let bytes = trimmed.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some(rel_path) = trimmed[i..].find("<path") {
            let tag_start = i + rel_path;
            let Some(rel_end) = trimmed[tag_start..].find('>') else {
                break;
            };
            let tag = &trimmed[tag_start..=tag_start + rel_end];
            if let Some(d_val) = extract_xml_attr(tag, "d")
                && !d_val.trim().is_empty()
            {
                out.push(d_val);
            }
            i = tag_start + rel_end + 1;
        } else {
            break;
        }
    }
    out
}

fn extract_xml_attr(tag: &str, attr: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let attr_bytes = attr.as_bytes();
    let mut i = 0usize;
    while i + attr_bytes.len() + 2 < bytes.len() {
        let prev_ok = i == 0 || bytes[i - 1].is_ascii_whitespace();
        if prev_ok && &bytes[i..i + attr_bytes.len()] == attr_bytes {
            let mut j = i + attr_bytes.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    let quote = bytes[j];
                    j += 1;
                    let start = j;
                    while j < bytes.len() && bytes[j] != quote {
                        j += 1;
                    }
                    return Some(tag[start..j].to_string());
                }
            }
        }
        i += 1;
    }
    None
}

/// 内部表現: 3 次ベジェ曲線セグメント列（正規化前の生座標）
#[derive(Debug, Clone, Copy)]
struct CubicSeg {
    p0: [f64; 2],
    p1: [f64; 2],
    p2: [f64; 2],
    p3: [f64; 2],
}

/// SVG `<path d="...">` をパースし、正規化座標 (`0.06..=0.94`, アスペクト比維持) の
/// `(Vec<MaskVertex>, closed)` へ変換する。
pub fn parse_svg_path_to_vertices(d_or_svg: &str) -> Result<(Vec<MaskVertex>, bool), String> {
    let extracted = extract_svg_path_data(d_or_svg);
    let d = extracted
        .first()
        .map(String::as_str)
        .ok_or_else(|| "SVG パスデータが空です".to_string())?;

    let (segments, closed) = parse_svg_d_to_cubics(d)?;
    if segments.is_empty() {
        return Err("SVG パスに描画セグメントが含まれていません".to_string());
    }

    // バウンディングボックスを算出（制御点＋曲線サンプルで安全に見積もる）
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for seg in &segments {
        for s in 0..=8 {
            let t = s as f64 / 8.0;
            let p = eval_cubic_f64(seg, t);
            min_x = min_x.min(p[0]);
            max_x = max_x.max(p[0]);
            min_y = min_y.min(p[1]);
            max_y = max_y.max(p[1]);
        }
    }
    let span_x = (max_x - min_x).max(1e-4);
    let span_y = (max_y - min_y).max(1e-4);
    let max_span = span_x.max(span_y);
    let pad = 0.06_f64;
    let inner = 1.0 - 2.0 * pad;
    let scale = inner / max_span;
    let ox = pad + (inner - span_x * scale) * 0.5 - min_x * scale;
    let oy = pad + (inner - span_y * scale) * 0.5 - min_y * scale;

    let norm =
        |p: [f64; 2]| -> [f32; 2] { [(p[0] * scale + ox) as f32, (p[1] * scale + oy) as f32] };

    let mut verts: Vec<MaskVertex> = Vec::with_capacity(segments.len() + 1);
    for (idx, seg) in segments.iter().enumerate() {
        let p0 = norm(seg.p0);
        let p1 = norm(seg.p1);
        let p2 = norm(seg.p2);
        let p3 = norm(seg.p3);
        if idx == 0 {
            verts.push(MaskVertex {
                x: p0[0],
                y: p0[1],
                in_handle: [0.0, 0.0],
                out_handle: [p1[0] - p0[0], p1[1] - p0[1]],
            });
        } else if let Some(prev) = verts.last_mut() {
            prev.out_handle = [p1[0] - prev.x, p1[1] - prev.y];
        }

        let is_last = idx + 1 == segments.len();
        if closed && is_last {
            // 始点と一致していれば始点の in_handle に吸収
            let first = verts[0];
            let dist = (p3[0] - first.x).hypot(p3[1] - first.y);
            if dist < 1e-3 {
                verts[0].in_handle = [p2[0] - first.x, p2[1] - first.y];
                continue;
            }
        }
        verts.push(MaskVertex {
            x: p3[0],
            y: p3[1],
            in_handle: [p2[0] - p3[0], p2[1] - p3[1]],
            out_handle: [0.0, 0.0],
        });
    }

    if verts.len() < 2 {
        return Err("SVG パスの頂点数が不足しています（2点以上必要）".to_string());
    }
    Ok((verts, closed))
}

fn eval_cubic_f64(seg: &CubicSeg, t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let b0 = u * u * u;
    let b1 = 3.0 * u * u * t;
    let b2 = 3.0 * u * t * t;
    let b3 = t * t * t;
    [
        b0 * seg.p0[0] + b1 * seg.p1[0] + b2 * seg.p2[0] + b3 * seg.p3[0],
        b0 * seg.p0[1] + b1 * seg.p1[1] + b2 * seg.p2[1] + b3 * seg.p3[1],
    ]
}

struct SvgScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> SvgScanner<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            bytes: s.as_bytes(),
            pos: 0,
        }
    }

    fn skip_sep(&mut self) {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b.is_ascii_whitespace() || b == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_sep();
        self.bytes.get(self.pos).copied()
    }

    fn has_number(&mut self) -> bool {
        self.peek()
            .is_some_and(|b| b.is_ascii_digit() || b == b'+' || b == b'-' || b == b'.')
    }

    fn next_flag(&mut self) -> Result<bool, String> {
        self.skip_sep();
        let Some(&b) = self.bytes.get(self.pos) else {
            return Err("SVG arc flag が欠落しています".to_string());
        };
        match b {
            b'0' => {
                self.pos += 1;
                Ok(false)
            }
            b'1' => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err(format!("不正な SVG arc flag: '{}'", b as char)),
        }
    }

    fn next_f64(&mut self) -> Result<f64, String> {
        self.skip_sep();
        let start = self.pos;
        if self.pos < self.bytes.len()
            && (self.bytes[self.pos] == b'+' || self.bytes[self.pos] == b'-')
        {
            self.pos += 1;
        }
        let mut saw_digit = false;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            saw_digit = true;
            self.pos += 1;
        }
        if self.pos < self.bytes.len() && self.bytes[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                saw_digit = true;
                self.pos += 1;
            }
        }
        if !saw_digit {
            return Err(format!("SVG 数値パース失敗（位置 {start}）"));
        }
        if self.pos < self.bytes.len()
            && (self.bytes[self.pos] == b'e' || self.bytes[self.pos] == b'E')
        {
            let exp_mark = self.pos;
            self.pos += 1;
            if self.pos < self.bytes.len()
                && (self.bytes[self.pos] == b'+' || self.bytes[self.pos] == b'-')
            {
                self.pos += 1;
            }
            let mut exp_digits = false;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                exp_digits = true;
                self.pos += 1;
            }
            if !exp_digits {
                self.pos = exp_mark;
            }
        }
        let slice = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "UTF-8 パース失敗".to_string())?;
        slice
            .parse::<f64>()
            .map_err(|e| format!("SVG 数値 '{slice}' パース失敗: {e}"))
    }
}

fn line_seg(p0: [f64; 2], p3: [f64; 2]) -> CubicSeg {
    CubicSeg {
        p0,
        p1: [p0[0] + (p3[0] - p0[0]) / 3.0, p0[1] + (p3[1] - p0[1]) / 3.0],
        p2: [
            p0[0] + (p3[0] - p0[0]) * (2.0 / 3.0),
            p0[1] + (p3[1] - p0[1]) * (2.0 / 3.0),
        ],
        p3,
    }
}

fn quad_to_cubic(p0: [f64; 2], q: [f64; 2], p3: [f64; 2]) -> CubicSeg {
    CubicSeg {
        p0,
        p1: [
            p0[0] + (2.0 / 3.0) * (q[0] - p0[0]),
            p0[1] + (2.0 / 3.0) * (q[1] - p0[1]),
        ],
        p2: [
            p3[0] + (2.0 / 3.0) * (q[0] - p3[0]),
            p3[1] + (2.0 / 3.0) * (q[1] - p3[1]),
        ],
        p3,
    }
}

/// SVG 楕円弧 (`A/a`) を SVG 1.1 Implementation Notes F.6 準拠で 3 次ベジェ列へ厳密変換する
#[allow(clippy::too_many_arguments)]
fn svg_arc_to_cubics(
    p0: [f64; 2],
    rx_in: f64,
    ry_in: f64,
    x_rot_deg: f64,
    large_arc: bool,
    sweep: bool,
    p1: [f64; 2],
) -> Vec<CubicSeg> {
    if (p0[0] - p1[0]).hypot(p0[1] - p1[1]) < 1e-6 {
        return Vec::new();
    }
    let mut rx = rx_in.abs();
    let mut ry = ry_in.abs();
    if rx < 1e-6 || ry < 1e-6 {
        return vec![line_seg(p0, p1)];
    }

    let phi = x_rot_deg.to_radians();
    let cos_p = phi.cos();
    let sin_p = phi.sin();

    let dx2 = (p0[0] - p1[0]) * 0.5;
    let dy2 = (p0[1] - p1[1]) * 0.5;
    let x1p = cos_p * dx2 + sin_p * dy2;
    let y1p = -sin_p * dx2 + cos_p * dy2;

    let lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lam > 1.0 {
        let s = lam.sqrt();
        rx *= s;
        ry *= s;
    }

    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let x1p2 = x1p * x1p;
    let y1p2 = y1p * y1p;
    let num = (rx2 * ry2 - rx2 * y1p2 - ry2 * x1p2).max(0.0);
    let den = (rx2 * y1p2 + ry2 * x1p2).max(1e-12);
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let coef = sign * (num / den).sqrt();
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);

    let cx = cos_p * cxp - sin_p * cyp + (p0[0] + p1[0]) * 0.5;
    let cy = sin_p * cxp + cos_p * cyp + (p0[1] + p1[1]) * 0.5;

    let vec_angle = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let cos_a = (dot / len.max(1e-12)).clamp(-1.0, 1.0);
        let ang = cos_a.acos();
        if ux * vy - uy * vx < 0.0 { -ang } else { ang }
    };

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;

    let theta1 = vec_angle(1.0, 0.0, ux, uy);
    let mut dtheta = vec_angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += TAU;
    }

    let seg_count = ((dtheta.abs() / (PI * 0.5)).ceil() as usize).clamp(1, 8);
    let step = dtheta / seg_count as f64;
    let alpha = (4.0 / 3.0) * (step * 0.25).tan();

    let map_pt = |ex: f64, ey: f64| -> [f64; 2] {
        [
            cx + cos_p * (rx * ex) - sin_p * (ry * ey),
            cy + sin_p * (rx * ex) + cos_p * (ry * ey),
        ]
    };

    let mut out = Vec::with_capacity(seg_count);
    for i in 0..seg_count {
        let t0 = theta1 + step * i as f64;
        let t1 = t0 + step;
        let (c0, s0) = (t0.cos(), t0.sin());
        let (c1, s1) = (t1.cos(), t1.sin());
        out.push(CubicSeg {
            p0: map_pt(c0, s0),
            p1: map_pt(c0 - alpha * s0, s0 + alpha * c0),
            p2: map_pt(c1 + alpha * s1, s1 - alpha * c1),
            p3: map_pt(c1, s1),
        });
    }
    out
}

fn parse_svg_d_to_cubics(d: &str) -> Result<(Vec<CubicSeg>, bool), String> {
    let mut sc = SvgScanner::new(d);
    let mut segs: Vec<CubicSeg> = Vec::new();
    let mut cur = [0.0_f64, 0.0];
    let mut start = [0.0_f64, 0.0];
    let mut last_cubic_ctrl: Option<[f64; 2]> = None;
    let mut last_quad_ctrl: Option<[f64; 2]> = None;
    let mut closed = false;
    let mut cmd: u8 = 0;

    while let Some(b) = sc.peek() {
        if b.is_ascii_alphabetic() {
            cmd = b;
            sc.pos += 1;
        } else if cmd == 0 {
            return Err(format!(
                "SVG パス先頭がコマンド文字ではありません: '{}'",
                b as char
            ));
        }

        match cmd {
            b'M' | b'm' => {
                let is_rel = cmd == b'm';
                let x = sc.next_f64()?;
                let y = sc.next_f64()?;
                cur = if is_rel {
                    [cur[0] + x, cur[1] + y]
                } else {
                    [x, y]
                };
                start = cur;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                // M/m の後続座標は暗黙的に L/l として扱う
                cmd = if is_rel { b'l' } else { b'L' };
            }
            b'L' | b'l' => {
                let is_rel = cmd == b'l';
                while sc.has_number() {
                    let x = sc.next_f64()?;
                    let y = sc.next_f64()?;
                    let next = if is_rel {
                        [cur[0] + x, cur[1] + y]
                    } else {
                        [x, y]
                    };
                    segs.push(line_seg(cur, next));
                    cur = next;
                }
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'H' | b'h' => {
                let is_rel = cmd == b'h';
                while sc.has_number() {
                    let x = sc.next_f64()?;
                    let next = [if is_rel { cur[0] + x } else { x }, cur[1]];
                    segs.push(line_seg(cur, next));
                    cur = next;
                }
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'V' | b'v' => {
                let is_rel = cmd == b'v';
                while sc.has_number() {
                    let y = sc.next_f64()?;
                    let next = [cur[0], if is_rel { cur[1] + y } else { y }];
                    segs.push(line_seg(cur, next));
                    cur = next;
                }
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'C' | b'c' => {
                let is_rel = cmd == b'c';
                while sc.has_number() {
                    let x1 = sc.next_f64()?;
                    let y1 = sc.next_f64()?;
                    let x2 = sc.next_f64()?;
                    let y2 = sc.next_f64()?;
                    let x = sc.next_f64()?;
                    let y = sc.next_f64()?;
                    let (p1, p2, p3) = if is_rel {
                        (
                            [cur[0] + x1, cur[1] + y1],
                            [cur[0] + x2, cur[1] + y2],
                            [cur[0] + x, cur[1] + y],
                        )
                    } else {
                        ([x1, y1], [x2, y2], [x, y])
                    };
                    segs.push(CubicSeg {
                        p0: cur,
                        p1,
                        p2,
                        p3,
                    });
                    last_cubic_ctrl = Some(p2);
                    last_quad_ctrl = None;
                    cur = p3;
                }
            }
            b'S' | b's' => {
                let is_rel = cmd == b's';
                while sc.has_number() {
                    let p1 = match last_cubic_ctrl {
                        Some(lc) => [2.0 * cur[0] - lc[0], 2.0 * cur[1] - lc[1]],
                        None => cur,
                    };
                    let x2 = sc.next_f64()?;
                    let y2 = sc.next_f64()?;
                    let x = sc.next_f64()?;
                    let y = sc.next_f64()?;
                    let (p2, p3) = if is_rel {
                        ([cur[0] + x2, cur[1] + y2], [cur[0] + x, cur[1] + y])
                    } else {
                        ([x2, y2], [x, y])
                    };
                    segs.push(CubicSeg {
                        p0: cur,
                        p1,
                        p2,
                        p3,
                    });
                    last_cubic_ctrl = Some(p2);
                    last_quad_ctrl = None;
                    cur = p3;
                }
            }
            b'Q' | b'q' => {
                let is_rel = cmd == b'q';
                while sc.has_number() {
                    let x1 = sc.next_f64()?;
                    let y1 = sc.next_f64()?;
                    let x = sc.next_f64()?;
                    let y = sc.next_f64()?;
                    let (q, p3) = if is_rel {
                        ([cur[0] + x1, cur[1] + y1], [cur[0] + x, cur[1] + y])
                    } else {
                        ([x1, y1], [x, y])
                    };
                    segs.push(quad_to_cubic(cur, q, p3));
                    last_quad_ctrl = Some(q);
                    last_cubic_ctrl = None;
                    cur = p3;
                }
            }
            b'T' | b't' => {
                let is_rel = cmd == b't';
                while sc.has_number() {
                    let q = match last_quad_ctrl {
                        Some(lq) => [2.0 * cur[0] - lq[0], 2.0 * cur[1] - lq[1]],
                        None => cur,
                    };
                    let x = sc.next_f64()?;
                    let y = sc.next_f64()?;
                    let p3 = if is_rel {
                        [cur[0] + x, cur[1] + y]
                    } else {
                        [x, y]
                    };
                    segs.push(quad_to_cubic(cur, q, p3));
                    last_quad_ctrl = Some(q);
                    last_cubic_ctrl = None;
                    cur = p3;
                }
            }
            b'A' | b'a' => {
                let is_rel = cmd == b'a';
                while sc.has_number() {
                    let rx = sc.next_f64()?;
                    let ry = sc.next_f64()?;
                    let x_rot = sc.next_f64()?;
                    let large_arc = sc.next_flag()?;
                    let sweep = sc.next_flag()?;
                    let x = sc.next_f64()?;
                    let y = sc.next_f64()?;
                    let p3 = if is_rel {
                        [cur[0] + x, cur[1] + y]
                    } else {
                        [x, y]
                    };
                    let arc_segs = svg_arc_to_cubics(cur, rx, ry, x_rot, large_arc, sweep, p3);
                    segs.extend(arc_segs);
                    cur = p3;
                }
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'Z' | b'z' => {
                if (cur[0] - start[0]).hypot(cur[1] - start[1]) > 1e-5 {
                    segs.push(line_seg(cur, start));
                }
                cur = start;
                closed = true;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            other => {
                return Err(format!("未対応の SVG パスコマンド: '{}'", other as char));
            }
        }
    }

    Ok((segs, closed))
}

// ============================================================================
// 等弧長リサンプリング ＆ パスモーフィング ＆ 接線角度評価
// ============================================================================

/// ベジェ頂点列を高密度ポリラインへ展開する
pub fn flatten_bezier_vertices(
    verts: &[MaskVertex],
    closed: bool,
    steps_per_seg: usize,
) -> Vec<[f32; 2]> {
    if verts.len() < 2 {
        return verts.iter().map(|v| [v.x, v.y]).collect();
    }
    let steps = steps_per_seg.clamp(4, 32);
    let seg_count = if closed { verts.len() } else { verts.len() - 1 };
    let mut pts = Vec::with_capacity(seg_count * steps + 1);
    for i in 0..seg_count {
        let v0 = &verts[i];
        let v1 = &verts[(i + 1) % verts.len()];
        let p0 = [v0.x, v0.y];
        let p1 = [v0.x + v0.out_handle[0], v0.y + v0.out_handle[1]];
        let p2 = [v1.x + v1.in_handle[0], v1.y + v1.in_handle[1]];
        let p3 = [v1.x, v1.y];
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let u = 1.0 - t;
            let b0 = u * u * u;
            let b1 = 3.0 * u * u * t;
            let b2 = 3.0 * u * t * t;
            let b3 = t * t * t;
            pts.push([
                b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
                b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
            ]);
        }
    }
    if !closed {
        let last = verts.last().unwrap();
        pts.push([last.x, last.y]);
    } else if let Some(&first) = pts.first() {
        pts.push(first);
    }
    pts
}

/// ポリラインを等弧長で `sample_count` 点へリサンプリングする（モーフィング頂点数正規化用）
pub fn resample_polyline_equal_arclength(pts: &[[f32; 2]], sample_count: usize) -> Vec<[f32; 2]> {
    let n = sample_count.clamp(4, 512);
    if pts.len() < 2 {
        return vec![pts.first().copied().unwrap_or([0.5, 0.5]); n];
    }
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0_f32);
    let mut total = 0.0_f32;
    for w in pts.windows(2) {
        total += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        cum.push(total);
    }
    if total < 1e-6 {
        return vec![pts[0]; n];
    }

    let mut out = Vec::with_capacity(n);
    let mut seg = 0usize;
    for i in 0..n {
        let target = total * (i as f32 / (n - 1).max(1) as f32);
        while seg + 1 < cum.len() - 1 && cum[seg + 1] < target {
            seg += 1;
        }
        let d0 = cum[seg];
        let d1 = cum[seg + 1];
        let frac = if (d1 - d0).abs() < 1e-6 {
            0.0
        } else {
            ((target - d0) / (d1 - d0)).clamp(0.0, 1.0)
        };
        let a = pts[seg];
        let b = pts[seg + 1];
        out.push([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
    }
    out
}

/// 異なる頂点数・トポロジを持つ 2 つの SVG パスを等弧長リサンプリングで滑らかにモーフィング補間する
pub fn morph_paths_equal_arclength(
    path_a: &[MaskVertex],
    closed_a: bool,
    path_b: &[MaskVertex],
    closed_b: bool,
    t: f32,
    sample_count: usize,
) -> Vec<[f32; 2]> {
    let n = sample_count.clamp(8, 256);
    let pa = resample_polyline_equal_arclength(&flatten_bezier_vertices(path_a, closed_a, 12), n);
    let pb = resample_polyline_equal_arclength(&flatten_bezier_vertices(path_b, closed_b, 12), n);
    let tc = t.clamp(0.0, 1.0);
    // 滑らかな S 字イージング (Smoothstep) ではなく線形（キーフレーム側でベジェ補間可能）
    pa.iter()
        .zip(pb.iter())
        .map(|(a, b)| [a[0] + (b[0] - a[0]) * tc, a[1] + (b[1] - a[1]) * tc])
        .collect()
}

/// ポリラインの正規化弧長位置 `frac` (`0.0..=1.0`) における座標と接線角度（ラジアン）を返す
pub fn sample_polyline_at_fraction(pts: &[[f32; 2]], frac: f32) -> ([f32; 2], f32) {
    if pts.len() < 2 {
        return (pts.first().copied().unwrap_or([0.5, 0.5]), 0.0);
    }
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0_f32);
    let mut total = 0.0_f32;
    for w in pts.windows(2) {
        total += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        cum.push(total);
    }
    if total < 1e-6 {
        return (pts[0], 0.0);
    }
    let target = total * frac.clamp(0.0, 1.0);
    let mut seg = 0usize;
    while seg + 1 < cum.len() - 1 && cum[seg + 1] < target {
        seg += 1;
    }
    let d0 = cum[seg];
    let d1 = cum[seg + 1];
    let u = if (d1 - d0).abs() < 1e-6 {
        0.0
    } else {
        ((target - d0) / (d1 - d0)).clamp(0.0, 1.0)
    };
    let a = pts[seg];
    let b = pts[seg + 1];
    let pos = [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u];
    let angle = (b[1] - a[1]).atan2(b[0] - a[0]);
    (pos, angle)
}

// ============================================================================
// #50 データモデル: SvgMorphConfig / TextOnPathConfig / WarpDisplaceConfig
// ============================================================================

fn default_svg_dim() -> f32 {
    420.0
}
fn default_stroke_w() -> f32 {
    5.0
}
fn default_one_f32() -> f32 {
    1.0
}
fn default_morph_samples() -> usize {
    72
}

/// `ItemKind::SvgMorph` 設定（SVG `<path>` インポート＆等弧長モーフィング）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgMorphConfig {
    #[serde(default = "default_svg_dim")]
    pub width: f32,
    #[serde(default = "default_svg_dim")]
    pub height: f32,
    /// 開始シェイプの SVG `<path d="...">` または `.svg` 本文
    pub path_a_d: String,
    /// 変形先シェイプの SVG `<path d="...">`（空なら静止 SVG 表示）
    #[serde(default)]
    pub path_b_d: String,
    /// モーフィング進行度 (`0.0..=1.0`, `"morph"` キーフレームでアニメーション可)
    #[serde(default)]
    pub morph_progress: f32,
    /// 自動ピンポン往復モーフィング速度 (Hz、0 で手動/キーフレーム制御)
    #[serde(default)]
    pub auto_morph_hz: f32,
    #[serde(default)]
    pub fill: VectorFillMode,
    pub stroke_color: RgbaColor,
    #[serde(default = "default_stroke_w")]
    pub stroke_width: f32,
    #[serde(default)]
    pub trim_start: f32,
    #[serde(default = "default_one_f32")]
    pub trim_end: f32,
    #[serde(default = "default_morph_samples")]
    pub sample_count: usize,
}

impl Default for SvgMorphConfig {
    fn default() -> Self {
        Self {
            width: default_svg_dim(),
            height: default_svg_dim(),
            path_a_d: SvgShapePreset::LightningBolt.svg_path_d().to_string(),
            path_b_d: SvgShapePreset::Heart.svg_path_d().to_string(),
            morph_progress: 0.0,
            auto_morph_hz: 0.5,
            fill: VectorFillMode::default(),
            stroke_color: RgbaColor::WHITE,
            stroke_width: default_stroke_w(),
            trim_start: 0.0,
            trim_end: 1.0,
            sample_count: default_morph_samples(),
        }
    }
}

fn default_text_path_d() -> String {
    SvgShapePreset::ArchCurve.svg_path_d().to_string()
}
fn default_true() -> bool {
    true
}
fn default_text_path_offset() -> f32 {
    0.12
}

/// テキストクリップ用 Text-on-Path（SVG 曲線に沿った接線回転タイポグラフィ）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextOnPathConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_text_path_d")]
    pub svg_path_d: String,
    /// パス上の開始オフセット (`0.0..=1.0`)
    #[serde(default = "default_text_path_offset")]
    pub path_offset: f32,
    /// 秒あたりのパス移動速度（スクロールマーキー演出）
    #[serde(default)]
    pub animate_speed: f32,
    /// 文字間トラッキング調整 (px)
    #[serde(default)]
    pub letter_spacing_px: f32,
    /// 曲線の接線角度へ各文字を自動回転追従させるか
    #[serde(default = "default_true")]
    pub follow_tangent: bool,
    /// 法線方向オフセット (px)
    #[serde(default)]
    pub vertical_offset_px: f32,
    /// パスガイド線をプレビューへ薄く表示するか
    #[serde(default)]
    pub show_guide: bool,
}

impl Default for TextOnPathConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            svg_path_d: default_text_path_d(),
            path_offset: default_text_path_offset(),
            animate_speed: 0.0,
            letter_spacing_px: 2.0,
            follow_tangent: true,
            vertical_offset_px: 0.0,
            show_guide: false,
        }
    }
}

/// ディスプレイスメント / グリッドワープのプリセット
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarpPreset {
    #[default]
    CustomGrid,
    FlagWave,
    BulgeLens,
    TwirlVortex,
    PerspectiveArch,
    RippleWater,
}

impl WarpPreset {
    pub const ALL: &'static [Self] = &[
        Self::CustomGrid,
        Self::FlagWave,
        Self::BulgeLens,
        Self::TwirlVortex,
        Self::PerspectiveArch,
        Self::RippleWater,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::CustomGrid => "🧩 4×4 グリッドメッシュワープ",
            Self::FlagWave => "🚩 フラッグウェーブ（旗揺れ）",
            Self::BulgeLens => "🔮 バルジ拡大（魚眼）",
            Self::TwirlVortex => "🌀 ツワール渦巻き",
            Self::PerspectiveArch => "🏛 アーチ湾曲",
            Self::RippleWater => "💧 水面リップル波紋",
        }
    }
}

fn default_grid_dim() -> usize {
    4
}
fn default_warp_amount() -> f32 {
    0.35
}
fn default_warp_freq() -> f32 {
    2.5
}
fn default_warp_speed() -> f32 {
    1.5
}

/// クリップ単位の非破壊ディスプレイスメント＆4×4双三次グリッドワープ設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarpDisplaceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub preset: WarpPreset,
    /// ワープ強度 (`-1.0..=1.0`)
    #[serde(default = "default_warp_amount")]
    pub amount: f32,
    /// 空間周波数（波の数）
    #[serde(default = "default_warp_freq")]
    pub frequency: f32,
    /// アニメーション速度 (Hz)
    #[serde(default = "default_warp_speed")]
    pub speed: f32,
    /// 自己輝度マップによる X/Y ピクセル変位強度 (px)
    #[serde(default)]
    pub luma_displace_px: f32,
    /// グリッド列数（既定 4）
    #[serde(default = "default_grid_dim")]
    pub grid_cols: usize,
    /// グリッド行数（既定 4）
    #[serde(default = "default_grid_dim")]
    pub grid_rows: usize,
    /// `grid_cols * grid_rows` 個の制御点変位 `[du, dv]`（正規化 -0.5..=0.5）
    #[serde(default)]
    pub control_offsets: Vec<[f32; 2]>,
}

impl Default for WarpDisplaceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preset: WarpPreset::FlagWave,
            amount: default_warp_amount(),
            frequency: default_warp_freq(),
            speed: default_warp_speed(),
            luma_displace_px: 0.0,
            grid_cols: default_grid_dim(),
            grid_rows: default_grid_dim(),
            control_offsets: vec![[0.0, 0.0]; 16],
        }
    }
}

impl WarpDisplaceConfig {
    pub fn ensure_grid_size(&mut self) {
        let cols = self.grid_cols.clamp(2, 8);
        let rows = self.grid_rows.clamp(2, 8);
        self.grid_cols = cols;
        self.grid_rows = rows;
        self.control_offsets.resize(cols * rows, [0.0, 0.0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_parser_handles_all_commands_including_arcs_and_xml() {
        let svg = r#"<svg viewBox="0 0 100 100"><path d="M10,50 A40,40 0 1,1 90,50 Q50,95 10,50 Z"/></svg>"#;
        let paths = extract_svg_path_data(svg);
        assert_eq!(paths.len(), 1);
        let (verts, closed) = parse_svg_path_to_vertices(svg).expect("parse svg");
        assert!(closed);
        assert!(verts.len() >= 3);
        for v in &verts {
            assert!(v.x.is_finite() && v.y.is_finite());
            assert!((0.0..=1.0).contains(&v.x));
            assert!((0.0..=1.0).contains(&v.y));
        }

        for &preset in SvgShapePreset::ALL {
            let (pv, _) = parse_svg_path_to_vertices(preset.svg_path_d()).expect("preset");
            assert!(pv.len() >= 2, "preset={preset:?}");
        }
    }

    #[test]
    fn equal_arclength_morph_interpolates_dissimilar_svg_shapes() {
        let (va, ca) =
            parse_svg_path_to_vertices(SvgShapePreset::LightningBolt.svg_path_d()).unwrap();
        let (vb, cb) = parse_svg_path_to_vertices(SvgShapePreset::Heart.svg_path_d()).unwrap();
        let m0 = morph_paths_equal_arclength(&va, ca, &vb, cb, 0.0, 64);
        let m5 = morph_paths_equal_arclength(&va, ca, &vb, cb, 0.5, 64);
        let m1 = morph_paths_equal_arclength(&va, ca, &vb, cb, 1.0, 64);
        assert_eq!(m0.len(), 64);
        assert_eq!(m5.len(), 64);
        assert_eq!(m1.len(), 64);
        // 中間点は両端の平均と厳密一致
        for i in 0..64 {
            assert!((m5[i][0] - 0.5 * (m0[i][0] + m1[i][0])).abs() < 1e-5);
            assert!((m5[i][1] - 0.5 * (m0[i][1] + m1[i][1])).abs() < 1e-5);
        }
    }

    #[test]
    fn text_on_path_tangent_follows_arch_curvature() {
        let (verts, closed) =
            parse_svg_path_to_vertices(SvgShapePreset::ArchCurve.svg_path_d()).unwrap();
        let poly = flatten_bezier_vertices(&verts, closed, 16);
        let (_, ang_left) = sample_polyline_at_fraction(&poly, 0.1);
        let (_, ang_mid) = sample_polyline_at_fraction(&poly, 0.5);
        let (_, ang_right) = sample_polyline_at_fraction(&poly, 0.9);
        // 上向きアーチ: 左側は右上向き（負Y→負角度）、頂上は水平(~0)、右側は右下向き（正角度）
        assert!(ang_left < ang_mid, "left={ang_left} mid={ang_mid}");
        assert!(ang_mid < ang_right, "mid={ang_mid} right={ang_right}");
    }
}
