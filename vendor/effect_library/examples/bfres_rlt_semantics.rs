//! Compare what two relocation tables MEAN, not how they are grouped.
//!
//! The RLT tells the game which 8-byte words hold pointers that must be rebased at load. The
//! same set of words can be encoded with different (struct_count, offset_count, padding_count)
//! groupings, so byte inequality is not proof of a defect — but a different *set* is, and it is
//! exactly the kind of defect that leaves a container parseable in software yet undrawable on
//! hardware, since the GPU follows pointers the loader has already mangled.
//!
//! Usage: cargo run --release --example bfres_rlt_semantics -- <dir-or-eff> [max_files]

use effect_library::bfres::ResFile;
use effect_library::NamcoEffectFile;
use std::collections::BTreeSet;
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
fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(d[at..at + 2].try_into().expect("2 bytes"))
}

/// Expand every relocation entry into the absolute offsets it marks as pointers.
fn relocated_positions(data: &[u8]) -> Option<BTreeSet<u32>> {
    let rlt = u32_at(data, 0x18) as usize;
    if rlt + 0x10 > data.len() || &data[rlt..rlt + 4] != b"_RLT" {
        return None;
    }
    let section_count = u32_at(data, rlt + 8) as usize;
    let sections = rlt + 0x10;
    // Entries are one contiguous run; each section's slice follows the previous section's.
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
            let padding_count = data[at + 7] as u32;
            let stride = (offset_count + padding_count) * 8;
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

    let (mut same, mut differ, mut undecodable) = (0usize, 0usize, 0usize);
    let mut shown = 0usize;
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(file) = NamcoEffectFile::load(&bytes) else {
            continue;
        };
        let Some(pool) = file
            .ptcl_file
            .as_ref()
            .and_then(|p| p.primitive_info.as_ref())
            .and_then(|p| p.binary_data.as_ref())
            .filter(|b| b.len() > 0x20)
        else {
            continue;
        };
        let Ok(ours) = ResFile::canonicalize(pool) else {
            continue;
        };
        let label = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        let (Some(a), Some(b)) = (relocated_positions(pool), relocated_positions(&ours)) else {
            undecodable += 1;
            continue;
        };
        if a == b {
            same += 1;
            continue;
        }
        differ += 1;
        if shown < 12 {
            shown += 1;
            let missing: Vec<_> = a
                .difference(&b)
                .take(8)
                .map(|v| format!("{v:#x}"))
                .collect();
            let extra: Vec<_> = b
                .difference(&a)
                .take(8)
                .map(|v| format!("{v:#x}"))
                .collect();
            println!(
                "{label}\n  game marks {} slots, we mark {}\n  we MISS  : {}\n  we ADD   : {}",
                a.len(),
                b.len(),
                if missing.is_empty() {
                    "-".into()
                } else {
                    missing.join(" ")
                },
                if extra.is_empty() {
                    "-".into()
                } else {
                    extra.join(" ")
                },
            );
        }
    }
    println!(
        "\nrelocation sets identical: {same}, differing: {differ}, undecodable: {undecodable}"
    );
}
