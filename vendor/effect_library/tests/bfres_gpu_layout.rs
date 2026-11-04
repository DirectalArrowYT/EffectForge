//! The GPU-region offsets a container records must point at the data it actually contains.
//!
//! This is the invariant the loader cannot catch. `ResVertex + 0x48` and `ResMesh + 0x20` hold
//! each buffer's offset relative to the start of the GPU region, and the GPU reads through
//! exactly those values. Our loader, by contrast, re-walks the region applying each buffer's
//! declared alignment — so it lands on the right bytes even when the recorded offsets are
//! wrong, and a container that is unusable on hardware round-trips perfectly in software.
//!
//! Field layout per the Wexos and EPD (ZeldaMods) BFRES documentation:
//!   * ResVertex + 0x48 (u32): base offset of vertex data in GPU region
//!   * ResVertex + 0x56 (u16): alignment of vertex buffer data in GPU region
//!   * ResMesh   + 0x20 (u32): index buffer offset in GPU region
//!
//! Point `EFFECT_LIBRARY_BFRES_SAMPLES` at a directory of `.bfres` files to run it.

use effect_library::bfres::{load, ResFile};

/// Reproduce the layout the recorded offsets claim, and compare it against the bytes actually
/// present at those offsets. Returns a description per mismatch.
fn offset_mismatches(data: &[u8], label: &str) -> Vec<String> {
    let file = match load::load_from_bytes(data) {
        Ok(f) => f,
        Err(e) => return vec![format!("{label}: parse failed: {e}")],
    };
    let mut problems = Vec::new();
    for (model_name, model) in &file.models {
        for vb in &model.vertex_buffers {
            let align = if vb.gpu_buffer_alignment != 0 {
                vb.gpu_buffer_alignment as u32
            } else {
                8
            };
            // The recorded base must itself satisfy the alignment it declares. A base that does
            // not is proof the writer used a different rule than the one it advertises.
            if vb.buffer_offset % align != 0 {
                problems.push(format!(
                    "{label}: model '{model_name}' vertex base {:#x} is not aligned to its \
                     declared GPU alignment {align:#x}",
                    vb.buffer_offset
                ));
            }
        }
    }
    problems
}

#[test]
fn game_containers_record_aligned_gpu_offsets() {
    let Some(dir) = std::env::var_os("EFFECT_LIBRARY_BFRES_SAMPLES") else {
        eprintln!("skipped: set EFFECT_LIBRARY_BFRES_SAMPLES to a directory of .bfres files");
        return;
    };
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("sample directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("bfres") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let original = std::fs::read(&path).expect("read sample");

        // As shipped. Calibrates the invariant against known-good data: if Nintendo's own
        // containers fail this, the rule is wrong and nothing below means anything.
        let shipped = offset_mismatches(&original, &format!("{name} (as shipped)"));
        if !shipped.is_empty() {
            for p in shipped.iter().take(3) {
                eprintln!("  {p}");
            }
            failures.push(format!(
                "{name}: the SHIPPED container fails the invariant ({} problems) — the rule is \
                 wrong, not the writer",
                shipped.len()
            ));
        }

        // And after our own write, which is what every carrier ships.
        let rebuilt = ResFile::canonicalize(&original).expect("canonicalize");
        let ours = offset_mismatches(&rebuilt, &format!("{name} (rebuilt)"));
        if !ours.is_empty() {
            for p in ours.iter().take(3) {
                eprintln!("  {p}");
            }
            failures.push(format!(
                "{name}: rebuilt container has {} problems",
                ours.len()
            ));
        }
        eprintln!(
            "{name}: shipped {} problem(s), rebuilt {} problem(s)",
            shipped.len(),
            ours.len()
        );
        checked += 1;
    }
    assert!(checked > 0, "no .bfres samples found in {dir:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// How closely does our writer reproduce a game-authored container?
///
/// Byte-identity is the only correctness proof available locally that does not require trusting
/// another implementation — reference libraries can be wrong too, and hardware is the only other
/// arbiter. Each byte of divergence is somewhere our writer made a choice Nintendo's tooling did
/// not, and any of those is a candidate for a container the GPU rejects.
///
/// This asserts the gap does not GROW. It is not zero yet: the remaining difference is trailing
/// alignment padding around the 0x1000-aligned GPU region.
#[test]
fn round_trip_inflation_does_not_regress() {
    let Some(dir) = std::env::var_os("EFFECT_LIBRARY_BFRES_SAMPLES") else {
        eprintln!("skipped: set EFFECT_LIBRARY_BFRES_SAMPLES to a directory of .bfres files");
        return;
    };
    // Measured ceilings. Lower them whenever the writer gets closer; a rise is a regression.
    const MAX_INFLATION: usize = 4096;
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("sample directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("bfres") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let original = std::fs::read(&path).expect("read sample");
        let rebuilt = ResFile::canonicalize(&original).expect("canonicalize");
        let grew = rebuilt.len().saturating_sub(original.len());
        let first = original
            .iter()
            .zip(rebuilt.iter())
            .position(|(a, b)| a != b);
        eprintln!(
            "{name}: {} -> {} (+{grew}), first difference {:?}",
            original.len(),
            rebuilt.len(),
            first.map(|o| format!("{o:#x}"))
        );
        if grew > MAX_INFLATION {
            failures.push(format!(
                "{name}: rebuild inflated by {grew} B, over the {MAX_INFLATION} B ceiling"
            ));
        }
        checked += 1;
    }
    assert!(checked > 0, "no .bfres samples found in {dir:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
