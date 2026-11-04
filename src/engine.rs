//! An open effect file and the edits waiting to go into it. No UI here.
//!
//! Every change is applied to a working copy of the built file, so changing one effect costs one
//! load and save of that file (about a second) rather than a whole bake.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::eff::fields::{self, AttrValue};
use crate::eff::mesh_io::{self, MeshData};
use crate::eff::model::{load_effect, EmitterDef, LoadedEffect};
use crate::eff::edits::{AuthoredEdit, EffMod, EmitterFieldEdits, TextureAddition, TransplantOp};

/// The template a new texture takes its format and size from. It keeps colour; the 512 template
/// (ef_cmn_impactflash00) keeps only alpha.
const TEXTURE_TEMPLATE: &str = "ef_cmn_impact05_ani";

pub struct Doc {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub loaded: LoadedEffect,
    /// "entry/emitter" -> primitive name.
    pub primitives: HashMap<String, String>,
    /// Primitive id (a name hash) -> name.
    pub primitive_ids: HashMap<u64, String>,
}

/// One emitter's pending attribute changes, by attribute id.
pub type Pending = BTreeMap<(usize, usize), BTreeMap<String, AttrValue>>;

#[derive(Clone, Debug)]
pub struct Target {
    pub entry: String,
    pub set_idx: usize,
    pub set_name: String,
    pub emitter_idx: usize,
    pub emitter_name: String,
}

impl Doc {
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        // Another file of the same name used this working copy, and its shaders are not this file's.
        crate::draw::shaders::global().lock().unwrap().forget(&working_copy(path));
        Self::from_bytes(path.to_path_buf(), bytes)
    }

    fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<Self> {
        let working = working_copy(&path);
        std::fs::write(&working, &bytes)?;
        let loaded = load_effect(&working)?;
        let mut primitives = HashMap::new();
        let mut primitive_ids = HashMap::new();
        for listing in mesh_io::list_primitives(&bytes)? {
            primitive_ids.insert(listing.id, listing.name.clone());
            for user in listing.used_by {
                primitives.insert(user, listing.name.clone());
            }
        }
        Ok(Self { path, bytes, loaded, primitives, primitive_ids })
    }

    pub fn effects(&self) -> Vec<(String, usize)> {
        let mut list: Vec<_> = self.loaded.entries.iter().filter_map(|e| Some((e.name.clone(), e.set_idx?))).collect();
        list.sort();
        list
    }

    pub fn emitters(&self, set_idx: usize) -> &[EmitterDef] {
        &self.loaded.ptcl.emitter_sets[set_idx].emitters
    }

    pub fn primitive_of(&self, entry: &str, emitter: &str) -> Option<&String> {
        self.primitives.get(&format!("{entry}/{emitter}"))
    }

    /// The file the preview reads: the working copy, edits included.
    pub fn working(&self) -> PathBuf {
        working_copy(&self.path)
    }

    pub fn mesh_by_id(&self, id: u64) -> Result<MeshData> {
        self.mesh(self.primitive_ids.get(&id).ok_or_else(|| anyhow!("no primitive {id:#x}"))?)
    }

    pub fn mesh(&self, primitive: &str) -> Result<MeshData> {
        mesh_io::from_gltf(&mesh_io::export_primitive_glb(&self.bytes, primitive)?)
    }

    /// The emitter's texture as drawn into the preview, with the name it has in the pool.
    pub fn texture(&self, emitter: &EmitterDef) -> Option<image::RgbaImage> {
        let index = emitter.texture_index? as usize;
        let info = self.loaded.ptcl.bntx_textures.get(index)?;
        let pool = self.loaded.texture_pool.as_ref()?;
        crate::eff::textures::decode_preview(pool, index, &info.tex_name, crate::eff::textures::Form::Raw, Some(256)).ok()
    }

    pub fn apply_values(&self, pending: &Pending) -> Result<Self> {
        let mut eff = EffMod::default();
        for ((set_idx, emitter_idx), attrs) in pending {
            let set = &self.loaded.ptcl.emitter_sets[*set_idx];
            eff.authored.push(AuthoredEdit {
                set_name: set.name.clone(),
                entry_name: entry_of(self, *set_idx),
                set_idx: *set_idx,
                emitter_name: set.emitters[*emitter_idx].name.clone(),
                emitter_idx: *emitter_idx,
                fields: EmitterFieldEdits { attrs: attrs.clone(), ..Default::default() },
            });
        }
        self.rebuilt(&eff)
    }

    pub fn replace_texture(&self, target: &Target, png: &Path) -> Result<Self> {
        let template = self.loaded.ptcl.bntx_textures.iter().find(|t| t.tex_name == TEXTURE_TEMPLATE)
            .ok_or_else(|| anyhow!("this file has no {TEXTURE_TEMPLATE} texture to shape a new one on"))?;
        // A pool texture must match its template's size.
        let source = image::open(png).with_context(|| format!("reading {}", png.display()))?;
        let fitted = source.resize_exact(template.width, template.height, image::imageops::FilterType::Triangle).to_rgba8();
        let name = format!("forge_{}_{}", clean(&target.entry), clean(&target.emitter_name));
        let name = crate::eff::textures::unique_texture_name(
            &self.loaded.ptcl.bntx_textures.iter().map(|t| t.tex_name.clone()).collect::<Vec<_>>(), &name);
        let fitted_path = working_copy(&self.path).with_extension("png");
        fitted.save(&fitted_path)?;
        let eff = EffMod {
            textures_added: vec![TextureAddition {
                texture_name: name.clone(),
                template_name: TEXTURE_TEMPLATE.to_string(),
                png_path: fitted_path.display().to_string(),
                raw: false,
            }],
            authored: vec![AuthoredEdit {
                set_name: target.set_name.clone(),
                entry_name: target.entry.clone(),
                set_idx: target.set_idx,
                emitter_name: target.emitter_name.clone(),
                emitter_idx: target.emitter_idx,
                fields: EmitterFieldEdits { texture_name: Some(name), ..Default::default() },
            }],
            ..Default::default()
        };
        self.rebuilt(&eff)
    }

    /// Give the emitter its own copy of `mesh`, under a primitive of its own.
    pub fn replace_mesh(&self, target: &Target, mesh: &MeshData) -> Result<Self> {
        let template = self.primitive_of(&target.entry, &target.emitter_name)
            .ok_or_else(|| anyhow!("{} draws no model: it is a sprite emitter", target.emitter_name))?.clone();
        let taken: Vec<String> = mesh_io::list_primitives(&self.bytes)?.into_iter().map(|p| p.name).collect();
        let stem = format!("forge_{}_{}", clean(&target.entry), clean(&target.emitter_name));
        let name = (0..).map(|i| format!("{stem}_{i}")).find(|n| !taken.contains(n)).unwrap();
        let bytes = mesh_io::add_primitive_for_emitters(&self.bytes, &template, &name, mesh,
            &[(target.entry.clone(), target.emitter_name.clone())])?;
        Self::from_bytes(self.path.clone(), bytes)
    }

    /// A copy of an effect under a new name, to work on without touching the original.
    pub fn duplicate(&self, entry: &str, set_idx: usize, new_name: &str) -> Result<Self> {
        let eff = EffMod {
            transplants: vec![TransplantOp {
                new_entry_name: new_name.to_string(),
                src_file_rel: String::new(),
                src_set_name: entry.to_string(),
                src_set_idx: set_idx,
                one_slot_slots: Vec::new(),
                replace_entry: None,
            }],
            ..Default::default()
        };
        self.rebuilt(&eff)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let backup = self.path.with_extension("eff.forge-backup");
        if !backup.exists() && self.path.exists() {
            std::fs::copy(&self.path, &backup)?;
        }
        std::fs::write(&self.path, &self.bytes)?;
        Ok(backup)
    }

    fn rebuilt(&self, eff: &EffMod) -> Result<Self> {
        let bytes = crate::eff::writer::rebuild_eff_bytes_for_slot(&self.bytes, eff, None, None)?;
        Self::from_bytes(self.path.clone(), bytes)
    }
}

/// Effects tuned by hand, kept in a file of their own so a rebuild of the generated file can't overwrite them. The
/// build puts each one back into the generated file in place of the version it made ([`merge_tuned`]).
///
/// The file lives at `<repo>/effects_tuned/<name>_tuned.eff` for a built `<name>.eff` inside a git repo, and its list
/// beside it as `<name>_tuned.json`: the file also carries every effect that was in the built file when it was made,
/// and only the listed ones are put back.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Tuned {
    #[serde(skip)]
    pub path: PathBuf,
    /// The built file these go back into.
    pub into: Option<PathBuf>,
    pub effects: Vec<String>,
}

impl Tuned {
    /// The tuned file for `path`: itself when it is one, else its repo's `effects_tuned` copy.
    pub fn path_for(path: &Path) -> Option<PathBuf> {
        let stem = path.file_stem()?.to_str()?;
        if stem.ends_with("_tuned") {
            return Some(path.to_path_buf());
        }
        let repo = path.ancestors().find(|dir| dir.join(".git").exists())?;
        Some(repo.join("effects_tuned").join(format!("{stem}_tuned.eff")))
    }

    pub fn is_tuned_file(path: &Path) -> bool {
        path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.ends_with("_tuned"))
    }

    pub fn load(path: &Path) -> Self {
        let mut tuned: Tuned = std::fs::read_to_string(path.with_extension("json")).ok()
            .and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        tuned.path = path.to_path_buf();
        tuned
    }

    pub fn save(&self) -> Result<()> {
        std::fs::write(self.path.with_extension("json"), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn has(&self, entry: &str) -> bool {
        self.effects.iter().any(|e| e.eq_ignore_ascii_case(entry))
    }
}

impl Doc {
    /// Copy `entry`, as it is now, into the tuned file, and list it there.
    pub fn stage(&self, entry: &str, set_idx: usize, tuned_path: &Path) -> Result<Tuned> {
        let mut tuned = Tuned::load(tuned_path);
        if tuned_path.exists() {
            let bytes = std::fs::read(tuned_path)?;
            let working = self.working();
            let op = TransplantOp {
                new_entry_name: entry.to_string(),
                src_file_rel: working.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string(),
                src_set_name: entry.to_string(),
                src_set_idx: set_idx,
                one_slot_slots: Vec::new(),
                replace_entry: entry_names(&bytes)?.iter().any(|n| n.eq_ignore_ascii_case(entry)).then(|| entry.to_string()),
            };
            let eff = EffMod { transplants: vec![op], ..Default::default() };
            let staged = crate::eff::writer::rebuild_eff_bytes_for_slot(&bytes, &eff, working.parent(), None)?;
            std::fs::write(tuned_path, staged)?;
        } else {
            // The first one: the whole file, which every later one replaces an effect of.
            std::fs::create_dir_all(tuned_path.parent().ok_or_else(|| anyhow!("no folder for {}", tuned_path.display()))?)?;
            std::fs::write(tuned_path, &self.bytes)?;
        }
        if !Tuned::is_tuned_file(&self.path) {
            tuned.into = Some(self.path.clone());
        }
        if !tuned.has(entry) {
            tuned.effects.push(entry.to_string());
            tuned.effects.sort();
        }
        tuned.save()?;
        Ok(tuned)
    }
}

fn entry_names(bytes: &[u8]) -> Result<Vec<String>> {
    Ok(effect_library::NamcoEffectFile::load(bytes).context("reading the effect file")?.entry_names)
}

/// Put every listed effect of the tuned file into `into`, each in place of the effect of the same name (or added, if
/// it has none). Returns the new bytes and how many went in.
pub fn merge_tuned(tuned_path: &Path, into: &[u8]) -> Result<(Vec<u8>, usize)> {
    let tuned = Tuned::load(tuned_path);
    let donor = std::fs::read(tuned_path).with_context(|| format!("reading {}", tuned_path.display()))?;
    let donor_names = entry_names(&donor)?;
    let names = entry_names(into)?;
    let mut eff = EffMod::default();
    for entry in &tuned.effects {
        let Some(set_idx) = donor_names.iter().position(|n| n.eq_ignore_ascii_case(entry)) else {
            crate::log::note(&format!("tuned list names {entry}, which the tuned file doesn't have"));
            continue;
        };
        eff.transplants.push(TransplantOp {
            new_entry_name: entry.clone(),
            src_file_rel: tuned_path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string(),
            src_set_name: entry.clone(),
            src_set_idx: set_idx,
            one_slot_slots: Vec::new(),
            replace_entry: names.iter().any(|n| n.eq_ignore_ascii_case(entry)).then(|| entry.clone()),
        });
    }
    let count = eff.transplants.len();
    if count == 0 {
        return Ok((into.to_vec(), 0));
    }
    Ok((crate::eff::writer::rebuild_eff_bytes_for_slot(into, &eff, tuned_path.parent(), None)?, count))
}

fn entry_of(doc: &Doc, set_idx: usize) -> String {
    doc.loaded.entries.iter().find(|e| e.set_idx == Some(set_idx)).map(|e| e.name.clone()).unwrap_or_default()
}

fn clean(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect()
}

fn working_copy(path: &Path) -> PathBuf {
    let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("effect");
    let dir = std::env::temp_dir().join("effect_forge");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{name}.working.eff"))
}

/// Turn a mesh a quarter turn at a time about an axis (0 X, 1 Y, 2 Z). A rotation keeps the winding.
pub fn quarter_turn(mesh: &mut MeshData, axis: usize) {
    let turn = |p: &mut [f32; 3]| {
        let [x, y, z] = *p;
        *p = match axis {
            0 => [x, -z, y],
            1 => [z, y, -x],
            _ => [-y, x, z],
        };
    };
    mesh.positions.iter_mut().for_each(turn);
    mesh.normals.iter_mut().for_each(turn);
}

/// Mirror a mesh across one axis (0 X, 1 Y, 2 Z). A mirror turns faces inside out, so the winding is reversed.
pub fn mirror(mesh: &mut MeshData, axis: usize) {
    for p in mesh.positions.iter_mut().chain(mesh.normals.iter_mut()) {
        p[axis] = -p[axis];
    }
    for face in mesh.indices.chunks_exact_mut(3) {
        face.swap(1, 2);
    }
}

/// A layer elsewhere in the file whose shader can stand in for another's.
#[derive(Clone, Debug)]
pub struct Donor {
    pub effect: String,
    pub layer: String,
    pub shader_index: f32,
}

/// Fields the shader translator bakes in (or may: only billboard and alpha0 type are proven), so a layer can only
/// change them by drawing with another layer's shader. Donors are ranked by how many of these they share.
pub const BAKED_MODES: &[&str] = &[
    "particle_data.billboard_type", "particle_color.alpha0_type", "particle_color.alpha1_type",
    "particle_color.color0_type", "particle_color.color1_type", "particle_color.is_soft_particle",
    "particle_color.is_fresnel_alpha", "particle_color.is_near_dist_alpha", "particle_color.is_far_dist_alpha",
    "particle_fluctuation.is_apply_alpha", "particle_fluctuation.is_applay_scale", "particle_fluctuation.is_applay_scale_y",
    "particle_fluctuation.is_wave_type", "texture_anim0.pattern_anim_type", "texture_anim0.is_scroll",
    "texture_anim0.is_rotate", "texture_anim0.is_scale", "texture_anim0.is_crossfade", "texture_anim1.pattern_anim_type",
    "texture_anim1.is_scroll", "texture_anim1.is_rotate", "texture_anim1.is_scale",
];

impl Doc {
    /// What a layer's shader is compiled for besides its modes: the texture combiner, mesh or sprite, and a second
    /// texture. Two layers that agree here draw alike, so one can use the other's shader.
    fn look(&self, entry: &str, em: &EmitterDef) -> (Vec<(usize, i64)>, bool, bool) {
        let combiner = fields::table().iter().enumerate()
            .filter(|(_, a)| a.id.starts_with("combiner."))
            .filter_map(|(i, _)| Some((i, em.attrs.get(i).copied()??.as_i64())))
            .collect();
        (combiner, self.primitive_of(entry, &em.name).is_some(), em.texture_index1.is_some())
    }

    /// Another layer in this file that looks like `em` and whose shader is compiled with every mode in `want`.
    /// `current` reads the layer's values as edited, so the best donor also keeps the layer's other modes.
    pub fn shader_donor(&self, entry: &str, em: &EmitterDef, want: &[(&str, f32)], current: impl Fn(&str) -> Option<f32>) -> Option<Donor> {
        let look = self.look(entry, em);
        let mut best: Option<(usize, Donor)> = None;
        for (effect, set_idx) in self.effects() {
            for donor in self.emitters(set_idx) {
                let value = |id: &str| attr(donor, id).map(|v| v.as_f32());
                if !want.iter().all(|(id, v)| value(id) == Some(*v)) || self.look(&effect, donor) != look {
                    continue;
                }
                let Some(shader_index) = value("shader_references.shader_index") else { continue };
                let shared = BAKED_MODES.iter()
                    .filter(|id| !want.iter().any(|(w, _)| w == *id))
                    .filter(|id| value(id) == current(id))
                    .count();
                if best.as_ref().map_or(true, |(score, _)| shared > *score) {
                    best = Some((shared, Donor { effect: effect.clone(), layer: donor.name.clone(), shader_index }));
                }
            }
        }
        best.map(|(_, d)| d)
    }
}

pub fn attr(emitter: &EmitterDef, id: &str) -> Option<AttrValue> {
    *emitter.attrs.get(fields::index_of(id)?)?
}

/// `value` in the type `like` has.
fn typed(like: AttrValue, value: f32) -> AttrValue {
    match like {
        AttrValue::Float(_) => AttrValue::Float(value),
        AttrValue::Int(_) => AttrValue::Int(value as i64),
        AttrValue::UInt(_) => AttrValue::UInt(value as u64),
    }
}

impl Doc {
    /// The effect of that name (any case): its entry name and set.
    pub fn find(&self, effect: &str) -> Result<(String, usize)> {
        self.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(effect)).ok_or_else(|| anyhow!("no effect {effect}"))
    }

    /// One value on every layer of an effect (`effect_forge set`). `value` is read as the field's own type.
    pub fn set_on_effect(&self, effect: &str, id: &str, value: &str) -> Result<Self> {
        let (_, set) = self.find(effect)?;
        let mut pending = Pending::new();
        for (index, emitter) in self.emitters(set).iter().enumerate() {
            let new = match attr(emitter, id).ok_or_else(|| anyhow!("no field {id} on {}", emitter.name))? {
                AttrValue::Float(_) => AttrValue::Float(value.parse()?),
                AttrValue::Int(_) => AttrValue::Int(value.parse()?),
                AttrValue::UInt(_) => AttrValue::UInt(value.parse()?),
            };
            pending.entry((set, index)).or_default().insert(id.to_string(), new);
        }
        self.apply_values(&pending)
    }

    /// Every layer of an effect drawn with a shader built for `id = value`, borrowed as the panel does
    /// (`effect_forge borrow`). Also says, per layer, whose shader it took.
    pub fn borrow_on_effect(&self, effect: &str, id: &str, value: f32) -> Result<(Self, Vec<String>)> {
        let (entry, set) = self.find(effect)?;
        let mut pending = Pending::new();
        let mut report = Vec::new();
        for (index, emitter) in self.emitters(set).iter().enumerate() {
            let current = |id: &str| attr(emitter, id).map(|v| v.as_f32());
            if current(id) == Some(value) {
                report.push(format!("{}: already", emitter.name));
                continue;
            }
            match self.shader_donor(&entry, emitter, &[(id, value)], current) {
                Some(d) => {
                    report.push(format!("{}: shader {} from {} / {}", emitter.name, d.shader_index, d.effect, d.layer));
                    let row = pending.entry((set, index)).or_default();
                    for (key, v) in [("shader_references.shader_index", d.shader_index), (id, value)] {
                        let like = attr(emitter, key).ok_or_else(|| anyhow!("no field {key} on {}", emitter.name))?;
                        row.insert(key.to_string(), typed(like, v));
                    }
                }
                None => report.push(format!("{}: no donor", emitter.name)),
            }
        }
        Ok((self.apply_values(&pending)?, report))
    }
}
