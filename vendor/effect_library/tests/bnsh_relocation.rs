//! The `_RLT` table must describe exactly the pointers the writer emitted.
//!
//! At load the runtime adds the image base to every slot the table lists, and each section is
//! relocated against its own base. So a listed slot holding 0 becomes a pointer to the base
//! address, and a pointer filed under the wrong section is fixed up against the wrong region —
//! both make the consumer fault while walking the container. These tests assert the invariants
//! that hold in every game-authored container, on output the writer produced itself, so they do
//! not need game files to run.

use effect_library::bnsh::{merge_variation_files, BnshFile};

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

fn parse_rlt(data: &[u8]) -> Rlt {
    let table = read_u32(data, 0x18);
    assert_eq!(&data[table..table + 4], b"_RLT", "relocation table magic");
    let section_count = read_u32(data, table + 8);
    let sections_at = table + 16;
    let mut sections = Vec::new();
    let mut ranges = Vec::new();
    for index in 0..section_count {
        let at = sections_at + index * 24;
        let position = read_u32(data, at + 8);
        let size = read_u32(data, at + 12);
        let first = read_u32(data, at + 16);
        let count = read_u32(data, at + 20);
        sections.push((position, size));
        ranges.push((first, count));
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

fn assert_relocations_describe_the_data(data: &[u8], label: &str) {
    let rlt = parse_rlt(data);
    assert!(
        !rlt.pointers.is_empty(),
        "{label}: no relocations at all — the runtime would leave every offset unresolved"
    );
    for &(slot, filed_under) in &rlt.pointers {
        assert!(
            slot + 8 <= data.len(),
            "{label}: relocation slot {slot:#x} lies past the end of the container"
        );
        let target = read_u64(data, slot);
        assert_ne!(
            target, 0,
            "{label}: slot {slot:#x} holds no offset, so relocating it fabricates a pointer to \
             the image base"
        );
        assert!(
            target < data.len(),
            "{label}: slot {slot:#x} points to {target:#x}, past the end of the container"
        );
        assert_eq!(
            section_of(&rlt.sections, target),
            filed_under,
            "{label}: slot {slot:#x} targets {target:#x} but is filed under section \
             {filed_under}, so it would be relocated against the wrong base"
        );
    }
}

/// A synthetic container exercising the writer end to end: several variations, shared byte code
/// (so the dedup path runs) and distinct object data.
fn sample_container(variations: usize) -> Vec<u8> {
    let mut file = BnshFile::read(&minimal_container()).expect("minimal container parses");
    let template = file.variations[0].clone();
    for index in 1..variations {
        let mut variation = template.clone();
        variation.binary_program.memory_data = vec![index as u8; 64];
        file.variations.push(variation);
    }
    file.write()
}

/// Smallest container the writer will produce: one variation with one stage.
fn minimal_container() -> Vec<u8> {
    use effect_library::bnsh::{
        BinaryHeader, BnshHeader, BnshShaderProgram, ShaderCode, ShaderVariation,
    };
    let mut program = BnshShaderProgram {
        binary_format: 1,
        memory_data: vec![0xab; 64],
        ..Default::default()
    };
    program.stages[0] = Some(ShaderCode {
        control_code: vec![0x11; 32],
        byte_code: vec![0x22; 128],
        reserved: [0; 32],
    });
    BnshFile {
        bin_header: BinaryHeader {
            magic: u64::from_le_bytes(*b"BNSH\0\0\0\0"),
            version_micro: 5,
            version_minor: 1,
            version_major: 2,
            byte_order: 0xfeff,
            alignment: 0xc,
            target_address_size: 0,
            name_offset: 0,
            flag: 0,
            block_offset: 0x60,
            relocation_table_offset: 0,
            file_size: 0,
        },
        header: BnshHeader {
            magic: u32::from_le_bytes(*b"grsc"),
            block_offset: 0,
            block_size: 0,
            padding: 0,
            api_type: 4,
            api_version: 0,
            code_target: 0,
            compiler_version: 0x302,
            num_variation: 1,
            variation_start_offset: 0,
            memory_pool_offset: 0,
            unknown2: 0x1100110001000f,
        },
        name: String::new(),
        variations: vec![ShaderVariation {
            binary_program: program,
        }],
    }
    .write()
}

#[test]
fn written_container_relocations_match_its_own_pointers() {
    for count in [1usize, 2, 5, 17] {
        let data = sample_container(count);
        assert_relocations_describe_the_data(&data, &format!("{count}-variation container"));
    }
}

#[test]
fn rewriting_preserves_the_shader_model() {
    let original = sample_container(5);
    let parsed = BnshFile::read(&original).expect("parses");
    let rewritten = parsed.write();
    let reread = BnshFile::read(&rewritten).expect("re-reads");
    assert_eq!(parsed.variations.len(), reread.variations.len());
    for (index, (before, after)) in parsed
        .variations
        .iter()
        .zip(reread.variations.iter())
        .enumerate()
    {
        assert_eq!(
            before.binary_program.memory_data, after.binary_program.memory_data,
            "variation {index} object data"
        );
        let (before, after) = (
            &before.binary_program.stages[0],
            &after.binary_program.stages[0],
        );
        let (before, after) = (before.as_ref().unwrap(), after.as_ref().unwrap());
        assert_eq!(
            before.byte_code, after.byte_code,
            "variation {index} byte code"
        );
        assert_eq!(
            before.control_code, after.control_code,
            "variation {index} control code"
        );
    }
}

#[test]
fn merged_container_keeps_every_variation_and_stays_consistent() {
    let left = sample_container(2);
    let right = sample_container(3);
    let merged = merge_variation_files(&[left, right]).expect("merges");
    let parsed = BnshFile::read(&merged).expect("merged container parses");
    assert_eq!(
        parsed.variations.len(),
        5,
        "a merge must keep every variation from both containers"
    );
    assert_relocations_describe_the_data(&merged, "merged container");
}

/// The same invariants, against GAME-AUTHORED containers.
///
/// The synthetic cases above exercise the writer's own idea of a container. A real one is
/// bigger, has many more variations, and — crucially — was laid out by Nintendo's tooling, so
/// it is the only thing that can catch the writer mis-describing a layout it did not invent.
/// Point `EFFECT_LIBRARY_BNSH_SAMPLES` at a directory of `.bnsh` files to run it.
///
/// Both halves matter and they fail differently:
///   * the container as SHIPPED must satisfy the invariants (proving the checker is right), and
///   * the container after OUR rewrite must satisfy them too (proving the writer is right).
#[test]
fn game_containers_and_their_rewrites_have_consistent_relocations() {
    let Some(dir) = std::env::var_os("EFFECT_LIBRARY_BNSH_SAMPLES") else {
        eprintln!("skipped: set EFFECT_LIBRARY_BNSH_SAMPLES to a directory of .bnsh files");
        return;
    };
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("sample directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("bnsh") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let original = std::fs::read(&path).expect("read sample");
        assert_relocations_describe_the_data(&original, &format!("{name} (as shipped)"));
        let parsed = BnshFile::read(&original).expect("game container parses");
        let rewritten = parsed.write();
        assert_relocations_describe_the_data(&rewritten, &format!("{name} (rewritten)"));
        checked += 1;
    }
    assert!(checked > 0, "no .bnsh samples found in {dir:?}");
}

/// SUBSETTING a game container must preserve the shader model of every variation it keeps.
///
/// The carrier does not ship a donor's container whole: it merges, then keeps only the
/// variations something addresses and renumbers the emitters onto them. Rewrite fidelity and
/// merge fidelity are covered above; this covers the step in between, which is the one an
/// emitter's shader index actually lands on. If a kept variation came back holding a different
/// program, every emitter using it would draw with the wrong shader.
#[test]
fn subsetting_a_game_container_preserves_the_kept_variations() {
    let Some(dir) = std::env::var_os("EFFECT_LIBRARY_BNSH_SAMPLES") else {
        eprintln!("skipped: set EFFECT_LIBRARY_BNSH_SAMPLES to a directory of .bnsh files");
        return;
    };
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("sample directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("bnsh") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let original = std::fs::read(&path).expect("read sample");
        let parsed = BnshFile::read(&original).expect("game container parses");
        // A scattered subset, the way a real carrier's needs fall out — not a prefix, which
        // would hide any dependence on position.
        let keep: Vec<usize> = (0..parsed.variations.len())
            .filter(|i| i % 7 == 3)
            .collect();
        if keep.is_empty() {
            continue;
        }
        let mut subset = parsed.clone();
        subset.variations = keep.iter().map(|i| parsed.variations[*i].clone()).collect();
        let written = subset.write();
        let reread = BnshFile::read(&written).expect("subset re-reads");
        assert_eq!(
            reread.variations.len(),
            keep.len(),
            "{name}: subset lost variations"
        );
        for (new_index, old_index) in keep.iter().enumerate() {
            let want = &parsed.variations[*old_index].binary_program;
            let got = &reread.variations[new_index].binary_program;
            assert_eq!(
                want.memory_data, got.memory_data,
                "{name}: variation {old_index} object data changed when kept at {new_index}"
            );
            for stage in 0..want.stages.len() {
                match (&want.stages[stage], &got.stages[stage]) {
                    (Some(a), Some(b)) => {
                        assert_eq!(
                            a.byte_code, b.byte_code,
                            "{name}: variation {old_index} stage {stage} byte code changed"
                        );
                        assert_eq!(
                            a.control_code, b.control_code,
                            "{name}: variation {old_index} stage {stage} control code changed"
                        );
                    }
                    (None, None) => {}
                    _ => panic!("{name}: variation {old_index} stage {stage} presence changed"),
                }
            }
        }
        assert_relocations_describe_the_data(&written, &format!("{name} (subset)"));
        eprintln!(
            "{name}: kept {} of {} variations intact",
            keep.len(),
            parsed.variations.len()
        );
        checked += 1;
    }
    assert!(checked > 0, "no .bnsh samples found in {dir:?}");
}
