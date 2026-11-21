use crate::color::{LinearImage, LinearPremul};
use psd::Psd;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use ve_core::{MouthShape, PsdCharacterConfig};

/// PSD ファイルの事前デコード済みレイヤーキャッシュ（目パチ・口パク高速切替用）
pub struct PsdCharacterAsset {
    pub width: u32,
    pub height: u32,
    /// PSD 内の下から上へのレイヤー順（`(layer_name, originally_visible, opacity, linear_img)`）
    layers: Vec<DecodedPsdLayer>,
}

struct DecodedPsdLayer {
    name: String,
    originally_visible: bool,
    opacity: f32,
    image: LinearImage,
}

impl PsdCharacterAsset {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|e| format!("PSD read error: {e}"))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let psd = Psd::from_bytes(bytes).map_err(|e| format!("PSD parse error: {e}"))?;
        let width = psd.width().max(1);
        let height = psd.height().max(1);

        let mut layers = Vec::with_capacity(psd.layers().len());
        // `psd.layers()` は Photoshop UI 準拠の上から下（手前から奥）の並び順のため、
        // SrcOver 合成用に逆順（奥から手前）で保持する
        for layer in psd.layers().iter().rev() {
            let rgba = layer.rgba();
            if rgba.len() == (width as usize) * (height as usize) * 4 {
                let image = LinearImage::from_rgba8_srgb(width, height, &rgba);
                layers.push(DecodedPsdLayer {
                    name: layer.name().to_string(),
                    originally_visible: layer.visible(),
                    opacity: f32::from(layer.opacity()) / 255.0,
                    image,
                });
            }
        }

        // レイヤーセクションが空のフラットPSDの場合は合成済み画像を単一レイヤーとして扱う
        if layers.is_empty() {
            let rgba = psd.rgba();
            if rgba.len() == (width as usize) * (height as usize) * 4 {
                layers.push(DecodedPsdLayer {
                    name: "Background".to_string(),
                    originally_visible: true,
                    opacity: 1.0,
                    image: LinearImage::from_rgba8_srgb(width, height, &rgba),
                });
            }
        }

        Ok(Self {
            width,
            height,
            layers,
        })
    }

    pub fn layer_names(&self) -> Vec<&str> {
        self.layers.iter().map(|l| l.name.as_str()).collect()
    }

    /// 現在の口形状 (`mouth`) と目パチ状態 (`eye_closed`) に応じて PSD 内レイヤーを Linear 空間で直接合成する。
    /// Photoshop / CLIP STUDIO 上で非表示（`visible == false`）になっている口・目差分レイヤーも正確に描画する。
    pub fn composite(
        &self,
        config: &PsdCharacterConfig,
        mouth: MouthShape,
        eye_closed: bool,
    ) -> LinearImage {
        let mut controlled: HashSet<&str> = HashSet::new();
        for name in config.mouth_layers.values() {
            controlled.insert(name.as_str());
        }
        if let Some(ref e) = config.eye_open_layer {
            controlled.insert(e.as_str());
        }
        if let Some(ref e) = config.eye_closed_layer {
            controlled.insert(e.as_str());
        }

        let active_mouth_name = resolve_mouth_layer_name(&config.mouth_layers, mouth);
        let active_eye_name = if eye_closed {
            config
                .eye_closed_layer
                .as_deref()
                .or(config.eye_open_layer.as_deref())
        } else {
            config.eye_open_layer.as_deref()
        };

        let explicit_bases: HashSet<&str> = config.base_layers.iter().map(String::as_str).collect();

        let mut canvas = LinearImage::new(self.width, self.height, LinearPremul::TRANSPARENT);
        for layer in &self.layers {
            let name = layer.name.as_str();
            let should_draw = if active_mouth_name == Some(name) || active_eye_name == Some(name) {
                true
            } else if controlled.contains(name) {
                false
            } else if !explicit_bases.is_empty() {
                explicit_bases.contains(name)
            } else {
                layer.originally_visible
            };

            if should_draw {
                canvas.blend_over(&layer.image, 0, 0, layer.opacity);
            }
        }
        canvas
    }
}

fn resolve_mouth_layer_name(
    map: &BTreeMap<MouthShape, String>,
    desired: MouthShape,
) -> Option<&str> {
    if let Some(name) = map.get(&desired) {
        return Some(name.as_str());
    }
    let fallbacks: &[MouthShape] = match desired {
        MouthShape::A | MouthShape::O => {
            &[MouthShape::Open, MouthShape::HalfOpen, MouthShape::Closed]
        }
        MouthShape::I | MouthShape::U | MouthShape::E => {
            &[MouthShape::HalfOpen, MouthShape::Open, MouthShape::Closed]
        }
        MouthShape::Open => &[MouthShape::A, MouthShape::HalfOpen, MouthShape::Closed],
        MouthShape::HalfOpen => &[MouthShape::E, MouthShape::Open, MouthShape::Closed],
        MouthShape::Closed => &[MouthShape::HalfOpen],
    };
    for fb in fallbacks {
        if let Some(name) = map.get(fb) {
            return Some(name.as_str());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PSD 仕様準拠の最小マルチレイヤーバイナリ（非圧縮 8bit RGBA）をテスト用に構築する
    fn build_synthetic_psd(width: u32, height: u32, layers: &[(&str, bool, [u8; 4])]) -> Vec<u8> {
        let mut out = Vec::new();
        // 1. File Header Section (26 bytes)
        out.extend_from_slice(b"8BPS");
        out.extend_from_slice(&1u16.to_be_bytes()); // version 1
        out.extend_from_slice(&[0u8; 6]); // reserved
        out.extend_from_slice(&4u16.to_be_bytes()); // 4 channels (RGBA)
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&8u16.to_be_bytes()); // depth 8-bit
        out.extend_from_slice(&3u16.to_be_bytes()); // RGB ColorMode

        // 2. Color Mode Data Section (empty)
        out.extend_from_slice(&0u32.to_be_bytes());
        // 3. Image Resources Section (empty)
        out.extend_from_slice(&0u32.to_be_bytes());

        // 4. Layer and Mask Information Section
        let px_count = (width * height) as usize;
        let chan_data_len = (2 + px_count) as u32; // 2 bytes compression=0 + raw channel bytes

        let mut layer_info = Vec::new();
        layer_info.extend_from_slice(&(layers.len() as i16).to_be_bytes());

        for &(name, visible, _) in layers {
            // Rect: top, left, bottom, right
            layer_info.extend_from_slice(&0i32.to_be_bytes());
            layer_info.extend_from_slice(&0i32.to_be_bytes());
            layer_info.extend_from_slice(&(height as i32).to_be_bytes());
            layer_info.extend_from_slice(&(width as i32).to_be_bytes());
            // 4 channels: R(0), G(1), B(2), A(-1)
            layer_info.extend_from_slice(&4u16.to_be_bytes());
            for ch_id in [0i16, 1, 2, -1] {
                layer_info.extend_from_slice(&ch_id.to_be_bytes());
                layer_info.extend_from_slice(&chan_data_len.to_be_bytes());
            }
            layer_info.extend_from_slice(b"8BIM");
            layer_info.extend_from_slice(b"norm");
            layer_info.push(255); // opacity
            layer_info.push(0); // clipping
            // flags: bit 1 (0x02) = invisible
            layer_info.push(if visible { 0x00 } else { 0x02 });
            layer_info.push(0); // filler

            // Extra data field: mask(4) + blending_ranges(4) + pascal_name(padded to 4 bytes)
            let name_bytes = name.as_bytes();
            let mut pascal = Vec::new();
            pascal.push(name_bytes.len() as u8);
            pascal.extend_from_slice(name_bytes);
            while !pascal.len().is_multiple_of(4) {
                pascal.push(0);
            }
            let extra_len = (4 + 4 + pascal.len()) as u32;
            layer_info.extend_from_slice(&extra_len.to_be_bytes());
            layer_info.extend_from_slice(&0u32.to_be_bytes()); // mask size
            layer_info.extend_from_slice(&0u32.to_be_bytes()); // blending ranges size
            layer_info.extend_from_slice(&pascal);
        }

        // Channel image data for each layer
        for &(_, _, rgba) in layers {
            for c in rgba {
                layer_info.extend_from_slice(&0u16.to_be_bytes()); // Raw compression
                layer_info.extend(std::iter::repeat_n(c, px_count));
            }
        }
        if !layer_info.len().is_multiple_of(2) {
            layer_info.push(0);
        }

        let mut layer_mask_section = Vec::new();
        layer_mask_section.extend_from_slice(&(layer_info.len() as u32).to_be_bytes());
        layer_mask_section.extend_from_slice(&layer_info);
        layer_mask_section.extend_from_slice(&0u32.to_be_bytes()); // global layer mask info

        out.extend_from_slice(&(layer_mask_section.len() as u32).to_be_bytes());
        out.extend_from_slice(&layer_mask_section);

        // 5. Image Data Section (merged image, raw compression)
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend(std::iter::repeat_n(0u8, px_count * 4));
        out
    }

    #[test]
    fn psd_native_switches_hidden_mouth_and_blink_layers() {
        // body(白), eye_open(青・表示), eye_closed(黄・非表示), mouth_closed(赤・非表示), mouth_open(緑・非表示)
        let bytes = build_synthetic_psd(
            8,
            8,
            &[
                ("body", true, [255, 255, 255, 255]),
                ("eye_open", true, [0, 0, 255, 255]),
                ("eye_closed", false, [255, 255, 0, 255]),
                ("mouth_closed", false, [255, 0, 0, 255]),
                ("mouth_open", false, [0, 255, 0, 255]),
            ],
        );
        let asset = PsdCharacterAsset::from_bytes(&bytes).unwrap();
        assert_eq!(asset.layer_names().len(), 5);

        let mut mouth_layers = BTreeMap::new();
        mouth_layers.insert(MouthShape::Closed, "mouth_closed".into());
        mouth_layers.insert(MouthShape::Open, "mouth_open".into());

        let cfg = PsdCharacterConfig {
            base_layers: vec!["body".into()],
            mouth_layers,
            eye_open_layer: Some("eye_open".into()),
            eye_closed_layer: Some("eye_closed".into()),
            ..Default::default()
        };

        // 閉じ口（赤が最前面）
        let img_closed = asset.composite(&cfg, MouthShape::Closed, false);
        assert!(img_closed.pixels[0].r > 0.9 && img_closed.pixels[0].g < 0.1);

        // 開き口（非表示だった緑レイヤーがアクティブになって最前面）
        let img_open = asset.composite(&cfg, MouthShape::Open, false);
        assert!(img_open.pixels[0].g > 0.9 && img_open.pixels[0].r < 0.1);

        // 目パチ時の判定
        assert!(!cfg.is_eye_closed_at(1.0));
        assert!(cfg.is_eye_closed_at(3.45));
    }
}
