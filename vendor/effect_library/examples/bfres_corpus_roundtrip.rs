//! Round-trip every BFRES pool in a corpus of game `.eff` files and localize the first divergence.
//!
//! The writer is only trustworthy if `save(load(x)) == x` byte-for-byte on files the game itself
//! ships. A divergence means the layout rule here differs from Nintendo's somewhere, and a merged
//! container built on that rule can be structurally parseable yet still draw nothing on hardware.
//!
//! Usage:
//!   cargo run --release --example bfres_corpus_roundtrip -- <dir-of-eff-files> [max_files]

use effect_library::bfres::ResFile;
use effect_library::NamcoEffectFile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn collect_effs(root: &Path, out: &mut Vec<PathBuf>) {
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

/// Byte offset of the first difference, with a window of context from each side.
fn first_divergence(a: &[u8], b: &[u8]) -> Option<(usize, String)> {
    let n = a.len().min(b.len());
    let at = (0..n).find(|&i| a[i] != b[i]).or({
        if a.len() != b.len() {
            Some(n)
        } else {
            None
        }
    })?;
    let lo = at.saturating_sub(16);
    let hi_a = (at + 16).min(a.len());
    let hi_b = (at + 16).min(b.len());
    Some((
        at,
        format!(
            "orig[{lo:#x}..] {}\n              ours[{lo:#x}..] {}",
            hex(&a[lo..hi_a]),
            hex(&b[lo..hi_b])
        ),
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("usage: <dir-of-eff-files> [max_files]"));
    let limit: usize = args
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);

    let mut paths = Vec::new();
    collect_effs(&root, &mut paths);
    paths.sort();
    paths.truncate(limit);

    let (mut with_pool, mut exact, mut parse_failed) = (0usize, 0usize, 0usize);
    // Divergence offset is meaningless across files; the shape of the mismatch is what clusters.
    let mut buckets: BTreeMap<String, (usize, String)> = BTreeMap::new();

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
            .filter(|b| !b.is_empty())
        else {
            continue;
        };
        with_pool += 1;
        let label = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();

        let rebuilt = match ResFile::canonicalize(pool) {
            Ok(v) => v,
            Err(e) => {
                parse_failed += 1;
                let entry = buckets
                    .entry("parse/save failed".to_string())
                    .or_insert((0, String::new()));
                entry.0 += 1;
                if entry.1.is_empty() {
                    entry.1 = format!("{label}: {e}");
                }
                continue;
            }
        };

        if rebuilt == *pool {
            exact += 1;
            continue;
        }

        let delta = rebuilt.len() as i64 - pool.len() as i64;
        let (at, context) = first_divergence(pool, &rebuilt).expect("differs");
        let key = format!("size {delta:+} B, first diff at {at:#x}");
        let entry = buckets.entry(key).or_insert((0, String::new()));
        entry.0 += 1;
        if entry.1.is_empty() {
            entry.1 = format!("{label} ({} B)\n              {context}", pool.len());
        }
    }

    println!("scanned {} .eff files", paths.len());
    println!("  with a BFRES pool : {with_pool}");
    println!("  byte-exact        : {exact}");
    println!("  parse/save failed : {parse_failed}");
    println!("  divergent         : {}", with_pool - exact - parse_failed);
    if !buckets.is_empty() {
        println!("\ndivergence classes (count, first example):");
        let mut sorted: Vec<_> = buckets.into_iter().collect();
        sorted.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
        for (key, (count, example)) in sorted.iter().take(25) {
            println!("  [{count:>4}] {key}\n              {example}");
        }
    }
}
