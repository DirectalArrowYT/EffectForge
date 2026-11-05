// The selected layer's shape, flat and see-through: drawn where the mask is lit. How see-through comes from the
// pipeline's blend constant.
@group(0) @binding(0) var mask: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let corner = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = textureLoad(mask, vec2i(position.xy), 0);
    if max(texel.r, max(texel.g, texel.b)) < 0.5 {
        discard;
    }
    return vec4f(0.55, 0.9, 1.0, 1.0);
}
