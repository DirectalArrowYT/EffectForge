//! Calibrate the BFRES writer against the game's own containers.
//!
//! A writer is only trustworthy if it reproduces files the game ships. These invariants are
//! measured across all 296 BFRES pools under an extracted `effect/` tree:
//!
//!   * body byte-exactness covers `ShaderAssign` synthesis, per-vertex-buffer 72-byte blocks, the
//!     buffer-info word, string counts, 256-byte alignment before `_RLT`, and the order models are
//!     written in;
//!   * relocation-set equality covers which slots are marked as pointers. This is the check that
//!     distinguishes a container the game can load from one that parses perfectly in software and
//!     draws nothing on hardware.
//!
//! Point `EFFECT_LIBRARY_EFF_CORPUS` at a directory of `.eff` files to run these.

use effect_library::bfres::{export_single_model, ResFile};
use effect_library::NamcoEffectFile;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Game containers whose relocation set we still do not reproduce.
///
/// Recorded rather than tolerated by a threshold, so that fixing one or breaking another both
/// show up. Their GPU regions are reproduced exactly; only the pointer-slot set differs.
const KNOWN_RELOCATION_GAPS: &[&str] = &[
    "fighter/link/ef_link.eff",
    "fighter/master/ef_master.eff",
    "stage/zelda_tower/ef_zelda_tower.eff",
];

fn corpus() -> Option<PathBuf> {
    std::env::var_os("EFFECT_LIBRARY_EFF_CORPUS").map(PathBuf::from)
}

fn collect_effs(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_effs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("eff") {
            // Skip this tool's own previews; they are outputs, not game data.
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with('_') {
                out.push(path);
            }
        }
    }
}

/// Every game pool in the corpus, as (label, bytes, model count).
fn pools(root: &Path) -> Vec<(String, Vec<u8>, usize)> {
    let mut paths = Vec::new();
    collect_effs(root, &mut paths);
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let Ok(bytes) = std::fs::read(&path) else {
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
        if prim.descriptors.is_empty() {
            continue;
        }
        let label = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        out.push((label, pool.clone(), prim.descriptors.len()));
    }
    out
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(d[at..at + 4].try_into().expect("4 bytes"))
}
fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(d[at..at + 2].try_into().expect("2 bytes"))
}

fn rlt_offset(data: &[u8]) -> usize {
    u32_at(data, 0x18) as usize
}

/// Every absolute offset the relocation table marks as a pointer to be rebased at load.
fn relocated_positions(data: &[u8]) -> Option<BTreeSet<u32>> {
    let rlt = rlt_offset(data);
    if rlt + 0x10 > data.len() || &data[rlt..rlt + 4] != b"_RLT" {
        return None;
    }
    let section_count = u32_at(data, rlt + 8) as usize;
    let sections = rlt + 0x10;
    let mut out = BTreeSet::new();
    let mut cursor = sections + section_count * 24;
    for s in 0..section_count {
        let sec = sections + s * 24;
        if sec + 24 > data.len() {
            return None;
        }
        let count = u32_at(data, sec + 20) as usize;
        for e in 0..count {
            let at = cursor + e * 8;
            if at + 8 > data.len() {
                return None;
            }
            let position = u32_at(data, at);
            let struct_count = u16_at(data, at + 4) as u32;
            let offset_count = data[at + 6] as u32;
            let stride = (offset_count + data[at + 7] as u32) * 8;
            for st in 0..struct_count {
                for o in 0..offset_count {
                    out.insert(position + st * stride + o * 8);
                }
            }
        }
        cursor += count * 8;
    }
    Some(out)
}

/// Split a container into single-model exports and merge them straight back — the app's path.
fn split_and_merge(pool: &[u8], models: usize) -> Vec<u8> {
    let blobs: Vec<Vec<u8>> = (0..models)
        .map(|i| export_single_model(pool, i).expect("extract model"))
        .collect();
    ResFile::merge_model_files(&blobs).expect("merge")
}

#[test]
fn resaving_a_game_container_reproduces_its_body() {
    let Some(root) = corpus() else {
        eprintln!("skipped: set EFFECT_LIBRARY_EFF_CORPUS to a directory of .eff files");
        return;
    };
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for (label, pool, _) in pools(&root) {
        checked += 1;
        let ours = ResFile::canonicalize(&pool).expect("re-save");
        // Everything up to `_RLT`: headers, models, string pool and GPU region. The table itself
        // is compared by meaning, not bytes, since entry grouping is a free choice.
        let (a, b) = (rlt_offset(&pool), rlt_offset(&ours));
        if pool[0x20..a] != ours[0x20..b] {
            failures.push(label);
        }
    }
    assert!(checked > 0, "corpus contained no BFRES pools");
    assert!(
        failures.is_empty(),
        "{} of {checked} containers did not round-trip byte-exactly: {:?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
}

#[test]
fn merging_a_game_containers_models_reproduces_its_gpu_region() {
    let Some(root) = corpus() else {
        eprintln!("skipped: set EFFECT_LIBRARY_EFF_CORPUS to a directory of .eff files");
        return;
    };
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for (label, pool, models) in pools(&root) {
        checked += 1;
        let merged = split_and_merge(&pool, models);
        // Section 1 of the RLT begins at the index buffer: the start of what the GPU reads.
        let rlt = rlt_offset(&pool);
        let gpu_start = u32_at(&pool, rlt + 0x10 + 24 + 8) as usize;
        if pool[gpu_start..rlt] != merged[gpu_start..rlt_offset(&merged)] {
            failures.push(label);
        }
    }
    assert!(checked > 0, "corpus contained no BFRES pools");
    assert!(
        failures.is_empty(),
        "{} of {checked} merged containers lost GPU-region fidelity: {:?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
}

#[test]
fn rewritten_containers_relocate_the_same_pointer_slots() {
    let Some(root) = corpus() else {
        eprintln!("skipped: set EFFECT_LIBRARY_EFF_CORPUS to a directory of .eff files");
        return;
    };
    let mut unexpected = Vec::new();
    let mut fixed = Vec::new();
    let mut checked = 0usize;
    for (label, pool, _) in pools(&root) {
        checked += 1;
        let ours = ResFile::canonicalize(&pool).expect("re-save");
        let game = relocated_positions(&pool).expect("game relocation table");
        let mine = relocated_positions(&ours).expect("our relocation table");
        let known = KNOWN_RELOCATION_GAPS.contains(&label.as_str());
        match (game == mine, known) {
            (false, false) => unexpected.push(label),
            (true, true) => fixed.push(label),
            _ => {}
        }
    }
    assert!(checked > 0, "corpus contained no BFRES pools");
    assert!(
        unexpected.is_empty(),
        "{} container(s) relocate a different set of slots than the game does — this is what \
         makes a rebuilt pool parse cleanly and draw nothing: {:?}",
        unexpected.len(),
        &unexpected[..unexpected.len().min(10)]
    );
    assert!(
        fixed.is_empty(),
        "these are listed in KNOWN_RELOCATION_GAPS but now match — remove them: {fixed:?}"
    );
}
