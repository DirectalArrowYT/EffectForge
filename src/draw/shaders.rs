//! The game's effect shaders of an .eff, translated for wgpu (`draw::gpu`) and cached on disk.
//!
//! An .eff carries its shaders as a BNSH: per variation, a Maxwell program for each stage, with reflection naming its
//! vertex attributes. The first time a file is drawn, every variation is translated to WGSL in-process (`draw::mx`)
//! on a thread of its own, and the result (each stage's WGSL, what it binds, its own constant data and attribute
//! locations) is kept as one file in `<storage>/eff_shaders`. Until it is ready the preview draws nothing for that
//! file, and a variation the translator cannot handle draws nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::draw::mx::wgsl::{self, Stage};

/// Raised when what a translation holds changes, so older ones are made again (4 was a folder of dumps).
const TRANSLATION: u32 = 5;

/// The BNSH stage slots of the two stages Forge draws with.
const VERTEX_SLOT: usize = 0;
const FRAGMENT_SLOT: usize = 4;

/// One translated stage of a variation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Program {
    pub wgsl: String,
    /// Bit n: reads constant buffer c<n>.
    pub cbufs: u32,
    /// Bit n: reads generic input n (vertex attribute or varying).
    pub inputs: u32,
    /// Texture units it samples, ascending.
    pub units: Vec<u32>,
    /// c1: the program's own constant data, cut from its code section where the control section says.
    pub constants: Vec<u8>,
    /// Vertex attribute names (`sysLocalPosAttr` ...) and the location each is read from.
    pub attributes: Vec<(String, u32)>,
}

impl Program {
    pub fn attribute(&self, name: &str) -> Option<u32> {
        self.attributes.iter().find(|(n, _)| n == name).map(|&(_, at)| at)
    }
}

/// Every stage of a file that translated, by variation (`shader_references.shader_index`).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Library {
    pub vertex: HashMap<u32, Program>,
    pub fragment: HashMap<u32, Program>,
    /// Stages tried, for the log and the tests.
    pub tried: usize,
}

fn word(bytes: &[u8], at: usize) -> usize {
    bytes.get(at..at + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
}

fn program(
    code: Option<&effect_library::bnsh::ShaderCode>,
    reflection: Option<&effect_library::bnsh::ShaderReflectionData>,
    stage: Stage,
    extra_outputs: u32,
) -> Result<Program> {
    let code = code.ok_or_else(|| anyhow!("the variation has no {stage:?} stage"))?;
    let (bytes, ctl) = (&code.byte_code, &code.control_code);
    let shader = wgsl::translate(wgsl::program(bytes, ctl)?, stage, extra_outputs)?;
    let (size, at) = (word(ctl, 0x6fc), word(ctl, 0x700));
    Ok(Program {
        wgsl: shader.wgsl,
        cbufs: shader.cbufs,
        inputs: shader.inputs,
        units: shader.units,
        constants: bytes.get(at..at + size).map(<[u8]>::to_vec).unwrap_or_default(),
        attributes: reflection.map_or_else(Vec::new, |r| {
            r.input_slots().into_iter().filter_map(|(name, at)| Some((name, u32::try_from(at).ok()?))).collect()
        }),
    })
}

impl Library {
    pub fn program(&self, variation: u32, stage: Stage) -> Option<&Program> {
        match stage {
            Stage::Vertex => self.vertex.get(&variation),
            Stage::Fragment => self.fragment.get(&variation),
        }
    }

    pub fn translated(&self) -> usize {
        self.vertex.len() + self.fragment.len()
    }

    /// Translate every variation of an .eff that has a fragment stage.
    pub fn translate(eff: &[u8]) -> Result<Self> {
        let namco = effect_library::NamcoEffectFile::load(eff).context("reading the effect file")?;
        let grsc = namco
            .ptcl_file
            .as_ref()
            .and_then(|p| p.shader_info.as_ref())
            .and_then(|s| s.binary_data.as_ref())
            .ok_or_else(|| anyhow!("the effect file has no shaders"))?;
        let bnsh = effect_library::bnsh::BnshFile::read(grsc).context("reading the shader binary")?;
        let mut library = Self::default();
        for (v, variation) in bnsh.variations.iter().enumerate() {
            let p = &variation.binary_program;
            if p.stages[FRAGMENT_SLOT].is_none() {
                continue;
            }
            let fs = program(p.stages[FRAGMENT_SLOT].as_ref(), p.reflections[FRAGMENT_SLOT].as_ref(), Stage::Fragment, 0);
            // The vertex stage writes every varying the fragment stage reads.
            let extra = fs.as_ref().map_or(0, |f| f.inputs);
            let vs = program(p.stages[VERTEX_SLOT].as_ref(), p.reflections[VERTEX_SLOT].as_ref(), Stage::Vertex, extra);
            library.tried += 2;
            if let Ok(vs) = vs {
                library.vertex.insert(v as u32, vs);
            }
            if let Ok(fs) = fs {
                library.fragment.insert(v as u32, fs);
            }
        }
        Ok(library)
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// Where an effect file's translation is kept: named by the file and a hash of its shader binary, so an edited file
/// gets its own.
fn cache_file(eff_path: &Path, eff: &[u8]) -> PathBuf {
    let stem = eff_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let hash = effect_library::NamcoEffectFile::load(eff)
        .ok()
        .and_then(|n| n.ptcl_file?.shader_info?.binary_data.map(|b| fnv(&b)))
        .unwrap_or(0);
    crate::scratch_dirs::app_storage_root().join("eff_shaders").join(format!("{stem}-{hash:016x}-t{TRANSLATION}.json"))
}

/// The file's translation from the cache, or made now and stored.
fn load(eff_path: &Path) -> Result<Library> {
    let eff = std::fs::read(eff_path).with_context(|| format!("reading {}", eff_path.display()))?;
    let file = cache_file(eff_path, &eff);
    if let Some(library) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
        return Ok(library);
    }
    eprintln!("[eff] translating the shaders of {}", eff_path.display());
    let library = Library::translate(&eff)?;
    eprintln!("[eff] {} of {} shader stages of {} translated", library.translated(), library.tried, eff_path.display());
    std::fs::create_dir_all(file.parent().unwrap())?;
    let part = file.with_extension("part");
    std::fs::write(&part, serde_json::to_vec(&library)?)?;
    std::fs::rename(&part, &file)?;
    Ok(library)
}

#[derive(Clone, Debug)]
pub enum State {
    Ready(Arc<Library>),
    Building,
    /// Translating them failed; the reason is in the log.
    Failed,
}

#[derive(Default)]
pub struct ShaderCache {
    jobs: HashMap<PathBuf, Arc<Mutex<State>>>,
}

impl ShaderCache {
    /// The file at this path has changed, so its shaders must be looked up again.
    pub fn forget(&mut self, eff_path: &Path) {
        self.jobs.remove(eff_path);
    }

    /// The translated shaders of this effect file, starting their build if they are not there yet.
    pub fn get(&mut self, eff_path: &Path) -> State {
        if let Some(job) = self.jobs.get(eff_path) {
            return job.lock().unwrap().clone();
        }
        let state = Arc::new(Mutex::new(State::Building));
        self.jobs.insert(eff_path.to_path_buf(), state.clone());
        let path = eff_path.to_path_buf();
        std::thread::spawn(move || {
            *state.lock().unwrap() = match load(&path) {
                Ok(library) => State::Ready(Arc::new(library)),
                Err(e) => {
                    let line = format!("the game's shaders for {} are not available: {e:#}", path.display());
                    eprintln!("[eff] {line}");
                    crate::log::note(&line);
                    State::Failed
                }
            };
        });
        State::Building
    }
}

/// The one cache of the running app.
pub fn global() -> &'static Mutex<ShaderCache> {
    static CACHE: std::sync::OnceLock<Mutex<ShaderCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Whether effects draw with the game's own shaders where they can (the viewport's toggle).
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The translated shaders of this effect file, if they are ready and the game's shaders are on.
pub fn ready(eff_path: &Path) -> Option<Arc<Library>> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    match global().lock().unwrap().get(eff_path) {
        State::Ready(library) => Some(library),
        _ => None,
    }
}
