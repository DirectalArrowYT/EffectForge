//! The game's effect shaders of an .eff, translated for wgpu (eff_gpu) and cached on disk.
//!
//! The first time an effect file is drawn, its shader variations are dumped (code, control section, reflection) and
//! tools/eft_shaders.py translates them (the recomp's Maxwell translator, then glslang) into
//! `<cache>/<file>-<hash>/spv`. That takes a while, so it runs on its own thread; until it finishes, and on a machine
//! without the tools, the preview draws nothing for that file.
//!
//! Tools: `EFFECT_FORGE_TOOLS` (default: this crate's tools), `python` on PATH.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};

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

/// Raised when tools/eft_shaders.py changes what it writes, so older translations are made again.
const TRANSLATION: u32 = 3;

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn tools_dir() -> PathBuf {
    std::env::var_os("EFFECT_FORGE_TOOLS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("tools"))
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
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("EffectForge")
        .join("eff_shaders")
        .join(format!("{stem}-{hash:016x}-t{TRANSLATION}"))
}

fn build(eff_path: &Path, dir: &Path) -> Result<()> {
    let eff = std::fs::read(eff_path).with_context(|| format!("reading {}", eff_path.display()))?;
    dump_variations(&eff, dir)?;
    let script = tools_dir().join("eft_shaders.py");
    if !script.exists() {
        bail!("{} not found (set EFFECT_FORGE_TOOLS)", script.display());
    }
    let out = std::process::Command::new("python")
        .arg(&script)
        .arg(dir)
        .output()
        .context("running python")?;
    if !out.status.success() || !dir.join("spv").join("index.json").exists() {
        bail!("eft_shaders.py failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
    }
    Ok(())
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
                if !dir.join("spv").join("index.json").exists() {
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
