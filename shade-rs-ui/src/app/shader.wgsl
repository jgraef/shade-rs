struct ShadeRs {
    time: f32,
    aspect: f32,
    mouse: vec2f,
}

@group(0) @binding(0)
var<uniform> input: ShadeRs;

struct VertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) position: vec2f,
}

struct FragmentOutput {
    @location(0) color: vec4f,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let vertex_position = vec2f(4.0 * f32(vertex_index & 1) - 1.0, 2.0 * f32(vertex_index & 2) - 1.0);
    out.clip_position = vec4f(vertex_position, 0.0, 1.0);
    out.position = out.clip_position.xy;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;
    let rgb = 0.5 + 0.5 * cos(input.time + in.position.xyx + vec3(0.0, input.mouse.xy));
    out.color = vec4f(rgb, 1.0);
    return out;
}
