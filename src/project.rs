//! A project: one .eff to work in, its effects sorted into folders, and where it exports to.
//!
//! `<name>.forge` is the project (JSON) and `<name>.project.eff` beside it is its effect file, which holds every
//! effect. Export writes that file to the mod (or anywhere), without the effects of folders named `UNUSED_...`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

pub const EXTENSION: &str = "forge";
const EFF_SUFFIX: &str = ".project.eff";

/// Folders whose names start with this are left out of the export.
pub const UNUSED_PREFIX: &str = "UNUSED_";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Project {
    /// The `.forge` file.
    #[serde(skip)]
    pub path: PathBuf,
    /// Where Export writes the file. Its place in the game (`effect/fighter/...`) is also the project file's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_to: Option<PathBuf>,
    #[serde(default)]
    pub folders: Vec<Folder>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    pub effects: Vec<String>,
}

impl Folder {
    pub fn is_unused(&self) -> bool {
        is_unused(&self.name)
    }
}

pub fn is_unused(folder: &str) -> bool {
    folder.len() >= UNUSED_PREFIX.len() && folder[..UNUSED_PREFIX.len()].eq_ignore_ascii_case(UNUSED_PREFIX)
}

pub fn is_project(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// The project a `<name>.project.eff` belongs to: `<name>.forge` beside it.
pub fn project_of_eff(eff: &Path) -> Option<PathBuf> {
    let name = eff.file_name()?.to_str()?;
    let stem = name.strip_suffix(EFF_SUFFIX)?;
    Some(eff.with_file_name(format!("{stem}.{EXTENSION}")))
}

impl Project {
    /// A new project at `path` holding `bytes` (the file it starts from), exporting to `export_to`.
    pub fn create(path: &Path, bytes: &[u8], export_to: Option<PathBuf>) -> Result<Project> {
        let project = Project { path: path.with_extension(EXTENSION), export_to, folders: Vec::new() };
        if project.eff_path().exists() {
            return Err(anyhow!("{} is already there; pick another name", project.eff_path().display()));
        }
        std::fs::write(project.eff_path(), bytes).with_context(|| format!("writing {}", project.eff_path().display()))?;
        project.save()?;
        Ok(project)
    }

    pub fn load(path: &Path) -> Result<Project> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut project: Project = serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        project.path = path.to_path_buf();
        Ok(project)
    }

    pub fn save(&self) -> Result<()> {
        std::fs::write(&self.path, serde_json::to_string_pretty(self)?).with_context(|| format!("writing {}", self.path.display()))
    }

    /// The project's own effect file.
    pub fn eff_path(&self) -> PathBuf {
        let stem = self.path.file_stem().and_then(|s| s.to_str()).unwrap_or("project");
        self.path.with_file_name(format!("{stem}{EFF_SUFFIX}"))
    }

    /// The folder `effect` is in.
    pub fn folder_of(&self, effect: &str) -> Option<usize> {
        self.folders.iter().position(|f| f.effects.iter().any(|e| e.eq_ignore_ascii_case(effect)))
    }

    /// Put `effect` in folder `to`, or in no folder.
    pub fn move_to(&mut self, effect: &str, to: Option<usize>) {
        for folder in &mut self.folders {
            folder.effects.retain(|e| !e.eq_ignore_ascii_case(effect));
        }
        if let Some(folder) = to.and_then(|i| self.folders.get_mut(i)) {
            folder.effects.push(effect.to_string());
            folder.effects.sort_by_key(|e| e.to_ascii_lowercase());
        }
    }

    pub fn rename_effect(&mut self, old: &str, new: &str) {
        for name in self.folders.iter_mut().flat_map(|f| f.effects.iter_mut()) {
            if name.eq_ignore_ascii_case(old) {
                *name = new.to_string();
            }
        }
    }

    pub fn add_folder(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("a folder needs a name"));
        }
        if self.folders.iter().any(|f| f.name.eq_ignore_ascii_case(name)) {
            return Err(anyhow!("there is already a folder called {name}"));
        }
        self.folders.push(Folder { name: name.to_string(), effects: Vec::new() });
        Ok(())
    }

    /// Mark folder `i` to be skipped on export or not: adds or takes off the `UNUSED_` prefix.
    pub fn set_skipped(&mut self, i: usize, skip: bool) -> Result<()> {
        let Some(folder) = self.folders.get(i) else { return Ok(()) };
        if folder.is_unused() == skip {
            return Ok(());
        }
        let name = if skip {
            format!("{UNUSED_PREFIX}{}", folder.name)
        } else {
            let rest = folder.name[UNUSED_PREFIX.len()..].trim_start_matches('_').to_string();
            if rest.is_empty() { "Folder".to_string() } else { rest }
        };
        self.rename_folder(i, &name)
    }

    pub fn rename_folder(&mut self, i: usize, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("a folder needs a name"));
        }
        if self.folders.iter().enumerate().any(|(j, f)| j != i && f.name.eq_ignore_ascii_case(name)) {
            return Err(anyhow!("there is already a folder called {name}"));
        }
        if let Some(folder) = self.folders.get_mut(i) {
            folder.name = name.to_string();
        }
        Ok(())
    }

    /// Take folder `i` away; its effects go back to no folder.
    pub fn remove_folder(&mut self, i: usize) {
        if i < self.folders.len() {
            self.folders.remove(i);
        }
    }

    /// Effects of the `UNUSED_` folders, which export leaves out.
    pub fn left_out(&self) -> Vec<String> {
        self.folders.iter().filter(|f| f.is_unused()).flat_map(|f| f.effects.iter().cloned()).collect()
    }

    /// Forget effects the file no longer has.
    pub fn tidy(&mut self, effects: &[String]) {
        for folder in &mut self.folders {
            folder.effects.retain(|e| effects.iter().any(|n| n.eq_ignore_ascii_case(e)));
        }
    }
}

/// `bytes` as exported: the effects of `left_out` taken out entirely (a script that still calls one finds nothing,
/// which plays nothing) and whatever only they used dropped. Returns the bytes and how many were left out.
pub fn exported(bytes: &[u8], left_out: &[String]) -> Result<(Vec<u8>, usize)> {
    let names = effect_library::NamcoEffectFile::load(bytes).context("reading the project's effect file")?.entry_names;
    let drop: Vec<&str> = names.iter().filter(|n| left_out.iter().any(|l| l.eq_ignore_ascii_case(n))).map(String::as_str).collect();
    if drop.is_empty() {
        return Ok((bytes.to_vec(), 0));
    }
    Ok((crate::eff::writer::drop_entries(bytes, &drop)?, drop.len()))
}

/// Write the export to `to`, keeping what was there as `.forge-backup` the first time.
pub fn export(bytes: &[u8], left_out: &[String], to: &Path) -> Result<usize> {
    let (out, skipped) = exported(bytes, left_out)?;
    let backup = to.with_extension("eff.forge-backup");
    if to.exists() && !backup.exists() {
        std::fs::copy(to, &backup).with_context(|| format!("backing up {}", to.display()))?;
    }
    std::fs::write(to, out).with_context(|| format!("writing {}", to.display()))?;
    Ok(skipped)
}
