//! User-local cache and temporary storage used by Effect Forge.

use std::path::PathBuf;





// ── The running emulator's SD root ────────────────────────────────────────────






/// Persistent cache root. `EFFECT_FORGE_CACHE_DIR` overrides the platform cache directory.
pub fn app_storage_root() -> PathBuf {
    std::env::var_os("EFFECT_FORGE_CACHE_DIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| dirs::cache_dir().map(|path| path.join("effect_forge")))
        .unwrap_or_else(|| std::env::temp_dir().join("effect_forge"))
}

/// Unique temporary directory for EffectLibrary operations that need filesystem scratch space.
pub fn app_scratch_dir(prefix: &str) -> anyhow::Result<tempfile::TempDir> {
    let base = app_storage_root().join("scratch");
    std::fs::create_dir_all(&base)?;
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(&base)
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to create scratch directory in {}: {error} (set EFFECT_FORGE_CACHE_DIR to another disk)",
                base.display()
            )
        })
}
