//! Drawing effects with the game's own shaders.
//!
//! Every emitter names one shader variation of its file (`shader_references.shader_index`): a Maxwell vertex and
//! fragment program in the file's BNSH, which this runs. The programs
//! are translated offline (tools/eft_shaders.py: the recomp's Maxwell translator, then glslang) into SPIR-V that wgpu
//! reads through naga, with this binding layout:
//!
//! * set 0: the vertex stage's constant buffer `c<n>` at binding n, the fragment stage's at 20 + n, each a read-only
//!   storage buffer of `uint`; the alpha test / point size block at 19 (vertex) and 39 (fragment).
//! * set 1: fragment texture unit u at binding u, vertex at 32 + u; each texture's sampler 64 above it.
//! * vertex input: generic attribute n at location n, always read as a `vec4`.
//!
//! The constant buffers are the game's own memory, byte for byte. What the shaders read, by NVN slot (`c<slot + 3>`):
//! 0 (c3), 1 (c4), 5 (c8) the camera, 6 (c9) the emitter block (header 0x50 + the emitter's static data),
//! 7 (c10), 8 (c11), 10 (c13), 11 (c14), 12 (c15), 13 (c16). c1 is the program's own constant data, c2 the driver's
//! texture table. eff_game builds each of them from the .eff,
//! checked against recomp captures of the game's own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use wgpu::util::DeviceExt;

pub const VERTEX: u32 = 0;
pub const FRAGMENT: u32 = 1;

const FRAGMENT_CBUF_BASE: u32 = 20;
const MXPC_BINDING: [u32; 2] = [19, 39];
const SAMPLER_SPLIT: u32 = 64;
/// c2, the driver's texture table (handles, then texture sizes from word 96): no translated effect shader reads it,
/// but the fragment stages declare it.
const TEXTURE_TABLE_BYTES: usize = 896;

/// A texture unit one stage samples, and the binding of its texture (the sampler is `SAMPLER_SPLIT` above).
#[derive(Clone, Copy, Debug)]
pub struct SamplerSlot {
    pub unit: u32,
    pub binding: u32,
}

/// What one stage of a variation reads.
#[derive(Clone, Debug)]
pub struct StageInfo {
    /// Bit n: the stage reads constant buffer c<n>.
    pub cbufs: u32,
    /// Bit n: the stage reads generic input n (vertex) or interpolant n (fragment).
    pub inputs: u32,
    pub samplers: Vec<SamplerSlot>,
    /// c1: the program's constant data, from its code section.
    pub constants: Vec<u8>,
    /// The program's reflection (the BNSH's names): vertex inputs by name -> location (`sysLocalPosAttr` ...),
    /// samplers by name -> texture unit (`sysTextureSampler0`, `sysDepthBufferTexture` ...), and constant buffers by
    /// name -> NVN slot (`sysEmitterDynamicUniformBlock` ...; the shader reads slot n as c(n + 3)).
    pub reflection: Reflection,
    code_hash: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Reflection {
    pub inputs: Vec<(String, i32)>,
    pub samplers: Vec<(String, i32)>,
    pub constant_buffers: Vec<(String, i32)>,
}

impl Reflection {
    fn read(path: &Path) -> Self {
        let Ok(text) = std::fs::read(path) else { return Self::default() };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&text) else { return Self::default() };
        let list = |key: &str| -> Vec<(String, i32)> {
            json[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_i64()? as i32)))
                        .collect()
                })
                .unwrap_or_default()
        };
        Self { inputs: list("inputs"), samplers: list("samplers"), constant_buffers: list("constant_buffers") }
    }

    pub fn input(&self, name: &str) -> Option<u32> {
        self.inputs.iter().find(|(n, _)| n == name).and_then(|&(_, l)| u32::try_from(l).ok())
    }


}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The translated shaders of one effect file: the shader dump (`v<i>_s<stage>.code` / `.ctl`, written by
/// eff_shader_cache) with eft_shaders.py's `spv/` beside it.
pub struct ShaderLibrary {
    dir: PathBuf,
    stages: HashMap<(u32, u32), StageInfo>,
    by_code: HashMap<(u32, u64), Vec<u32>>,
}

impl ShaderLibrary {
    pub fn open(dir: &Path) -> Result<Self> {
        let index_path = dir.join("spv").join("index.json");
        let index: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&index_path).with_context(|| format!("reading {}", index_path.display()))?,
        )?;
        let variations = index["variations"].as_object().ok_or_else(|| anyhow!("{}: no variations", index_path.display()))?;
        let mut stages = HashMap::new();
        let mut by_code: HashMap<(u32, u64), Vec<u32>> = HashMap::new();
        for (key, v) in variations {
            let variation: u32 = key.parse()?;
            for (stage, name, file_stage) in [(VERTEX, "vs", 0), (FRAGMENT, "fs", 4)] {
                let s = &v[name];
                if s["ok"].as_bool() != Some(true) {
                    continue;
                }
                let base = dir.join(format!("v{variation:04}_s{file_stage}"));
                // A stage whose dump is missing is left out (the variation is then not drawable), as is one the
                // translation failed on.
                let (Ok(code), Ok(ctl)) = (std::fs::read(base.with_extension("code")), std::fs::read(base.with_extension("ctl"))) else {
                    continue;
                };
                let (len, c_size, c_off) = (
                    le_u32(&ctl, 0x6f8).unwrap_or(0) as usize,
                    le_u32(&ctl, 0x6fc).unwrap_or(0) as usize,
                    le_u32(&ctl, 0x700).unwrap_or(0) as usize,
                );
                // The program as the game binds it: `len` bytes from the program header (+0x30 of the section) on,
                // header included. A captured program is matched by exactly these bytes (`Program::bytes`).
                let program = code.get(0x30..0x30 + len).ok_or_else(|| anyhow!("{}: short code", base.display()))?;
                let info = StageInfo {
                    cbufs: s["cbufs"].as_u64().unwrap_or(0) as u32,
                    inputs: s["in"].as_u64().unwrap_or(0) as u32,
                    samplers: s["samplers"]
                        .as_array()
                        .map(|list| {
                            list.iter()
                                .map(|x| SamplerSlot {
                                    unit: x["unit"].as_u64().unwrap_or(0) as u32,
                                    binding: x["binding"].as_u64().unwrap_or(0) as u32,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    constants: code.get(c_off..c_off + c_size).map(|c| c.to_vec()).unwrap_or_default(),
                    reflection: Reflection::read(&dir.join(format!("v{variation:04}_s{file_stage}.reflect.json"))),
                    code_hash: fnv(program),
                };
                by_code.entry((stage, info.code_hash)).or_default().push(variation);
                stages.insert((variation, stage), info);
            }
        }
        Ok(Self { dir: dir.to_path_buf(), stages, by_code })
    }

    pub fn stage(&self, variation: u32, stage: u32) -> Option<&StageInfo> {
        self.stages.get(&(variation, stage))
    }



    fn spirv(&self, variation: u32, stage: u32) -> Result<Vec<u32>> {
        let path = self.dir.join("spv").join(format!("v{variation:04}_s{}.spv", if stage == VERTEX { 0 } else { 4 }));
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.len() % 4 != 0 {
            bail!("{} is not SPIR-V", path.display());
        }
        Ok(bytes.chunks_exact(4).map(|w| u32::from_le_bytes(w.try_into().unwrap())).collect())
    }
}

/// One vertex input, already widened to `vec4`s (see [`decode_attribute`]).
#[derive(Clone, Debug)]
pub struct VertexInput {
    pub location: u32,
    pub per_instance: bool,
    pub data: Vec<[f32; 4]>,
}

/// A sampler in NVN's terms.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct SamplerKey {
    /// 0 nearest, 1 linear, then nearest / linear with nearest mips (2, 3) or linear mips (4, 5).
    pub min: u8,
    pub mag: u8,
    /// 1 repeat, 6 mirrored repeat, anything else clamps.
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

pub struct TextureInput<'a> {
    pub stage: u32,
    pub unit: u32,
    pub view: &'a wgpu::TextureView,
    pub sampler: SamplerKey,
}

/// The fixed-function state of a draw, in the OpenGL enums the capture records.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct DrawState {
    /// NVN primitive: 0 points, 1 lines, 3 line strip, 4 triangles, 5 triangle strip.
    pub prim: u32,
    pub indexed: bool,
    /// src rgb, dst rgb, equation rgb, src alpha, dst alpha, equation alpha.
    pub blend: Option<[u32; 6]>,
    pub depth_test: Option<u32>,
    pub depth_write: bool,
}

pub struct DrawInputs<'a> {
    pub variation: u32,
    /// (stage, n, contents of c<n>). c1 and c2 come from the library; anything else not listed reads zeros.
    pub cbufs: Vec<(u32, u32, &'a [u8])>,
    pub vertices: Vec<VertexInput>,
    pub indices: Option<Vec<u32>>,
    pub state: DrawState,
    pub alpha_func: i32,
    pub alpha_ref: f32,
    pub count: u32,
    pub instances: u32,
    pub first: u32,
    pub base_vertex: i32,
    pub base_instance: u32,
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
    first: u32,
    base_vertex: i32,
    base_instance: u32,
}

impl PreparedDraw {
    pub fn encode(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.groups[0], &[]);
        pass.set_bind_group(1, &self.groups[1], &[]);
        for (slot, buffer) in self.vertex_buffers.iter().enumerate() {
            pass.set_vertex_buffer(slot as u32, buffer.slice(..));
        }
        let instances = self.base_instance..self.base_instance + self.instances;
        match &self.index {
            Some(index) => {
                pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(self.first..self.first + self.count, self.base_vertex, instances);
            }
            None => pass.draw(self.first..self.first + self.count, instances),
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct PipelineKey {
    variation: u32,
    /// Per vertex input location, in buffer order: (location, per instance).
    vertex_layout: Vec<(u32, bool)>,
    state: DrawState,
    color: wgpu::TextureFormat,
    depth: Option<wgpu::TextureFormat>,
}

struct Layouts {
    groups: [wgpu::BindGroupLayout; 2],
    pipeline: wgpu::PipelineLayout,
}

pub struct GameEffectRenderer {
    pub library: ShaderLibrary,
    modules: HashMap<(u32, u32), wgpu::ShaderModule>,
    layouts: HashMap<u32, Layouts>,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    samplers: HashMap<SamplerKey, wgpu::Sampler>,
    /// Bound to every unit a draw does not supply: white, so an unbound depth texture reads "far".
    fallback: wgpu::TextureView,
}

fn blend_factor(gl: u32) -> wgpu::BlendFactor {
    use wgpu::BlendFactor as F;
    match gl {
        0 => F::Zero,
        1 => F::One,
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

fn blend_operation(gl: u32) -> wgpu::BlendOperation {
    use wgpu::BlendOperation as O;
    match gl {
        0x800A => O::Subtract,
        0x800B => O::ReverseSubtract,
        0x8007 => O::Min,
        0x8008 => O::Max,
        _ => O::Add,
    }
}

fn blend_component(src: u32, dst: u32, eq: u32) -> wgpu::BlendComponent {
    let operation = blend_operation(eq);
    // Min and max ignore the factors in OpenGL; wgpu requires them to be One.
    if matches!(operation, wgpu::BlendOperation::Min | wgpu::BlendOperation::Max) {
        return wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation };
    }
    wgpu::BlendComponent { src_factor: blend_factor(src), dst_factor: blend_factor(dst), operation }
}

fn compare(gl: u32) -> wgpu::CompareFunction {
    use wgpu::CompareFunction as C;
    match gl {
        0x200 => C::Never,
        0x201 => C::Less,
        0x202 => C::Equal,
        0x203 => C::LessEqual,
        0x204 => C::Greater,
        0x205 => C::NotEqual,
        0x206 => C::GreaterEqual,
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

fn padded(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out.resize(bytes.len().div_ceil(16).max(1) * 16, 0);
    out
}

impl GameEffectRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, library: ShaderLibrary) -> Self {
        let fallback = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("game effect fallback texture"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255, 255, 255, 255],
            )
            .create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            library,
            modules: HashMap::new(),
            layouts: HashMap::new(),
            pipelines: HashMap::new(),
            samplers: HashMap::new(),
            fallback,
        }
    }

    fn module(&mut self, device: &wgpu::Device, variation: u32, stage: u32) -> Result<wgpu::ShaderModule> {
        if let Some(m) = self.modules.get(&(variation, stage)) {
            return Ok(m.clone());
        }
        let words = self.library.spirv(variation, stage)?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&format!("effect shader v{variation} s{stage}")),
            source: wgpu::ShaderSource::SpirV(words.into()),
        });
        self.modules.insert((variation, stage), module.clone());
        Ok(module)
    }

    fn layouts(&mut self, device: &wgpu::Device, variation: u32) -> Result<&Layouts> {
        if !self.layouts.contains_key(&variation) {
            let mut buffers = Vec::new();
            let mut textures = Vec::new();
            for stage in [VERTEX, FRAGMENT] {
                let info = self.library.stage(variation, stage).ok_or_else(|| anyhow!("variation {variation} has no stage {stage}"))?;
                let visibility = if stage == VERTEX { wgpu::ShaderStages::VERTEX } else { wgpu::ShaderStages::FRAGMENT };
                let base = if stage == VERTEX { 0 } else { FRAGMENT_CBUF_BASE };
                let storage = wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                };
                for n in (0..18).filter(|n| info.cbufs >> n & 1 != 0) {
                    buffers.push(wgpu::BindGroupLayoutEntry { binding: base + n, visibility, ty: storage, count: None });
                }
                buffers.push(wgpu::BindGroupLayoutEntry { binding: MXPC_BINDING[stage as usize], visibility, ty: storage, count: None });
                for slot in &info.samplers {
                    textures.push(wgpu::BindGroupLayoutEntry {
                        binding: slot.binding,
                        visibility,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    });
                    textures.push(wgpu::BindGroupLayoutEntry {
                        binding: slot.binding + SAMPLER_SPLIT,
                        visibility,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    });
                }
            }
            let group0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("game effect buffers"), entries: &buffers });
            let group1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("game effect textures"), entries: &textures });
            let pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("game effect pipeline layout"),
                bind_group_layouts: &[Some(&group0), Some(&group1)],
                immediate_size: 0,
            });
            self.layouts.insert(variation, Layouts { groups: [group0, group1], pipeline });
        }
        Ok(&self.layouts[&variation])
    }

    fn sampler(&mut self, device: &wgpu::Device, key: SamplerKey) -> wgpu::Sampler {
        self.samplers
            .entry(key)
            .or_insert_with(|| {
                let filter = |linear: bool| if linear { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
                let wrap = |w: u8| match w {
                    1 => wgpu::AddressMode::Repeat,
                    6 => wgpu::AddressMode::MirrorRepeat,
                    _ => wgpu::AddressMode::ClampToEdge,
                };
                let mipmapped = key.min >= 2;
                let lod_min = f32::from_bits(key.lod_min_bits).clamp(0.0, 32.0);
                let lod_max = f32::from_bits(key.lod_max_bits).clamp(lod_min, 32.0);
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("game effect sampler"),
                    address_mode_u: wrap(key.wrap_s),
                    address_mode_v: wrap(key.wrap_t),
                    address_mode_w: wgpu::AddressMode::ClampToEdge,
                    mag_filter: filter(key.mag != 0),
                    min_filter: filter(key.min % 2 == 1),
                    mipmap_filter: if key.min >= 4 { wgpu::MipmapFilterMode::Linear } else { wgpu::MipmapFilterMode::Nearest },
                    lod_min_clamp: if mipmapped { lod_min } else { 0.0 },
                    lod_max_clamp: if mipmapped { lod_max } else { 0.0 },
                    ..Default::default()
                })
            })
            .clone()
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        inputs: &DrawInputs<'_>,
        color: wgpu::TextureFormat,
        depth: Option<wgpu::TextureFormat>,
    ) -> Result<PreparedDraw> {
        let variation = inputs.variation;
        let vs_inputs = self.library.stage(variation, VERTEX).ok_or_else(|| anyhow!("variation {variation}: no vertex stage"))?.inputs;

        // Vertex buffers: one per input location the vertex stage reads; a location the draw does not supply reads
        // (0, 0, 0, 1), OpenGL's value for a disabled attribute, from a single instance-rate element.
        let mut vertex_layout = Vec::new();
        let mut vertex_buffers = Vec::new();
        for location in (0..32).filter(|l| vs_inputs >> l & 1 != 0) {
            let supplied = inputs.vertices.iter().find(|v| v.location == location);
            let (data, per_instance): (Vec<[f32; 4]>, bool) = match supplied {
                Some(v) if !v.data.is_empty() => (v.data.clone(), v.per_instance),
                _ => (vec![[0.0, 0.0, 0.0, 1.0]], true),
            };
            let stride_zero = supplied.is_none_or(|v| v.data.is_empty());
            vertex_layout.push((location, per_instance));
            vertex_buffers.push((device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("game effect vertex input"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::VERTEX,
            }), stride_zero));
        }
        let key = PipelineKey { variation, vertex_layout: vertex_layout.clone(), state: inputs.state, color, depth };
        let pipeline = match self.pipelines.get(&key) {
            Some(p) => p.clone(),
            None => {
                let vs = self.module(device, variation, VERTEX)?;
                let fs = self.module(device, variation, FRAGMENT)?;
                let strides: Vec<bool> = vertex_buffers.iter().map(|(_, zero)| *zero).collect();
                let attributes: Vec<[wgpu::VertexAttribute; 1]> = vertex_layout
                    .iter()
                    .map(|&(location, _)| [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: location }])
                    .collect();
                let buffers: Vec<wgpu::VertexBufferLayout> = vertex_layout
                    .iter()
                    .zip(&attributes)
                    .zip(&strides)
                    .map(|((&(_, per_instance), attrs), &zero)| wgpu::VertexBufferLayout {
                        array_stride: if zero { 0 } else { 16 },
                        step_mode: if per_instance { wgpu::VertexStepMode::Instance } else { wgpu::VertexStepMode::Vertex },
                        attributes: attrs,
                    })
                    .collect();
                let state = inputs.state;
                let topology = topology(state.prim)?;
                let blend = state.blend.map(|b| wgpu::BlendState {
                    color: blend_component(b[0], b[1], b[2]),
                    alpha: blend_component(b[3], b[4], b[5]),
                });
                let layout = &self.layouts(device, variation)?.pipeline;
                let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&format!("game effect v{variation}")),
                    layout: Some(layout),
                    vertex: wgpu::VertexState { module: &vs, entry_point: Some("main"), buffers: &buffers, compilation_options: Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: &fs,
                        entry_point: Some("main"),
                        targets: &[Some(wgpu::ColorTargetState { format: color, blend, write_mask: wgpu::ColorWrites::ALL })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology,
                        strip_index_format: (state.indexed && topology.is_strip()).then_some(wgpu::IndexFormat::Uint32),
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: depth.map(|format| wgpu::DepthStencilState {
                        format,
                        depth_write_enabled: Some(state.depth_write && state.depth_test.is_some()),
                        depth_compare: Some(state.depth_test.map(compare).unwrap_or(wgpu::CompareFunction::Always)),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
                self.pipelines.insert(key, p.clone());
                p
            }
        };

        // Set 0: constant buffers and the alpha test.
        let mut buffers: Vec<(u32, wgpu::Buffer)> = Vec::new();
        for stage in [VERTEX, FRAGMENT] {
            let info = self.library.stage(variation, stage).unwrap();
            let base = if stage == VERTEX { 0 } else { FRAGMENT_CBUF_BASE };
            for n in (0..18).filter(|n| info.cbufs >> n & 1 != 0) {
                let contents = match n {
                    1 => padded(&info.constants),
                    2 => vec![0u8; TEXTURE_TABLE_BYTES],
                    _ => inputs
                        .cbufs
                        .iter()
                        .find(|(s, i, _)| *s == stage && *i == n)
                        .map(|(_, _, b)| padded(b))
                        .unwrap_or_else(|| vec![0u8; 256]),
                };
                buffers.push((base + n, device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("game effect constant buffer"),
                    contents: &contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })));
            }
            let mxpc: [u32; 4] = [inputs.alpha_func as u32, inputs.alpha_ref.to_bits(), 1f32.to_bits(), 0];
            buffers.push((MXPC_BINDING[stage as usize], device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("game effect alpha test"),
                contents: bytemuck::cast_slice(&mxpc),
                usage: wgpu::BufferUsages::STORAGE,
            })));
        }
        // Set 1: textures and samplers.
        let mut views: Vec<(u32, wgpu::TextureView, wgpu::Sampler)> = Vec::new();
        for stage in [VERTEX, FRAGMENT] {
            let slots = self.library.stage(variation, stage).unwrap().samplers.clone();
            for slot in slots {
                let given = inputs.textures.iter().find(|t| t.stage == stage && t.unit == slot.unit);
                let view = given.map(|t| t.view.clone()).unwrap_or_else(|| self.fallback.clone());
                let sampler = self.sampler(device, given.map(|t| t.sampler).unwrap_or_default());
                views.push((slot.binding, view, sampler));
            }
        }
        let layouts = self.layouts(device, variation)?;
        let group0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("game effect buffers"),
            layout: &layouts.groups[0],
            entries: &buffers
                .iter()
                .map(|(binding, b)| wgpu::BindGroupEntry { binding: *binding, resource: b.as_entire_binding() })
                .collect::<Vec<_>>(),
        });
        let mut entries = Vec::new();
        for (binding, view, sampler) in &views {
            entries.push(wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::TextureView(view) });
            entries.push(wgpu::BindGroupEntry { binding: binding + SAMPLER_SPLIT, resource: wgpu::BindingResource::Sampler(sampler) });
        }
        let group1 = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("game effect textures"), layout: &layouts.groups[1], entries: &entries });
        let index = inputs.indices.as_ref().map(|indices| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("game effect indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            })
        });
        Ok(PreparedDraw {
            pipeline,
            groups: [group0, group1],
            vertex_buffers: vertex_buffers.into_iter().map(|(b, _)| b).collect(),
            index,
            count: inputs.count,
            instances: inputs.instances.max(1),
            first: inputs.first,
            base_vertex: inputs.base_vertex,
            base_instance: inputs.base_instance,
        })
    }
}

/// The camera constant buffer: the shaders' c8 (NVN slot 5, 320 bytes), for a right-handed view (looking down -z)
/// and a projection to 0..1 depth (the game's is `Mat4::perspective_infinite_rh` with near 2).
///
/// Laid out as the game writes it, every matrix row-major: +0 view, +64 projection, +128 projection * view, +192 the
/// view's rotation inverted (camera to world, no translation), +256 view row 2 (the camera's back axis in world space),
/// +272 the camera position, +288 near, far, near * far, far. +304 is copied from `tail` (a capture has
/// (1, 1, 0.01, 0); not yet known).
pub fn camera_block(view: glam::Mat4, proj: glam::Mat4, near: f32, far: f32, tail: [f32; 4]) -> Vec<u8> {
    let mut f = Vec::with_capacity(80);
    let rows = |m: glam::Mat4, f: &mut Vec<f32>| {
        for r in 0..4 {
            f.extend_from_slice(&m.row(r).to_array());
        }
    };
    rows(view, &mut f);
    rows(proj, &mut f);
    rows(proj * view, &mut f);
    let rotation = glam::Mat4::from_mat3(glam::Mat3::from_mat4(view).transpose());
    rows(rotation, &mut f);
    f.extend_from_slice(&view.row(2).truncate().extend(0.0).to_array());
    f.extend_from_slice(&view.inverse().w_axis.truncate().extend(0.0).to_array());
    f.extend_from_slice(&[near, far, near * far, far]);
    f.extend_from_slice(&tail);
    bytemuck::cast_slice(&f).to_vec()
}

/// A texture as the game's shaders sample it (texture_import::decode_for_sampling), on the GPU with every mip.
pub fn upload_sampled(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &crate::texture_import::SampledTexture,
    label: &str,
) -> wgpu::TextureView {
    let (w, h) = texture.levels[0].dimensions();
    let data: Vec<u8> = texture.levels.iter().flat_map(|l| l.as_raw().iter().copied()).collect();
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: texture.levels.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: if texture.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm },
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data,
        )
        .create_view(&Default::default())
}





impl DrawState {
}

impl SamplerKey {
}
