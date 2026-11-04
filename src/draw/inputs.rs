//! Effect Forge's own inputs for the game's effect shaders (eff_gpu).
//!
//! Every input a captured effect draw binds, rebuilt from the .eff, the effect's placement and its clock, so the
//! game's shaders can draw an effect the game never ran. Each was checked against the game's own, captured from the recomp:
//!
//! * c9, `sysEmitterStaticUniformBlock`: [`crate::draw::block::emitter_block`], byte for byte.
//! * c8, `sysViewUniformBlock`: [`crate::draw::gpu::camera_block`], from the viewport's camera.
//! * c10, `sysEmitterDynamicUniformBlock`: [`dynamic_block`]: the effect's colour, the emitter's clock and particle
//!   count, and its matrix (placement x the emitter's own transform from emitter_info).
//! * c13 / c16, the custom shader blocks: the game's global values ([`RESERVED_PARAM`], [`CUSTOM_BLOCK2`]).
//! * vertex inputs: [`particle_inputs`], each particle as born ([`crate::draw::sim::spawns`]), routed to the
//!   shader's own attribute names (its reflection).
//! * blend / depth / cull: [`draw_state`], from render_state.

use crate::draw::gpu::{DrawState, Reflection, VertexInput};

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The emitter's own transform within its effect (emitter_info): translate, then rotate (radians, Z then Y then X,
/// the order that fits the game's matrices best), then scale.
pub fn emitter_local(stored: &[u8]) -> (glam::Mat4, glam::Mat4) {
    let v = |at: usize| glam::Vec3::new(f32_at(stored, at), f32_at(stored, at + 4), f32_at(stored, at + 8));
    let (t, r, s) = (v(0x770), v(0x788), v(0x7a0));
    let rotation = glam::Mat4::from_rotation_z(r.z) * glam::Mat4::from_rotation_y(r.y) * glam::Mat4::from_rotation_x(r.x);
    let rt = glam::Mat4::from_translation(t) * rotation;
    (rt * glam::Mat4::from_scale(s), rt)
}

/// The per-emitter, per-frame block (c10).
#[derive(Clone, Copy, Debug)]
pub struct Dynamic {
    /// The effect's colour multipliers (1 unless the script tints it), for the emitter's two colours.
    pub colour0: [f32; 4],
    pub colour1: [f32; 4],
    /// Frames on the emitter's clock.
    pub time: f32,
    pub particles: u32,
    /// The effect's particle scale (x, y): 1 unless set on the effect.
    pub particle_scale: [f32; 2],
    /// The effect's size: (a factor that is 1 in ordinary play, then the size per axis). The spawn call's size, times
    /// the fighter's scale in game; the mesh and billboard shaders scale by it on top of the matrix.
    pub set_scale: [f32; 4],
    /// World matrix of the emitter, with and without its scale.
    pub matrix: glam::Mat4,
    pub matrix_rt: glam::Mat4,
}

/// c10, laid out as the game writes it: colour0, colour1, (time, particle count, particle scale x, y), the effect's
/// size, then both matrices row-major.
pub fn dynamic_block(d: &Dynamic) -> Vec<u8> {
    let mut f = Vec::with_capacity(48);
    f.extend_from_slice(&d.colour0);
    f.extend_from_slice(&d.colour1);
    f.extend_from_slice(&[d.time, d.particles as f32, d.particle_scale[0], d.particle_scale[1]]);
    f.extend_from_slice(&d.set_scale);
    for m in [d.matrix, d.matrix_rt] {
        for r in 0..4 {
            f.extend_from_slice(&m.row(r).to_array());
        }
    }
    bytemuck::cast_slice(&f).to_vec()
}

/// c13, `sysCustomShaderReservedUniformBlockParam`, as the game had it for most draws of a capture.
pub const RESERVED_PARAM: [f32; 4] = [0.5, 2.0, 0.0, 1.0];
/// c16, `sysCustomShaderUniformBlock2`, as the game has it in ordinary play. Floats 18 and 19 are (1, 0) there; the
/// ef_common cycle run, after effects that leave a global state behind, had (0.5, 0.5), which shrinks attack arcs.
pub const CUSTOM_BLOCK2: [f32; 24] = [
    0.5, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, -99999.0, 1.0, 0.0, 1.0, 0.0, 0.0,
    0.0,
];

/// Blend, depth and culling from the emitter's render_state, as the game set them (every captured draw):
/// blend_type 0 alpha, 1 additive, 2 subtractive; is_depth_mask 0 = depth tested, not written, 1 = no depth test;
/// display_side 1 / 2 culls. `prim` and `indexed` are the draw's own.
pub fn draw_state(stored: &[u8], prim: u32, indexed: bool) -> DrawState {
    const SRC_ALPHA: u32 = 0x302;
    const ONE_MINUS_SRC_ALPHA: u32 = 0x303;
    const ONE: u32 = 1;
    const ADD: u32 = 0x8006;
    const REVERSE_SUBTRACT: u32 = 0x800B;
    let (dst, eq) = match stored[0x89e] {
        1 => (ONE, ADD),
        2 => (ONE, REVERSE_SUBTRACT),
        _ => (ONE_MINUS_SRC_ALPHA, ADD),
    };
    DrawState {
        prim,
        indexed,
        // The alpha channel's own blend (ZERO * src + DST_ALPHA * dst) keeps the target's alpha.
        blend: Some([SRC_ALPHA, dst, eq, 0, 0x304, ADD]),
        depth_test: (stored[0x89b] == 0).then_some(0x203),
        depth_write: false,
    }
}


/// The particles' vertex inputs, routed by the vertex stage's reflection. `emitter_rows` are the emitter matrix's
/// rows for `sysEmtMat0..2Attr` (particles that keep the emitter's matrix from their birth).
pub fn particle_inputs(reflection: &Reflection, spawns: &[crate::draw::sim::Spawn], emitter: glam::Mat4) -> Vec<VertexInput> {
    let mut out = Vec::new();
    let mut add = |name: &str, data: Vec<[f32; 4]>| {
        if let Some(location) = reflection.input(name) {
            out.push(VertexInput { location, per_instance: true, data });
        }
    };
    add("sysLocalPosAttr", spawns.iter().map(|s| s.position.extend(s.life).to_array()).collect());
    add("sysLocalVecAttr", spawns.iter().map(|s| s.velocity.extend(s.birth).to_array()).collect());
    // How far the particle moved since the last frame: velocity-facing particles (billboard types 5 and 6) stretch
    // along it, and with none of it they collapse to nothing.
    add("sysLocalDiffAttr", spawns.iter().map(|s| s.velocity.extend(0.0).to_array()).collect());
    add("sysScaleAttr", spawns.iter().map(|s| s.scale.extend(1.0).to_array()).collect());
    add("sysRandomAttr", spawns.iter().map(|s| s.random).collect());
    add("sysInitRotateAttr", spawns.iter().map(|s| s.rotation.extend(0.0).to_array()).collect());
    for (r, name) in ["sysEmtMat0Attr", "sysEmtMat1Attr", "sysEmtMat2Attr"].into_iter().enumerate() {
        add(name, vec![emitter.row(r).to_array(); spawns.len()]);
    }
    out
}

/// What drawing an emitter with the game's shaders needs from the file, read once when the file loads.
#[derive(Clone, Debug)]
pub struct GameEmitter {
    /// c9.
    pub block: std::sync::Arc<Vec<u8>>,
    /// The emitter's stored data (render_state and the rest read at draw time).
    pub stored: std::sync::Arc<Vec<u8>>,
    pub shader_index: u32,
    /// The pool texture and sampler of each texture unit 0..2.
    pub samplers: [Option<(usize, crate::draw::gpu::SamplerKey)>; 3],
    /// c13 (`sysCustomShaderReservedUniformBlockParam`): the emitter's CSDP data. Every shader that reads c13 belongs
    /// to an emitter with one (all captured draws); for the rest it is never read.
    pub custom: Option<Vec<u8>>,
}

/// A sampler as the emitter sets it: its wrap modes (mirror, repeat, clamp, mirror once), linear filtering with mips,
/// and its LOD limit.
fn sampler_key(s: &effect_library::TextureSampler) -> crate::draw::gpu::SamplerKey {
    use effect_library::enums::WrapMode;
    // NVN wrap codes as eff_gpu reads them: 1 repeat, 6 mirrored repeat, 7 clamp. Mirror once has no wgpu mode; with
    // the block's uv_scale of 2 it covers 0..2, which mirrored repeat draws the same.
    let wrap = |w: &WrapMode| match w {
        WrapMode::Repeat => 1,
        WrapMode::Mirror | WrapMode::MirrorOnce => 6,
        _ => 7,
    };
    crate::draw::gpu::SamplerKey {
        min: 5,
        mag: 1,
        wrap_s: wrap(&s.wrap_u),
        wrap_t: wrap(&s.wrap_v),
        lod_min_bits: 0f32.to_bits(),
        lod_max_bits: s.max_lod.max(0.0).to_bits(),
    }
}

impl GameEmitter {
    pub fn read(emitter: &effect_library::Emitter, textures: &[effect_library::ptcl_file::TextureDescriptor], version: u16) -> Option<Self> {
        let stored = emitter.binary_data.as_ref().or(emitter.cached_binary.as_ref())?;
        let samplers = [&emitter.data.sampler0, &emitter.data.sampler1, &emitter.data.sampler2].map(|sampler| {
            let s = sampler.as_ref()?;
            let index = textures.iter().position(|t| t.id == s.texture_id)?;
            Some((index, sampler_key(s)))
        });
        Some(Self {
            block: std::sync::Arc::new(crate::draw::block::emitter_block(emitter, version)?),
            stored: std::sync::Arc::new(stored.clone()),
            shader_index: u32::try_from(emitter.data.shader_references.shader_index).ok()?,
            samplers,
            custom: emitter.subsections.iter().find(|s| s.magic == "CSDP").map(|s| s.data.clone()),
        })
    }
}

/// A mesh emitter's own vertices, routed by the vertex stage's reflection: `sysPosAttr` (position, 1),
/// `sysNormalAttr`, `sysTexCoordAttr` (uv0, uv1), `sysVertexColor0Attr` (white where the primitive has none).
pub fn mesh_inputs(reflection: &Reflection, mesh: &crate::eff::mesh_io::MeshData) -> Vec<VertexInput> {
    let n = mesh.positions.len();
    let mut out = Vec::new();
    let mut add = |name: &str, data: Vec<[f32; 4]>| {
        if let Some(location) = reflection.input(name) {
            out.push(VertexInput { location, per_instance: false, data });
        }
    };
    add("sysPosAttr", mesh.positions.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect());
    add("sysNormalAttr", (0..n).map(|i| mesh.normals.get(i).map_or([0.0, 0.0, 1.0, 0.0], |v| [v[0], v[1], v[2], 0.0])).collect());
    add(
        "sysTexCoordAttr",
        (0..n)
            .map(|i| {
                let a = mesh.uv0.get(i).copied().unwrap_or_default();
                let b = mesh.uv1.get(i).copied().unwrap_or(a);
                [a[0], a[1], b[0], b[1]]
            })
            .collect(),
    );
    add("sysVertexColor0Attr", (0..n).map(|i| mesh.colors.get(i).copied().unwrap_or([1.0; 4])).collect());
    out
}

/// One emitter of a live effect, this frame, for the game's shaders.
#[derive(Clone)]
pub struct GameDraw {
    /// The effect file: whose shaders draw it (eff_shader_cache).
    pub file: std::path::PathBuf,
    pub emitter: GameEmitter,
    pub dynamic: Dynamic,
    pub spawns: Vec<crate::draw::sim::Spawn>,
    pub mesh: Option<std::sync::Arc<crate::eff::mesh_io::MeshData>>,
    /// Texture unit 0..2: its key (for the GPU copy) and its pixels as the shader samples them.
    pub textures: [Option<(TextureKey, std::sync::Arc<crate::eff::textures::SampledTexture>)>; 3],
}

/// Everything about a draw but its textures and camera, as the renderer takes it.
pub struct Prepared {
    pub variation: u32,
    /// (stage, c-index, contents) for both stages: c9, c10, c13, c16.
    pub cbufs: Vec<(u32, u32, Vec<u8>)>,
    pub vertices: Vec<VertexInput>,
    pub indices: Option<Vec<u32>>,
    pub state: DrawState,
    pub count: u32,
    pub instances: u32,
}

/// The draw's inputs, routed through its shader's reflection. `None` when it has no particles or its shader is not in
/// the library.
pub fn prepare(draw: &GameDraw, library: &crate::draw::gpu::ShaderLibrary) -> Option<Prepared> {
    if draw.spawns.is_empty() {
        return None;
    }
    let variation = draw.emitter.shader_index;
    let reflection = &library.stage(variation, crate::draw::gpu::VERTEX)?.reflection;
    library.stage(variation, crate::draw::gpu::FRAGMENT)?;
    let mut vertices = crate::draw::inputs::particle_inputs(reflection, &draw.spawns, draw.dynamic.matrix);
    let (prim, count, indices) = match &draw.mesh {
        Some(mesh) => {
            vertices.extend(mesh_inputs(reflection, mesh));
            (4, mesh.indices.len() as u32, Some(mesh.indices.clone()))
        }
        None => (5, 4, None),
    };
    let custom = draw.emitter.custom.clone().unwrap_or_else(|| bytemuck::cast_slice(&RESERVED_PARAM).to_vec());
    let mut cbufs = Vec::new();
    for stage in [crate::draw::gpu::VERTEX, crate::draw::gpu::FRAGMENT] {
        cbufs.push((stage, 9, draw.emitter.block.to_vec()));
        cbufs.push((stage, 10, dynamic_block(&draw.dynamic)));
        cbufs.push((stage, 13, custom.clone()));
        // Float 0 halves every colour on its way into the game's target, and its post-processing doubles
        // them again. There is no such pass here, so the shaders draw at full strength.
        let mut block2 = CUSTOM_BLOCK2;
        block2[0] = 1.0;
        cbufs.push((stage, 16, bytemuck::cast_slice(&block2).to_vec()));
    }
    Some(Prepared {
        variation,
        cbufs,
        vertices,
        state: draw_state(&draw.emitter.stored, prim, indices.is_some()),
        indices,
        count,
        instances: draw.spawns.len() as u32,
    })
}

/// A texture of one effect file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureKey {
    pub file: std::path::PathBuf,
    pub index: usize,
}
