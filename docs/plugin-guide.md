# プラグイン＆カスタム WGSL シェーダー開発ガイド

`video-editing-app_AG` は、**カスタム WGSL シェーダーのホットリロード**と、Rust クレート `ve-plugin-api` による**エフェクト／ジェネレータプラグイン拡張**の 2 系統の拡張 API を提供します。

---

## 1. カスタム WGSL シェーダー（ホットリロード対応）

アプリ起動時に `shaders/` ディレクトリ配下の `.wgsl` ファイルが自動登録されます（インスペクタの「カスタム WGSL ホットリロード」欄から任意のパスを追加することも可能です）。

### サンプルシェーダー（[shaders/custom_chromatic_aberration.wgsl](../shaders/custom_chromatic_aberration.wgsl)）

```wgsl
struct CustomParams {
    strength: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: CustomParams;

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@fragment
fn fs_chromatic_aberration(in: VertexOut) -> @location(0) vec4<f32> {
    let shift = vec2<f32>(params.strength * 0.008, 0.0);
    let r = textureSample(src_tex, src_sampler, in.uv + shift).r;
    let g = textureSample(src_tex, src_sampler, in.uv).g;
    let b = textureSample(src_tex, src_sampler, in.uv - shift).b;
    let a = textureSample(src_tex, src_sampler, in.uv).a;
    return vec4<f32>(r, g, b, a);
}
```

### ホットリロードの安全機構
* ファイルを保存すると、裏側で `naga` フロントエンド＆バリデータが構文・型チェックを実行します。
* **構文エラー時:** 最後に正常だったリビジョンのシェーダーを維持したまま、UI 上に正確なコンパイル診断メッセージを表示します（アプリはクラッシュしません）。
* AI エージェントからも内蔵 MCP ツール `validate_custom_wgsl` を使って事前に検証できます。

---

## 2. Rust プラグイン開発 (`ve-plugin-api`)

`ve_plugin_api::EffectPlugin` トレイトを実装することで、オブジェクト個別エフェクトチェーンおよび AviUtl2 風フィルタオブジェクトの両方から利用できるカスタムエフェクトを追加できます。

```rust
use std::collections::BTreeMap;
use ve_plugin_api::{
    EffectPlugin, PLUGIN_API_VERSION, PluginCapability, PluginManifest, PluginParamDef,
};

pub struct MyWarmGlowPlugin;

impl EffectPlugin for MyWarmGlowPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            api_version: PLUGIN_API_VERSION,
            id: "community.warm_glow".into(),
            name: "ウォームグロー".into(),
            version: "0.1.0".into(),
            author: "your-name".into(),
            description: "暖色寄りのソフトゲインを適用します".into(),
            capabilities: vec![PluginCapability::VideoEffect],
            params: vec![PluginParamDef {
                key: "amount".into(),
                label: "強さ".into(),
                default_value: 0.2,
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
        let amount = params.get("amount").copied().unwrap_or(0.2).clamp(0.0, 1.0);
        for px in pixels {
            // pixels は Linear sRGB + Premultiplied Alpha
            px[0] *= 1.0 + amount * 0.4;
            px[2] *= 1.0 - amount * 0.2;
        }
    }
}
```

### レジストリ登録
```rust
use std::sync::Arc;
use ve_render::RenderGraph;

let mut graph = RenderGraph::new();
graph.plugins.register_effect(Arc::new(MyWarmGlowPlugin))?;
```

`PLUGIN_API_VERSION` が一致しないプラグインは登録時に安全に拒否されます（`PluginError::IncompatibleApiVersion`）。
