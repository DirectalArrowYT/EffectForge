//! The game's effect shaders of an .eff, translated for wgpu (`draw::gpu`) and cached on disk.
//!
//! The first time an effect file is drawn, its shader variations are dumped (code, control section, reflection) and
//! translated to WGSL in-process (`draw::mx`) into `<cache>/<file>-<hash>/wgsl`, with an index.json of what each stage
//! binds. It runs on its own thread; until it finishes the preview draws nothing for that file, and a variation the
//! translator cannot handle draws nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};

#[derive(Clone, Debug)]
pub enum State {
    Ready(PathBuf),
    Building,
    Failed(String),
}

#[derive(Default)]
pub struct ShaderCache {
    jobs: HashMap<PathBuf, Arc<Mutex<State>>>,
}

/// Raised when the translation changes, so older translations are made again (3 was tools/eft_shaders.py's SPIR-V).
const TRANSLATION: u32 = 4;

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// Every shader variation of an .eff, as files in `out`: `v<i>_s<stage>.code` / `.ctl` (the NVN code and control
/// sections) and `v<i>_s<stage>.reflect.json` (which attribute, sampler and constant block each slot is).
pub fn dump_variations(eff: &[u8], out: &Path) -> Result<usize> {
    let namco = effect_library::NamcoEffectFile::load(eff).context("reading the effect file")?;
    let info = namco
        .ptcl_file
        .as_ref()
        .and_then(|p| p.shader_info.as_ref())
        .ok_or_else(|| anyhow!("the effect file has no shaders"))?;
    let grsc = info.binary_data.as_ref().ok_or_else(|| anyhow!("the effect file's shader binary is empty"))?;
    let bnsh = effect_library::bnsh::BnshFile::read(grsc).context("reading the shader binary")?;
    std::fs::create_dir_all(out)?;
    let mut written = 0;
    for (v, variation) in bnsh.variations.iter().enumerate() {
        for (s, stage) in variation.binary_program.stages.iter().enumerate() {
            let Some(stage) = stage else { continue };
            std::fs::write(out.join(format!("v{v:04}_s{s}.code")), &stage.byte_code)?;
            std::fs::write(out.join(format!("v{v:04}_s{s}.ctl")), &stage.control_code)?;
            if let Some(r) = &variation.binary_program.reflections[s] {
                let list = |slots: Vec<(String, i32)>| {
                    slots.iter().map(|(n, s)| format!("[\"{n}\", {s}]")).collect::<Vec<_>>().join(", ")
                };
                let json = format!(
                    "{{\"inputs\": [{}], \"samplers\": [{}], \"constant_buffers\": [{}]}}\n",
                    list(r.input_slots()),
                    list(r.sampler_slots()),
                    list(r.constant_buffer_slots())
                );
                std::fs::write(out.join(format!("v{v:04}_s{s}.reflect.json")), json)?;
            }
            written += 1;
        }
    }
    Ok(written)
}

/// Where an effect file's translated shaders live: named by the file and a hash of its shader binary, so an edited
/// file gets its own.
pub fn cache_dir(eff_path: &Path, eff: &[u8]) -> PathBuf {
    let stem = eff_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let hash = effect_library::NamcoEffectFile::load(eff)
        .ok()
        .and_then(|n| n.ptcl_file?.shader_info?.binary_data.map(|b| fnv(&b)))
        .unwrap_or(0);
    crate::scratch_dirs::app_storage_root()
        .join("eff_shaders")
        .join(format!("{stem}-{hash:016x}-t{TRANSLATION}"))
}

fn build(eff_path: &Path, dir: &Path) -> Result<()> {
    let eff = std::fs::read(eff_path).with_context(|| format!("reading {}", eff_path.display()))?;
    dump_variations(&eff, dir)?;
    let (ok, all) = translate_dir(dir)?;
    eprintln!("[eff] {ok} of {all} shader stages of {} translated", eff_path.display());
    Ok(())
}

/// Translate a [`dump_variations`] folder into `<dir>/wgsl`: `v<i>_s<stage>.wgsl` and index.json (per variation and
/// stage: ok, cbufs, in, out, samplers [{unit, binding}], and why when not ok). Returns (stages translated, stages).
pub fn translate_dir(dir: &Path) -> Result<(usize, usize)> {
    use crate::draw::mx::wgsl::{self, Stage};
    let out = dir.join("wgsl");
    std::fs::create_dir_all(&out)?;
    let mut variations: Vec<u32> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix('v')?.strip_suffix("_s4.code")?.parse().ok()
        })
        .collect();
    variations.sort();
    let stage_json = |r: &Result<wgsl::Shader>, stage: Stage| -> serde_json::Value {
        match r {
            Ok(s) => {
                let base = if stage == Stage::Vertex { wgsl::VERTEX_TEXTURE_BASE } else { 0 };
                let samplers: Vec<serde_json::Value> =
                    s.units.iter().map(|&u| serde_json::json!({ "unit": u, "binding": base + u })).collect();
                serde_json::json!({ "ok": true, "cbufs": s.cbufs, "in": s.inputs, "out": s.outputs, "samplers": samplers })
            }
            Err(e) => serde_json::json!({ "ok": false, "why": format!("{e:#}") }),
        }
    };
    let read = |v: u32, s: u32| -> Result<(Vec<u8>, Vec<u8>)> {
        let base = dir.join(format!("v{v:04}_s{s}"));
        Ok((std::fs::read(base.with_extension("code"))?, std::fs::read(base.with_extension("ctl"))?))
    };
    let mut index = serde_json::Map::new();
    let (mut ok, mut all) = (0, 0);
    for v in variations {
        let fs = read(v, 4).and_then(|(code, ctl)| wgsl::translate(wgsl::program(&code, &ctl)?, Stage::Fragment, 0));
        // the vertex stage writes every varying the fragment stage reads
        let extra = fs.as_ref().map(|s| s.inputs).unwrap_or(0);
        let vs = read(v, 0).and_then(|(code, ctl)| wgsl::translate(wgsl::program(&code, &ctl)?, Stage::Vertex, extra));
        for (r, s) in [(&vs, 0), (&fs, 4)] {
            all += 1;
            if let Ok(shader) = r {
                std::fs::write(out.join(format!("v{v:04}_s{s}.wgsl")), &shader.wgsl)?;
                ok += 1;
            }
        }
        index.insert(v.to_string(), serde_json::json!({ "vs": stage_json(&vs, Stage::Vertex), "fs": stage_json(&fs, Stage::Fragment) }));
    }
    let json = serde_json::json!({ "translator": "forge-mx", "variations": index });
    std::fs::write(out.join("index.json"), serde_json::to_vec_pretty(&json)?)?;
    Ok((ok, all))
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
            let result = std::fs::read(&path).map_err(anyhow::Error::from).and_then(|eff| {
                let dir = cache_dir(&path, &eff);
                if !dir.join("wgsl").join("index.json").exists() {
                    eprintln!("[eff] translating the shaders of {} into {}", path.display(), dir.display());
                    build(&path, &dir)?;
                }
                Ok(dir)
            });
            *state.lock().unwrap() = match result {
                Ok(dir) => State::Ready(dir),
                Err(e) => {
                    eprintln!("[eff] the game's shaders for {} are not available: {e:#}", path.display());
                    State::Failed(format!("{e:#}"))
                }
            };
        });
        State::Building
    }
}

/// The one cache of the running app: the runtime asks it whether an effect file can draw with the game's shaders,
/// and the renderer where they are.
pub fn global() -> &'static Mutex<ShaderCache> {
    static CACHE: std::sync::OnceLock<Mutex<ShaderCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Whether effects draw with the game's own shaders where they can (the viewport's toggle).
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The translated shaders of this effect file, if they are ready and the game's shaders are on.
pub fn ready(eff_path: &Path) -> Option<PathBuf> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    match global().lock().unwrap().get(eff_path) {
        State::Ready(dir) => Some(dir),
        _ => None,
    }
}
