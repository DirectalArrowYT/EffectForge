//! Where Forge keeps its own files: translated shaders, scratch space, the log and the recent-files list.
//!
//! All of it sits in `forge_data` beside the executable, so it lives on whichever drive Forge was put on rather than
//! the system drive. `EFFECT_FORGE_CACHE_DIR` puts it somewhere else.

use std::path::PathBuf;

pub fn app_storage_root() -> PathBuf {
    std::env::var_os("EFFECT_FORGE_CACHE_DIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| Some(std::env::current_exe().ok()?.parent()?.join("forge_data")))
        .unwrap_or_else(|| std::env::temp_dir().join("effect_forge"))
}

/// A fresh directory for a writer step that needs files on disk, removed when dropped.
pub fn app_scratch_dir(prefix: &str) -> anyhow::Result<tempfile::TempDir> {
    let base = app_storage_root().join("scratch");
    std::fs::create_dir_all(&base)?;
    tempfile::Builder::new().prefix(prefix).tempdir_in(&base).map_err(|error| {
        anyhow::anyhow!(
            "failed to create scratch directory in {}: {error} (set EFFECT_FORGE_CACHE_DIR to another disk)",
            base.display()
        )
    })
}
