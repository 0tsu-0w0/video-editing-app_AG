pub mod builtin;
pub mod wasm_runtime;

pub use wasm_runtime::{
    WasmEffectPlugin, WasmError, WasmInstance, WasmModule, WasmPluginManager, WasmPluginStatus,
    WasmVal, build_sample_wasm_grayscale, build_sample_wasm_invert, build_sample_wasm_warm_tint,
    emit_sample_wasm_plugins,
};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;
use ve_core::{FrameRate, RationalTime, TimelineItem};

/// 現在の安定プラグイン API バージョン
pub const PLUGIN_API_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("Incompatible plugin API version: plugin={found}, host={expected}")]
    IncompatibleApiVersion { found: u32, expected: u32 },
    #[error("Plugin already registered: {0}")]
    DuplicateId(String),
    #[error("Plugin not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    VideoEffect,
    TimelineGenerator,
    CustomWgslShader,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginParamDef {
    pub key: String,
    pub label: String,
    pub default_value: f32,
    pub min_value: f32,
    pub max_value: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub api_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub capabilities: Vec<PluginCapability>,
    pub params: Vec<PluginParamDef>,
}

/// 映像エフェクトプラグインの統一トレイト（Linear sRGB + Premultiplied Alpha ピクセル処理）
pub trait EffectPlugin: Send + Sync {
    fn manifest(&self) -> PluginManifest;

    /// `pixels` は `[r, g, b, a]`（Linear sRGB / Premultiplied Alpha）の行優先配列
    fn process_linear_premul_rgba(
        &self,
        pixels: &mut [[f32; 4]],
        width: u32,
        height: u32,
        time_sec: f64,
        params: &BTreeMap<String, f32>,
    );
}

#[derive(Debug, Clone)]
pub struct GeneratorContext {
    pub start_time: RationalTime,
    pub fps: FrameRate,
    pub canvas_width: u32,
    pub canvas_height: u32,
}

/// タイムラインアイテム自動生成プラグイン（テロップテンプレート・パーティクル等）
pub trait TimelineGeneratorPlugin: Send + Sync {
    fn manifest(&self) -> PluginManifest;
    fn generate_items(&self, ctx: &GeneratorContext) -> Vec<TimelineItem>;
}

/// ホスト側のプラグインレジストリ（APIバージョン検証・実行ディスパッチ・WASMホットリロード対応）
#[derive(Default)]
pub struct PluginRegistry {
    effects: BTreeMap<String, Arc<dyn EffectPlugin>>,
    generators: BTreeMap<String, Arc<dyn TimelineGeneratorPlugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 組み込みリファレンスプラグインを登録した標準レジストリを生成する
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        let _ = reg.register_effect(Arc::new(builtin::VignetteEffectPlugin));
        let _ = reg.register_effect(Arc::new(builtin::SepiaEffectPlugin));
        reg
    }

    pub fn register_effect(&mut self, plugin: Arc<dyn EffectPlugin>) -> Result<(), PluginError> {
        let manifest = plugin.manifest();
        if manifest.api_version != PLUGIN_API_VERSION {
            return Err(PluginError::IncompatibleApiVersion {
                found: manifest.api_version,
                expected: PLUGIN_API_VERSION,
            });
        }
        if self.effects.contains_key(&manifest.id) {
            return Err(PluginError::DuplicateId(manifest.id));
        }
        self.effects.insert(manifest.id, plugin);
        Ok(())
    }

    /// ホットリロード用: 既存 ID があれば安全に最新プラグインへ差し替える
    pub fn register_or_replace_effect(
        &mut self,
        plugin: Arc<dyn EffectPlugin>,
    ) -> Result<(), PluginError> {
        let manifest = plugin.manifest();
        if manifest.api_version != PLUGIN_API_VERSION {
            return Err(PluginError::IncompatibleApiVersion {
                found: manifest.api_version,
                expected: PLUGIN_API_VERSION,
            });
        }
        self.effects.insert(manifest.id, plugin);
        Ok(())
    }

    pub fn unregister_effect(&mut self, id: &str) -> bool {
        self.effects.remove(id).is_some()
    }

    pub fn register_generator(
        &mut self,
        plugin: Arc<dyn TimelineGeneratorPlugin>,
    ) -> Result<(), PluginError> {
        let manifest = plugin.manifest();
        if manifest.api_version != PLUGIN_API_VERSION {
            return Err(PluginError::IncompatibleApiVersion {
                found: manifest.api_version,
                expected: PLUGIN_API_VERSION,
            });
        }
        if self.generators.contains_key(&manifest.id) {
            return Err(PluginError::DuplicateId(manifest.id));
        }
        self.generators.insert(manifest.id, plugin);
        Ok(())
    }

    pub fn get_effect(&self, id: &str) -> Option<&Arc<dyn EffectPlugin>> {
        self.effects.get(id)
    }

    pub fn list_manifests(&self) -> Vec<PluginManifest> {
        let mut list: Vec<PluginManifest> = self.effects.values().map(|p| p.manifest()).collect();
        list.extend(self.generators.values().map(|g| g.manifest()));
        list
    }

    pub fn apply_effect(
        &self,
        effect_id: &str,
        pixels: &mut [[f32; 4]],
        width: u32,
        height: u32,
        time_sec: f64,
        params: &BTreeMap<String, f32>,
    ) -> Result<(), PluginError> {
        let plugin = self
            .effects
            .get(effect_id)
            .ok_or_else(|| PluginError::NotFound(effect_id.to_string()))?;
        plugin.process_linear_premul_rgba(pixels, width, height, time_sec, params);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BadVersionPlugin;
    impl EffectPlugin for BadVersionPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                api_version: 999,
                id: "plugin.bad".into(),
                name: "Bad".into(),
                version: "0.0.1".into(),
                author: "test".into(),
                description: String::new(),
                capabilities: vec![PluginCapability::VideoEffect],
                params: vec![],
            }
        }
        fn process_linear_premul_rgba(
            &self,
            _pixels: &mut [[f32; 4]],
            _width: u32,
            _height: u32,
            _time_sec: f64,
            _params: &BTreeMap<String, f32>,
        ) {
        }
    }

    #[test]
    fn validates_api_version_and_executes_builtin_plugins() {
        let mut reg = PluginRegistry::with_builtins();
        assert_eq!(reg.list_manifests().len(), 2);

        let err = reg.register_effect(Arc::new(BadVersionPlugin)).unwrap_err();
        assert!(matches!(err, PluginError::IncompatibleApiVersion { .. }));

        // ビネット: 四隅が中央より暗くなること
        let mut pixels = vec![[1.0, 1.0, 1.0, 1.0]; 16 * 16];
        let mut params = BTreeMap::new();
        params.insert("strength".into(), 1.0);
        reg.apply_effect("plugin.vignette", &mut pixels, 16, 16, 0.0, &params)
            .unwrap();
        let corner = pixels[0][0];
        let center = pixels[8 * 16 + 8][0];
        assert!(center > corner, "center={center}, corner={corner}");
    }
}
