use crate::{EffectPlugin, PluginCapability, PluginManifest, PluginParamDef};
use std::collections::BTreeMap;

/// リファレンス実装プラグイン1: ビネット（周辺減光）エフェクト
pub struct VignetteEffectPlugin;

impl EffectPlugin for VignetteEffectPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            api_version: crate::PLUGIN_API_VERSION,
            id: "plugin.vignette".to_string(),
            name: "ビネット (Plugin)".to_string(),
            version: "0.1.0".to_string(),
            author: "video-editing-app_AG".to_string(),
            description: "画面四隅を滑らかに減光するリファレンスエフェクトプラグイン".to_string(),
            capabilities: vec![PluginCapability::VideoEffect],
            params: vec![
                PluginParamDef {
                    key: "strength".to_string(),
                    label: "強さ".to_string(),
                    default_value: 0.55,
                    min_value: 0.0,
                    max_value: 1.5,
                },
                PluginParamDef {
                    key: "softness".to_string(),
                    label: "柔らかさ".to_string(),
                    default_value: 0.65,
                    min_value: 0.1,
                    max_value: 2.0,
                },
            ],
        }
    }

    fn process_linear_premul_rgba(
        &self,
        pixels: &mut [[f32; 4]],
        width: u32,
        height: u32,
        _time_sec: f64,
        params: &BTreeMap<String, f32>,
    ) {
        let total = (width as usize).saturating_mul(height as usize);
        if total == 0 || pixels.len() < total {
            return;
        }
        let strength = params
            .get("strength")
            .copied()
            .unwrap_or(0.55)
            .clamp(0.0, 2.0);
        let softness = params.get("softness").copied().unwrap_or(0.65).max(0.05);
        let factor = (strength * 0.5) / (softness * softness);
        let inv_w = 2.0 / width as f32;
        let inv_h = 2.0 / height as f32;

        for y in 0..height {
            let ny = (y as f32 + 0.5) * inv_h - 1.0;
            let ny2 = ny * ny;
            let row = (y as usize) * (width as usize);
            for x in 0..width {
                let nx = (x as f32 + 0.5) * inv_w - 1.0;
                let dist_sq = nx * nx + ny2;
                let atten = (1.0 - dist_sq * factor).clamp(0.0, 1.0);
                let px = &mut pixels[row + x as usize];
                px[0] *= atten;
                px[1] *= atten;
                px[2] *= atten;
            }
        }
    }
}

/// リファレンス実装プラグイン2: セピア調カラーフィルタ
pub struct SepiaEffectPlugin;

impl EffectPlugin for SepiaEffectPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            api_version: crate::PLUGIN_API_VERSION,
            id: "plugin.sepia".to_string(),
            name: "セピア調 (Plugin)".to_string(),
            version: "0.1.0".to_string(),
            author: "video-editing-app_AG".to_string(),
            description: "Linear sRGB 空間で温かみのあるセピア調へ変換するプラグイン".to_string(),
            capabilities: vec![PluginCapability::VideoEffect],
            params: vec![PluginParamDef {
                key: "mix".to_string(),
                label: "適用量".to_string(),
                default_value: 0.85,
                min_value: 0.0,
                max_value: 1.0,
            }],
        }
    }

    fn process_linear_premul_rgba(
        &self,
        pixels: &mut [[f32; 4]],
        _width: u32,
        _height: u32,
        _time_sec: f64,
        params: &BTreeMap<String, f32>,
    ) {
        let mix = params.get("mix").copied().unwrap_or(0.85).clamp(0.0, 1.0);
        for px in pixels {
            let a = px[3];
            if a <= 1e-6 {
                continue;
            }
            let r = px[0] / a;
            let g = px[1] / a;
            let b = px[2] / a;
            let sr = (r * 0.393 + g * 0.769 + b * 0.189).min(1.5);
            let sg = (r * 0.349 + g * 0.686 + b * 0.168).min(1.5);
            let sb = (r * 0.272 + g * 0.534 + b * 0.131).min(1.5);
            px[0] = (r + (sr - r) * mix) * a;
            px[1] = (g + (sg - g) * mix) * a;
            px[2] = (b + (sb - b) * mix) * a;
        }
    }
}
