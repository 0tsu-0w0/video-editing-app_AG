// v1.0 #18: Linear sRGB Premultiplied Alpha GPU Compute Shader Compositor
// 2 枚の Linear RGBA32F バッファを 8x8 ワークグループ並列で合成する。

struct ComputeCompositeParams {
    width: u32,
    height: u32,
    blend_mode: u32, // 0=Normal, 1=Add, 2=Multiply, 3=Screen
    opacity: f32,
};

@group(0) @binding(0) var<storage, read> dst_buf: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> src_buf: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> out_buf: array<vec4<f32>>;
@group(0) @binding(3) var<uniform> params: ComputeCompositeParams;

@compute @workgroup_size(8, 8, 1)
fn cs_composite(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    let idx = gid.y * params.width + gid.x;
    let d = dst_buf[idx];
    let s_raw = src_buf[idx];
    let op = clamp(params.opacity, 0.0, 1.0);
    let s = s_raw * op;

    let inv_sa = max(1.0 - s.a, 0.0);
    let out_a = clamp(s.a + d.a * inv_sa, 0.0, 1.0);

    var out_rgb: vec3<f32>;
    if (params.blend_mode == 1u) {
        // Add
        out_rgb = d.rgb + s.rgb;
    } else if (params.blend_mode == 2u) {
        // Multiply (premultiplied form)
        out_rgb = s.rgb * d.rgb + s.rgb * max(1.0 - d.a, 0.0) + d.rgb * inv_sa;
    } else if (params.blend_mode == 3u) {
        // Screen (premultiplied form)
        out_rgb = s.rgb + d.rgb - s.rgb * d.rgb;
    } else {
        // Normal (Porter-Duff Source-Over)
        out_rgb = s.rgb + d.rgb * inv_sa;
    }

    out_buf[idx] = vec4<f32>(max(out_rgb, vec3<f32>(0.0)), out_a);
}
