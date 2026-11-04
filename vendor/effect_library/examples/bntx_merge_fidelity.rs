//! Calibrate the BNTX writer against the game's own texture archives.
//!
//! Same discipline as `bfres_merge_fidelity`: the app never re-saves an archive untouched, it
//! splits one into per-texture exports and merges back the subset an effect references. So the
//! question that matters is whether splitting a GAME archive and merging it straight back
//! reproduces it — in bytes, and in size.
//!
//! Size matters on its own here. The carrier is shipped to the plugin as base64 inside a single
//! JSON frame, so every byte the archive carries costs about 1.33 on the wire, and textures are
//! ~90% of a built carrier.
//!
//! Usage: cargo run --release --example bntx_merge_fidelity -- <dir-or-eff> [max_files]

use effect_library::bntx;
use effect_library::NamcoEffectFile;
use std::path::{Path, PathBuf};

fn collect_effs(root: &Path, out: &mut Vec<PathBuf>) {
    if root.is_file() {
        out.push(root.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_effs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("eff") {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with('_') {
                out.push(path);
            }
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("usage: <dir-or-eff> [max_files]"));
    let limit: usize = args
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);

    let mut paths = Vec::new();
    collect_effs(&root, &mut paths);
    paths.sort();
    paths.truncate(limit);

    let (mut checked, mut exact, mut failed) = (0usize, 0usize, 0usize);
    let (mut orig_total, mut merged_total, mut parts_total) = (0i64, 0i64, 0i64);
    let mut shown = 0usize;
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(file) = NamcoEffectFile::load(&bytes) else {
            continue;
        };
        let Some(tex) = file
            .ptcl_file
            .as_ref()
            .and_then(|p| p.texture_info.as_ref())
        else {
            continue;
        };
        let Some(archive) = tex.binary_data.as_ref().filter(|b| b.len() > 0x40) else {
            continue;
        };
        if tex.descriptors.is_empty() {
            continue;
        }
        let label = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        checked += 1;

        // Split into per-texture archives, exactly as the carrier build does.
        let mut parts = Vec::with_capacity(tex.descriptors.len());
        let mut broke = None;
        for (i, d) in tex.descriptors.iter().enumerate() {
            match bntx::build_single_texture_bntx_public(archive, i, &d.name) {
                Ok(b) => parts.push(b),
                Err(e) => {
                    broke = Some(format!("texture {i} '{}': {e}", d.name));
                    break;
                }
            }
        }
        if let Some(why) = broke {
            failed += 1;
            if shown < 6 {
                shown += 1;
                println!("{label}: split failed: {why}");
            }
            continue;
        }
        let parts_bytes: usize = parts.iter().map(|p| p.len()).sum();
        let merged = match bntx::merge_texture_files(&parts) {
            Ok(m) => m,
            Err(e) => {
                failed += 1;
                if shown < 6 {
                    shown += 1;
                    println!("{label}: merge failed: {e}");
                }
                continue;
            }
        };

        orig_total += archive.len() as i64;
        merged_total += merged.len() as i64;
        parts_total += parts_bytes as i64;

        if merged == *archive {
            exact += 1;
        } else if shown < 6 {
            shown += 1;
            let at = (0..merged.len().min(archive.len()))
                .find(|i| merged[*i] != archive[*i])
                .unwrap_or(merged.len().min(archive.len()));
            println!(
                "{label}: {} textures, {} B -> {} B ({:+}), first diff at {at:#x}",
                tex.descriptors.len(),
                archive.len(),
                merged.len(),
                merged.len() as i64 - archive.len() as i64,
            );
        }
    }

    println!("\narchives checked : {checked}");
    println!("  byte-exact     : {exact}");
    println!("  divergent      : {}", checked - exact - failed);
    println!("  failed         : {failed}");
    println!("\ntotal original bytes : {orig_total}");
    println!(
        "total merged bytes   : {merged_total} ({:+})",
        merged_total - orig_total
    );
    println!(
        "sum of split parts   : {parts_total} ({:+} vs original)",
        parts_total - orig_total
    );
}
