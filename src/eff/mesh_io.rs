//! Effect meshes in and out: an `.eff` primitive to a `.glb` Blender opens, and back.
//!
//! ## What a primitive looks like
//!
//! Surveyed across ef_common, ef_edge and a converted mod's effects (287 primitives) rather than assumed:
//! every one is a single shape of u16-indexed triangles, but the VERTEX LAYOUT varies — seventeen
//! distinct layouts. Positions are f32 (`0x1805`) or half-float (`0x1505`); UVs are half-float,
//! 16-bit unorm, 16-bit SNORM (`0x1202`) or 8-bit; colour is present on about half, as u8 or half.
//! So reading goes through one generic decoder keyed by the format word, and writing always
//! produces one canonical layout — f32 position, packed normal, u8 colour, half-float UVs, the
//! most common layout in the corpus — whatever the primitive being replaced used.
//!
//! ## The descriptor must follow the layout
//!
//! A PTCL primitive descriptor carries, beside its id, the INDEX of each attribute in the model's
//! attribute list (`position_index`, `color_index`, `tex_coord0_index`...). Changing the layout
//! without rewriting those points the runtime's position at a UV, which draws nothing or noise.
//! [`replace_primitive`] therefore returns the attribute order it wrote, and the caller applies
//! it to the descriptor with [`apply_attribute_order`].
//!
//! ## Blender
//!
//! glTF is Y-up, like the game, and Blender's glTF exporter converts from its Z-up on the way
//! out, so a mesh keeps its orientation through the round trip. Vertex colour (`COLOR_0`) carries
//! alpha, which effect rings use to fade their edges — the reason this is glTF and not OBJ.

use anyhow::{anyhow, bail, Context, Result};
use effect_library::bfres::{Model, ResFile};

/// One mesh, decoded to plain floats.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    /// Empty when the source has none.
    pub normals: Vec<[f32; 3]>,
    /// RGBA, 0..1. Empty when the source has none, which the game reads as white.
    pub colors: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    /// The second UV set a two-texture emitter samples. Empty when absent.
    pub uv1: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Axis-aligned bounds, as (min, max).
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in &self.positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
        if self.positions.is_empty() {
            return ([0.0; 3], [0.0; 3]);
        }
        (min, max)
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
        if let Some(bad) = self.indices.iter().find(|i| **i as usize >= n) {
            bail!("index {bad} is past the last of {n} vertices");
        }
        for (name, len) in [
            ("normals", self.normals.len()),
            ("colours", self.colors.len()),
            ("uv0", self.uv0.len()),
            ("uv1", self.uv1.len()),
        ] {
            if len != 0 && len != n {
                bail!("{len} {name} for {n} vertices");
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Vertex formats
// ---------------------------------------------------------------------------------------------

/// Bytes one attribute occupies, from the width half of its format word.
fn format_size(format: u16) -> Option<usize> {
    Some(match format >> 8 {
        0x02 => 1,
        0x09 => 2,
        0x0b | 0x0e => 4,
        0x12 => 4,
        0x15 => 8,
        0x17 => 8,
        0x18 => 12,
        0x19 => 16,
        _ => return None,
    })
}

/// Decode one attribute to four floats (missing components are 0, alpha 1).
///
/// The low byte is the interpretation: 1 unorm, 2 snorm, 3 uint, 5 float.
pub fn decode(bytes: &[u8], format: u16) -> Option<[f32; 4]> {
    let kind = format & 0xff;
    let size = format_size(format)?;
    let b = bytes.get(..size)?;
    let mut out = [0.0, 0.0, 0.0, 1.0];
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let f32_at = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    match format >> 8 {
        // 8-bit channels.
        0x02 | 0x09 | 0x0b => {
            for (i, byte) in b.iter().enumerate() {
                out[i] = match kind {
                    1 => *byte as f32 / 255.0,
                    2 => ((*byte as i8) as f32 / 127.0).max(-1.0),
                    _ => *byte as f32,
                };
            }
        }
        // 10_10_10_2, x in the low bits.
        0x0e => {
            let word = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            for i in 0..3 {
                let bits = (word >> (i * 10)) & 0x3ff;
                out[i] = match kind {
                    2 => {
                        let signed = ((bits << 22) as i32) >> 22;
                        (signed as f32 / 511.0).max(-1.0)
                    }
                    _ => bits as f32 / 1023.0,
                };
            }
            out[3] = ((word >> 30) & 0x3) as f32 / 3.0;
        }
        // 16-bit channels.
        0x12 | 0x15 => {
            for i in 0..size / 2 {
                let v = u16_at(i * 2);
                out[i] = match kind {
                    1 => v as f32 / 65535.0,
                    2 => ((v as i16) as f32 / 32767.0).max(-1.0),
                    5 => half_to_f32(v),
                    _ => v as f32,
                };
            }
        }
        // 32-bit float channels.
        0x17 | 0x18 | 0x19 => {
            if kind != 5 {
                return None;
            }
            for i in 0..size / 4 {
                out[i] = f32_at(i * 4);
            }
        }
        _ => return None,
    }
    Some(out)
}

pub fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = ((bits >> 10) & 0x1f) as i32;
    let mantissa = (bits & 0x3ff) as f32;
    sign * match exponent {
        0 => mantissa / 1024.0 * 2f32.powi(-14),
        0x1f => {
            if mantissa == 0.0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

pub fn f32_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    if value.is_nan() {
        return sign | 0x7e00;
    }
    if exponent >= 0x1f {
        return sign | 0x7c00;
    }
    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        let m = (mantissa | 0x80_0000) >> (1 - exponent);
        return sign | ((m + 0x2000) >> 14) as u16;
    }
    let rounded = (((exponent as u32) << 10) | (mantissa >> 13)) + ((mantissa >> 12) & 1);
    sign | rounded.min(0x7bff) as u16
}

// ---------------------------------------------------------------------------------------------
// Model <-> MeshData
// ---------------------------------------------------------------------------------------------

/// Decode a primitive's first shape.
pub fn read_model(model: &Model) -> Result<MeshData> {
    let vb = model
        .vertex_buffers
        .first()
        .ok_or_else(|| anyhow!("primitive has no vertex buffer"))?;
    let count = vb.vertex_count as usize;
    let column = |name: &str| -> Result<Option<Vec<[f32; 4]>>> {
        let Some(attr) = vb.attributes.get(name) else {
            return Ok(None);
        };
        let buffer_index = attr.buffer_index as usize;
        let data = vb
            .buffers
            .get(buffer_index)
            .ok_or_else(|| anyhow!("{name} is in buffer {buffer_index}, which does not exist"))?;
        let stride = *vb.buffer_strides.get(buffer_index).unwrap_or(&0) as usize;
        if stride == 0 {
            bail!("{name}'s buffer has no stride");
        }
        let mut values = Vec::with_capacity(count);
        for i in 0..count {
            let at = i * stride + attr.offset as usize;
            let value = data
                .get(at..)
                .and_then(|rest| decode(rest, attr.format))
                .ok_or_else(|| anyhow!("{name} format {:#06x} unreadable", attr.format))?;
            values.push(value);
        }
        Ok(Some(values))
    };

    let positions = column("_p0")?
        .ok_or_else(|| anyhow!("primitive has no position attribute"))?
        .into_iter()
        .map(|v| [v[0], v[1], v[2]])
        .collect();
    let normals = column("_n0")?
        .unwrap_or_default()
        .into_iter()
        .map(|v| [v[0], v[1], v[2]])
        .collect();
    let colors = column("_c0")?.unwrap_or_default();
    let uv = |name: &str| -> Result<Vec<[f32; 2]>> {
        Ok(column(name)?
            .unwrap_or_default()
            .into_iter()
            .map(|v| [v[0], v[1]])
            .collect())
    };

    let shape = model
        .shapes
        .values()
        .next()
        .ok_or_else(|| anyhow!("primitive has no shape"))?;
    let mesh = shape
        .meshes
        .first()
        .ok_or_else(|| anyhow!("primitive shape has no mesh"))?;
    if mesh.primitive_type != 3 {
        bail!("primitive type {} is not a triangle list", mesh.primitive_type);
    }
    let indices = match mesh.index_format {
        1 => mesh
            .index_data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
            .collect(),
        2 => mesh
            .index_data
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        other => bail!("index format {other} is not u16 or u32"),
    };
    Ok(MeshData {
        positions,
        normals,
        colors,
        uv0: uv("_u0")?,
        uv1: uv("_u1")?,
        indices,
    })
}

const POSITION_F32: u16 = 0x1805;
const NORMAL_PACKED: u16 = 0x0e02;
const COLOR_U8: u16 = 0x0b01;
const UV_HALF: u16 = 0x1205;

fn pack_normal(n: [f32; 3]) -> u32 {
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let n = if length > 1e-6 {
        [n[0] / length, n[1] / length, n[2] / length]
    } else {
        [0.0, 1.0, 0.0]
    };
    let mut word = 0u32;
    for (i, c) in n.iter().enumerate() {
        let v = (c.clamp(-1.0, 1.0) * 511.0).round() as i32;
        word |= ((v as u32) & 0x3ff) << (i * 10);
    }
    word
}

/// Rewrite a primitive's geometry with `mesh`, in the canonical layout.
///
/// Returns the attribute names in the order written, for [`apply_attribute_order`]. The model's
/// name, material and skeleton are kept, so the descriptor id and whatever the runtime binds by
/// material still match.
pub fn write_model(model: &mut Model, mesh: &MeshData) -> Result<Vec<String>> {
    mesh.check()?;
    let vb = model
        .vertex_buffers
        .first_mut()
        .ok_or_else(|| anyhow!("primitive has no vertex buffer to write into"))?;
    if vb.vertex_skin_count > 0 {
        bail!("this primitive is skinned; replacing skinned effect meshes is not supported");
    }
    let had = |name: &str| vb.attributes.contains_key(name);
    let with_normal = had("_n0") || !mesh.normals.is_empty();
    let with_color = had("_c0") || !mesh.colors.is_empty();
    let with_uv1 = had("_u1") || !mesh.uv1.is_empty();

    let mut layout: Vec<(&str, u16, u16)> = vec![("_p0", POSITION_F32, 0)];
    let mut offset = 12u16;
    if with_normal {
        layout.push(("_n0", NORMAL_PACKED, offset));
        offset += 4;
    }
    if with_color {
        layout.push(("_c0", COLOR_U8, offset));
        offset += 4;
    }
    layout.push(("_u0", UV_HALF, offset));
    offset += 4;
    if with_uv1 {
        layout.push(("_u1", UV_HALF, offset));
        offset += 4;
    }
    let stride = offset as usize;

    let template = vb
        .attributes
        .get("_p0")
        .or_else(|| vb.attributes.values().next())
        .cloned()
        .ok_or_else(|| anyhow!("primitive has no attributes to model the layout on"))?;
    vb.attributes.clear();
    for (name, format, at) in &layout {
        let mut attr = template.clone();
        attr.name = name.to_string();
        attr.buffer_index = 0;
        attr.offset = *at;
        attr.format = *format;
        vb.attributes.insert(name.to_string(), attr);
    }

    let count = mesh.positions.len();
    let mut data = Vec::with_capacity(count * stride);
    for i in 0..count {
        for c in mesh.positions[i] {
            data.extend_from_slice(&c.to_le_bytes());
        }
        if with_normal {
            let n = mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
            data.extend_from_slice(&pack_normal(n).to_le_bytes());
        }
        if with_color {
            let c = mesh.colors.get(i).copied().unwrap_or([1.0; 4]);
            for channel in c {
                data.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        let uv0 = mesh.uv0.get(i).copied().unwrap_or([0.0; 2]);
        let uv1 = mesh.uv1.get(i).copied().unwrap_or(uv0);
        let uvs: &[[f32; 2]] = if with_uv1 { &[uv0, uv1] } else { &[uv0] };
        for uv in uvs {
            for c in uv {
                data.extend_from_slice(&f32_to_half(*c).to_le_bytes());
            }
        }
    }
    debug_assert_eq!(data.len(), count * stride);

    let gpu_flags = vb.buffer_gpu_flags.first().copied().unwrap_or(0);
    let unk: Vec<u8> = vb.buffer_unk_data.iter().copied().take(72).collect();
    vb.buffer_sizes = vec![data.len() as u32];
    vb.buffer_strides = vec![stride as u32];
    vb.buffer_gpu_flags = vec![gpu_flags];
    vb.buffer_unk_data = if unk.len() == 72 { unk } else { vec![0; 72] };
    vb.buffers = vec![data];
    vb.vertex_count = count as u32;

    // Geometry and bounds.
    let (min, max) = mesh.bounds();
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let extent = [
        (max[0] - min[0]) * 0.5,
        (max[1] - min[1]) * 0.5,
        (max[2] - min[2]) * 0.5,
    ];
    let radius = mesh
        .positions
        .iter()
        .map(|p| {
            let d = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        })
        .fold(0.0f32, f32::max);

    let shape = model
        .shapes
        .values_mut()
        .next()
        .ok_or_else(|| anyhow!("primitive has no shape"))?;
    shape.meshes.truncate(1);
    let target = shape
        .meshes
        .first_mut()
        .ok_or_else(|| anyhow!("primitive shape has no mesh"))?;
    target.primitive_type = 3;
    target.index_format = 1;
    target.index_count = mesh.indices.len() as u32;
    target.first_vertex = 0;
    target.index_data = mesh
        .indices
        .iter()
        .flat_map(|i| (*i as u16).to_le_bytes())
        .collect();
    target.sub_meshes.truncate(1);
    if let Some(sub) = target.sub_meshes.first_mut() {
        sub.offset = 0;
        sub.count = mesh.indices.len() as u32;
    }
    for b in &mut shape.sub_mesh_boundings {
        b.center.x = center[0];
        b.center.y = center[1];
        b.center.z = center[2];
        b.extent.x = extent[0];
        b.extent.y = extent[1];
        b.extent.z = extent[2];
    }
    for v in &mut shape.bounding_radius_list {
        v.x = center[0];
        v.y = center[1];
        v.z = center[2];
        v.w = radius;
    }
    for r in &mut shape.radius_array {
        *r = radius;
    }

    Ok(layout.iter().map(|(name, _, _)| name.to_string()).collect())
}

/// Point a descriptor's attribute indices at the order [`write_model`] wrote.
pub fn apply_attribute_order(
    descriptor: &mut effect_library::ptcl_file::PrimitiveDescriptor,
    order: &[String],
) {
    let index_of = |name: &str| {
        order
            .iter()
            .position(|n| n == name)
            .map(|i| i as i8)
            .unwrap_or(-1)
    };
    descriptor.position_index = index_of("_p0");
    descriptor.normal_index = index_of("_n0");
    descriptor.tangent_index = index_of("_t0");
    descriptor.color_index = index_of("_c0");
    descriptor.tex_coord0_index = index_of("_u0");
    descriptor.tex_coord1_index = index_of("_u1");
}

// ---------------------------------------------------------------------------------------------
// Pools
// ---------------------------------------------------------------------------------------------

/// Every model of a primitive pool, as single-model exports.
pub fn split_pool(blob: &[u8], count: usize) -> Result<Vec<Vec<u8>>> {
    let mut session = None;
    (0..count)
        .map(|index| {
            effect_library::bfres::export_single_model_with_session(&mut session, blob, index)
                .with_context(|| format!("extracting primitive {index}"))
        })
        .collect()
}

/// Decode one primitive of a pool.
pub fn read_primitive(blob: &[u8], index: usize) -> Result<(String, MeshData)> {
    let single = ResFile::export_single_model(blob, index)
        .with_context(|| format!("extracting primitive {index}"))?;
    let (name, model) = ResFile::parse_model_export(single)?;
    Ok((name, read_model(&model)?))
}


// ---------------------------------------------------------------------------------------------
// Whole .eff files
// ---------------------------------------------------------------------------------------------

/// One primitive of an `.eff`, as the editor lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PrimitiveListing {
    pub name: String,
    pub id: u64,
    pub vertices: usize,
    pub triangles: usize,
    /// `entry/emitter` for every emitter that draws this primitive.
    pub used_by: Vec<String>,
}

fn load_eff(eff: &[u8]) -> Result<effect_library::NamcoEffectFile> {
    effect_library::NamcoEffectFile::load(eff).map_err(|e| anyhow!("parsing the eff: {e}"))
}

/// Every emitter in the file, with the entry that plays its set. Sets no entry plays are
/// reported under the set's own name.
fn for_each_emitter(
    namco: &mut effect_library::NamcoEffectFile,
    mut f: impl FnMut(&str, &mut effect_library::structs::Emitter),
) {
    let mut entry_of: std::collections::HashMap<usize, String> = Default::default();
    for (i, entry) in namco.entries.iter().enumerate() {
        if let (Some(set), Some(name)) = (
            (entry.emitter_set_id as usize).checked_sub(1),
            namco.entry_names.get(i),
        ) {
            entry_of.entry(set).or_insert_with(|| name.clone());
        }
    }
    let Some(ptcl) = namco.ptcl_file.as_mut() else {
        return;
    };
    fn walk(
        emitters: &mut [effect_library::structs::Emitter],
        label: &str,
        f: &mut impl FnMut(&str, &mut effect_library::structs::Emitter),
    ) {
        for em in emitters {
            f(label, em);
            walk(&mut em.children, label, f);
        }
    }
    for (i, set) in ptcl.emitter_list.emitter_sets.iter_mut().enumerate() {
        let label = entry_of.get(&i).cloned().unwrap_or_else(|| set.name.clone());
        walk(&mut set.emitters, &label, &mut f);
    }
}

/// The primitives an `.eff` holds, with who draws each.
pub fn list_primitives(eff: &[u8]) -> Result<Vec<PrimitiveListing>> {
    let mut namco = load_eff(eff)?;
    let mut users: std::collections::HashMap<u64, Vec<String>> = Default::default();
    for_each_emitter(&mut namco, |entry, em| {
        for id in [
            em.data.particle_data.primitive_id,
            em.data.particle_data.primitive_ex_id,
        ] {
            if id != 0 && id != u64::MAX {
                let name = format!("{entry}/{}", em.data.display_name());
                let list = users.entry(id).or_default();
                if !list.contains(&name) {
                    list.push(name);
                }
            }
        }
    });
    let Some(info) = namco
        .ptcl_file
        .as_ref()
        .and_then(|p| p.primitive_info.as_ref())
    else {
        return Ok(Vec::new());
    };
    let Some(blob) = info.binary_data.as_ref() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (index, descriptor) in info.descriptors.iter().enumerate() {
        let (name, mesh) = read_primitive(blob, index)?;
        out.push(PrimitiveListing {
            name,
            id: descriptor.id,
            vertices: mesh.positions.len(),
            triangles: mesh.indices.len() / 3,
            used_by: users.remove(&descriptor.id).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// (index, pool size) of the primitive whose model is called `name`.
fn primitive_index(eff: &effect_library::NamcoEffectFile, name: &str) -> Result<(usize, usize)> {
    let info = eff
        .ptcl_file
        .as_ref()
        .and_then(|p| p.primitive_info.as_ref())
        .ok_or_else(|| anyhow!("the eff has no primitives"))?;
    let blob = info
        .binary_data
        .as_ref()
        .ok_or_else(|| anyhow!("the eff has no primitive pool"))?;
    let count = info.descriptors.len();
    let mut session = None;
    for index in 0..count {
        let single =
            effect_library::bfres::export_single_model_with_session(&mut session, blob, index)?;
        let (model_name, _) = ResFile::parse_model_export(single)?;
        if model_name.eq_ignore_ascii_case(name) {
            return Ok((index, count));
        }
    }
    bail!("the eff has no primitive named {name}")
}

fn pool_of(namco: &effect_library::NamcoEffectFile) -> &[u8] {
    namco
        .ptcl_file
        .as_ref()
        .and_then(|p| p.primitive_info.as_ref())
        .and_then(|info| info.binary_data.as_deref())
        .unwrap_or(&[])
}

/// One primitive of an `.eff`, as a `.glb`.
pub fn export_primitive_glb(eff: &[u8], name: &str) -> Result<Vec<u8>> {
    let namco = load_eff(eff)?;
    let (index, _) = primitive_index(&namco, name)?;
    let (model_name, mesh) = read_primitive(pool_of(&namco), index)?;
    to_glb(&model_name, &mesh)
}


/// Add a new primitive shaped like `template` and point the listed emitters at it.
///
/// The mesh counterpart of a texture addition: a primitive is shared by every emitter drawing
/// it -- `EF_cmn_ring00` is in dozens of effects -- so changing one effect's shape means giving
/// it a private copy. `targets` are (entry, emitter) names; each must draw `template`.
pub fn add_primitive_for_emitters(
    eff: &[u8],
    template: &str,
    new_name: &str,
    mesh: &MeshData,
    targets: &[(String, String)],
) -> Result<Vec<u8>> {
    let mut namco = load_eff(eff)?;
    let (index, count) = primitive_index(&namco, template)?;
    if primitive_index(&namco, new_name).is_ok() {
        bail!("the eff already has a primitive named {new_name}");
    }
    let mut singles = split_pool(pool_of(&namco), count)?;
    let file = effect_library::bfres::load::load_from_bytes(&singles[index])
        .map_err(|e| anyhow!("reading {template}: {e:?}"))?;
    let mut model = file
        .models
        .values()
        .next()
        .cloned()
        .ok_or_else(|| anyhow!("{template} holds no model"))?;
    let order = write_model(&mut model, mesh)?;
    model.name = new_name.to_string();
    let copy = ResFile::rebuild_from_base_and_models(
        &singles[index],
        None,
        &[(new_name.to_string(), model)],
    )?;
    singles.push(copy);
    let pool = ResFile::merge_model_files(&singles)?;

    let info = namco
        .ptcl_file
        .as_mut()
        .and_then(|p| p.primitive_info.as_mut())
        .expect("primitive_index found the table");
    info.binary_data = Some(pool);
    let taken: Vec<u64> = info.descriptors.iter().map(|d| d.id).collect();
    let mut descriptor = info.descriptors[index].clone();
    descriptor.id = crate::eff::textures::unused_descriptor_id(&taken, new_name);
    apply_attribute_order(&mut descriptor, &order);
    let (old_id, new_id) = (info.descriptors[index].id, descriptor.id);
    info.descriptors.push(descriptor);

    let mut hit = vec![false; targets.len()];
    for_each_emitter(&mut namco, |entry, em| {
        let name = em.data.display_name();
        for (i, (want_entry, want_emitter)) in targets.iter().enumerate() {
            if !entry.eq_ignore_ascii_case(want_entry) || name != *want_emitter {
                continue;
            }
            let data = &mut em.data.particle_data;
            if data.primitive_id == old_id {
                data.primitive_id = new_id;
                hit[i] = true;
            }
            if data.primitive_ex_id == old_id {
                data.primitive_ex_id = new_id;
                hit[i] = true;
            }
        }
    });
    if let Some(miss) = hit.iter().position(|h| !h) {
        bail!("{}/{} does not draw {template}", targets[miss].0, targets[miss].1);
    }
    Ok(namco.save()?)
}

// ---------------------------------------------------------------------------------------------
// glTF
// ---------------------------------------------------------------------------------------------

/// Write `mesh` as a self-contained `.glb`.
pub fn to_glb(name: &str, mesh: &MeshData) -> Result<Vec<u8>> {
    mesh.check()?;
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut attributes = serde_json::Map::new();

    let mut push = |bin: &mut Vec<u8>,
                    bytes: Vec<u8>,
                    target: u32,
                    accessor: serde_json::Value|
     -> usize {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        views.push(serde_json::json!({
            "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len(), "target": target,
        }));
        bin.extend_from_slice(&bytes);
        let mut accessor = accessor;
        accessor["bufferView"] = (views.len() - 1).into();
        accessors.push(accessor);
        accessors.len() - 1
    };
    let floats = |rows: &mut dyn Iterator<Item = f32>| -> Vec<u8> {
        rows.flat_map(|f| f.to_le_bytes()).collect()
    };

    let (min, max) = mesh.bounds();
    let count = mesh.positions.len();
    let position = push(
        &mut bin,
        floats(&mut mesh.positions.iter().flatten().copied()),
        34962,
        serde_json::json!({"componentType": 5126, "count": count, "type": "VEC3", "min": min, "max": max}),
    );
    attributes.insert("POSITION".into(), position.into());
    if !mesh.normals.is_empty() {
        let normals: Vec<f32> = mesh
            .normals
            .iter()
            .flat_map(|n| {
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if l > 1e-6 {
                    [n[0] / l, n[1] / l, n[2] / l]
                } else {
                    [0.0, 1.0, 0.0]
                }
            })
            .collect();
        let at = push(
            &mut bin,
            floats(&mut normals.into_iter()),
            34962,
            serde_json::json!({"componentType": 5126, "count": count, "type": "VEC3"}),
        );
        attributes.insert("NORMAL".into(), at.into());
    }
    if !mesh.colors.is_empty() {
        let at = push(
            &mut bin,
            floats(&mut mesh.colors.iter().flatten().map(|c| c.clamp(0.0, 1.0))),
            34962,
            serde_json::json!({"componentType": 5126, "count": count, "type": "VEC4"}),
        );
        attributes.insert("COLOR_0".into(), at.into());
    }
    for (key, uvs) in [("TEXCOORD_0", &mesh.uv0), ("TEXCOORD_1", &mesh.uv1)] {
        if uvs.is_empty() {
            continue;
        }
        let at = push(
            &mut bin,
            floats(&mut uvs.iter().flatten().copied()),
            34962,
            serde_json::json!({"componentType": 5126, "count": count, "type": "VEC2"}),
        );
        attributes.insert(key.into(), at.into());
    }
    let indices = push(
        &mut bin,
        mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
        34963,
        serde_json::json!({"componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR"}),
    );
    while bin.len() % 4 != 0 {
        bin.push(0);
    }

    let json = serde_json::json!({
        "asset": {"version": "2.0", "generator": "Effect Forge mesh export"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": name, "mesh": 0}],
        "meshes": [{"name": name, "primitives": [{"attributes": attributes, "indices": indices, "mode": 4}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
    });
    let mut json_bytes = serde_json::to_vec(&json)?;
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    Ok(out)
}

/// Read every triangle mesh of a glTF scene into one [`MeshData`], node transforms applied.
///
/// Meshes are merged because an effect primitive is one shape: a Blender scene with a ring and
/// a disc both meant for this primitive should arrive as both, not as whichever came first.
pub fn from_gltf(bytes: &[u8]) -> Result<MeshData> {
    let (document, buffers, _) = gltf::import_slice(bytes).context("reading the glTF")?;
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or_else(|| anyhow!("the glTF has no scene"))?;
    let mut out = MeshData::default();
    let mut any_color = false;
    let mut any_uv1 = false;

    fn walk(
        node: gltf::Node,
        parent: glam::Mat4,
        buffers: &[gltf::buffer::Data],
        out: &mut MeshData,
        any_color: &mut bool,
        any_uv1: &mut bool,
    ) -> Result<()> {
        let world = parent * glam::Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            let normal_matrix = glam::Mat3::from_mat4(world).inverse().transpose();
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let Some(positions) = reader.read_positions() else {
                    continue;
                };
                let base = out.positions.len() as u32;
                let positions: Vec<[f32; 3]> = positions
                    .map(|p| world.transform_point3(glam::Vec3::from(p)).to_array())
                    .collect();
                let n = positions.len();
                out.positions.extend(positions);
                match reader.read_normals() {
                    Some(normals) => out.normals.extend(
                        normals.map(|v| (normal_matrix * glam::Vec3::from(v)).normalize_or_zero().to_array()),
                    ),
                    None => out.normals.extend(std::iter::repeat([0.0, 1.0, 0.0]).take(n)),
                }
                match reader.read_colors(0) {
                    Some(colors) => {
                        *any_color = true;
                        out.colors.extend(colors.into_rgba_f32());
                    }
                    None => out.colors.extend(std::iter::repeat([1.0; 4]).take(n)),
                }
                match reader.read_tex_coords(0) {
                    Some(uvs) => out.uv0.extend(uvs.into_f32()),
                    None => out.uv0.extend(std::iter::repeat([0.0; 2]).take(n)),
                }
                match reader.read_tex_coords(1) {
                    Some(uvs) => {
                        *any_uv1 = true;
                        out.uv1.extend(uvs.into_f32());
                    }
                    None => {
                        let start = out.uv0.len() - n;
                        let copy: Vec<[f32; 2]> = out.uv0[start..].to_vec();
                        out.uv1.extend(copy);
                    }
                }
                match reader.read_indices() {
                    Some(indices) => out.indices.extend(indices.into_u32().map(|i| i + base)),
                    None => out.indices.extend((0..n as u32).map(|i| i + base)),
                }
            }
        }
        for child in node.children() {
            walk(child, world, buffers, out, any_color, any_uv1)?;
        }
        Ok(())
    }

    for node in scene.nodes() {
        walk(node, glam::Mat4::IDENTITY, &buffers, &mut out, &mut any_color, &mut any_uv1)?;
    }
    if !any_color {
        out.colors.clear();
    }
    if !any_uv1 {
        out.uv1.clear();
    }
    out.check()?;
    Ok(out)
}
