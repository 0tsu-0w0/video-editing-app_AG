// Linear sRGB (Rgba16Float) + Premultiplied Alpha フルスクリーン合成＆出力シェーダー

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOut {
    // フルスクリーントライアングル
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

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

fn linear_to_srgb_channel(c: f32) -> f32 {
    let x = clamp(c, 0.0, 1.0);
    if (x <= 0.0031308) {
        return x * 12.92;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

@fragment
fn fs_to_srgb(in: VertexOut) -> @location(0) vec4<f32> {
    let premul = textureSample(src_tex, src_sampler, in.uv);
    let a = clamp(premul.a, 0.0, 1.0);
    var rgb = vec3<f32>(0.0);
    if (a > 1e-6) {
        rgb = premul.rgb / a;
    }
    return vec4<f32>(
        linear_to_srgb_channel(rgb.r),
        linear_to_srgb_channel(rgb.g),
        linear_to_srgb_channel(rgb.b),
        a,
    );
}
