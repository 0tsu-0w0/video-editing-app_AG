// Phase 2 基本 WGSL エフェクトチェーンシェーダー（Linear sRGB Rgba16Float + Premultiplied Alpha）
// - ブラー (box/gaussian)
// - グロー (threshold + blur + additive)
// - 色調補正 (exposure / contrast / saturation)
// - クリッピング (top / bottom / left / right)

struct EffectParams {
    p0: f32,
    p1: f32,
    p2: f32,
    p3: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: EffectParams;

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) idx: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 3.0,  1.0),
    );
    let p = positions[idx];
    var out: VertexOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

@fragment
fn fs_color_grade(in: VertexOut) -> @location(0) vec4<f32> {
    let premul = textureSample(src_tex, src_sampler, in.uv);
    let a = premul.a;
    if (a <= 1e-6) {
        return vec4<f32>(0.0);
    }
    var rgb = premul.rgb / a;
    let exposure = params.p0;
    let contrast = params.p1;
    let saturation = params.p2;

    rgb = rgb * pow(2.0, exposure);
    rgb = (rgb - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5);
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    rgb = mix(vec3<f32>(luma), rgb, saturation);
    rgb = max(rgb, vec3<f32>(0.0));
    return vec4<f32>(rgb * a, a);
}

@fragment
fn fs_blur_9tap(in: VertexOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(src_tex));
    let texel = vec2<f32>(params.p0, params.p1) / max(dims, vec2<f32>(1.0));
    var acc = vec4<f32>(0.0);
    for (var i: i32 = -4; i <= 4; i = i + 1) {
        acc = acc + textureSample(src_tex, src_sampler, in.uv + texel * f32(i));
    }
    return acc / 9.0;
}

@fragment
fn fs_glow(in: VertexOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(src_tex));
    let radius = max(params.p0, 1.0);
    let threshold = params.p1;
    let intensity = params.p2;
    let step_uv = vec2<f32>(radius) / max(dims, vec2<f32>(1.0));
    let base = textureSample(src_tex, src_sampler, in.uv);

    var bloom = vec3<f32>(0.0);
    var count = 0.0;
    for (var dy: i32 = -2; dy <= 2; dy = dy + 1) {
        for (var dx: i32 = -2; dx <= 2; dx = dx + 1) {
            let s = textureSample(src_tex, src_sampler, in.uv + vec2<f32>(f32(dx), f32(dy)) * step_uv);
            let luma = dot(s.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
            if (luma > threshold) {
                bloom = bloom + s.rgb * (luma - threshold);
            }
            count = count + 1.0;
        }
    }
    return vec4<f32>(base.rgb + (bloom / max(count, 1.0)) * intensity, base.a);
}

@fragment
fn fs_clip_rect(in: VertexOut) -> @location(0) vec4<f32> {
    let left = params.p0;
    let right = params.p1;
    let top = params.p2;
    let bottom = params.p3;
    if (in.uv.x < left || in.uv.x > (1.0 - right) || in.uv.y < top || in.uv.y > (1.0 - bottom)) {
        return vec4<f32>(0.0);
    }
    return textureSample(src_tex, src_sampler, in.uv);
}
