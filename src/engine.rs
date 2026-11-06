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
use crate::eff::edits::{AuthoredEdit, EffMod, EmitterFieldEdits, EmitterRoster, EmitterSlot, TextureAddition, TransplantOp};

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
    /// What each shader variation is, read from the shader binary the first time it is asked for.
    variations: std::sync::OnceLock<Vec<Variation>>,
}

/// The vertex attributes that carry the emitter's matrix at each particle's birth. A layer whose particles stay where
/// they were born (follow type 1 or 2) needs a shader that reads them: one that doesn't places every particle on the
/// emitter where it is now, so they ride along with it and leave no trail. Smash's own files never break this rule.
pub const BIRTH_MATRIX: [&str; 3] = ["sysEmtMat0Attr", "sysEmtMat1Attr", "sysEmtMat2Attr"];

/// One shader variation of a file, as far as finding a stand-in for it goes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Variation {
    /// Its vertex stage reads the birth matrix.
    pub reads_birth: bool,
    /// A hash of the fragment stage's code: equal means it colours the same.
    pub fragment: u64,
    /// The other vertex attributes it reads, sorted.
    pub inputs: Vec<String>,
}

/// Every shader variation of an .eff, by `shader_references.shader_index`.
pub fn variations(bytes: &[u8]) -> Result<Vec<Variation>> {
    let namco = effect_library::NamcoEffectFile::load(bytes).context("reading the effect file")?;
    let Some(grsc) = namco.ptcl_file.as_ref().and_then(|p| p.shader_info.as_ref()).and_then(|s| s.binary_data.as_ref()) else {
        return Ok(Vec::new());
    };
    let bnsh = effect_library::bnsh::BnshFile::read(grsc).context("reading the shader binary")?;
    let hash = |bytes: &[u8]| bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    Ok(bnsh.variations.iter().map(|v| {
        let p = &v.binary_program;
        let names: Vec<String> = p.reflections[0].as_ref().map(|r| r.input_slots().into_iter().map(|(n, _)| n).collect()).unwrap_or_default();
        let mut inputs: Vec<String> = names.iter().filter(|n| !BIRTH_MATRIX.contains(&n.as_str())).cloned().collect();
        inputs.sort();
        Variation {
            reads_birth: names.iter().any(|n| n == BIRTH_MATRIX[0]),
            fragment: p.stages[4].as_ref().map_or(0, |f| hash(&f.byte_code)),
            inputs,
        }
    }).collect())
}

/// What a shader is compiled for on a layer's side: the modes baked into it, the texture combiner, a second texture,
/// and how the layer follows its bone. Two layers that agree here, on shaders that colour the same and read the same
/// inputs, draw the same.
pub(crate) fn shader_key(em: &EmitterDef, current: &dyn Fn(&str) -> Option<f32>) -> Vec<i64> {
    let combiner = fields::table().iter().filter(|a| a.id.starts_with("combiner.")).map(|a| a.id);
    BAKED_MODES.iter().copied().chain(combiner).chain(["emitter_info.follow_type"])
        .map(|id| current(id).map_or(i64::MIN, |v| v as i64))
        .chain([em.texture_index1.is_some() as i64])
        .collect()
}

/// The field behind each place of [`shader_key`] (the last, a second texture, has none).
fn shader_key_ids() -> Vec<&'static str> {
    let combiner = fields::table().iter().filter(|a| a.id.starts_with("combiner.")).map(|a| a.id);
    BAKED_MODES.iter().copied().chain(combiner).chain(["emitter_info.follow_type", ""]).collect()
}

/// A way of drawing that is compiled into a layer's shader, so switching it means drawing with another shader: one
/// built the same way in every other respect.
pub struct Style {
    pub label: &'static str,
    pub hover: &'static str,
    /// The field that says so, and its value when on (off is 0).
    pub field: &'static str,
    pub on: f32,
    /// Only for layers with a model.
    pub models_only: bool,
    /// Fields that tune the style, taken from the layer the shader comes from when it is switched on.
    pub params: &'static [&'static str],
}

pub const STYLES: &[Style] = &[
    Style {
        label: "Soft edges",
        hover: "Fades out where it cuts into the stage or a fighter instead of leaving a hard line, as smoke and glows do.",
        field: "particle_color.is_soft_particle",
        on: 2.0,
        models_only: false,
        params: &["emitter_static.soft_edge_param1", "emitter_static.soft_edge_param2", "emitter_static.soft_partcile_dist", "emitter_static.soft_particle_volume"],
    },
    Style {
        label: "Rim fade",
        hover: "The model fades where its surface turns away from the camera, so its outline goes soft (shields, auras).",
        field: "particle_color.is_fresnel_alpha",
        on: 1.0,
        models_only: true,
        params: &["emitter_static.fresnel_alpha_param1", "emitter_static.fresnel_alpha_param2"],
    },
    Style {
        label: "Heat haze",
        hover: "Bends what is behind it instead of drawing colour; the texture says where. The preview can't show it: check it in game.",
        field: "combiner.shader_type",
        on: 1.0,
        models_only: false,
        params: &["emitter_static.coefficient0", "emitter_static.coefficient1"],
    },
];

/// Vertex inputs a shader reads from the layer's model, and the model attribute behind each.
const MODEL_INPUTS: &[(&str, &str)] = &[("sysNormalAttr", "_n0"), ("sysVertexColor0Attr", "_c0")];

/// Where [`Doc::restyle`] found the shader, and the compiled-in fields the layer took with it.
#[derive(Clone, Debug)]
pub struct Restyled {
    pub from: String,
    pub taken: Vec<String>,
}

/// A layer of one of the game's own files whose shader is wanted: its file under the game folder, the effect that
/// plays it, and its name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShaderDonor {
    pub file: String,
    pub effect: String,
    pub layer: String,
}

type DonorKey = (Variation, Vec<i64>);

/// Read the game's shaders in now (in the background) so the first Draw style switch doesn't wait for them.
pub fn prewarm_game_shaders() {
    if let Some(root) = game_root() {
        std::thread::spawn(move || {
            game_shaders(&root);
        });
    }
}

/// The game's fighter and common effect files, read once: for each kind of shader and layer, one layer that has it.
fn game_shaders(root: &Path) -> &'static HashMap<DonorKey, ShaderDonor> {
    static INDEX: std::sync::OnceLock<HashMap<DonorKey, ShaderDonor>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        let started = std::time::Instant::now();
        let mut files = Vec::new();
        for dir in ["effect/fighter", "effect/system"] {
            for folder in std::fs::read_dir(root.join(dir)).into_iter().flatten().flatten() {
                for file in std::fs::read_dir(folder.path()).into_iter().flatten().flatten() {
                    if file.path().extension().is_some_and(|e| e == "eff") {
                        files.push(file.path());
                    }
                }
            }
        }
        files.sort();
        let mut index = HashMap::new();
        for path in &files {
            let Ok(bytes) = std::fs::read(path) else { continue };
            let (Ok(vars), Ok(loaded)) = (variations(&bytes), load_effect(path)) else { continue };
            let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
            // transplants find a donor by the effect that plays it
            for entry in &loaded.entries {
                let Some(set) = entry.set_idx.and_then(|i| loaded.ptcl.emitter_sets.get(i)) else { continue };
                for em in &set.emitters {
                    let current = |id: &str| attr(em, id).map(|v| v.as_f32());
                    let Some(v) = current("shader_references.shader_index").and_then(|v| vars.get(v as usize)) else { continue };
                    index.entry((v.clone(), shader_key(em, &current)))
                        .or_insert_with(|| ShaderDonor { file: rel.clone(), effect: entry.name.clone(), layer: em.name.clone() });
                }
            }
        }
        crate::log::note(&format!("read the shaders of {} game files in {:.1}s", files.len(), started.elapsed().as_secs_f32()));
        index
    })
}

/// A layer whose shader doesn't match how it follows its bone.
#[derive(Clone, Debug)]
pub struct TrailMismatch {
    pub effect: String,
    pub set_idx: usize,
    pub layer: usize,
    pub name: String,
    pub follow_type: i64,
    pub shader: u32,
    /// Where a shader that matches comes from, if one was found.
    pub fix: Option<String>,
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
        Ok(Self { path, bytes, loaded, primitives, primitive_ids, variations: Default::default() })
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
        self.rebuilt(&self.values_mod(pending))
    }

    /// The edits that put `pending` in.
    fn values_mod(&self, pending: &Pending) -> EffMod {
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
        eff
    }

    pub fn replace_texture(&self, target: &Target, png: &Path) -> Result<Self> {
        Ok(self.replace_texture_from(target, png, TextureFrom::default())?.0)
    }

    /// The layer's texture swapped for a picture made into one as `how` says, with notes on a shape that won't read
    /// (no transparency anywhere, or nothing at all).
    pub fn replace_texture_from(&self, target: &Target, png: &Path, how: TextureFrom) -> Result<(Self, Vec<String>)> {
        // the usual template, else the texture the layer has now
        let own = self.loaded.ptcl.emitter_sets.get(target.set_idx).and_then(|s| s.emitters.get(target.emitter_idx))
            .and_then(|e| e.texture_index).and_then(|i| self.loaded.ptcl.bntx_textures.get(i as usize));
        let template = self.loaded.ptcl.bntx_textures.iter().find(|t| t.tex_name == TEXTURE_TEMPLATE)
            .or(own.filter(|t| t.width > 0 && t.height > 0))
            .ok_or_else(|| anyhow!("this file has no {TEXTURE_TEMPLATE} texture to shape a new one on"))?;
        // A pool texture must match its template's size.
        let source = image::open(png).with_context(|| format!("reading {}", png.display()))?;
        let (shaped, notes) = texture_pixels(source.to_rgba8(), how);
        let fitted = image::DynamicImage::ImageRgba8(shaped)
            .resize_exact(template.width, template.height, image::imageops::FilterType::Triangle).to_rgba8();
        let name = format!("forge_{}_{}", clean(&target.entry), clean(&target.emitter_name));
        let name = crate::eff::textures::unique_texture_name(
            &self.loaded.ptcl.bntx_textures.iter().map(|t| t.tex_name.clone()).collect::<Vec<_>>(), &name);
        let fitted_path = working_copy(&self.path).with_extension("png");
        fitted.save(&fitted_path)?;
        let eff = EffMod {
            textures_added: vec![TextureAddition {
                texture_name: name.clone(),
                template_name: template.tex_name.clone(),
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
        Ok((self.rebuilt(&eff)?, notes))
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
                replace_entry: None,
                ..Default::default()
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

    /// A new effect, `name`, made of a starter piece's layers.
    pub fn new_effect(&self, name: &str, piece: &Piece, model: Option<&MeshData>) -> Result<Self> {
        let name = check_effect_name(name)?;
        if self.loaded.entries.iter().any(|e| e.name.eq_ignore_ascii_case(name)) {
            return Err(anyhow!("this file already has an effect called {name}"));
        }
        let made = self.from_piece(piece, TransplantOp { new_entry_name: name.to_string(), ..Default::default() }, game_root())?;
        made.finish_piece(piece, name, 0, model)
    }

    /// A starter piece's layers added to `entry`, after the ones it has. `model` is the model of a blank model piece.
    pub fn add_layer(&self, entry: &str, piece: &Piece, model: Option<&MeshData>) -> Result<Self> {
        let (_, set_idx) = self.find(entry)?;
        let before = self.emitters(set_idx).len();
        let made = self.from_piece(piece, TransplantOp { into_entry: Some(entry.to_string()), ..Default::default() }, game_root())?;
        made.finish_piece(piece, entry, before, model)
    }

    /// The piece's values on its layers (those of `entry` from `first` on), then, for a blank, the plain texture
    /// and the model.
    fn finish_piece(self, piece: &Piece, entry: &str, first: usize, model: Option<&MeshData>) -> Result<Self> {
        let (entry, set_idx) = self.find(entry)?;
        let count = self.emitters(set_idx).len();
        let mut doc = self;
        if !piece.set.is_empty() {
            let mut pending = Pending::new();
            for layer in first..count {
                let em = &doc.emitters(set_idx)[layer];
                for (id, &v) in &piece.set {
                    if let Some(like) = attr(em, id) {
                        pending.entry((set_idx, layer)).or_default().insert(id.clone(), typed(like, v));
                    }
                }
            }
            if !pending.is_empty() {
                doc = doc.apply_values(&pending)?;
            }
        }
        if piece.blank.is_some() {
            let png = working_copy(&doc.path).with_file_name(if piece.is_model() { "blank_model.png" } else { "blank_sprite.png" });
            blank_texture(piece.is_model()).save(&png).with_context(|| format!("writing {}", png.display()))?;
            for layer in first..count {
                let target = doc.target_of(&entry, set_idx, layer);
                doc = doc.replace_texture(&target, &png)?;
                if let Some(mesh) = model {
                    doc = doc.replace_mesh(&doc.target_of(&entry, set_idx, layer), mesh)?;
                }
            }
        }
        Ok(doc)
    }

    /// Layer `layer` of effect `entry` as an edit names it.
    pub fn target_of(&self, entry: &str, set_idx: usize, layer: usize) -> Target {
        let set = &self.loaded.ptcl.emitter_sets[set_idx];
        Target {
            entry: entry.to_string(), set_idx, set_name: set.name.clone(), emitter_idx: layer,
            emitter_name: set.emitters[layer].name.clone(),
        }
    }

    /// `op` with the piece as its donor, read from under `root`.
    pub(crate) fn from_piece(&self, piece: &Piece, op: TransplantOp, root: Option<PathBuf>) -> Result<Self> {
        let root = root.ok_or_else(|| anyhow!("choose the game files folder first: the kit is read from the game's own effects"))?;
        let op = TransplantOp {
            src_file_rel: piece.file.clone(),
            src_set_name: piece.effect.clone(),
            only_emitters: piece.layers.clone(),
            ..op
        };
        self.rebuilt_from(&EffMod { transplants: vec![op], ..Default::default() }, Some(&root))
    }

    /// The layers under layer `emitter_idx`, not counting it: the ones after it that sit deeper.
    fn layer_end(emitters: &[EmitterDef], emitter_idx: usize) -> usize {
        let depth = emitters[emitter_idx].depth;
        emitters[emitter_idx + 1..].iter().position(|e| e.depth <= depth).map_or(emitters.len(), |n| emitter_idx + 1 + n)
    }

    /// The effect with a copy of layer `emitter_idx` (and the layers under it) placed just after them. The copies are
    /// named `<name>_2` (then `_3`, ...), since values are kept by layer name. Returns the file and where the
    /// copy is.
    pub fn duplicate_layer(&self, entry: &str, set_idx: usize, emitter_idx: usize) -> Result<(Self, usize)> {
        let emitters = self.emitters(set_idx);
        emitters.get(emitter_idx).ok_or_else(|| anyhow!("no layer {emitter_idx}"))?;
        let end = Self::layer_end(emitters, emitter_idx);
        let slot = |i: usize, name: String| {
            let e = &emitters[i];
            EmitterSlot { source_idx: i, source_name: e.name.clone(), name, depth: e.depth, source_set: String::new() }
        };
        let mut taken: Vec<String> = emitters.iter().map(|e| e.name.to_ascii_lowercase()).collect();
        let mut slots: Vec<EmitterSlot> = (0..end).map(|i| slot(i, emitters[i].name.clone())).collect();
        for i in emitter_idx..end {
            let name = copy_name(&emitters[i].name, &taken);
            taken.push(name.to_ascii_lowercase());
            slots.push(slot(i, name));
        }
        slots.extend((end..emitters.len()).map(|i| slot(i, emitters[i].name.clone())));
        let roster = EmitterRoster {
            set_name: self.loaded.ptcl.emitter_sets[set_idx].name.clone(),
            entry_name: entry.to_string(),
            set_idx,
            slots,
        };
        Ok((self.rebuilt(&EffMod { rosters: vec![roster], ..Default::default() })?, end))
    }

    /// The effect with layer `emitter_idx` called `new`: a name no other layer of the effect has.
    pub fn rename_layer(&self, entry: &str, set_idx: usize, emitter_idx: usize, new: &str) -> Result<Self> {
        let new = check_layer_name(new)?;
        let emitters = self.emitters(set_idx);
        let old = &emitters.get(emitter_idx).ok_or_else(|| anyhow!("{entry} has layers 1 to {}", emitters.len()))?.name;
        if old == new {
            return Err(anyhow!("that is already its name"));
        }
        if emitters.iter().enumerate().any(|(i, e)| i != emitter_idx && e.name.eq_ignore_ascii_case(new)) {
            return Err(anyhow!("{entry} already has a layer called {new}"));
        }
        let slots = emitters.iter().enumerate().map(|(i, e)| EmitterSlot {
            source_idx: i, source_name: e.name.clone(), name: if i == emitter_idx { new.to_string() } else { e.name.clone() },
            depth: e.depth, source_set: String::new(),
        }).collect();
        let roster = EmitterRoster {
            set_name: self.loaded.ptcl.emitter_sets[set_idx].name.clone(),
            entry_name: entry.to_string(),
            set_idx,
            slots,
        };
        self.rebuilt(&EffMod { rosters: vec![roster], ..Default::default() })
    }

    /// The effect without layer `emitter_idx` (and the layers under it).
    pub fn remove_layer(&self, entry: &str, set_idx: usize, emitter_idx: usize) -> Result<Self> {
        let emitters = self.emitters(set_idx);
        emitters.get(emitter_idx).ok_or_else(|| anyhow!("no layer {emitter_idx}"))?;
        // its children follow it, deeper than it
        let end = Self::layer_end(emitters, emitter_idx);
        if end - emitter_idx == emitters.len() {
            return Err(anyhow!("that is every layer the effect has; an effect needs one"));
        }
        let slots = emitters.iter().enumerate().filter(|(i, _)| !(emitter_idx..end).contains(i))
            .map(|(i, e)| EmitterSlot { source_idx: i, source_name: e.name.clone(), name: e.name.clone(), depth: e.depth, source_set: String::new() })
            .collect();
        let roster = EmitterRoster {
            set_name: self.loaded.ptcl.emitter_sets[set_idx].name.clone(),
            entry_name: entry.to_string(),
            set_idx,
            slots,
        };
        self.rebuilt(&EffMod { rosters: vec![roster], ..Default::default() })
    }

    /// Effects copied in from another file, each under its own name. One this file already has by that name is
    /// replaced when `replace`, else left as it is. Returns the file and the names left as they were.
    pub fn import(&self, from: &Path, effects: &[String], replace: bool) -> Result<(Self, Vec<String>)> {
        let donor = std::fs::read(from).with_context(|| format!("reading {}", from.display()))?;
        let donor_names = entry_names(&donor)?;
        let file = from.file_name().and_then(|n| n.to_str()).ok_or_else(|| anyhow!("no file name in {}", from.display()))?;
        let mut transplants = Vec::new();
        let mut kept = Vec::new();
        for name in effects {
            if !donor_names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                return Err(anyhow!("{} has no effect called {name}", from.display()));
            }
            let here = self.loaded.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).map(|e| e.name.clone());
            if here.is_some() && !replace {
                kept.push(name.clone());
                continue;
            }
            transplants.push(TransplantOp {
                new_entry_name: name.clone(),
                src_file_rel: file.to_string(),
                src_set_name: name.clone(),
                replace_entry: here,
                ..Default::default()
            });
        }
        if transplants.is_empty() {
            return Ok((Self::from_bytes(self.path.clone(), self.bytes.clone())?, kept));
        }
        let doc = self.rebuilt_from(&EffMod { transplants, ..Default::default() }, from.parent())?;
        Ok((doc, kept))
    }

    /// The file with effect `old` renamed `new`.
    pub fn rename(&self, old: &str, new: &str) -> Result<Self> {
        let new = check_effect_name(new)?;
        if old == new {
            return Err(anyhow!("that is already its name"));
        }
        if self.loaded.entries.iter().any(|e| e.name.eq_ignore_ascii_case(new) && !e.name.eq_ignore_ascii_case(old)) {
            return Err(anyhow!("this file already has an effect called {new}"));
        }
        Self::from_bytes(self.path.clone(), crate::eff::writer::rename_entry(&self.bytes, old, new)?)
    }

    fn rebuilt(&self, eff: &EffMod) -> Result<Self> {
        self.rebuilt_from(eff, None)
    }

    /// With the file's place in the game, so an effect copied in gets the type its spawns need, and `root` for donors
    /// from other files.
    fn rebuilt_from(&self, eff: &EffMod, root: Option<&Path>) -> Result<Self> {
        let mut eff = eff.clone();
        if eff.source_rel.is_empty() {
            eff.source_rel = place_in_game(&self.path);
        }
        let bytes = crate::eff::writer::rebuild(&self.bytes, &eff, root)?;
        Self::from_bytes(self.path.clone(), bytes)
    }
}

/// `name` trimmed, if the game can take it as an effect name: letters, digits and `_`.
pub fn check_effect_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("an effect needs a name"));
    }
    if let Some(c) = name.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '_')) {
        return Err(anyhow!("effect names take letters, digits and _ only, not '{c}'"));
    }
    Ok(name)
}

/// A layer name as the game's own are: letters, digits, `_` and `-`, up to 63 of them.
pub fn check_layer_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("a layer needs a name"));
    }
    if let Some(c) = name.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-')) {
        return Err(anyhow!("layer names take letters, digits, _ and - only, not '{c}'"));
    }
    if name.len() > 63 {
        return Err(anyhow!("layer names are at most 63 characters"));
    }
    Ok(name)
}

/// Where a file sits in the game, as `effect/fighter/<name>/<file>.eff`: from its own path, or, for a tuned file, from
/// the built file it goes back into, or, for a project's file, from where the project exports. Empty when none says. The writer gives an effect copied into a fighter's file
/// the fighter type its spawns need.
pub fn place_in_game(path: &Path) -> String {
    let from = |path: &Path| -> Option<String> {
        let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
        let at = parts.iter().rposition(|p| p == "effect")?;
        Some(parts[at..].join("/"))
    };
    from(path)
        .or_else(|| Tuned::is_tuned_file(path).then(|| Tuned::load(path).into).flatten().and_then(|into| from(&into)))
        .or_else(|| {
            let project = crate::project::Project::load(&crate::project::project_of_eff(path)?).ok()?;
            from(project.export_to.as_deref()?)
        })
        .unwrap_or_default()
}

/// One of the game's own layers (or a few that play together) to start an effect from or add to one. `file` is under
/// the game root ([`game_root`]).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Piece {
    pub group: String,
    pub name: String,
    pub blurb: String,
    pub file: String,
    pub effect: String,
    pub layers: Vec<String>,
    /// A blank to build on rather than a look: `sprite` or `model`. Its texture is swapped for a plain white one (and,
    /// for a model, its model for the one given), so nothing of the donor shows.
    #[serde(default)]
    pub blank: Option<String>,
    /// Values set on the piece's layers once they are in: for a blank, what makes it one still, plain particle.
    #[serde(default)]
    pub set: BTreeMap<String, f32>,
}

impl Piece {
    pub fn is_model(&self) -> bool {
        self.blank.as_deref() == Some("model")
    }
}

/// The plain texture a blank starts with: a white disc that softens to its edge for a sprite, flat white for a model
/// (its own shape is the look).
fn blank_texture(model: bool) -> image::RgbaImage {
    image::RgbaImage::from_fn(128, 128, |x, y| {
        if model {
            return image::Rgba([255, 255, 255, 255]);
        }
        let d = (((x as f32 - 63.5).powi(2) + (y as f32 - 63.5).powi(2)).sqrt() / 64.0).min(1.0);
        // the disc is in the colour too: some shaders take the opacity from the red channel
        let a = ((1.0 - d).powf(0.8) * 255.0).round() as u8;
        image::Rgba([a, a, a, a])
    })
}

/// The starter kit, from `kit.json`.
pub fn kit() -> &'static [Piece] {
    static KIT: std::sync::OnceLock<Vec<Piece>> = std::sync::OnceLock::new();
    KIT.get_or_init(|| serde_json::from_str(include_str!("kit.json")).expect("kit.json"))
}

fn game_root_file() -> PathBuf {
    crate::scratch_dirs::app_storage_root().join("game_root.txt")
}

/// The folder holding the game's `effect` folder, which the kit's pieces are read from: `FORGE_GAME_ROOT`, else the one
/// chosen in the window. `None` until there is one.
pub fn game_root() -> Option<PathBuf> {
    let chosen = std::env::var_os("FORGE_GAME_ROOT").map(PathBuf::from)
        .or_else(|| std::fs::read_to_string(game_root_file()).ok().map(|s| PathBuf::from(s.trim())))?;
    holding_effect(&chosen)
}

/// `dir` if it holds an `effect` folder, its parent if it is one.
fn holding_effect(dir: &Path) -> Option<PathBuf> {
    if dir.join("effect").is_dir() {
        Some(dir.to_path_buf())
    } else if dir.file_name().is_some_and(|n| n.eq_ignore_ascii_case("effect")) && dir.is_dir() {
        dir.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// Remember `dir` as the game root. Fails when it holds no `effect` folder.
pub fn set_game_root(dir: &Path) -> Result<PathBuf> {
    let root = holding_effect(dir).ok_or_else(|| anyhow!("{} has no effect folder in it", dir.display()))?;
    std::fs::create_dir_all(crate::scratch_dirs::app_storage_root())?;
    std::fs::write(game_root_file(), root.display().to_string())?;
    Ok(root)
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
                replace_entry: entry_names(&bytes)?.iter().any(|n| n.eq_ignore_ascii_case(entry)).then(|| entry.to_string()),
                ..Default::default()
            };
            let eff = EffMod { source_rel: place_in_game(&self.path), transplants: vec![op], ..Default::default() };
            let staged = crate::eff::writer::rebuild(&bytes, &eff, working.parent())?;
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

/// `name` with `_2` (or `_3`, ...) on the end, as an added layer is named, not one of `taken` (lowercase), and short enough for the file's
/// 63-byte layer names.
fn copy_name(name: &str, taken: &[String]) -> String {
    (2..).map(|n| {
        let tail = format!("_{n}");
        let base: String = name.chars().take_while(|c| c.is_ascii()).take(63 - tail.len()).collect();
        format!("{base}{tail}")
    })
    .find(|n| !taken.contains(&n.to_ascii_lowercase()))
    .expect("a free name")
}

/// The names of the effects in an .eff on disk.
pub fn effect_names(path: &Path) -> Result<Vec<String>> {
    entry_names(&std::fs::read(path).with_context(|| format!("reading {}", path.display()))?)
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
    let mut eff = EffMod { source_rel: place_in_game(tuned_path), ..Default::default() };
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
            replace_entry: names.iter().any(|n| n.eq_ignore_ascii_case(entry)).then(|| entry.clone()),
            ..Default::default()
        });
    }
    let count = eff.transplants.len();
    if count == 0 {
        return Ok((into.to_vec(), 0));
    }
    Ok((crate::eff::writer::rebuild(into, &eff, tuned_path.parent())?, count))
}

fn entry_of(doc: &Doc, set_idx: usize) -> String {
    doc.loaded.entries.iter().find(|e| e.set_idx == Some(set_idx)).map(|e| e.name.clone()).unwrap_or_default()
}

fn clean(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect()
}

fn working_copy(path: &Path) -> PathBuf {
    let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("effect");
    let dir = crate::scratch_dirs::app_storage_root().join("working");
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

/// Grow or shrink a mesh by `factor` about its origin.
pub fn scale_mesh(mesh: &mut MeshData, factor: f32) {
    for p in mesh.positions.iter_mut() {
        *p = p.map(|c| c * factor);
    }
}

/// Move a mesh so the middle of its bounds is at the origin, or, with `on_ground`, so it stands on the origin:
/// centred across X and Z, its lowest point at Y 0.
pub fn centre_mesh(mesh: &mut MeshData, on_ground: bool) {
    let (lo, hi) = mesh.bounds();
    let mut shift: [f32; 3] = std::array::from_fn(|a| (lo[a] + hi[a]) / 2.0);
    if on_ground {
        shift[1] = lo[1];
    }
    for p in mesh.positions.iter_mut() {
        *p = std::array::from_fn(|a| p[a] - shift[a]);
    }
}

/// A mesh's width, height and depth.
pub fn mesh_size(mesh: &MeshData) -> [f32; 3] {
    let (lo, hi) = mesh.bounds();
    std::array::from_fn(|a| hi[a] - lo[a])
}

/// The ways a model can be fitted on its way in, as the command line spells them.
pub const MESH_OPTIONS: &str = "turn-x|turn-y|turn-z=90|180|270, mirror=x|y|z, scale=<factor>, size=<largest side>, centre, ground";

/// Apply one fitting option to a mesh: `turn-y=90`, `mirror=x`, `scale=2`, `size=10` (the largest side becomes 10),
/// `centre` (the middle at the origin) or `ground` (standing on the origin). Options apply in the order given.
pub fn fit_mesh(mesh: &mut MeshData, option: &str) -> Result<()> {
    let axis = |a: &str| ["x", "y", "z"].iter().position(|n| n.eq_ignore_ascii_case(a));
    let number = |v: &str| v.parse::<f32>().ok().filter(|f| f.is_finite() && *f > 0.0);
    let bad = || anyhow!("{option}: a model option is one of {MESH_OPTIONS}");
    match option.split_once('=') {
        Some((turn, degrees)) if turn.starts_with("turn-") => {
            let a = axis(&turn[5..]).ok_or_else(bad)?;
            let quarters = match degrees.trim().parse::<i32>().map_err(|_| bad())?.rem_euclid(360) {
                q @ (0 | 90 | 180 | 270) => q / 90,
                _ => return Err(anyhow!("{option}: models turn by 90, 180 or 270 degrees")),
            };
            (0..quarters).for_each(|_| quarter_turn(mesh, a));
        }
        Some(("mirror", a)) => mirror(mesh, axis(a).ok_or_else(bad)?),
        Some(("scale", f)) => scale_mesh(mesh, number(f).ok_or_else(bad)?),
        Some(("size", f)) => {
            let largest = mesh_size(mesh).into_iter().fold(0.0, f32::max);
            if largest <= 0.0 {
                return Err(anyhow!("{option}: the model has no size to fit"));
            }
            scale_mesh(mesh, number(f).ok_or_else(bad)? / largest);
        }
        None if option.eq_ignore_ascii_case("centre") || option.eq_ignore_ascii_case("center") => centre_mesh(mesh, false),
        None if option.eq_ignore_ascii_case("ground") => centre_mesh(mesh, true),
        _ => return Err(bad()),
    }
    Ok(())
}

/// Fields the shader translator bakes in (or may: only billboard and alpha0 type are proven), so a layer can only
/// change them by drawing with another layer's shader.
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
    /// The file's shader variations (empty if it has no shaders or they can't be read).
    pub fn variations(&self) -> &[Variation] {
        self.variations.get_or_init(|| variations(&self.bytes).unwrap_or_else(|e| {
            crate::log::note(&format!("reading the shader variations failed: {e:#}"));
            Vec::new()
        }))
    }

    /// Whether variation `shader` keeps particles where they were born (reads the birth matrix). None when unknown.
    pub fn shader_reads_birth(&self, shader: u32) -> Option<bool> {
        self.variations().get(shader as usize).map(|v| v.reads_birth)
    }

    /// The variation that layer `em` should draw with given its values (`current`): one that colours the same and reads
    /// the same inputs as its shader now, reads the birth matrix exactly when the layer stays where it is born, and is
    /// used in this file by a layer with the same [`shader_key`]. None when this file has none.
    fn matching_shader(&self, em: &EmitterDef, current: &dyn Fn(&str) -> Option<f32>) -> Option<(u32, DonorKey)> {
        let variations = self.variations();
        let own = variations.get(current("shader_references.shader_index")? as usize)?;
        let want = Variation { reads_birth: current("emitter_info.follow_type")? != 0.0, ..own.clone() };
        let key = shader_key(em, current);
        let found = self.loaded.ptcl.emitter_sets.iter().flat_map(|s| &s.emitters).find_map(|other| {
            let v = attr(other, "shader_references.shader_index")?.as_i64() as u32;
            (variations.get(v as usize) == Some(&want) && shader_key(other, &|id| attr(other, id).map(|a| a.as_f32())) == key).then_some(v)
        });
        Some((found.unwrap_or(u32::MAX), (want, key)))
    }

    /// Every layer whose shader doesn't match its follow type (of effect `only`, or all).
    pub fn trail_mismatches(&self, only: Option<&str>) -> Vec<TrailMismatch> {
        let mut out = Vec::new();
        for (effect, set_idx) in self.effects() {
            if only.is_some_and(|o| !o.eq_ignore_ascii_case(&effect)) {
                continue;
            }
            for (layer, em) in self.emitters(set_idx).iter().enumerate() {
                let current = |id: &str| attr(em, id).map(|v| v.as_f32());
                let (Some(follow), Some(shader)) = (current("emitter_info.follow_type"), current("shader_references.shader_index")) else { continue };
                if self.shader_reads_birth(shader as u32).is_none_or(|reads| reads == (follow != 0.0)) {
                    continue;
                }
                out.push(TrailMismatch { effect: effect.clone(), set_idx, layer, name: em.name.clone(), follow_type: follow as i64, shader: shader as u32, fix: None });
            }
        }
        out
    }

    /// The file with every layer of `only` (or of every effect) that [`trail_mismatches`](Self::trail_mismatches)
    /// finds drawing with a shader that matches: one this file already has, else one copied in from the game's own
    /// files (a layer that uses it is added to the effect, which brings the shader, and taken out again). Nothing else
    /// about a layer changes. Returns it and the mismatches, with what fixed each.
    pub fn fix_trail_shaders(&self, only: Option<&str>) -> Result<(Self, Vec<TrailMismatch>)> {
        let mut found = self.trail_mismatches(only);
        if found.is_empty() {
            return Ok((Self::from_bytes(self.path.clone(), self.bytes.clone())?, found));
        }
        // shaders this file lacks: one game layer per kind, added to the first effect that needs it
        let mut transplants: Vec<TransplantOp> = Vec::new();
        let mut donors: Vec<ShaderDonor> = Vec::new();
        let root = game_root();
        for m in &mut found {
            let em = &self.emitters(m.set_idx)[m.layer];
            let current = |id: &str| attr(em, id).map(|v| v.as_f32());
            let Some((v, key)) = self.matching_shader(em, &current) else { continue };
            if v != u32::MAX {
                continue;
            }
            let Some(donor) = root.as_deref().and_then(|root| game_shaders(root).get(&key)) else { continue };
            m.fix = Some(format!("{} / {} / {}", donor.file, donor.effect, donor.layer));
            if !donors.contains(donor) {
                donors.push(donor.clone());
                transplants.push(TransplantOp {
                    into_entry: Some(m.effect.clone()),
                    src_file_rel: donor.file.clone(),
                    src_set_name: donor.effect.clone(),
                    only_emitters: vec![donor.layer.clone()],
                    ..Default::default()
                });
            }
        }
        let with_donors = if transplants.is_empty() { None } else {
            Some(self.rebuilt_from(&EffMod { transplants: transplants.clone(), ..Default::default() }, root.as_deref())?)
        };
        let base = with_donors.as_ref().unwrap_or(self);
        let mut pending = Pending::new();
        for m in &mut found {
            let em = &base.emitters(m.set_idx)[m.layer];
            let current = |id: &str| attr(em, id).map(|v| v.as_f32());
            let Some((v, _)) = base.matching_shader(em, &current).filter(|(v, _)| *v != u32::MAX) else {
                m.fix = None;
                continue;
            };
            if m.fix.is_none() {
                m.fix = Some(format!("shader {v} of this file"));
            }
            let like = attr(em, "shader_references.shader_index").ok_or_else(|| anyhow!("no shader index on {}", m.name))?;
            pending.entry((m.set_idx, m.layer)).or_default().insert("shader_references.shader_index".into(), typed(like, v as f32));
        }
        let mut eff = base.values_mod(&pending);
        // the added layers go again: each effect keeps the layers it had
        for op in &transplants {
            let entry = op.into_entry.as_deref().unwrap_or_default();
            let (_, set_idx) = self.find(entry)?;
            if eff.rosters.iter().any(|r| r.set_idx == set_idx) {
                continue;
            }
            eff.rosters.push(base.roster_of(entry, set_idx, self.emitters(set_idx).len()));
        }
        Ok((base.rebuilt(&eff)?, found))
    }

    /// The file with layer `layer` of `entry` switched to `value` for `style`, drawing with a shader built that way: the
    /// one, in this file or the game's own files, built most like the layer's own. It must follow the bone, face the
    /// camera and read the vertex inputs as the layer's does, and have a second texture only if the layer has; any
    /// other compiled-in field it differs in, the layer takes from it, so values and shader agree (the colour and fade
    /// types count most against a shader, then which texture channels it reads, the rest of the combiner least). A shader from the game comes with a layer that
    /// has it, added to the effect and taken out again. Switching on also takes the style's tuning (`params`) from that
    /// layer. Returns where the shader came from and the fields taken with it, or None (and the file as it was) when
    /// no file has one.
    pub fn restyle(&self, entry: &str, set_idx: usize, layer: usize, style: &Style, value: f32) -> Result<(Self, Option<Restyled>)> {
        let em = self.emitters(set_idx).get(layer).ok_or_else(|| anyhow!("no layer {layer} in {entry}"))?;
        if attr(em, style.field).map(|v| v.as_f32()) == Some(value) {
            return Ok((Self::from_bytes(self.path.clone(), self.bytes.clone())?, Some(Restyled { from: "its own".into(), taken: Vec::new() })));
        }
        self.reshade(entry, set_idx, layer, &[(style.field, value)], if value != 0.0 { style.params } else { &[] })
    }

    /// The file with layer `layer` of `entry` set to `want` and drawing with a shader built for it, found as
    /// [`Doc::restyle`] finds one (`params` come from the layer that has it). The layer may already hold the wanted
    /// values, as after the panel has set them: its own shader is never the answer, since it was built before.
    pub fn reshade(&self, entry: &str, set_idx: usize, layer: usize, want: &[(&str, f32)], params: &[&str]) -> Result<(Self, Option<Restyled>)> {
        let unchanged = || Self::from_bytes(self.path.clone(), self.bytes.clone());
        let em = self.emitters(set_idx).get(layer).ok_or_else(|| anyhow!("no layer {layer} in {entry}"))?;
        let stored = |id: &str| attr(em, id).map(|v| v.as_f32());
        let wanted = |id: &str| want.iter().find(|(w, _)| *w == id).map(|(_, v)| *v);
        let current = |id: &str| wanted(id).or_else(|| stored(id));
        let shader = stored("shader_references.shader_index").ok_or_else(|| anyhow!("{} has no shader index", em.name))?;
        let own = self.variations().get(shader as usize).ok_or_else(|| anyhow!("shader {shader} of {} can't be read", em.name))?;
        let reads_birth = current("emitter_info.follow_type").unwrap_or(0.0) != 0.0;
        // what the game feeds each particle depends on the layer's settings, so those inputs must be the same; what
        // comes from the model (normals, vertex colour) only has to be there
        let per_particle = |inputs: &[String]| -> Vec<String> { inputs.iter().filter(|n| !MODEL_INPUTS.iter().any(|(m, _)| m == n)).cloned().collect() };
        let mut inputs = per_particle(&own.inputs);
        // a particle that turns is fed its start angle, which only some shaders read: without it the turn draws nothing
        let turning = turns(&current);
        if turning && !inputs.iter().any(|n| n == ROTATION_INPUT) {
            inputs.push(ROTATION_INPUT.to_string());
            inputs.sort();
        }
        // a way of facing the camera or following decides what the game feeds, so when one is asked for, the inputs
        // come with it (the layer may already hold the value its shader doesn't draw yet)
        let feeds_change = ["particle_data.billboard_type", "emitter_info.follow_type"].iter().any(|id| wanted(id).is_some());
        let model = match self.primitive_of(entry, &em.name) {
            Some(p) => Some(self.mesh(p)?),
            None => None,
        };
        let model_has = |input: &str| model.as_ref().is_some_and(|m| match input {
            "sysNormalAttr" => !m.normals.is_empty(),
            _ => !m.colors.is_empty(),
        });
        // what the shader reads from the model now (its vertex colours' fade, its normals' shading) it keeps reading
        let kept_from_model: Vec<String> = own.inputs.iter()
            .filter(|n| MODEL_INPUTS.iter().any(|(m, _)| m == *n) && model_has(n)).cloned().collect();
        let ids = shader_key_ids();
        let key = shader_key(em, &current);
        // what a difference costs: the places that must agree cost too much to take
        const MUST: usize = usize::MAX / 1024;
        let cost: Vec<usize> = ids.iter().map(|id| match *id {
            "" | "particle_data.billboard_type" | "emitter_info.follow_type" => MUST,
            id if wanted(id).is_some() => MUST,
            id if id.starts_with("particle_color.") && id.ends_with("_type") => 40,
            // which channel of a texture gives the shape
            id if id.starts_with("combiner.tex_") && id.ends_with("_input_type") => 45,
            id if id.starts_with("combiner.") => 5,
            _ => 10,
        }).collect();
        let fit = |v: &Variation, other: &[i64]| -> Option<usize> {
            if v.reads_birth != reads_birth || other.len() != key.len() || (!feeds_change && per_particle(&v.inputs) != inputs)
                || (turning && !v.inputs.iter().any(|n| n == ROTATION_INPUT))
                || v.inputs.iter().any(|n| MODEL_INPUTS.iter().any(|(m, _)| m == n) && !model_has(n))
                || kept_from_model.iter().any(|k| !v.inputs.contains(k))
            {
                return None;
            }
            let total: usize = (0..key.len()).filter(|&i| other[i] != key[i]).map(|i| cost[i]).sum();
            (total < MUST).then_some(total)
        };
        let variations = self.variations();
        let in_file = self.effects().into_iter().flat_map(|(effect, s)| {
            self.emitters(s).iter().enumerate().map(move |(l, other)| (effect.clone(), s, l, other))
        }).filter(|(_, s, l, _)| (*s, *l) != (set_idx, layer)).filter_map(|(effect, s, l, other)| {
            let value = |id: &str| attr(other, id).map(|v| v.as_f32());
            let v = variations.get(value("shader_references.shader_index")? as usize)?;
            Some((fit(v, &shader_key(other, &value))?, effect, s, l))
        }).min_by_key(|(score, effect, s, l)| (*score, effect.clone(), *s, *l));
        let root = game_root();
        let in_game = || -> Option<(usize, &'static ShaderDonor)> {
            game_shaders(root.as_deref()?).iter()
                .filter_map(|((v, other), donor)| Some((fit(v, other)?, donor)))
                .min_by_key(|(score, d)| (*score, d.file.clone(), d.effect.clone(), d.layer.clone()))
        };
        // the file's own when nothing in the game is closer
        let game = match &in_file {
            Some((0, ..)) => None,
            Some((near, ..)) => in_game().filter(|(score, _)| score < near),
            None => in_game(),
        };
        let (base, at, from, added) = match (in_file, game) {
            (Some((_, effect, s, l)), None) => {
                let from = format!("{effect} / {}", self.emitters(s)[l].name);
                (None, (s, l), from, None)
            }
            (_, None) => return Ok((unchanged()?, None)),
            (_, Some((_, donor))) => {
                let op = TransplantOp {
                    into_entry: Some(entry.to_string()),
                    src_file_rel: donor.file.clone(),
                    src_set_name: donor.effect.clone(),
                    only_emitters: vec![donor.layer.clone()],
                    ..Default::default()
                };
                let base = self.rebuilt_from(&EffMod { transplants: vec![op], ..Default::default() }, root.as_deref())?;
                let kept = self.emitters(set_idx).len();
                // the one layer that came in, renamed if the effect already has its name
                if base.emitters(set_idx).len() <= kept {
                    return Err(anyhow!("{} didn't come in", donor.layer));
                }
                let l = kept;
                (Some(base), (set_idx, l), format!("{} / {} / {}", donor.file, donor.effect, donor.layer), Some(kept))
            }
        };
        let doc = base.as_ref().unwrap_or(self);
        let em = &doc.emitters(set_idx)[layer];
        let donor = &doc.emitters(at.0)[at.1];
        let mut values: BTreeMap<String, AttrValue> = BTreeMap::new();
        for (id, value) in want {
            let like = attr(em, id).ok_or_else(|| anyhow!("{} has no {id}", em.name))?;
            values.insert(id.to_string(), typed(like, *value));
        }
        let theirs = shader_key(donor, &|id| attr(donor, id).map(|v| v.as_f32()));
        let mut taken = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            if !id.is_empty() && wanted(id).is_none() && theirs.get(i) != key.get(i) {
                if let Some(v) = attr(donor, id) {
                    values.insert(id.to_string(), v);
                    taken.push(id.to_string());
                }
            }
        }
        for id in params {
            if let Some(v) = attr(donor, id) {
                values.insert(id.to_string(), v);
            }
        }
        // a colour or opacity that changed between fixed and keyed keeps what it showed
        let mut set = |id: String, v: f32| {
            if let Some(like) = attr(em, &id) {
                values.insert(id, typed(like, v));
            }
        };
        for (which, channels) in [("color0", &["_r", "_g", "_b"][..]), ("color1", &["_r", "_g", "_b"]), ("alpha0", &[""]), ("alpha1", &[""])] {
            let ty = format!("particle_color.{which}_type");
            let Some(now) = attr(donor, &ty).map(|v| v.as_f32()).filter(|_| taken.contains(&ty)) else { continue };
            let was = stored(&ty).unwrap_or(0.0);
            let axes = ["x", "y", "z"];
            if now == 2.0 && was != 2.0 {
                set(format!("emitter_static.num_{which}_keys"), 1.0);
                for (i, axis) in axes.iter().enumerate() {
                    let constant = stored(&format!("particle_color.{which}{}", channels[i.min(channels.len() - 1)])).unwrap_or(1.0);
                    set(format!("emitter_static.{which}.keys[0].{axis}"), constant);
                }
            } else if was == 2.0 && now != 2.0 {
                for (i, channel) in channels.iter().enumerate() {
                    let key = stored(&format!("emitter_static.{which}.keys[0].{}", axes[i])).unwrap_or(1.0);
                    set(format!("particle_color.{which}{channel}"), key);
                }
            }
        }
        // a pulse the shader switches on stays still (the shader divides by its cycle, so that can't be 0)
        if taken.iter().any(|id| id.starts_with("particle_fluctuation.")) {
            for axis in ["x", "y"] {
                set(format!("emitter_static.amplitude_{axis}"), 0.0);
                if stored(&format!("emitter_static.cycle_{axis}")).unwrap_or(0.0) <= 0.0 {
                    set(format!("emitter_static.cycle_{axis}"), 1.0);
                }
            }
        }
        let shader = attr(donor, "shader_references.shader_index").ok_or_else(|| anyhow!("no shader index on {}", donor.name))?;
        values.insert("shader_references.shader_index".into(), shader);
        let mut pending = Pending::new();
        pending.insert((set_idx, layer), values);
        let mut eff = doc.values_mod(&pending);
        if let Some(kept) = added {
            eff.rosters.push(doc.roster_of(entry, set_idx, kept));
        }
        Ok((doc.rebuilt(&eff)?, Some(Restyled { from, taken })))
    }

    /// Whether the layer is set to turn but its shader doesn't read the start angle, so it draws unturned.
    pub fn turn_ignored(&self, em: &EmitterDef) -> bool {
        let value = |id: &str| attr(em, id).map(|v| v.as_f32());
        turns(&value) && value("shader_references.shader_index")
            .and_then(|s| self.variations().get(s as usize))
            .is_some_and(|v| !v.inputs.iter().any(|n| n == ROTATION_INPUT))
    }

    /// What the shader of each layer of an effect (or layer `only`) is built for: the inputs its vertex stage reads,
    /// whether it reads where the layer was born, and the values of the fields compiled into it, one line each.
    pub fn shader_caps(&self, entry: &str, set_idx: usize, only: Option<usize>) -> Vec<String> {
        let ids = shader_key_ids();
        let mut lines = Vec::new();
        for (layer, em) in self.emitters(set_idx).iter().enumerate().filter(|(l, _)| only.is_none_or(|o| o == *l)) {
            let value = |id: &str| attr(em, id).map(|v| v.as_f32());
            let Some(shader) = value("shader_references.shader_index") else { continue };
            let Some(v) = self.variations().get(shader as usize) else {
                lines.push(format!("{} {}: shader {shader} can't be read", layer + 1, em.name));
                continue;
            };
            let model = if self.primitive_of(entry, &em.name).is_some() { "model" } else { "sprite" };
            lines.push(format!("{} {} ({model}): shader {shader}, reads {}{}", layer + 1, em.name,
                if v.inputs.is_empty() { "nothing else".to_string() } else { v.inputs.join(", ") },
                if v.reads_birth { ", and where it was born" } else { "" }));
            let key = shader_key(em, &value);
            let built: Vec<String> = ids.iter().zip(&key).filter(|(id, _)| !id.is_empty())
                .map(|(id, k)| format!("{id}={k}")).collect();
            lines.push(format!("    built for {}", built.join(" ")));
            if self.turn_ignored(em) {
                lines.push(format!("    set to turn, but the shader doesn't read {ROTATION_INPUT}, so it draws unturned"));
            }
            for why in why_unseen(em) {
                lines.push(format!("    may not show: {why}"));
            }
        }
        lines
    }

    /// The roster that keeps the first `kept` layers of set `set_idx` (played by `entry`) and drops the rest.
    fn roster_of(&self, entry: &str, set_idx: usize, kept: usize) -> EmitterRoster {
        EmitterRoster {
            set_name: self.loaded.ptcl.emitter_sets[set_idx].name.clone(),
            entry_name: entry.to_string(),
            set_idx,
            slots: self.emitters(set_idx)[..kept].iter().enumerate()
                .map(|(i, e)| EmitterSlot { source_idx: i, source_name: e.name.clone(), name: e.name.clone(), depth: e.depth, source_set: String::new() })
                .collect(),
        }
    }

}

/// Where a swapped-in picture's shape (its alpha) comes from. Effect textures often keep a shape in one colour
/// channel, several to a file, or draw on black with no alpha at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaFrom {
    #[default]
    Own,
    Red,
    Green,
    Blue,
    /// The brightest of red, green and blue: a glow drawn on black.
    Brightness,
}

impl AlphaFrom {
    pub const ALL: [(AlphaFrom, &'static str); 5] = [
        (AlphaFrom::Own, "its alpha"), (AlphaFrom::Red, "red"), (AlphaFrom::Green, "green"), (AlphaFrom::Blue, "blue"),
        (AlphaFrom::Brightness, "brightness"),
    ];

    /// `own`, `r`, `g`, `b` or `brightness` (or a label).
    pub fn parse(word: &str) -> Option<Self> {
        match word.to_ascii_lowercase().as_str() {
            "own" | "alpha" | "a" | "its alpha" => Some(AlphaFrom::Own),
            "r" | "red" => Some(AlphaFrom::Red),
            "g" | "green" => Some(AlphaFrom::Green),
            "b" | "blue" => Some(AlphaFrom::Blue),
            "brightness" | "lum" | "max" => Some(AlphaFrom::Brightness),
            _ => None,
        }
    }
}

/// How a picture becomes a layer's texture: where its shape comes from, and whether its colour gives way to white so
/// the layer's colours alone tint it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureFrom {
    pub alpha: AlphaFrom,
    pub white: bool,
}

/// `image` made into texture pixels as `how` says, with notes on a shape that won't read.
pub fn texture_pixels(mut image: image::RgbaImage, how: TextureFrom) -> (image::RgbaImage, Vec<String>) {
    for p in image.pixels_mut() {
        let [r, g, b, a] = p.0;
        let alpha = match how.alpha {
            AlphaFrom::Own => a,
            AlphaFrom::Red => r,
            AlphaFrom::Green => g,
            AlphaFrom::Blue => b,
            AlphaFrom::Brightness => r.max(g).max(b),
        };
        p.0 = if how.white { [255, 255, 255, alpha] } else { [r, g, b, alpha] };
    }
    let (low, high) = image.pixels().fold((255u8, 0u8), |(lo, hi), p| (lo.min(p.0[3]), hi.max(p.0[3])));
    let mut notes = Vec::new();
    if high < 8 {
        notes.push("its shape is empty, so the layer draws nothing: take the shape from another channel".to_string());
    } else if low >= 250 {
        notes.push(if how.alpha == AlphaFrom::Own {
            "it has no transparency, so the whole square draws: take the shape from a colour channel or its brightness"
        } else {
            "that channel is solid everywhere, so the whole square draws"
        }.to_string());
    }
    (image, notes)
}

/// The panel's own controls by name, for the command line: what each takes and what it does.
pub const PANEL_NAMES: [(&str, &str); 16] = [
    ("size", "size on every axis"),
    ("size-varies", "each particle up to this many % smaller"),
    ("colour", "r,g,b (0 to 1) or #rrggbb, where the texture is bright; Colour 2 follows while they match"),
    ("colour2", "r,g,b or #rrggbb, where the texture is dark"),
    ("brightness", "multiplies the colour; above 1 glows"),
    ("opacity", "0 to 1"),
    ("speed", "animated texture speed, 1 the game's normal"),
    ("over", "1 draws over the stage"),
    ("glow", "1 is additive"),
    ("turn-x", "start angle in degrees about X; borrows a shader that turns when needed"),
    ("turn-y", "start angle in degrees about Y"),
    ("turn-z", "start angle in degrees about Z"),
    ("spin-x", "degrees a frame about X"),
    ("spin-y", "degrees a frame about Y"),
    ("spin-z", "degrees a frame about Z"),
    ("roll-random", "each particle starts turned up to this many degrees either way about Z"),
];

/// The fields one of the panel's controls (see [`PANEL_NAMES`]) sets to `text`, for a layer whose values are `value`.
pub fn panel_values(value: &dyn Fn(&str) -> Option<f32>, name: &str, text: &str) -> Result<Vec<(String, f32)>> {
    let number = || text.parse::<f32>().map_err(|_| anyhow!("{name}={text}: a number"));
    let rgb = || -> Result<[f32; 3]> {
        if let Some(hex) = text.strip_prefix('#').filter(|h| h.len() == 6) {
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map(|b| b as f32 / 255.0);
            return Ok([byte(0)?, byte(2)?, byte(4)?]);
        }
        let parts: Vec<f32> = text.split(',').map(str::parse).collect::<Result<_, _>>()
            .map_err(|_| anyhow!("{name}={text}: r,g,b or #rrggbb"))?;
        parts.try_into().map_err(|_| anyhow!("{name}={text}: three numbers, r,g,b"))
    };
    let colour = |which: &str, rgb: [f32; 3]| -> Vec<(String, f32)> {
        let mut out = Vec::new();
        for (i, (channel, axis)) in [("r", "x"), ("g", "y"), ("b", "z")].into_iter().enumerate() {
            out.push((format!("particle_color.{which}_{channel}"), rgb[i]));
            out.push((format!("emitter_static.{which}.keys[0].{axis}"), rgb[i]));
        }
        if which == "color1" {
            // a shader built for animated colour reads every key, and a fixed colour leaves the rest black
            out.push(("particle_color.color1_type".into(), 2.0));
            out.push(("emitter_static.num_color1_keys".into(), 1.0));
        }
        out
    };
    let flag = |on: bool| if on { 1.0 } else { 0.0 };
    Ok(match name {
        "size" => ["x", "y", "z"].map(|a| (format!("particle_scale.scale_{a}"), 0.0)).into_iter().map(|(id, _)| Ok((id, number()?))).collect::<Result<_>>()?,
        "size-varies" => ["x", "y", "z"].into_iter().map(|a| Ok((format!("particle_scale.scale_random_{a}"), number()?))).collect::<Result<_>>()?,
        "colour" => {
            let rgb = rgb()?;
            let together = ["r", "g", "b"].iter().all(|c| {
                (value(&format!("particle_color.color1_{c}")).unwrap_or(1.0) - value(&format!("particle_color.color0_{c}")).unwrap_or(1.0)).abs() < 1e-3
            });
            let mut out = colour("color0", rgb);
            if together {
                out.extend(colour("color1", rgb));
            }
            out
        }
        "colour2" => colour("color1", rgb()?),
        "brightness" => vec![("emitter_static.color_scale".into(), number()?)],
        "opacity" => {
            let a = number()?;
            let mut out = vec![("particle_color.alpha0".to_string(), a)];
            out.extend(["x", "y", "z"].map(|axis| (format!("emitter_static.alpha0.keys[0].{axis}"), a)));
            out
        }
        "speed" => {
            let speed = number()?;
            if speed <= 0.0 {
                return Err(anyhow!("speed={text}: above 0"));
            }
            vec![("emitter_static.tex_pattern_anim0.frequency".into(), 1.0 / speed)]
        }
        "over" => vec![("render_state.is_depth_test".into(), flag(number()? == 0.0))],
        "glow" => vec![("render_state.blend_type".into(), flag(number()? != 0.0))],
        "roll-random" => vec![("emitter_static.rotate_init_rand_z".into(), number()?.to_radians()), ("particle_data.is_rotate_z".into(), 1.0)],
        _ => match name.split_once('-') {
            Some((how @ ("turn" | "spin"), axis @ ("x" | "y" | "z"))) => {
                let field = if how == "turn" { "rotate_init" } else { "rotate_add" };
                vec![(format!("emitter_static.{field}_{axis}"), number()?.to_radians()), (format!("particle_data.is_rotate_{axis}"), 1.0)]
            }
            _ => return Err(anyhow!("{name} isn't one of the panel's names")),
        },
    })
}

/// Why a layer may show nothing, from its values and a run of its particles: empty when nothing points to it.
/// Each reason says what to change.
pub fn why_unseen(em: &EmitterDef) -> Vec<String> {
    use crate::draw::sim::{spawns_moving, EmitterSim};
    let value = |id: &str| attr(em, id).map(|v| v.as_f32()).unwrap_or(0.0);
    let mut why = Vec::new();
    let sim = EmitterSim::read(em);
    // a run over the layer's whole life: start, emitting, and its last particles dying
    let horizon = (sim.emission_start + sim.emission_duration.clamp(1.0, 300.0) + sim.life.clamp(1.0, 300.0)).min(900.0);
    let samples: Vec<_> = (0..=48).map(|i| horizon * i as f32 / 48.0).flat_map(|t| spawns_moving(&sim, t, 1, None)).collect();
    // a fraction a firing builds up over firings in the game, so only none at all, or no life, is certain
    if samples.is_empty() && (sim.rate < 0.001 || sim.life < 1.0) {
        why.push(format!("no particle is ever alive (emits {} a firing, each living {} frames, from frame {}): give it a rate and a life",
            sim.rate, sim.life, sim.emission_start));
    } else if value("particle_data.billboard_type") == 5.0 && samples.iter().all(|s| s.velocity.length() < 1e-7) {
        // the game gives such layers the smallest speed (0.001) just for a direction; none at all leaves none to face
        why.push("it stretches along its motion, but its particles have no speed of their own, so it shows only while the effect itself moves: give them some (0.001 is enough) or face the camera instead".into());
    }
    if ["x", "y"].iter().any(|a| value(&format!("particle_scale.scale_{a}")) == 0.0) {
        why.push("its size is 0".into());
    }
    if value("emitter_static.color_scale") == 0.0 {
        why.push("its brightness is 0, unless the move's script sets a colour".into());
    }
    let keyed = |which: &str| value(&format!("particle_color.{which}_type")) == 2.0;
    let opacity: Vec<f32> = if keyed("alpha0") {
        (0..(value("emitter_static.num_alpha0_keys") as usize).clamp(1, 8)).map(|i| value(&format!("emitter_static.alpha0.keys[{i}].x"))).collect()
    } else {
        vec![value("particle_color.alpha0")]
    };
    if opacity.iter().all(|a| *a < 0.004) {
        why.push("its opacity is 0 all its life, unless the move's script sets one".into());
    }
    let black = |which: &str| -> bool {
        if keyed(which) {
            let keys = (value(&format!("emitter_static.num_{which}_keys")) as usize).clamp(1, 8);
            (0..keys).all(|i| ["x", "y", "z"].iter().all(|a| value(&format!("emitter_static.{which}.keys[{i}].{a}")) < 0.004))
        } else {
            ["r", "g", "b"].iter().all(|c| value(&format!("particle_color.{which}_{c}")) < 0.004)
        }
    };
    if value("render_state.blend_type") == 1.0 && black("color0") && black("color1") {
        why.push("it glows (adds its colour), and its colours are black, which adds nothing".into());
    }
    why
}

/// The vertex input that carries a particle's start angle; a shader that doesn't read it draws every particle unturned.
pub const ROTATION_INPUT: &str = "sysInitRotateAttr";

/// Whether a layer with these values turns its particles about any axis.
pub fn turns(value: &dyn Fn(&str) -> Option<f32>) -> bool {
    ["x", "y", "z"].iter().any(|axis| value(&format!("particle_data.is_rotate_{axis}")).unwrap_or(0.0) != 0.0)
}

pub fn attr(emitter: &EmitterDef, id: &str) -> Option<AttrValue> {
    *emitter.attrs.get(fields::index_of(id)?)?
}

/// `value` in the type `like` has.
pub fn typed(like: AttrValue, value: f32) -> AttrValue {
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
    /// Layer `only` of an effect, or every layer, with each `name=value` set: a panel name ([`PANEL_NAMES`]) sets
    /// what that control sets, anything else is a field id. A layer left turning on a shader that can't is given one
    /// that can, as the panel does. Says what it did, a line a layer.
    pub fn set_values(&self, effect: &str, only: Option<usize>, sets: &[(&str, &str)]) -> Result<(Self, Vec<String>)> {
        let (entry, set) = self.find(effect)?;
        let count = self.emitters(set).len();
        if only.is_some_and(|o| o >= count) {
            return Err(anyhow!("{effect} has layers 1 to {count}"));
        }
        let mut pending = Pending::new();
        let mut report = Vec::new();
        for (index, emitter) in self.emitters(set).iter().enumerate().filter(|(i, _)| only.is_none_or(|o| o == *i)) {
            let value = |id: &str| attr(emitter, id).map(|v| v.as_f32());
            let values = pending.entry((set, index)).or_default();
            let mut changed = Vec::new();
            for (name, text) in sets {
                let fields = if PANEL_NAMES.iter().any(|(n, _)| n == name) {
                    panel_values(&value, name, text)?
                } else {
                    let kind = crate::eff::fields::index_of(name).map(|i| crate::eff::fields::table()[i].kind);
                    let parsed = match kind {
                        Some(kind) => crate::eff::fields::enum_value(kind, text),
                        None => text.parse::<f32>().ok(),
                    };
                    let wants = match kind {
                        Some(crate::eff::fields::AttrKind::Enum(names)) => format!("a number or one of: {}", names.join(", ")),
                        _ => "a number".into(),
                    };
                    vec![(name.to_string(), parsed.ok_or_else(|| anyhow!("{name}={text}: {wants}"))?)]
                };
                for (id, v) in fields {
                    let like = attr(emitter, &id).ok_or_else(|| {
                        anyhow!("{} has no field {id} (a panel name is one of: {})", emitter.name,
                            PANEL_NAMES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "))
                    })?;
                    values.insert(id.clone(), typed(like, v));
                    changed.push(id);
                }
            }
            changed.dedup();
            report.push(format!("{}: {}", emitter.name, changed.join(" ")));
        }
        let mut doc = self.apply_values(&pending)?;
        for index in (0..count).filter(|i| only.is_none_or(|o| o == *i)) {
            if doc.turn_ignored(&doc.emitters(set)[index]) {
                let axis = ["x", "y", "z"].into_iter()
                    .find(|a| attr(&doc.emitters(set)[index], &format!("particle_data.is_rotate_{a}")).is_some_and(|v| v.as_f32() != 0.0))
                    .unwrap_or("z");
                let (next, found) = doc.reshade(&entry, set, index, &[(&format!("particle_data.is_rotate_{axis}"), 1.0)], &[])?;
                let name = doc.emitters(set)[index].name.clone();
                report.push(match found {
                    Some(found) => format!("{name}: turns, so it draws with the shader of {}", found.from),
                    None => format!("{name}: no shader that turns was found, so it draws unturned"),
                });
                doc = next;
            }
        }
        Ok((doc, report))
    }

    /// Layer `only` of an effect, or every layer, set to `id = value` and drawing with a shader built for it, found
    /// as the panel finds one ([`Doc::reshade`]; `effect_forge borrow`). Says, per layer, whose shader it took.
    pub fn borrow_on_effect(&self, effect: &str, only: Option<usize>, id: &str, value: f32) -> Result<(Self, Vec<String>)> {
        let (entry, set) = self.find(effect)?;
        let mut doc = Self::from_bytes(self.path.clone(), self.bytes.clone())?;
        let mut report = Vec::new();
        for index in 0..self.emitters(set).len() {
            if only.is_some_and(|o| o != index) {
                continue;
            }
            let name = self.emitters(set)[index].name.clone();
            let em = &doc.emitters(set)[index];
            if attr(em, id).map(|v| v.as_f32()) == Some(value) && !doc.turn_ignored(em) {
                report.push(format!("{name}: already"));
                continue;
            }
            let (next, found) = doc.reshade(&entry, set, index, &[(id, value)], &[])?;
            doc = next;
            report.push(match found {
                Some(r) if r.taken.is_empty() => format!("{name}: shader from {}", r.from),
                Some(r) => format!("{name}: shader from {}; took {}", r.from, r.taken.join(", ")),
                None => format!("{name}: no shader draws it that way; unchanged"),
            });
        }
        Ok((doc, report))
    }
}
