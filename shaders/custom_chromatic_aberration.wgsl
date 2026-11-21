// カスタム WGSL シェーダーサンプル（ホットリロード対応）
// 保存するとアプリ実行中に naga で自動検証され、即座に反映されます。

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
