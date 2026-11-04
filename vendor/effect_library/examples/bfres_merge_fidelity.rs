//! Prove the merge path, not just the round-trip.
//!
//! The app never re-saves a container untouched: it extracts each donor model as a single-model
//! export and merges N of them into one pool. That is `export_single_model` + `merge_model_files`,
//! and it is the path that decides whether a carrier holding several mesh-backed effects draws
//! all of them. Splitting a game container into its models and merging them straight back must
//! reproduce the original, or the merge is inventing layout the game never had.
//!
//! Usage: cargo run --release --example bfres_merge_fidelity -- <dir-or-eff> [max_files]

use effect_library::bfres::{export_single_model, ResFile};
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
            out.push(path);
        }
    }
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(d[at..at + 4].try_into().expect("4 bytes"))
}

/// Start of the GPU data region, taken from the RLT's second section descriptor.
///
/// Section 0 covers the headers/strings; section 1 begins at the index buffer, which is where the
/// data the GPU reads starts.
fn gpu_region_start(data: &[u8]) -> usize {
    let rlt = u32_at(data, 0x18) as usize;
    if rlt + 0x10 > data.len() || &data[rlt..rlt + 4] != b"_RLT" {
        return usize::MAX;
    }
    if (u32_at(data, rlt + 8) as usize) < 2 {
        return usize::MAX;
    }
    u32_at(data, rlt + 0x10 + 24 + 8) as usize
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    let n = a.len().min(b.len());
    (0..n)
        .find(|&i| a[i] != b[i])
        .or(if a.len() == b.len() { None } else { Some(n) })
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

    let (mut checked, mut body_exact, mut failed) = (0usize, 0usize, 0usize);
    let mut gpu_exact = 0usize;
    let mut shown = 0usize;
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(file) = NamcoEffectFile::load(&bytes) else {
            continue;
        };
        let Some(prim) = file
            .ptcl_file
            .as_ref()
            .and_then(|p| p.primitive_info.as_ref())
        else {
            continue;
        };
        let Some(pool) = prim.binary_data.as_ref().filter(|b| b.len() > 0x20) else {
            continue;
        };
        let label = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        let model_count = prim.descriptors.len();
        if model_count == 0 {
            continue;
        }
        checked += 1;

        let mut blobs = Vec::with_capacity(model_count);
        let mut broke = None;
        for i in 0..model_count {
            match export_single_model(pool, i) {
                Ok(b) => blobs.push(b),
                Err(e) => {
                    broke = Some(format!("extract model {i}: {e}"));
                    break;
                }
            }
        }
        if let Some(why) = broke {
            failed += 1;
            if shown < 8 {
                shown += 1;
                println!("{label}: {why}");
            }
            continue;
        }
        let merged = match ResFile::merge_model_files(&blobs) {
            Ok(m) => m,
            Err(e) => {
                failed += 1;
                if shown < 8 {
                    shown += 1;
                    println!("{label}: merge: {e}");
                }
                continue;
            }
        };

        // Compare the body only; RLT entry coalescing is a grouping choice that
        // `bfres_rlt_semantics` checks separately.
        let (a_rlt, b_rlt) = (u32_at(pool, 0x18) as usize, u32_at(&merged, 0x18) as usize);
        if a_rlt > pool.len() || b_rlt > merged.len() {
            failed += 1;
            continue;
        }
        // The GPU region is what the shader reads: index buffer, vertex buffer and memory pool,
        // running from the buffer-section start to the RLT. String-pool ORDER may legitimately
        // differ (references are absolute offsets, and models merged from different containers
        // have no single original order), but a byte of geometry may not.
        let a_gpu = u32_at(pool, 0x18) as usize;
        let gpu_start = gpu_region_start(pool).min(gpu_region_start(&merged));
        let gpu_same = gpu_start < a_rlt
            && gpu_start < b_rlt
            && pool[gpu_start..a_gpu] == merged[gpu_start..b_rlt];
        if gpu_same {
            gpu_exact += 1;
        }
        match first_diff(&pool[0x20..a_rlt], &merged[0x20..b_rlt]) {
            None => body_exact += 1,
            Some(at) => {
                if shown < 8 && !gpu_same {
                    shown += 1;
                    println!(
                        "{label}: {model_count} models, GPU REGION DIFFERS (body from {:#x})",
                        at + 0x20,
                    );
                }
            }
        }
    }
    println!("\nmerge-reconstructed containers: {checked}");
    println!("  body byte-exact : {body_exact}");
    println!("  GPU region exact: {gpu_exact}");
    println!("  body divergent  : {}", checked - body_exact - failed);
    println!("  failed          : {failed}");
}
