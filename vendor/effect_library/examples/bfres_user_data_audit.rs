//! Report containers whose models/materials/bones carry user data.
//!
//! The writer emits user-data COUNTS but never the arrays themselves, so any container that has
//! some would round-trip with a non-zero count pointing at a null array. Whether that matters
//! depends on whether real effect containers use the feature at all.
//!
//! Usage: cargo run --release --example bfres_user_data_audit -- <dir-or-eff>

use effect_library::bfres::load;
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

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("usage: <dir-or-eff>"));
    let mut paths = Vec::new();
    collect_effs(&root, &mut paths);
    paths.sort();

    let mut with_user_data = 0usize;
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
        let Ok(parsed) = load::load_from_bytes(pool) else {
            continue;
        };
        let (mut m, mut mat, mut bone) = (0usize, 0usize, 0usize);
        for model in parsed.models.values() {
            m += model.user_data.len();
            for material in model.materials.values() {
                mat += material.user_data.len();
            }
            for b in &model.skeleton.bones {
                bone += b.1.user_data.len();
            }
        }
        if m + mat + bone > 0 {
            with_user_data += 1;
            println!(
                "{}: model={m} material={mat} bone={bone}",
                path.strip_prefix(&root).unwrap_or(path).display()
            );
        }
    }
    println!(
        "\ncontainers carrying user data: {with_user_data} of {}",
        paths.len()
    );
}
