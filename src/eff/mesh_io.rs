//! Effect meshes (primitives): listing them, out to `.glb`, and in from glTF as a new primitive.
//!
//! A primitive is one model in the `.eff`'s BFRES pool: one shape of u16 triangles. Its vertex layout varies from
//! file to file (positions f32 or half, UVs half, unorm, snorm or 8-bit, colour often absent), so reading decodes
//! each attribute by its format word. Writing always uses one layout: f32 position, packed normal, u8 colour, half
//! UVs. The primitive's descriptor in the PTCL says which attribute slot is which, so a new primitive's descriptor is
//! pointed at the layout it was written in.
//!
//! glTF is Y-up like the game (Blender converts from Z-up on export), and its vertex colour has the alpha effect
//! rings fade their edges with.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Context, Result};
use effect_library::bfres::{Model, ResFile};
use effect_library::structs::Emitter;
use effect_library::NamcoEffectFile;

/// A mesh in plain floats. The optional attributes are empty when absent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// RGBA 0..1. None reads as white.
    pub colors: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    /// The second UV set, for emitters that sample two textures.
    pub uv1: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// (min, max) corners; zeros when empty.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        if self.positions.is_empty() {
            return ([0.0; 3], [0.0; 3]);
        }
        self.positions.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), p| {
            (std::array::from_fn(|a| lo[a].min(p[a])), std::array::from_fn(|a| hi[a].max(p[a])))
        })
    }

    fn check(&self) -> Result<()> {
        let n = self.positions.len();
        if n == 0 || self.indices.is_empty() {
            bail!("the mesh has no triangles");
        }
        if n > u16::MAX as usize {
            bail!("{n} vertices; an effect primitive holds at most 65535");
        }
        if self.indices.len() % 3 != 0 {
            bail!("index count {} is not a triangle list", self.indices.len());
        }
        if let Some(bad) = self.indices.iter().find(|&&i| i as usize >= n) {
            bail!("index {bad} is past the last of {n} vertices");
        }
        let lens = [("normals", self.normals.len()), ("colours", self.colors.len()), ("uv0", self.uv0.len()), ("uv1", self.uv1.len())];
        if let Some((name, len)) = lens.into_iter().find(|&(_, len)| len != 0 && len != n) {
            bail!("{len} {name} for {n} vertices");
        }
        Ok(())
    }
}

fn unit(n: [f32; 3]) -> [f32; 3] {
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if length > 1e-6 {
        n.map(|c| c / length)
    } else {
        [0.0, 1.0, 0.0]
    }
}

// Vertex attributes. A format word's high byte is the width, the low byte how to read it: 1 unorm, 2 snorm,
// 5 float, anything else the plain integer.

/// One attribute as four floats; components it lacks are 0 and alpha 1.
fn read_attribute(bytes: &[u8], format: u16) -> Option<[f32; 4]> {
    let (width, kind) = (format >> 8, format & 0xff);
    let size = match width {
        0x02 => 1,
        0x09 => 2,
        0x0b | 0x0e | 0x12 => 4,
        0x15 | 0x17 => 8,
        0x18 => 12,
        0x19 => 16,
        _ => return None,
    };
    let b = bytes.get(..size)?;
    let mut out = [0.0, 0.0, 0.0, 1.0];
    match width {
        0x02 | 0x09 | 0x0b => {
            for (o, &v) in out.iter_mut().zip(b) {
                *o = match kind {
                    1 => v as f32 / 255.0,
                    2 => (v as i8 as f32 / 127.0).max(-1.0),
                    _ => v as f32,
                };
            }
        }
        // 10:10:10:2, x lowest.
        0x0e => {
            let word = u32::from_le_bytes(b.try_into().unwrap());
            for (i, o) in out.iter_mut().take(3).enumerate() {
                let bits = (word >> (i * 10)) & 0x3ff;
                *o = if kind == 2 { ((((bits << 22) as i32) >> 22) as f32 / 511.0).max(-1.0) } else { bits as f32 / 1023.0 };
            }
            out[3] = (word >> 30) as f32 / 3.0;
        }
        0x12 | 0x15 => {
            for (o, c) in out.iter_mut().zip(b.chunks_exact(2)) {
                let v = u16::from_le_bytes([c[0], c[1]]);
                *o = match kind {
                    1 => v as f32 / 65535.0,
                    2 => (v as i16 as f32 / 32767.0).max(-1.0),
                    5 => half_to_f32(v),
                    _ => v as f32,
                };
            }
        }
        _ if kind == 5 => {
            for (o, c) in out.iter_mut().zip(b.chunks_exact(4)) {
                *o = f32::from_le_bytes(c.try_into().unwrap());
            }
        }
        _ => return None,
    }
    Some(out)
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = ((bits >> 10) & 0x1f) as i32;
    let fraction = (bits & 0x3ff) as f32 / 1024.0;
    sign * match exponent {
        0 => fraction * 2f32.powi(-14),
        0x1f if fraction == 0.0 => f32::INFINITY,
        0x1f => f32::NAN,
        _ => (1.0 + fraction) * 2f32.powi(exponent - 15),
    }
}

/// Rounds half up; past the largest finite half gives it, not infinity, unless the float was already too big.
fn f32_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 112;
    let mantissa = bits & 0x7f_ffff;
    if value.is_nan() {
        sign | 0x7e00
    } else if exponent >= 0x1f {
        sign | 0x7c00
    } else if exponent < -10 {
        sign
    } else if exponent <= 0 {
        sign | ((((mantissa | 0x80_0000) >> (1 - exponent)) + 0x2000) >> 14) as u16
    } else {
        sign | ((((exponent as u32) << 10) | (mantissa >> 13)) + ((mantissa >> 12) & 1)).min(0x7bff) as u16
    }
}

/// The model's first shape as a mesh.
fn read_model(model: &Model) -> Result<MeshData> {
    let vb = model.vertex_buffers.first().ok_or_else(|| anyhow!("primitive has no vertex buffer"))?;
    let column = |name: &str| -> Result<Vec<[f32; 4]>> {
        let Some(attr) = vb.attributes.get(name) else { return Ok(Vec::new()) };
        let buffer = attr.buffer_index as usize;
        let data = vb.buffers.get(buffer).ok_or_else(|| anyhow!("{name} is in buffer {buffer}, which does not exist"))?;
        let stride = vb.buffer_strides.get(buffer).copied().unwrap_or(0) as usize;
        if stride == 0 {
            bail!("{name}'s buffer has no stride");
        }
        (0..vb.vertex_count as usize)
            .map(|i| {
                data.get(i * stride + attr.offset as usize..)
                    .and_then(|rest| read_attribute(rest, attr.format))
                    .ok_or_else(|| anyhow!("{name} format {:#06x} unreadable", attr.format))
            })
            .collect()
    };
    let xyz = |v: Vec<[f32; 4]>| v.into_iter().map(|v| [v[0], v[1], v[2]]).collect();
    let uv = |name| Ok::<_, anyhow::Error>(column(name)?.into_iter().map(|v| [v[0], v[1]]).collect());

    if !vb.attributes.contains_key("_p0") {
        bail!("primitive has no position attribute");
    }
    let positions = xyz(column("_p0")?);
    let normals = xyz(column("_n0")?);
    let colors = column("_c0")?;
    let mesh = model
        .shapes
        .values()
        .next()
        .ok_or_else(|| anyhow!("primitive has no shape"))?
        .meshes
        .first()
        .ok_or_else(|| anyhow!("primitive shape has no mesh"))?;
    if mesh.primitive_type != 3 {
        bail!("primitive type {} is not a triangle list", mesh.primitive_type);
    }
    let indices = match mesh.index_format {
        1 => mesh.index_data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect(),
        2 => mesh.index_data.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect(),
        other => bail!("index format {other} is not u16 or u32"),
    };
    Ok(MeshData { positions, normals, colors, uv0: uv("_u0")?, uv1: uv("_u1")?, indices })
}

/// Replace the model's geometry with `mesh` in Forge's layout, keeping its name, material and skeleton. Returns the
/// attribute names in slot order. A normal, colour or second UV the model had stays in the layout even if the mesh
/// lacks it (filled with up, white, or the first UV).
fn write_model(model: &mut Model, mesh: &MeshData) -> Result<Vec<String>> {
    mesh.check()?;
    let vb = model.vertex_buffers.first_mut().ok_or_else(|| anyhow!("primitive has no vertex buffer to write into"))?;
    if vb.vertex_skin_count > 0 {
        bail!("this primitive is skinned; replacing skinned effect meshes is not supported");
    }
    let keeps = |name: &str, mesh_has: bool| vb.attributes.contains_key(name) || mesh_has;
    let (normal, colour, uv1) = (keeps("_n0", !mesh.normals.is_empty()), keeps("_c0", !mesh.colors.is_empty()), keeps("_u1", !mesh.uv1.is_empty()));
    // (name, format, bytes): f32 position, 10:10:10 snorm normal, u8 unorm colour, half UVs.
    let layout: Vec<(&str, u16)> = [("_p0", 0x1805, true), ("_n0", 0x0e02, normal), ("_c0", 0x0b01, colour), ("_u0", 0x1205, true), ("_u1", 0x1205, uv1)]
        .into_iter()
        .filter(|&(_, _, on)| on)
        .map(|(name, format, _)| (name, format))
        .collect();
    let stride = 12 + 4 * (layout.len() - 1);

    let template = vb
        .attributes
        .get("_p0")
        .or_else(|| vb.attributes.values().next())
        .cloned()
        .ok_or_else(|| anyhow!("primitive has no attributes to model the layout on"))?;
    vb.attributes.clear();
    let mut offset = 0;
    for &(name, format) in &layout {
        let mut attr = template.clone();
        (attr.name, attr.buffer_index, attr.offset, attr.format) = (name.to_string(), 0, offset, format);
        vb.attributes.insert(name.to_string(), attr);
        offset += if name == "_p0" { 12 } else { 4 };
    }

    let mut data = Vec::with_capacity(mesh.positions.len() * stride);
    for (i, p) in mesh.positions.iter().enumerate() {
        data.extend(p.iter().flat_map(|c| c.to_le_bytes()));
        if normal {
            let n = unit(mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]));
            let word = n.iter().enumerate().fold(0u32, |w, (a, c)| w | ((((c.clamp(-1.0, 1.0) * 511.0).round() as i32) as u32 & 0x3ff) << (a * 10)));
            data.extend(word.to_le_bytes());
        }
        if colour {
            data.extend(mesh.colors.get(i).copied().unwrap_or([1.0; 4]).map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
        let first = mesh.uv0.get(i).copied().unwrap_or([0.0; 2]);
        let second = mesh.uv1.get(i).copied().unwrap_or(first);
        for uv in [Some(first), uv1.then_some(second)].into_iter().flatten() {
            data.extend(uv.iter().flat_map(|&c| f32_to_half(c).to_le_bytes()));
        }
    }

    let gpu_flags = vb.buffer_gpu_flags.first().copied().unwrap_or(0);
    if vb.buffer_unk_data.len() < 72 {
        vb.buffer_unk_data = vec![0; 72];
    }
    vb.buffer_unk_data.truncate(72);
    vb.buffer_sizes = vec![data.len() as u32];
    vb.buffer_strides = vec![stride as u32];
    vb.buffer_gpu_flags = vec![gpu_flags];
    vb.buffers = vec![data];
    vb.vertex_count = mesh.positions.len() as u32;

    let (min, max) = mesh.bounds();
    let center: [f32; 3] = std::array::from_fn(|a| (min[a] + max[a]) * 0.5);
    let extent: [f32; 3] = std::array::from_fn(|a| (max[a] - min[a]) * 0.5);
    let radius = mesh
        .positions
        .iter()
        .map(|p| (0..3).map(|a| (p[a] - center[a]) * (p[a] - center[a])).sum::<f32>().sqrt())
        .fold(0.0f32, f32::max);

    let shape = model.shapes.values_mut().next().ok_or_else(|| anyhow!("primitive has no shape"))?;
    shape.meshes.truncate(1);
    let target = shape.meshes.first_mut().ok_or_else(|| anyhow!("primitive shape has no mesh"))?;
    (target.primitive_type, target.index_format, target.first_vertex) = (3, 1, 0);
    target.index_count = mesh.indices.len() as u32;
    target.index_data = mesh.indices.iter().flat_map(|&i| (i as u16).to_le_bytes()).collect();
    target.sub_meshes.truncate(1);
    if let Some(sub) = target.sub_meshes.first_mut() {
        (sub.offset, sub.count) = (0, mesh.indices.len() as u32);
    }
    for b in &mut shape.sub_mesh_boundings {
        (b.center.x, b.center.y, b.center.z) = (center[0], center[1], center[2]);
        (b.extent.x, b.extent.y, b.extent.z) = (extent[0], extent[1], extent[2]);
    }
    for v in &mut shape.bounding_radius_list {
        (v.x, v.y, v.z, v.w) = (center[0], center[1], center[2], radius);
    }
    shape.radius_array.iter_mut().for_each(|r| *r = radius);
    Ok(layout.iter().map(|(name, _)| name.to_string()).collect())
}

/// One primitive of an `.eff`.
#[derive(Debug, Clone, PartialEq)]
pub struct PrimitiveListing {
    pub name: String,
    pub id: u64,
    pub vertices: usize,
    pub triangles: usize,
    /// `effect/emitter` for every emitter that draws it.
    pub used_by: Vec<String>,
}

fn load(eff: &[u8]) -> Result<NamcoEffectFile> {
    NamcoEffectFile::load(eff).map_err(|e| anyhow!("parsing the eff: {e}"))
}

/// Every emitter with the first effect that plays its set (the set's own name when none does).
fn each_emitter(file: &mut NamcoEffectFile, mut f: impl FnMut(&str, &mut Emitter)) {
    let mut effect_of: HashMap<usize, String> = HashMap::new();
    for (entry, name) in file.entries.iter().zip(&file.entry_names) {
        if let Some(set) = (entry.emitter_set_id as usize).checked_sub(1) {
            effect_of.entry(set).or_insert_with(|| name.clone());
        }
    }
    fn walk(list: &mut [Emitter], label: &str, f: &mut impl FnMut(&str, &mut Emitter)) {
        for e in list {
            f(label, e);
            walk(&mut e.children, label, f);
        }
    }
    let Some(ptcl) = file.ptcl_file.as_mut() else { return };
    for (i, set) in ptcl.emitter_list.emitter_sets.iter_mut().enumerate() {
        let label = effect_of.get(&i).cloned().unwrap_or_else(|| set.name.clone());
        walk(&mut set.emitters, &label, &mut f);
    }
}

fn pool(file: &NamcoEffectFile) -> Option<&effect_library::ptcl_file::PrimitiveInfo> {
    file.ptcl_file.as_ref()?.primitive_info.as_ref()
}

/// Each model of the pool as a BFRES of its own, with its name and parsed model.
fn models(file: &NamcoEffectFile) -> Result<Vec<(Vec<u8>, String, Model)>> {
    let Some(info) = pool(file) else { return Ok(Vec::new()) };
    let Some(blob) = info.binary_data.as_deref() else { return Ok(Vec::new()) };
    let mut session = None;
    (0..info.descriptors.len())
        .map(|index| {
            let single = effect_library::bfres::export_single_model_with_session(&mut session, blob, index)
                .with_context(|| format!("extracting primitive {index}"))?;
            let (name, model) = ResFile::parse_model_export(single.clone())?;
            Ok((single, name, model))
        })
        .collect()
}

/// The primitives of an `.eff`, with the emitters that draw each.
pub fn list_primitives(eff: &[u8]) -> Result<Vec<PrimitiveListing>> {
    let mut file = load(eff)?;
    let mut users: HashMap<u64, Vec<String>> = HashMap::new();
    each_emitter(&mut file, |effect, e| {
        let p = &e.data.particle_data;
        for id in [p.primitive_id, p.primitive_ex_id].into_iter().filter(|&id| id != 0 && id != u64::MAX) {
            let user = format!("{effect}/{}", e.data.display_name());
            let list = users.entry(id).or_default();
            if !list.contains(&user) {
                list.push(user);
            }
        }
    });
    let ids: Vec<u64> = pool(&file).map(|info| info.descriptors.iter().map(|d| d.id).collect()).unwrap_or_default();
    models(&file)?
        .into_iter()
        .zip(ids)
        .map(|((_, name, model), id)| {
            let mesh = read_model(&model)?;
            Ok(PrimitiveListing {
                name,
                id,
                vertices: mesh.positions.len(),
                triangles: mesh.indices.len() / 3,
                used_by: users.remove(&id).unwrap_or_default(),
            })
        })
        .collect()
}

fn find(models: &[(Vec<u8>, String, Model)], name: &str) -> Option<usize> {
    models.iter().position(|(_, n, _)| n.eq_ignore_ascii_case(name))
}

fn no_primitive(file: &NamcoEffectFile, name: &str) -> anyhow::Error {
    match pool(file) {
        None => anyhow!("the eff has no primitives"),
        Some(info) if info.binary_data.is_none() => anyhow!("the eff has no primitive pool"),
        Some(_) => anyhow!("the eff has no primitive named {name}"),
    }
}

/// The primitive called `name` as a `.glb`.
pub fn export_primitive_glb(eff: &[u8], name: &str) -> Result<Vec<u8>> {
    let file = load(eff)?;
    let models = models(&file)?;
    let index = find(&models, name).ok_or_else(|| no_primitive(&file, name))?;
    to_glb(&models[index].1, &read_model(&models[index].2)?)
}

/// A new primitive `new_name` made from `mesh` in `template`'s place in the pool (its material, skeleton and
/// descriptor), drawn by the `targets` (effect, emitter) instead of `template`. A primitive is shared by every
/// emitter that draws it, so this is how one effect gets a shape of its own. Each target must draw `template`.
pub fn add_primitive_for_emitters(eff: &[u8], template: &str, new_name: &str, mesh: &MeshData, targets: &[(String, String)]) -> Result<Vec<u8>> {
    let mut file = load(eff)?;
    let models = models(&file)?;
    let index = find(&models, template).ok_or_else(|| no_primitive(&file, template))?;
    if find(&models, new_name).is_some() {
        bail!("the eff already has a primitive named {new_name}");
    }
    let template_file = &models[index].0;
    let mut model = effect_library::bfres::load::load_from_bytes(template_file)
        .map_err(|e| anyhow!("reading {template}: {e:?}"))?
        .models
        .values()
        .next()
        .cloned()
        .ok_or_else(|| anyhow!("{template} holds no model"))?;
    let order = write_model(&mut model, mesh)?;
    model.name = new_name.to_string();
    let copy = ResFile::rebuild_from_base_and_models(template_file, None, &[(new_name.to_string(), model)])?;
    let mut singles: Vec<Vec<u8>> = models.into_iter().map(|(single, _, _)| single).collect();
    singles.push(copy);
    let merged = ResFile::merge_model_files(&singles)?;

    let info = file.ptcl_file.as_mut().and_then(|p| p.primitive_info.as_mut()).expect("the pool was read");
    info.binary_data = Some(merged);
    let taken: Vec<u64> = info.descriptors.iter().map(|d| d.id).collect();
    let mut descriptor = info.descriptors[index].clone();
    descriptor.id = crate::eff::textures::unused_descriptor_id(&taken, new_name);
    let slot = |name: &str| order.iter().position(|n| n == name).map_or(-1, |i| i as i8);
    descriptor.position_index = slot("_p0");
    descriptor.normal_index = slot("_n0");
    descriptor.tangent_index = slot("_t0");
    descriptor.color_index = slot("_c0");
    descriptor.tex_coord0_index = slot("_u0");
    descriptor.tex_coord1_index = slot("_u1");
    let (old_id, new_id) = (info.descriptors[index].id, descriptor.id);
    info.descriptors.push(descriptor);

    let mut hit = vec![false; targets.len()];
    each_emitter(&mut file, |effect, e| {
        let name = e.data.display_name();
        for (hit, (want_effect, want_emitter)) in hit.iter_mut().zip(targets) {
            if effect.eq_ignore_ascii_case(want_effect) && name == *want_emitter {
                let p = &mut e.data.particle_data;
                for id in [&mut p.primitive_id, &mut p.primitive_ex_id] {
                    if *id == old_id {
                        (*id, *hit) = (new_id, true);
                    }
                }
            }
        }
    });
    if let Some(miss) = hit.iter().position(|h| !h) {
        bail!("{}/{} does not draw {template}", targets[miss].0, targets[miss].1);
    }
    Ok(file.save()?)
}

/// `mesh` as a self-contained `.glb`: float attributes, normals made unit length, colours clamped, u32 indices.
pub fn to_glb(name: &str, mesh: &MeshData) -> Result<Vec<u8>> {
    use serde_json::json;
    mesh.check()?;
    let count = mesh.positions.len();
    let (min, max) = mesh.bounds();
    let floats = |values: Vec<f32>| values.into_iter().flat_map(f32::to_le_bytes).collect::<Vec<u8>>();
    let flat = |rows: Vec<Vec<f32>>| floats(rows.concat());

    // (glTF attribute, data, accessor type)
    let mut parts: Vec<(&str, Vec<u8>, &str)> = vec![("POSITION", flat(mesh.positions.iter().map(|p| p.to_vec()).collect()), "VEC3")];
    if !mesh.normals.is_empty() {
        parts.push(("NORMAL", flat(mesh.normals.iter().map(|&n| unit(n).to_vec()).collect()), "VEC3"));
    }
    if !mesh.colors.is_empty() {
        parts.push(("COLOR_0", flat(mesh.colors.iter().map(|c| c.map(|v| v.clamp(0.0, 1.0)).to_vec()).collect()), "VEC4"));
    }
    for (key, uvs) in [("TEXCOORD_0", &mesh.uv0), ("TEXCOORD_1", &mesh.uv1)] {
        if !uvs.is_empty() {
            parts.push((key, flat(uvs.iter().map(|uv| uv.to_vec()).collect()), "VEC2"));
        }
    }

    let (mut bin, mut views, mut accessors, mut attributes) = (Vec::new(), Vec::new(), Vec::new(), serde_json::Map::new());
    let mut add = |bytes: Vec<u8>, target: u32, mut accessor: serde_json::Value| {
        bin.resize(bin.len().next_multiple_of(4), 0);
        views.push(json!({"buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len(), "target": target}));
        bin.extend(bytes);
        accessor["bufferView"] = (views.len() - 1).into();
        accessors.push(accessor);
        accessors.len() - 1
    };
    for (key, bytes, kind) in parts {
        let mut accessor = json!({"componentType": 5126, "count": count, "type": kind});
        if key == "POSITION" {
            (accessor["min"], accessor["max"]) = (json!(min), json!(max));
        }
        attributes.insert(key.into(), add(bytes, 34962, accessor).into());
    }
    let indices = add(
        mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
        34963,
        json!({"componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR"}),
    );
    bin.resize(bin.len().next_multiple_of(4), 0);

    let mut text = serde_json::to_vec(&json!({
        "asset": {"version": "2.0", "generator": "Effect Forge mesh export"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": name, "mesh": 0}],
        "meshes": [{"name": name, "primitives": [{"attributes": attributes, "indices": indices, "mode": 4}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
    }))?;
    text.resize(text.len().next_multiple_of(4), b' ');

    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    for chunk in [&b"glTF"[..], &2u32.to_le_bytes(), &(total as u32).to_le_bytes(), &(text.len() as u32).to_le_bytes(), b"JSON", &text] {
        out.extend_from_slice(chunk);
    }
    for chunk in [&(bin.len() as u32).to_le_bytes()[..], b"BIN\0", &bin] {
        out.extend_from_slice(chunk);
    }
    Ok(out)
}

/// Every triangle mesh in a glTF scene as one mesh, node transforms applied (one primitive is one shape, so a ring
/// and a disc meant for it come in together). A vertex without a normal points up, without colour is white, without
/// UVs gets 0 (and the first set for the second); colour and the second UV set are dropped if no mesh has them.
/// Each vertex's normal as the area-weighted sum of its triangles' (up where it has none).
fn smooth_normals(mesh: &mut MeshData) {
    let mut sum = vec![glam::Vec3::ZERO; mesh.positions.len()];
    for t in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| glam::Vec3::from(mesh.positions[i as usize]));
        let face = (b - a).cross(c - a);
        for &i in t {
            sum[i as usize] += face;
        }
    }
    mesh.normals = sum.into_iter().map(|n| n.try_normalize().unwrap_or(glam::Vec3::Y).to_array()).collect();
}

pub fn from_gltf(bytes: &[u8]) -> Result<MeshData> {
    let (document, buffers, _) = gltf::import_slice(bytes).context("reading the glTF")?;
    let scene = document.default_scene().or_else(|| document.scenes().next()).ok_or_else(|| anyhow!("the glTF has no scene"))?;
    let mut out = MeshData::default();
    let (mut any_normal, mut any_colour, mut any_uv1) = (false, false, false);
    let mut stack: Vec<(gltf::Node, glam::Mat4)> = scene.nodes().map(|n| (n, glam::Mat4::IDENTITY)).collect();
    stack.reverse();
    while let Some((node, parent)) = stack.pop() {
        let world = parent * glam::Mat4::from_cols_array_2d(&node.transform().matrix());
        for primitive in node.mesh().iter().flat_map(|m| m.primitives()) {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(positions) = reader.read_positions() else { continue };
            let base = out.positions.len() as u32;
            out.positions.extend(positions.map(|p| world.transform_point3(p.into()).to_array()));
            let n = out.positions.len() - base as usize;
            let normal_matrix = glam::Mat3::from_mat4(world).inverse().transpose();
            match reader.read_normals() {
                Some(normals) => {
                    any_normal = true;
                    out.normals.extend(normals.map(|v| (normal_matrix * glam::Vec3::from(v)).normalize_or_zero().to_array()));
                }
                None => out.normals.extend(std::iter::repeat_n([0.0, 1.0, 0.0], n)),
            }
            match reader.read_colors(0) {
                Some(colours) => {
                    any_colour = true;
                    out.colors.extend(colours.into_rgba_f32());
                }
                None => out.colors.extend(std::iter::repeat_n([1.0; 4], n)),
            }
            let uv0: Vec<[f32; 2]> = reader.read_tex_coords(0).map_or_else(|| vec![[0.0; 2]; n], |uvs| uvs.into_f32().collect());
            match reader.read_tex_coords(1) {
                Some(uvs) => {
                    any_uv1 = true;
                    out.uv1.extend(uvs.into_f32());
                }
                None => out.uv1.extend_from_slice(&uv0),
            }
            out.uv0.extend(uv0);
            match reader.read_indices() {
                Some(indices) => out.indices.extend(indices.into_u32().map(|i| i + base)),
                None => out.indices.extend(base..base + n as u32),
            }
        }
        let at = stack.len();
        stack.extend(node.children().map(|c| (c, world)));
        stack[at..].reverse();
    }
    if !any_normal {
        smooth_normals(&mut out);
    }
    if !any_colour {
        out.colors.clear();
    }
    if !any_uv1 {
        out.uv1.clear();
    }
    out.check()?;
    Ok(out)
}
