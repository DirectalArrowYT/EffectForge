//! The effect preview: every layer of one effect, drawn by the game's own shaders into a texture
//! the window shows. The effect sits at the origin with no fighter or bone, as a world spawn does.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use eframe::egui;
use eframe::egui_wgpu::RenderState;

use crate::draw::inputs::{Dynamic, GameDraw};
use crate::draw::gpu::{DrawInputs, GameEffectRenderer, PreparedDraw, ShaderLibrary, TextureInput, FRAGMENT, VERTEX};
use crate::eff::mesh_io::MeshData;
use crate::draw::inputs::TextureKey;
use crate::draw::sim::{spawns_moving, EmitterSim, Slots};
use crate::engine::Doc;
use crate::eff::textures::{decode_for_sampling, SampledTexture};

/// Half the height of the view at the origin, in Smash units.
pub const HALF_VIEW: f32 = 30.0;
/// The swing the preview's emitter follows when it moves: a circle in the camera's plane, about a hand's reach
/// across, once round in this many frames.
const SWING_RADIUS: f32 = 8.0;
const SWING_FRAMES: f32 = 40.0;

/// Where the swinging emitter is at frame `t`.
pub fn swing_at(t: f32) -> glam::Vec3 {
    let turn = t / SWING_FRAMES * std::f32::consts::TAU;
    glam::Vec3::new(turn.cos() - 1.0, turn.sin(), 0.0) * SWING_RADIUS
}
const FOV_Y: f32 = 0.5;
/// The shaders work in linear light: the view they draw through encodes it for the screen, and the
/// window samples the same pixels as plain bytes.
const DRAW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SHOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Target {
    size: [u32; 2],
    view: wgpu::TextureView,
    id: egui::TextureId,
    /// The outlined layer alone, drawn on black.
    mask: wgpu::TextureView,
    mask_bind: wgpu::BindGroup,
}

pub struct Preview {
    render: RenderState,
    slots: Slots,
    target: Option<Target>,
    renderer: Option<(PathBuf, GameEffectRenderer)>,
    /// What the open file's draws sample, built as needed and dropped when the file changes.
    meshes: HashMap<u64, Option<Arc<MeshData>>>,
    pixels: HashMap<usize, Option<Arc<SampledTexture>>>,
    views: HashMap<TextureKey, wgpu::TextureView>,
    pub age: f32,
    pub playing: bool,
    pub orbit: egui::Vec2,
    /// Draw only this layer.
    pub solo: Option<usize>,
    /// Half the height of the view at the origin, in Smash units.
    pub half_view: f32,
    /// Draw a yellow line round this layer.
    pub outline: Option<usize>,
    /// Move the emitter round a circle, as a hand would: trails only show on an emitter that moves.
    pub swing: bool,
    outline_pipeline: Option<(wgpu::RenderPipeline, wgpu::BindGroupLayout)>,
}

impl Preview {
    pub fn new(render: RenderState) -> Self {
        // wgpu panics on these by default. The effect at fault is named by the log's last line.
        render.device.on_uncaptured_error(Arc::new(|error| crate::log::note(&format!("GPU error: {error}"))));
        Self {
            render, slots: Slots::new(), target: None, renderer: None, meshes: HashMap::new(), pixels: HashMap::new(),
            views: HashMap::new(), age: 0.0, playing: true, orbit: egui::Vec2::ZERO, solo: None, half_view: HALF_VIEW,
            outline: None, swing: false, outline_pipeline: None,
        }
    }

    /// The file changed: anything read from it may be stale.
    pub fn invalidate(&mut self) {
        self.meshes.clear();
        self.pixels.clear();
        self.views.clear();
    }

    /// How long the effect takes to play through once.
    pub fn length(&self, doc: &Doc, set_idx: usize) -> f32 {
        doc.emitters(set_idx).iter().map(|em| {
            let sim = EmitterSim::read(em, &self.slots);
            sim.emission_start + sim.emission_duration.min(240.0) + sim.life
        }).fold(30.0, f32::max).min(600.0)
    }

    /// Each layer's draw, with its index among the effect's layers.
    fn draws(&mut self, doc: &Doc, set_idx: usize) -> Vec<(usize, GameDraw)> {
        let file = doc.working();
        let mut out = Vec::new();
        for (index, emitter) in doc.emitters(set_idx).iter().enumerate() {
            if self.solo.is_some_and(|solo| solo != index) {
                continue;
            }
            let Some(game) = doc.loaded.game.get(set_idx).and_then(|set| set.get(index)).cloned().flatten() else { continue };
            let sim = EmitterSim::read(emitter, &self.slots);
            let seed = (set_idx as u64) << 32 ^ (index as u64) << 8;
            let (local, local_rt) = crate::draw::inputs::emitter_local(&game.stored);
            let speed = SWING_RADIUS * std::f32::consts::TAU / SWING_FRAMES;
            let travelled = |t: f32| t * speed;
            let mut spawned = spawns_moving(&sim, self.age, seed, self.swing.then_some(&travelled as &dyn Fn(f32) -> f32));
            if self.swing {
                // Particles that stay where they were born keep the swing's position at their birth; the rest go
                // round with it. Positions are in the emitter's own frame, so the move is taken into it.
                let into_emitter = local.inverse();
                for spawn in &mut spawned {
                    let at = if sim.follow_type == 1 { spawn.birth } else { self.age };
                    spawn.position += into_emitter.transform_vector3(swing_at(at));
                }
            }
            let dynamic = Dynamic {
                colour0: [1.0; 4],
                colour1: [1.0; 4],
                time: self.age,
                particles: spawned.len() as u32,
                particle_scale: [1.0, 1.0],
                set_scale: [1.0; 4],
                matrix: local,
                matrix_rt: local_rt,
            };
            let mesh = sim.primitive_id.and_then(|id| {
                self.meshes.entry(id).or_insert_with(|| doc.mesh_by_id(id).ok().map(Arc::new)).clone()
            });
            let pool = doc.loaded.texture_pool.as_deref().unwrap_or_default();
            let textures = game.samplers.map(|sampler| {
                let (index, _) = sampler?;
                let name = &doc.loaded.ptcl.bntx_textures.get(index)?.tex_name;
                let pixels = self.pixels.entry(index).or_insert_with(|| decode_for_sampling(pool, index, name).ok().map(Arc::new)).clone()?;
                Some((TextureKey { file: file.clone(), index }, pixels))
            });
            out.push((index, GameDraw { file: file.clone(), emitter: game, dynamic, spawns: spawned, mesh, textures }));
        }
        out
    }

    fn camera(&self, aspect: f32) -> Vec<u8> {
        let distance = self.half_view / (FOV_Y * 0.5).tan();
        let eye = glam::Quat::from_rotation_y(-self.orbit.x) * glam::Quat::from_rotation_x(-self.orbit.y) * glam::Vec3::new(0.0, 0.0, distance);
        let view = glam::Mat4::look_at_rh(eye, glam::Vec3::ZERO, glam::Vec3::Y);
        let proj = glam::Mat4::perspective_infinite_rh(FOV_Y, aspect, 2.0);
        crate::draw::gpu::camera_block(view, proj, 2.0, 100_000.0, [1.0, 1.0, 0.01, 0.0])
    }

    fn outline_pipeline(&mut self) -> &(wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        if self.outline_pipeline.is_none() {
            let device = &self.render.device;
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("outline"),
                source: wgpu::ShaderSource::Wgsl(include_str!("outline.wgsl").into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("outline"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("outline"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("outline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: DRAW_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            self.outline_pipeline = Some((pipeline, layout));
        }
        self.outline_pipeline.as_ref().unwrap()
    }

    fn target(&mut self, size: [u32; 2]) -> &Target {
        if self.target.as_ref().is_none_or(|t| t.size != size) {
            let layout = self.outline_pipeline().1.clone();
            let device = &self.render.device;
            let mask_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("outline mask"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SHOW_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[DRAW_FORMAT],
            });
            let mask = mask_texture.create_view(&wgpu::TextureViewDescriptor { format: Some(DRAW_FORMAT), ..Default::default() });
            let mask_read = mask_texture.create_view(&Default::default());
            let mask_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("outline"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&mask_read) }],
            });
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("effect preview"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SHOW_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[DRAW_FORMAT],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(DRAW_FORMAT), ..Default::default() });
            let show = texture.create_view(&Default::default());
            let mut egui_renderer = self.render.renderer.write();
            let id = match self.target.take() {
                Some(old) => {
                    egui_renderer.update_egui_texture_from_wgpu_texture(device, &show, wgpu::FilterMode::Linear, old.id);
                    old.id
                }
                None => egui_renderer.register_native_texture(device, &show, wgpu::FilterMode::Linear),
            };
            drop(egui_renderer);
            self.target = Some(Target { size, view, id, mask, mask_bind });
        }
        self.target.as_ref().unwrap()
    }

    /// Draw this frame, and say which window texture holds it. `None` until the file's shaders are translated.
    pub fn render(&mut self, doc: &Doc, set_idx: usize, size: [u32; 2]) -> Result<Option<egui::TextureId>> {
        let Some(dir) = crate::draw::shaders::ready(&doc.working()) else { return Ok(None) };
        let device = self.render.device.clone();
        let queue = self.render.queue.clone();
        if self.renderer.as_ref().is_none_or(|(d, _)| *d != dir) {
            let library = ShaderLibrary::open(&dir)?;
            self.renderer = Some((dir, GameEffectRenderer::new(&device, &queue, library)));
        }
        let draws = self.draws(doc, set_idx);
        let camera = self.camera(size[0] as f32 / size[1] as f32);
        let format = DRAW_FORMAT;
        for (_, draw) in &draws {
            for (key, pixels) in draw.textures.iter().flatten() {
                if !self.views.contains_key(key) {
                    let view = crate::draw::gpu::upload_sampled(&device, &queue, pixels, "effect texture");
                    self.views.insert(key.clone(), view);
                }
            }
        }
        let (_, renderer) = self.renderer.as_mut().ok_or_else(|| anyhow!("no renderer"))?;
        let mut prepared: Vec<(usize, PreparedDraw)> = Vec::new();
        let mut problems = Vec::new();
        for (index, draw) in &draws {
            let Some(ready) = crate::draw::inputs::prepare(draw, &renderer.library) else { continue };
            let textures: Vec<TextureInput> = draw.textures.iter().enumerate().filter_map(|(unit, texture)| {
                let (key, _) = texture.as_ref()?;
                Some(TextureInput { stage: FRAGMENT, unit: unit as u32, view: self.views.get(key)?, sampler: draw.emitter.samplers[unit]?.1 })
            }).collect();
            let mut cbufs: Vec<(u32, u32, &[u8])> = ready.cbufs.iter().map(|(s, n, b)| (*s, *n, b.as_slice())).collect();
            cbufs.push((VERTEX, 8, &camera));
            cbufs.push((FRAGMENT, 8, &camera));
            let inputs = DrawInputs {
                variation: ready.variation, cbufs, vertices: ready.vertices, indices: ready.indices, state: ready.state,
                // every captured effect draw had the alpha test off
                alpha_func: 8, alpha_ref: 0.0, count: ready.count, instances: ready.instances, first: 0, base_vertex: 0,
                base_instance: 0, textures,
            };
            match renderer.prepare(&device, &inputs, format, None) {
                Ok(draw) => prepared.push((*index, draw)),
                Err(e) => problems.push(format!("{e:#}")),
            }
        }
        if let Some(problem) = problems.first() {
            eprintln!("[preview] {} of {} layers failed: {problem}", problems.len(), draws.len());
        }
        let outline = self.outline.filter(|layer| prepared.iter().any(|(index, _)| index == layer));
        self.outline_pipeline();
        let target = self.target(size);
        let id = target.id;
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effect preview"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.013, g: 0.012, b: 0.024, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for (_, draw) in &prepared {
                draw.encode(&mut pass);
            }
        }
        if let Some(layer) = outline {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("outline mask"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.mask,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                for (_, draw) in prepared.iter().filter(|(index, _)| *index == layer) {
                    draw.encode(&mut pass);
                }
            }
            let (pipeline, _) = self.outline_pipeline.as_ref().unwrap();
            let target = self.target.as_ref().unwrap();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("outline"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &target.mask_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        Ok(Some(id))
    }
}
