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
fn shader_key(em: &EmitterDef, current: &dyn Fn(&str) -> Option<f32>) -> Vec<i64> {
    let combiner = fields::table().iter().filter(|a| a.id.starts_with("combiner.")).map(|a| a.id);
    BAKED_MODES.iter().copied().chain(combiner).chain(["emitter_info.follow_type"])
        .map(|id| current(id).map_or(i64::MIN, |v| v as i64))
        .chain([em.texture_index1.is_some() as i64])
        .collect()
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
    pub fn new_effect(&self, name: &str, piece: &Piece) -> Result<Self> {
        let name = check_effect_name(name)?;
        if self.loaded.entries.iter().any(|e| e.name.eq_ignore_ascii_case(name)) {
            return Err(anyhow!("this file already has an effect called {name}"));
        }
        self.from_piece(piece, TransplantOp { new_entry_name: name.to_string(), ..Default::default() }, game_root())
    }

    /// A starter piece's layers added to `entry`, after the ones it has.
    pub fn add_layer(&self, entry: &str, piece: &Piece) -> Result<Self> {
        self.from_piece(piece, TransplantOp { into_entry: Some(entry.to_string()), ..Default::default() }, game_root())
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
            let kept = self.emitters(set_idx).len();
            let emitters = base.emitters(set_idx);
            if eff.rosters.iter().any(|r| r.set_idx == set_idx) {
                continue;
            }
            eff.rosters.push(EmitterRoster {
                set_name: base.loaded.ptcl.emitter_sets[set_idx].name.clone(),
                entry_name: entry.to_string(),
                set_idx,
                slots: emitters[..kept].iter().enumerate()
                    .map(|(i, e)| EmitterSlot { source_idx: i, source_name: e.name.clone(), name: e.name.clone(), depth: e.depth, source_set: String::new() })
                    .collect(),
            });
        }
        Ok((base.rebuilt(&eff)?, found))
    }

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
