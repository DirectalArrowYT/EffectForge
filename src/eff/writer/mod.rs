//! Writing an .eff: the file with a set of edits (`EffMod`) applied, through effect_library's byte-exact writer.
//!
//! Order matters and is fixed in [`rebuild`]: transplants (they only append sets), then pruning of the shader
//! library, added textures, emitter lists, field edits, entry edits, texture imports and removals.

mod emitters;
mod prune;
mod textures;
mod transplant;

use anyhow::{anyhow, Context, Result};

use crate::eff::edits::EffMod;
use emitters::*;
use prune::*;
use textures::*;
use transplant::*;

/// The file `src_bytes` with every edit in `eff` applied. `donor_root` is where cross-file transplants find their
/// donors (`TransplantOp::src_file_rel` is relative to it).
pub fn rebuild(
    src_bytes: &[u8],
    eff: &EffMod,
    donor_root: Option<&std::path::Path>,
) -> Result<Vec<u8>> {
    let mut namco = effect_library::NamcoEffectFile::load(src_bytes)
        .context("effect_library failed to parse the source .eff")?;
    let destination_is_fighter = eff
        .source_rel
        .to_ascii_lowercase()
        .starts_with("effect/fighter/");
    // Transplant ops FIRST (they only append sets, so pre-existing set indices are
    // stable), then authored edits — which may target a freshly cloned set by name
    // (editing the copy). Donors from other files are merged in here too.
    for op in &eff.transplants {
        let same_file = op.src_file_rel.is_empty() || op.src_file_rel == eff.source_rel;
        if same_file {
            apply_transplant_same_file(&mut namco, op)?;
        } else {
            let root = donor_root.ok_or_else(|| {
                anyhow!(
                    "transplant '{}': cross-file donor '{}' needs the export root",
                    op.new_entry_name,
                    op.src_file_rel
                )
            })?;
            let donor_bytes = std::fs::read(root.join(&op.src_file_rel)).with_context(|| {
                format!(
                    "cross-file transplant donor '{}' unreadable",
                    op.src_file_rel
                )
            })?;
            let donor = effect_library::NamcoEffectFile::load(&donor_bytes)
                .context("effect_library failed to parse the donor .eff")?;
            apply_transplant_cross_file(
                &mut namco,
                &donor,
                op,
                destination_is_fighter,
            )?;
        }
    }
    // Strip the shader library down to what this file addresses. Cross-file transplants merge in each donor's ENTIRE BNSH — Pickel ships 370 variations and
    // one transplanted effect uses twelve — so without this a single transplant makes the
    // player's ef_*.eff carry a second fighter's whole shader library.
    //
    // Orphans go first: a replace-mode transplant leaves the set it repointed away from holding
    // live emitters, and compaction scans sets, so an orphan would keep its share of the donor
    // library reachable. Clearing them cannot touch anything the game shipped — every set in all
    // 328 game corpus files is reachable from the entry table.
    //
    // The file's own effects all stay: this is the fighter's own eff and everything in it has to
    // keep playing. Textures are not pruned either, and deliberately —
    // `apply_transplant_cross_file` copies only the textures the cloned emitters reference, so
    // there is no unreferenced-texture bloat here to collect, while a prune this late could drop
    // a pool texture that an authored swap or an import below is about to name.
    clear_unreachable_emitter_sets(&mut namco);
    compact_shader_containers(&mut namco)?;
    verify_shader_indices_resolve(&namco, "eff export")?;

    // Added textures before the authored edits (a swap resolves its target by name, so it has to exist first), replacements
    // after (they land on whichever texture will actually be sampled), removals last (the swaps
    // that moved emitters off them have run by then).
    //
    // Warnings go to stderr: a stale edit is skipped rather than failing the whole file.
    let mut warnings = Vec::new();
    apply_texture_additions(&mut namco, &eff.textures_added, &mut warnings)?;
    {
        let ptcl = namco
            .ptcl_file
            .as_mut()
            .ok_or_else(|| anyhow!("source .eff has no embedded PTCL"))?;
        // Emitter lists first: they decide which emitters EXIST, and an authored edit addresses
        // one of the survivors by name and index. The other order would apply edits to emitters
        // the roster is about to drop, and miss the duplicates it is about to add.
        for roster in &eff.rosters {
            apply_roster(ptcl, roster)?;
        }
        for edit in &eff.authored {
            apply_authored(ptcl, edit)?;
        }
    }
    // Spawn structure last of the eff's own edits: it resolves emitter sets by name, so every
    // set a transplant was going to add is in place by now.
    for edit in &eff.entry_edits {
        apply_entry_edit(&mut namco, edit)?;
    }
    // Apply this after spawn edits too: an entry edit can deliberately repoint a kind or part
    // at the roster-cleared set, and that must not reintroduce the unsafe zero-emitter handle.
    clear_empty_emitter_set_references(&mut namco);
    apply_texture_imports(&mut namco, &eff.textures, &mut warnings)?;
    apply_texture_removals(&mut namco, &eff.textures_removed, &mut warnings)?;
    for warning in &warnings {
        eprintln!("[forge] warning: {warning}");
    }
    namco
        .save()
        .context("effect_library failed to re-encode the .eff")
}

/// Reduce an eff to the named entries plus only the resources those entries reference
/// (`effect_forge stage` with `keep_prefixes`: one character per slotted file).
///
/// Entries and sets are kept and merely emptied, never deleted, so every id still resolves —
/// deleting an entry leaves the game's own spawn requests for it resolving against nothing.
pub fn keep_only_entries(src: &[u8], keep_entries: &[&str]) -> Result<Vec<u8>> {
    let mut file = effect_library::NamcoEffectFile::load(src)
        .context("effect_library failed to parse the .eff")?;
    empty_other_effects(&mut file, keep_entries);
    clear_empty_emitter_set_references(&mut file);
    prune_unreferenced_resources(&mut file)?;
    compact_shader_containers(&mut file)?;
    file.save()
        .context("effect_library failed to encode the reduced .eff")
}

/// The file with effect `old` called `new`. Only the name changes: its layers, and the sets
/// they live in, stay as they are.
pub fn rename_entry(src: &[u8], old: &str, new: &str) -> Result<Vec<u8>> {
    let mut file = effect_library::NamcoEffectFile::load(src)
        .context("effect_library failed to parse the .eff")?;
    let at = file.entry_names.iter().position(|n| n.eq_ignore_ascii_case(old))
        .ok_or_else(|| anyhow!("no effect called {old}"))?;
    if file.entry_names.iter().enumerate().any(|(i, n)| i != at && n.eq_ignore_ascii_case(new)) {
        return Err(anyhow!("there is already an effect called {new}"));
    }
    file.entry_names[at] = new.to_string();
    file.save().context("effect_library failed to re-encode the .eff")
}

/// Read-only recursive emitter visit (children after parent).
pub(super) fn visit_emitters_ref<F: FnMut(&effect_library::structs::Emitter)>(
    emitters: &[effect_library::structs::Emitter],
    f: &mut F,
) {
    for em in emitters {
        f(em);
        visit_emitters_ref(&em.children, f);
    }
}

/// Depth-first, parent-before-children traversal with a running flat index.
pub(super) fn visit_emitters<F: FnMut(usize, &mut effect_library::structs::Emitter)>(
    emitters: &mut [effect_library::structs::Emitter],
    idx: &mut usize,
    f: &mut F,
) {
    for em in emitters.iter_mut() {
        f(*idx, em);
        *idx += 1;
        visit_emitters(&mut em.children, idx, f);
    }
}
