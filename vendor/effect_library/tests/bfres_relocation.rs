//! The BFRES `_RLT` table must describe exactly the pointers the writer emitted.
//!
//! Same contract as `bnsh_relocation.rs`, and the same reason: at load the runtime adds the
//! image base to every slot the table lists, relocating each against its own section base. A
//! slot the table omits stays a raw file offset; a slot filed under the wrong section is fixed
//! up against the wrong base. Either way the consumer walks into the wrong memory — which for a
//! model container means geometry that draws as garbage rather than failing loudly.
//!
//! This exists because the model container is the one part of the pipeline whose output was
//! never checked against game-authored input. Point `EFFECT_LIBRARY_BFRES_SAMPLES` at a
//! directory of `.bfres` files to run it.

use effect_library::bfres::ResFile;

struct Rlt {
    sections: Vec<(usize, usize)>,
    /// slot position -> section index it is filed under
    pointers: Vec<(usize, usize)>,
}

fn read_u32(data: &[u8], at: usize) -> usize {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
}

fn read_u64(data: &[u8], at: usize) -> usize {
    u64::from_le_bytes(data[at..at + 8].try_into().unwrap()) as usize
}

/// BFRES and BNSH share the Nintendo binary-resource header, so the relocation table is laid
/// out identically: offset at 0x18, `_RLT` magic, section descriptors, then pointer runs.
fn parse_rlt(data: &[u8]) -> Rlt {
    let table = read_u32(data, 0x18);
    assert!(
        table + 4 <= data.len(),
        "relocation table offset {table:#x} lies past the end of the container"
    );
    assert_eq!(&data[table..table + 4], b"_RLT", "relocation table magic");
    let section_count = read_u32(data, table + 8);
    let sections_at = table + 16;
    let mut sections = Vec::new();
    let mut ranges = Vec::new();
    for index in 0..section_count {
        let at = sections_at + index * 24;
        sections.push((read_u32(data, at + 8), read_u32(data, at + 12)));
        ranges.push((read_u32(data, at + 16), read_u32(data, at + 20)));
    }
    let entries_at = sections_at + section_count * 24;
    let mut pointers = Vec::new();
    for (section_index, (first, count)) in ranges.into_iter().enumerate() {
        for entry in first..first + count {
            let at = entries_at + entry * 8;
            let position = read_u32(data, at);
            let struct_count =
                u16::from_le_bytes(data[at + 4..at + 6].try_into().unwrap()) as usize;
            let offset_count = data[at + 6] as usize;
            let padding_count = data[at + 7] as usize;
            let mut cursor = position;
            for _ in 0..struct_count {
                for slot in 0..offset_count {
                    pointers.push((cursor + slot * 8, section_index));
                }
                cursor += offset_count * 8 + padding_count * 8;
            }
        }
    }
    Rlt { sections, pointers }
}

fn section_of(sections: &[(usize, usize)], target: usize) -> usize {
    sections
        .iter()
        .position(|&(position, size)| size != 0 && target >= position && target < position + size)
        .unwrap_or(0)
}

/// Returns the problems rather than panicking on the first, so a failing container reports how
/// widespread the damage is instead of one arbitrary slot.
fn relocation_problems(data: &[u8]) -> Vec<String> {
    let rlt = parse_rlt(data);
    let mut problems = Vec::new();
    if rlt.pointers.is_empty() {
        problems.push("no relocations at all — every offset would stay unresolved".to_string());
        return problems;
    }
    for &(slot, filed_under) in &rlt.pointers {
        if slot + 8 > data.len() {
            problems.push(format!("slot {slot:#x} lies past the end of the container"));
            continue;
        }
        let target = read_u64(data, slot);
        // A ZERO slot is legitimate here, unlike in BNSH: game-authored containers are full of
        // them (949 in ef_kirby.bfres as shipped), so they are optional pointers the runtime
        // either skips or is content to see resolve to the image base. Calibrating the checker
        // against known-good input is the only reason that is knowable — asserting the stricter
        // BNSH rule here would have produced a thousand false positives.
        if target == 0 {
            continue;
        }
        if target >= data.len() {
            problems.push(format!(
                "slot {slot:#x} points to {target:#x}, past the end of the container"
            ));
            continue;
        }
        let actual = section_of(&rlt.sections, target);
        if actual != filed_under {
            problems.push(format!(
                "slot {slot:#x} targets {target:#x} (section {actual}) but is filed under \
                 section {filed_under}, so it relocates against the wrong base"
            ));
        }
    }
    problems
}

fn report(label: &str, data: &[u8]) -> Vec<String> {
    let problems = relocation_problems(data);
    if problems.is_empty() {
        eprintln!("  {label}: relocations consistent ({} B)", data.len());
    } else {
        eprintln!(
            "  {label}: {} relocation problem(s), first 3:",
            problems.len()
        );
        for p in problems.iter().take(3) {
            eprintln!("      {p}");
        }
    }
    problems
}

#[test]
fn game_containers_and_their_rewrites_have_consistent_relocations() {
    let Some(dir) = std::env::var_os("EFFECT_LIBRARY_BFRES_SAMPLES") else {
        eprintln!("skipped: set EFFECT_LIBRARY_BFRES_SAMPLES to a directory of .bfres files");
        return;
    };
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("sample directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("bfres") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let original = std::fs::read(&path).expect("read sample");
        eprintln!("{name}:");
        // As shipped — this validates the CHECKER against known-good data. If this fails, the
        // invariant is wrong, not the writer.
        if !report("as shipped", &original).is_empty() {
            failures.push(format!("{name}: the shipped container fails the invariant"));
        }
        // After our own round-trip, which is what every carrier ships.
        match ResFile::canonicalize(&original) {
            Ok(rewritten) => {
                if !report("canonicalized", &rewritten).is_empty() {
                    failures.push(format!("{name}: canonicalize produces bad relocations"));
                }
            }
            Err(e) => failures.push(format!("{name}: canonicalize failed: {e}")),
        }
        checked += 1;
    }
    assert!(checked > 0, "no .bfres samples found in {dir:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Canonicalizing must preserve every MODEL, not just a walkable pointer graph.
///
/// The relocation test above proves the container is structurally sound; this proves it still
/// contains the same geometry. Per-model export is the comparison surface: if extracting model
/// `i` from the original and from the canonicalized container yields the same bytes, that
/// model's vertex/index data and attribute layout came through unchanged.
#[test]
fn canonicalizing_preserves_every_model() {
    let Some(dir) = std::env::var_os("EFFECT_LIBRARY_BFRES_SAMPLES") else {
        eprintln!("skipped: set EFFECT_LIBRARY_BFRES_SAMPLES to a directory of .bfres files");
        return;
    };
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("sample directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("bfres") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let original = std::fs::read(&path).expect("read sample");
        let canonical = match ResFile::canonicalize(&original) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{name}: canonicalize failed: {e}"));
                continue;
            }
        };
        let count = ResFile::first_model_attribute_indices(&original)
            .map(|m| m.len())
            .unwrap_or(0);
        let mut differing = 0usize;
        let mut index = 0usize;
        loop {
            let before = effect_library::bfres::export_single_model(&original, index);
            let after = effect_library::bfres::export_single_model(&canonical, index);
            match (before, after) {
                (Ok(a), Ok(b)) => {
                    if a != b {
                        differing += 1;
                    }
                }
                (Err(_), Err(_)) => break,
                (a, b) => {
                    failures.push(format!(
                        "{name}: model {index} extractable from one container but not the \
                         other ({}/{})",
                        a.is_ok(),
                        b.is_ok()
                    ));
                    break;
                }
            }
            index += 1;
            if index > 512 {
                break;
            }
        }
        eprintln!(
            "{name}: {index} models compared ({count} attribute entries), {differing} differ"
        );
        if differing != 0 {
            failures.push(format!(
                "{name}: {differing} of {index} models changed under canonicalize"
            ));
        }
        checked += 1;
    }
    assert!(checked > 0, "no .bfres samples found in {dir:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Merging extracted models back into one container must preserve every model.
///
/// This is the path a carrier takes whenever it cannot ship a donor's pool verbatim: each kept
/// model is exported on its own and the results are merged. It is also the path that has been
/// blamed, in a comment, for models that "construct and return handles, then draw nothing" —
/// without ever being measured. Measure it: extract every model, merge them, then extract each
/// one again and compare.
#[test]
fn merging_extracted_models_preserves_them() {
    let Some(dir) = std::env::var_os("EFFECT_LIBRARY_BFRES_SAMPLES") else {
        eprintln!("skipped: set EFFECT_LIBRARY_BFRES_SAMPLES to a directory of .bfres files");
        return;
    };
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("sample directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("bfres") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let original = std::fs::read(&path).expect("read sample");
        let mut models: Vec<Vec<u8>> = Vec::new();
        while let Ok(model) = effect_library::bfres::export_single_model(&original, models.len()) {
            models.push(model);
            if models.len() > 512 {
                break;
            }
        }
        let merged = match ResFile::merge_model_files(&models) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{name}: merge failed: {e}"));
                continue;
            }
        };
        let problems = relocation_problems(&merged);
        let mut differing = 0usize;
        for (index, want) in models.iter().enumerate() {
            match effect_library::bfres::export_single_model(&merged, index) {
                Ok(got) if got == *want => {}
                Ok(_) => differing += 1,
                Err(e) => {
                    failures.push(format!("{name}: model {index} lost in merge: {e}"));
                    break;
                }
            }
        }
        eprintln!(
            "{name}: {} models, merged {} B (original {} B), {differing} differ, {} relocation \
             problem(s)",
            models.len(),
            merged.len(),
            original.len(),
            problems.len()
        );
        if differing != 0 {
            failures.push(format!(
                "{name}: {differing} of {} models changed under merge",
                models.len()
            ));
        }
        if !problems.is_empty() {
            for p in problems.iter().take(3) {
                eprintln!("      {p}");
            }
            failures.push(format!(
                "{name}: merged container has {} relocation problem(s)",
                problems.len()
            ));
        }
        checked += 1;
    }
    assert!(checked > 0, "no .bfres samples found in {dir:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Models from TWO different containers must survive being merged into one.
///
/// This is the mixed-carrier case: a carrier holding its own mesh effect plus a foreign donor's.
/// It was assumed impossible ("a carrier can hold only one") on the strength of the same
/// unmeasured "not GPU-valid" claim, so it is worth an explicit test rather than a comment.
#[test]
fn models_from_two_containers_merge_intact() {
    let Some(dir) = std::env::var_os("EFFECT_LIBRARY_BFRES_SAMPLES") else {
        eprintln!("skipped: set EFFECT_LIBRARY_BFRES_SAMPLES to a directory of .bfres files");
        return;
    };
    let mut sources: Vec<(String, Vec<Vec<u8>>)> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("sample directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("bfres") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).expect("read sample");
        let mut models = Vec::new();
        while let Ok(m) = effect_library::bfres::export_single_model(&bytes, models.len()) {
            models.push(m);
            if models.len() > 512 {
                break;
            }
        }
        sources.push((name, models));
    }
    if sources.len() < 2 {
        eprintln!("skipped: need at least two .bfres samples");
        return;
    }
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    let combined: Vec<Vec<u8>> = sources.iter().flat_map(|(_, m)| m.clone()).collect();
    let merged = ResFile::merge_model_files(&combined).expect("cross-container merge");
    let problems = relocation_problems(&merged);
    // Compare model CONTENT, not the exported bytes: a name collision across containers is
    // resolved by renaming, so the name string legitimately differs for those models. What must
    // not change is the geometry.
    let mut differing = 0usize;
    let mut renamed = 0usize;
    for (index, want) in combined.iter().enumerate() {
        let got = match effect_library::bfres::export_single_model(&merged, index) {
            Ok(got) => got,
            Err(e) => panic!("model {index} lost in cross-container merge: {e}"),
        };
        if got == *want {
            continue;
        }
        let (want_name, want_model) =
            ResFile::parse_model_export(want.clone()).expect("source model parses");
        let (got_name, got_model) = ResFile::parse_model_export(got).expect("merged model parses");
        if format!("{want_model:?}") == format!("{got_model:?}") {
            // Same geometry. The bytes differ either because the model was renamed to resolve a
            // collision, or because the writer laid the export out differently — neither
            // changes what is drawn.
            let _ = (&want_name, &got_name);
            renamed += 1;
        } else {
            differing += 1;
        }
    }
    eprintln!("  {renamed} model(s) same geometry, different bytes (rename or relayout)");
    eprintln!(
        "cross-container merge of {:?}: {} models, {} B, {differing} differ, {} relocation \
         problem(s)",
        sources.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        combined.len(),
        merged.len(),
        problems.len()
    );
    assert_eq!(differing, 0, "models changed when merged across containers");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
