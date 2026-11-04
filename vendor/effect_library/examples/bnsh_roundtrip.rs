//! Diagnostic for the BNSH writer: re-serializing a game-authored container must preserve the
//! shader model exactly, and must describe its own pointers correctly in the `_RLT` table.
//!
//! Usage: cargo run --example bnsh_roundtrip -- [--dump-dir DIR] <file.bnsh>...

use effect_library::bnsh::{merge_variation_files, BnshFile};

fn model_matches(left: &BnshFile, right: &BnshFile) -> Result<(), String> {
    if left.variations.len() != right.variations.len() {
        return Err(format!(
            "variation count {} vs {}",
            left.variations.len(),
            right.variations.len()
        ));
    }
    for (index, (a, b)) in left
        .variations
        .iter()
        .zip(right.variations.iter())
        .enumerate()
    {
        let (a, b) = (&a.binary_program, &b.binary_program);
        if a.memory_data != b.memory_data {
            return Err(format!("variation {index}: object data differs"));
        }
        if a.binary_format != b.binary_format || a.flags != b.flags || a.code_type != b.code_type {
            return Err(format!("variation {index}: program header differs"));
        }
        for stage in 0..a.stages.len() {
            let (x, y) = (&a.stages[stage], &b.stages[stage]);
            match (x, y) {
                (Some(x), Some(y)) => {
                    if x.byte_code != y.byte_code {
                        return Err(format!(
                            "variation {index} stage {stage}: byte code differs"
                        ));
                    }
                    if x.control_code != y.control_code {
                        return Err(format!(
                            "variation {index} stage {stage}: control code differs"
                        ));
                    }
                }
                (None, None) => {}
                _ => return Err(format!("variation {index} stage {stage}: presence differs")),
            }
        }
    }
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut dump_dir: Option<String> = None;
    if args.peek().map(|a| a == "--dump-dir").unwrap_or(false) {
        args.next();
        dump_dir = args.next();
    }
    let paths: Vec<String> = args.collect();
    let mut loaded: Vec<(String, Vec<u8>)> = Vec::new();

    for path in &paths {
        let original = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                println!("{path}: unreadable ({e})");
                continue;
            }
        };
        let parsed = match BnshFile::read(&original) {
            Ok(file) => file,
            Err(e) => {
                println!("{path}: parse failed ({e})");
                continue;
            }
        };
        let rewritten = parsed.write();
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        if let Some(dir) = &dump_dir {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(
                std::path::Path::new(dir).join(format!("{name}.rewritten")),
                &rewritten,
            );
        }
        let reread = BnshFile::read(&rewritten);
        let verdict = match &reread {
            Ok(again) => match model_matches(&parsed, again) {
                Ok(()) => "model preserved".to_string(),
                Err(e) => format!("MODEL LOST: {e}"),
            },
            Err(e) => format!("REREAD FAILED: {e}"),
        };
        println!(
            "{name}: variations={} original={} rewritten={} — {verdict}",
            parsed.variations.len(),
            original.len(),
            rewritten.len(),
        );
        loaded.push((name, original));
    }

    // Merging is the capability the runtime carrier needs: two containers in, one out, with
    // every variation from both intact and addressable at its relocated index.
    if loaded.len() >= 2 {
        let files: Vec<Vec<u8>> = loaded.iter().map(|(_, bytes)| bytes.clone()).collect();
        match merge_variation_files(&files) {
            Ok(merged) => {
                let expected: usize = files
                    .iter()
                    .map(|f| BnshFile::read(f).map(|b| b.variations.len()).unwrap_or(0))
                    .sum();
                match BnshFile::read(&merged) {
                    Ok(file) => println!(
                        "\nmerge of {} containers: {} bytes, variations={} (expected {}) {}",
                        files.len(),
                        merged.len(),
                        file.variations.len(),
                        expected,
                        if file.variations.len() == expected {
                            "OK"
                        } else {
                            "MISMATCH"
                        }
                    ),
                    Err(e) => println!("\nmerge re-read failed: {e}"),
                }
                if let Some(dir) = &dump_dir {
                    let _ = std::fs::write(std::path::Path::new(dir).join("merged.bnsh"), &merged);
                }
            }
            Err(e) => println!("\nmerge failed: {e}"),
        }
    }
}
