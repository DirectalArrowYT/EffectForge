// A yellow line just outside the selected layer: drawn where the mask is dark but something within two pixels isn't.
@group(0) @binding(0) var mask: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let corner = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn lit(at: vec2i, size: vec2i) -> f32 {
    let texel = textureLoad(mask, clamp(at, vec2i(0), size - 1), 0);
    return max(texel.r, max(texel.g, texel.b));
}

@fragment
fn fs(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2i(textureDimensions(mask));
    let here = vec2i(position.xy);
    if lit(here, size) > 0.04 {
        discard;
    }
    var near = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            near = max(near, lit(here + vec2i(dx, dy), size));
        }
    }
    if near <= 0.04 {
        discard;
    }
    return vec4f(1.0, 0.85, 0.0, 1.0);
}
