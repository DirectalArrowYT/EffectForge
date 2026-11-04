//! Split a BFRES round-trip divergence into "body" versus "relocation table".
//!
//! Every game file first diverges at 0x18, which is the RLT offset field — a forward pointer, so
//! it tells us nothing about where content actually changed. What matters is whether the model
//! data and GPU region are reproduced correctly and only the RLT differs, or whether the body is
//! wrong too. The RLT is what the game walks to fix up pointers at load; a wrong one yields a
//! container that parses in software and draws nothing on hardware.
//!
//! Usage: cargo run --release --example bfres_diff_regions -- <dir-or-eff> [max_files]

use effect_library::bfres::ResFile;
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

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().expect("4 bytes"))
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

    let mut body_exact = 0usize;
    let mut body_diff = 0usize;
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

        let (orig_rlt, ours_rlt) = (u32_at(pool, 0x18) as usize, u32_at(&ours, 0x18) as usize);
        if orig_rlt > pool.len() || ours_rlt > ours.len() {
            println!("{label}: RLT offset out of range, skipping");
            continue;
        }
        // Everything from the end of the header pointers to the start of the RLT: models,
        // string table, GPU region. This is the part the shader actually consumes.
        let orig_body = &pool[0x20..orig_rlt];
        let ours_body = &ours[0x20..ours_rlt];
        let body = match first_diff(orig_body, ours_body) {
            None => {
                body_exact += 1;
                "identical".to_string()
            }
            Some(at) => {
                body_diff += 1;
                format!(
                    "differs at {:#x} (body {} -> {} B)",
                    at + 0x20,
                    orig_body.len(),
                    ours_body.len()
                )
            }
        };
        println!(
            "{label}\n  body {body}\n  rlt  {} -> {} B  (offset {orig_rlt:#x} -> {ours_rlt:#x})",
            pool.len() - orig_rlt,
            ours.len() - ours_rlt,
        );
    }
    println!("\nbody identical: {body_exact}, body divergent: {body_diff}");
}
