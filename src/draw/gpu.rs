//! Running the game's effect shaders on wgpu.
//!
//! A draw names a shader variation of its file (`shader_references.shader_index`), whose two translated stages
//! ([`crate::draw::shaders::Library`]) bind, as `draw::mx::wgsl` lays them out:
//!
//! * set 0: constant buffer c<n> of the vertex stage at binding n and of the fragment stage at 20 + n, each a
//!   read-only storage buffer; each stage's alpha test / point size block at 19 above its base.
//! * set 1: texture unit u of the fragment stage at binding u and of the vertex stage at 32 + u, with its sampler
//!   64 above.
//! * vertex input n at location n, a `vec4` of floats.
//!
//! The constant buffers hold what the game keeps in them, byte for byte (by c-index: 8 the camera, 9 the emitter
//! block, 10 the per-frame emitter state, 13 and 16 custom data, see `draw::inputs`). c1 is the program's own
//! constant data and c2 the driver's texture table; any other buffer a stage reads and the draw does not give reads
//! zeros.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use wgpu::util::DeviceExt;

use crate::draw::mx::wgsl::{Stage, FRAGMENT_CBUF_BASE, SAMPLER_SPLIT, VERTEX_TEXTURE_BASE};
use crate::draw::shaders::{Library, Program};

const STAGES: [Stage; 2] = [Stage::Vertex, Stage::Fragment];
/// Constant buffers c0..c17 can be bound.
const CBUF_COUNT: u32 = 18;
/// c2's size: 96 words of texture handles, then their sizes. No translated effect shader reads it, but fragment
/// stages declare it.
const TEXTURE_TABLE_BYTES: usize = 896;
/// Contents of a buffer a stage reads and the draw does not give.
const EMPTY_CBUF_BYTES: usize = 256;
/// The alpha test / point size block: test function 8 (always pass), reference 0, point size 1.0. Every captured
/// effect draw had the test off.
const ALPHA_TEST_OFF: [u32; 4] = [8, 0, 0x3f80_0000, 0];
/// OpenGL's value of a vertex attribute with no array.
const MISSING_ATTRIBUTE: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn cbuf_base(stage: Stage) -> u32 {
    match stage {
        Stage::Vertex => 0,
        Stage::Fragment => FRAGMENT_CBUF_BASE,
    }
}

fn alpha_test_binding(stage: Stage) -> u32 {
    cbuf_base(stage) + CBUF_COUNT + 1
}

fn texture_binding(stage: Stage, unit: u32) -> u32 {
    match stage {
        Stage::Vertex => VERTEX_TEXTURE_BASE + unit,
        Stage::Fragment => unit,
    }
}

fn visibility(stage: Stage) -> wgpu::ShaderStages {
    match stage {
        Stage::Vertex => wgpu::ShaderStages::VERTEX,
        Stage::Fragment => wgpu::ShaderStages::FRAGMENT,
    }
}

/// Set bits of a mask below `limit`, ascending.
fn bits(mask: u32, limit: u32) -> impl Iterator<Item = u32> {
    (0..limit).filter(move |n| mask >> n & 1 != 0)
}

/// One vertex input: a `vec4` per vertex, or per particle when `per_instance`.
#[derive(Clone, Debug)]
pub struct VertexInput {
    pub location: u32,
    pub per_instance: bool,
    pub data: Vec<[f32; 4]>,
}

/// A sampler in NVN's terms.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct SamplerKey {
    /// 0 nearest, 1 linear; 2 / 3 the same with nearest mips, 4 / 5 with linear mips.
    pub min: u8,
    /// 0 nearest, otherwise linear.
    pub mag: u8,
    /// 1 repeat, 6 mirrored repeat, anything else clamps to the edge.
    pub wrap_s: u8,
    pub wrap_t: u8,
    pub lod_min_bits: u32,
    pub lod_max_bits: u32,
}

impl Default for SamplerKey {
    fn default() -> Self {
        Self { min: 5, mag: 1, wrap_s: 1, wrap_t: 1, lod_min_bits: 0f32.to_bits(), lod_max_bits: 1000f32.to_bits() }
    }
}

impl SamplerKey {
    fn descriptor(&self) -> wgpu::SamplerDescriptor<'static> {
        let filter = |linear: bool| if linear { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
        let wrap = |mode: u8| match mode {
            1 => wgpu::AddressMode::Repeat,
            6 => wgpu::AddressMode::MirrorRepeat,
            _ => wgpu::AddressMode::ClampToEdge,
        };
        // Without mips the level is pinned to 0.
        let (lod_min, lod_max) = if self.min >= 2 {
            let min = f32::from_bits(self.lod_min_bits).clamp(0.0, 32.0);
            (min, f32::from_bits(self.lod_max_bits).clamp(min, 32.0))
        } else {
            (0.0, 0.0)
        };
        wgpu::SamplerDescriptor {
            label: Some("effect sampler"),
            address_mode_u: wrap(self.wrap_s),
            address_mode_v: wrap(self.wrap_t),
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter(self.mag != 0),
            min_filter: filter(self.min % 2 == 1),
            mipmap_filter: if self.min >= 4 { wgpu::MipmapFilterMode::Linear } else { wgpu::MipmapFilterMode::Nearest },
            lod_min_clamp: lod_min,
            lod_max_clamp: lod_max,
            ..Default::default()
        }
    }
}

pub struct TextureInput<'a> {
    pub stage: Stage,
    pub unit: u32,
    pub view: &'a wgpu::TextureView,
    pub sampler: SamplerKey,
}

/// The fixed-function state of a draw, in NVN / OpenGL enums.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct DrawState {
    /// 0 points, 1 lines, 3 line strip, 4 triangles, 5 triangle strip.
    pub prim: u32,
    pub indexed: bool,
    /// src rgb, dst rgb, equation rgb, src alpha, dst alpha, equation alpha.
    pub blend: Option<[u32; 6]>,
    pub depth_test: Option<u32>,
    pub depth_write: bool,
}

fn blend_factor(gl: u32) -> wgpu::BlendFactor {
    use wgpu::BlendFactor as F;
    match gl {
        0 => F::Zero,
        0x300 => F::Src,
        0x301 => F::OneMinusSrc,
        0x302 => F::SrcAlpha,
        0x303 => F::OneMinusSrcAlpha,
        0x304 => F::DstAlpha,
        0x305 => F::OneMinusDstAlpha,
        0x306 => F::Dst,
        0x307 => F::OneMinusDst,
        0x308 => F::SrcAlphaSaturated,
        0x8001 => F::Constant,
        0x8002 => F::OneMinusConstant,
        _ => F::One,
    }
}

fn blend_component(src: u32, dst: u32, equation: u32) -> wgpu::BlendComponent {
    use wgpu::BlendOperation as O;
    let operation = match equation {
        0x800A => O::Subtract,
        0x800B => O::ReverseSubtract,
        0x8007 => O::Min,
        0x8008 => O::Max,
        _ => O::Add,
    };
    // OpenGL ignores the factors of min and max; wgpu wants them One.
    let (src, dst) = match operation {
        O::Min | O::Max => (1, 1),
        _ => (src, dst),
    };
    wgpu::BlendComponent { src_factor: blend_factor(src), dst_factor: blend_factor(dst), operation }
}

fn depth_compare(gl: Option<u32>) -> wgpu::CompareFunction {
    use wgpu::CompareFunction as C;
    match gl {
        Some(0x200) => C::Never,
        Some(0x201) => C::Less,
        Some(0x202) => C::Equal,
        Some(0x203) => C::LessEqual,
        Some(0x204) => C::Greater,
        Some(0x205) => C::NotEqual,
        Some(0x206) => C::GreaterEqual,
        _ => C::Always,
    }
}

fn topology(prim: u32) -> Result<wgpu::PrimitiveTopology> {
    use wgpu::PrimitiveTopology as T;
    Ok(match prim {
        0 => T::PointList,
        1 => T::LineList,
        3 => T::LineStrip,
        4 => T::TriangleList,
        5 => T::TriangleStrip,
        other => bail!("NVN primitive {other} has no wgpu topology"),
    })
}

/// A constant buffer's contents, grown to a whole number of 16-byte rows (at least one).
fn rows_of(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out.resize(bytes.len().div_ceil(16).max(1) * 16, 0);
    out
}

pub struct DrawInputs<'a> {
    pub variation: u32,
    /// (stage, n, contents of c<n>). c1 and c2 come from the library.
    pub cbufs: Vec<(Stage, u32, &'a [u8])>,
    pub vertices: Vec<VertexInput>,
    pub indices: Option<Vec<u32>>,
    pub state: DrawState,
    pub count: u32,
    pub instances: u32,
    pub textures: Vec<TextureInput<'a>>,
}

/// A draw with every GPU resource it needs, ready to record into a pass.
pub struct PreparedDraw {
    pipeline: wgpu::RenderPipeline,
    groups: [wgpu::BindGroup; 2],
    vertex_buffers: Vec<wgpu::Buffer>,
    index: Option<wgpu::Buffer>,
    count: u32,
    instances: u32,
}

impl PreparedDraw {
    pub fn encode(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        for (set, group) in self.groups.iter().enumerate() {
            pass.set_bind_group(set as u32, group, &[]);
        }
        for (slot, buffer) in self.vertex_buffers.iter().enumerate() {
            pass.set_vertex_buffer(slot as u32, buffer.slice(..));
        }
        match &self.index {
            Some(index) => {
                pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.count, 0, 0..self.instances);
            }
            None => pass.draw(0..self.count, 0..self.instances),
        }
    }
}

/// How each vertex input location the vertex stage reads is fed: per particle or per vertex, and whether it is one
/// constant value (stride 0).
#[derive(Clone, Hash, PartialEq, Eq)]
struct VertexSlot {
    location: u32,
    per_instance: bool,
    constant: bool,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct PipelineKey {
    variation: u32,
    slots: Vec<VertexSlot>,
    state: DrawState,
    color: wgpu::TextureFormat,
    depth: Option<wgpu::TextureFormat>,
    /// Every covered pixel plain white, whatever the game's fragment stage would do: the layer's shape alone.
    flat: bool,
}

/// The fragment stage of a flat draw.
const FLAT_FRAGMENT: &str = "@fragment\nfn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 1.0, 1.0, 1.0); }\n";

struct Layout {
    groups: [wgpu::BindGroupLayout; 2],
    pipeline: wgpu::PipelineLayout,
}

/// Both stages of a variation, or why it cannot draw.
fn programs(library: &Library, variation: u32) -> Result<[&Program; 2]> {
    let get = |stage| library.program(variation, stage).ok_or_else(|| anyhow!("variation {variation} has no {stage:?} stage"));
    Ok([get(Stage::Vertex)?, get(Stage::Fragment)?])
}

pub struct GameEffectRenderer {
    pub library: Arc<Library>,
    modules: HashMap<(u32, Stage), wgpu::ShaderModule>,
    layouts: HashMap<u32, Layout>,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    samplers: HashMap<SamplerKey, wgpu::Sampler>,
    /// Bound to every unit a draw does not fill: white, so an unbound depth texture reads "far".
    white: wgpu::TextureView,
}

impl GameEffectRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, library: Arc<Library>) -> Self {
        let white = upload(device, queue, "unbound texture", 1, 1, 1, wgpu::TextureFormat::Rgba8Unorm, &[255; 4]);
        Self {
            library,
            modules: HashMap::new(),
            layouts: HashMap::new(),
            pipelines: HashMap::new(),
            samplers: HashMap::new(),
            white,
        }
    }

    fn module(&mut self, device: &wgpu::Device, variation: u32, stage: Stage) -> Result<wgpu::ShaderModule> {
        if let Some(module) = self.modules.get(&(variation, stage)) {
            return Ok(module.clone());
        }
        let program = self.library.program(variation, stage).ok_or_else(|| anyhow!("variation {variation}: no {stage:?} stage"))?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&format!("effect shader v{variation} {stage:?}")),
            source: wgpu::ShaderSource::Wgsl(program.wgsl.as_str().into()),
        });
        self.modules.insert((variation, stage), module.clone());
        Ok(module)
    }

    fn layout(&mut self, device: &wgpu::Device, variation: u32) -> Result<&Layout> {
        if !self.layouts.contains_key(&variation) {
            let library = self.library.clone();
            let programs = programs(&library, variation)?;
            let mut buffers = Vec::new();
            let mut textures = Vec::new();
            for (stage, program) in STAGES.into_iter().zip(programs) {
                let visibility = visibility(stage);
                let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
                let storage = wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                };
                buffers.extend(bits(program.cbufs, CBUF_COUNT).map(|n| entry(cbuf_base(stage) + n, storage)));
                buffers.push(entry(alpha_test_binding(stage), storage));
                for &unit in &program.units {
                    let binding = texture_binding(stage, unit);
                    textures.push(entry(
                        binding,
                        wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                    ));
                    textures.push(entry(binding + SAMPLER_SPLIT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)));
                }
            }
            let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
            };
            let groups = [group("effect buffers", &buffers), group("effect textures", &textures)];
            let pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("effect pipeline layout"),
                bind_group_layouts: &[Some(&groups[0]), Some(&groups[1])],
                immediate_size: 0,
            });
            self.layouts.insert(variation, Layout { groups, pipeline });
        }
        Ok(&self.layouts[&variation])
    }

    fn pipeline(&mut self, device: &wgpu::Device, key: PipelineKey) -> Result<wgpu::RenderPipeline> {
        if let Some(pipeline) = self.pipelines.get(&key) {
            return Ok(pipeline.clone());
        }
        let variation = key.variation;
        let vs = self.module(device, variation, Stage::Vertex)?;
        let fs = if key.flat {
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("flat"), source: wgpu::ShaderSource::Wgsl(FLAT_FRAGMENT.into()) })
        } else {
            self.module(device, variation, Stage::Fragment)?
        };
        let attributes: Vec<[wgpu::VertexAttribute; 1]> = key
            .slots
            .iter()
            .map(|s| [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: s.location }])
            .collect();
        let buffers: Vec<wgpu::VertexBufferLayout> = key
            .slots
            .iter()
            .zip(&attributes)
            .map(|(s, attributes)| wgpu::VertexBufferLayout {
                array_stride: if s.constant { 0 } else { 16 },
                step_mode: if s.per_instance { wgpu::VertexStepMode::Instance } else { wgpu::VertexStepMode::Vertex },
                attributes,
            })
            .collect();
        let state = key.state;
        let topology = topology(state.prim)?;
        let layout = &self.layout(device, variation)?.pipeline;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("effect v{variation}")),
            layout: Some(layout),
            vertex: wgpu::VertexState { module: &vs, entry_point: Some("main"), buffers: &buffers, compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &fs,
                entry_point: Some("main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: key.color,
                    blend: state.blend.filter(|_| !key.flat).map(|b| wgpu::BlendState {
                        color: blend_component(b[0], b[1], b[2]),
                        alpha: blend_component(b[3], b[4], b[5]),
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology,
                strip_index_format: (state.indexed && topology.is_strip()).then_some(wgpu::IndexFormat::Uint32),
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: key.depth.map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(state.depth_write && state.depth_test.is_some()),
                depth_compare: Some(depth_compare(state.depth_test)),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, pipeline.clone());
        Ok(pipeline)
    }

    fn sampler(&mut self, device: &wgpu::Device, key: SamplerKey) -> wgpu::Sampler {
        self.samplers.entry(key).or_insert_with(|| device.create_sampler(&key.descriptor())).clone()
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        inputs: &DrawInputs<'_>,
        color: wgpu::TextureFormat,
        depth: Option<wgpu::TextureFormat>,
    ) -> Result<PreparedDraw> {
        self.prepare_as(device, inputs, color, depth, false)
    }

    /// The draw's shape alone, every pixel it covers white (see [`PipelineKey::flat`]).
    pub fn prepare_flat(&mut self, device: &wgpu::Device, inputs: &DrawInputs<'_>, color: wgpu::TextureFormat) -> Result<PreparedDraw> {
        self.prepare_as(device, inputs, color, None, true)
    }

    fn prepare_as(
        &mut self,
        device: &wgpu::Device,
        inputs: &DrawInputs<'_>,
        color: wgpu::TextureFormat,
        depth: Option<wgpu::TextureFormat>,
        flat: bool,
    ) -> Result<PreparedDraw> {
        let variation = inputs.variation;
        let library = self.library.clone();
        let programs = programs(&library, variation)?;

        // One vertex buffer per location the vertex stage reads; a location the draw leaves out reads
        // MISSING_ATTRIBUTE from a single element.
        let mut slots = Vec::new();
        let mut vertex_buffers = Vec::new();
        for location in bits(programs[0].inputs, 32) {
            let given = inputs.vertices.iter().find(|v| v.location == location).filter(|v| !v.data.is_empty());
            let (data, per_instance) = match given {
                Some(v) => (v.data.as_slice(), v.per_instance),
                None => (std::slice::from_ref(&MISSING_ATTRIBUTE), true),
            };
            slots.push(VertexSlot { location, per_instance, constant: given.is_none() });
            vertex_buffers.push(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("effect vertex input"),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::VERTEX,
            }));
        }
        let pipeline = self.pipeline(device, PipelineKey { variation, slots, state: inputs.state, color, depth, flat })?;

        // Set 0: each stage's constant buffers, then its alpha test block.
        let mut buffers = Vec::new();
        for (stage, program) in STAGES.into_iter().zip(programs) {
            for n in bits(program.cbufs, CBUF_COUNT) {
                let contents = match n {
                    1 => rows_of(&program.constants),
                    2 => vec![0; TEXTURE_TABLE_BYTES],
                    _ => inputs
                        .cbufs
                        .iter()
                        .find(|&&(s, i, _)| s == stage && i == n)
                        .map_or_else(|| vec![0; EMPTY_CBUF_BYTES], |&(_, _, bytes)| rows_of(bytes)),
                };
                buffers.push((cbuf_base(stage) + n, storage(device, "effect constant buffer", &contents)));
            }
            buffers.push((alpha_test_binding(stage), storage(device, "effect alpha test", bytemuck::cast_slice(&ALPHA_TEST_OFF))));
        }
        // Set 1: each unit a stage samples, from the draw or white.
        let mut textures = Vec::new();
        for (stage, program) in STAGES.into_iter().zip(programs) {
            for &unit in &program.units {
                let given = inputs.textures.iter().find(|t| t.stage == stage && t.unit == unit);
                let view = given.map_or_else(|| self.white.clone(), |t| t.view.clone());
                let sampler = self.sampler(device, given.map(|t| t.sampler).unwrap_or_default());
                textures.push((texture_binding(stage, unit), view, sampler));
            }
        }

        let layout = self.layout(device, variation)?;
        let buffer_entries: Vec<wgpu::BindGroupEntry> =
            buffers.iter().map(|(binding, b)| wgpu::BindGroupEntry { binding: *binding, resource: b.as_entire_binding() }).collect();
        let texture_entries: Vec<wgpu::BindGroupEntry> = textures
            .iter()
            .flat_map(|(binding, view, sampler)| {
                [
                    wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: binding + SAMPLER_SPLIT, resource: wgpu::BindingResource::Sampler(sampler) },
                ]
            })
            .collect();
        let group = |label, layout, entries: &[wgpu::BindGroupEntry]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries })
        };
        let groups = [
            group("effect buffers", &layout.groups[0], &buffer_entries),
            group("effect textures", &layout.groups[1], &texture_entries),
        ];
        let index = inputs.indices.as_ref().map(|indices| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("effect indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            })
        });
        Ok(PreparedDraw { pipeline, groups, vertex_buffers, index, count: inputs.count, instances: inputs.instances.max(1) })
    }
}

fn storage(device: &wgpu::Device, label: &str, contents: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE })
}

#[allow(clippy::too_many_arguments)]
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    mips: u32,
    format: wgpu::TextureFormat,
    data: &[u8],
) -> wgpu::TextureView {
    let descriptor = wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    device.create_texture_with_data(queue, &descriptor, wgpu::util::TextureDataOrder::LayerMajor, data).create_view(&Default::default())
}

/// A texture as the game's shaders sample it (`textures::decode_for_sampling`), on the GPU with every mip.
pub fn upload_sampled(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &crate::eff::textures::SampledTexture,
    label: &str,
) -> wgpu::TextureView {
    let (width, height) = texture.levels[0].dimensions();
    let data: Vec<u8> = texture.levels.iter().flat_map(|level| level.as_raw().iter().copied()).collect();
    let format = if texture.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
    upload(device, queue, label, width, height, texture.levels.len() as u32, format, &data)
}

/// The camera constant buffer, c8 (320 bytes), for a right-handed view looking down -z and a projection to 0..1
/// depth (the game's is `Mat4::perspective_infinite_rh` with near 2).
///
/// As the game writes it, every matrix row by row: +0 view, +64 projection, +128 projection * view, +192 the view's
/// rotation inverted (camera to world, no translation), +256 the view's third row (the camera's back axis in world
/// space), +272 the camera's position, +288 near, far, near * far, far, +304 `tail` (a capture holds
/// (1, 1, 0.01, 0); its meaning is not known).
pub fn camera_block(view: glam::Mat4, proj: glam::Mat4, near: f32, far: f32, tail: [f32; 4]) -> Vec<u8> {
    let rotation = glam::Mat4::from_mat3(glam::Mat3::from_mat4(view).transpose());
    let mut floats: Vec<f32> = [view, proj, proj * view, rotation].iter().flat_map(|m| m.transpose().to_cols_array()).collect();
    floats.extend(view.row(2).truncate().extend(0.0).to_array());
    floats.extend(view.inverse().w_axis.truncate().extend(0.0).to_array());
    floats.extend([near, far, near * far, far]);
    floats.extend(tail);
    bytemuck::cast_slice(&floats).to_vec()
}
