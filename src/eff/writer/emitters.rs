//! Emitter edits: field values, emitter lists (rosters) and what an entry spawns.

use anyhow::{anyhow, Result};

use super::{visit_emitters, visit_emitters_ref};
use crate::eff::edits::{AuthoredEdit, EmitterFieldEdits};

pub(super) fn apply_authored(ptcl: &mut effect_library::PtclFile, edit: &AuthoredEdit) -> Result<()> {
    // A texture swap names the pool texture; the emitter addresses textures by GUID. Resolve
    // it here, while the descriptor table is still reachable — `sets` borrows `ptcl` mutably
    // below.
    let swap_to = match edit.fields.texture_name.as_deref() {
        Some(wanted) => {
            let descriptors = ptcl
                .texture_info
                .as_ref()
                .map(|info| info.descriptors.as_slice())
                .unwrap_or_default();
            let id = descriptors.iter().find(|d| d.name == wanted).map(|d| d.id);
            if id.is_none() {
                // Dropping the swap leaves the original texture — wrong, but it still renders.
                // Writing a GUID no descriptor holds would make the emitter sample nothing.
                eprintln!(
                    "[forge] warning: texture swap on '{}' wants '{wanted}', which this \
                     pool does not hold — swap skipped",
                    edit.emitter_name
                );
            }
            id
        }
        None => None,
    };

    let sets = &mut ptcl.emitter_list.emitter_sets;
    // Scope resolution is deliberately strict: an edit names ONE set and ONE emitter, so
    // when the stored index already holds the stored name that pair wins outright. Falling
    // straight to `position()` retargeted the edit at the FIRST set of that name, which a
    // transplant clone of a multi-variant entry can easily duplicate.
    let set_named = |s: &effect_library::structs::EmitterSet| {
        !edit.set_name.is_empty() && s.name == edit.set_name
    };
    let set_idx = if sets.get(edit.set_idx).map(set_named).unwrap_or(false) {
        edit.set_idx
    } else {
        sets.iter().position(set_named).unwrap_or(edit.set_idx)
    };
    let set = sets.get_mut(set_idx).ok_or_else(|| {
        anyhow!(
            "emitter set '{}' (idx {}) not found in source eff",
            edit.set_name,
            edit.set_idx
        )
    })?;
    if !edit.set_name.is_empty() && set.name != edit.set_name {
        eprintln!(
            "[forge] warning: set name mismatch ('{}' vs '{}') — applying by index {}",
            set.name, edit.set_name, set_idx
        );
    }

    // Resolve the target emitter FIRST over a read-only, flat parent-first traversal (the
    // same order the panel lists them), then write to exactly that one index. Same rule as
    // the set above: stored-name-at-stored-index wins, so identically-named sibling emitters
    // can never pull an edit off the emitter the user actually selected.
    let mut names: Vec<String> = Vec::new();
    visit_emitters_ref(&set.emitters, &mut |em| names.push(em.data.display_name()));
    let named = |i: usize| {
        !edit.emitter_name.is_empty() && names.get(i).map(|n| *n == edit.emitter_name) == Some(true)
    };
    let target = if named(edit.emitter_idx) {
        Some(edit.emitter_idx)
    } else if let Some(i) = (0..names.len()).find(|i| named(*i)) {
        eprintln!(
            "[forge] warning: emitter '{}' of set '{}' moved from index {} to {}",
            edit.emitter_name, set.name, edit.emitter_idx, i
        );
        Some(i)
    } else if edit.emitter_idx < names.len() {
        if !edit.emitter_name.is_empty() {
            eprintln!(
                "[forge] warning: emitter '{}' not found by name in set '{}' — using index {}",
                edit.emitter_name, set.name, edit.emitter_idx
            );
        }
        Some(edit.emitter_idx)
    } else {
        None
    };
    let target = target.ok_or_else(|| {
        anyhow!(
            "emitter '{}' (idx {}) not found in set '{}'",
            edit.emitter_name,
            edit.emitter_idx,
            set.name
        )
    })?;

    let mut flat_idx = 0usize;
    visit_emitters(&mut set.emitters, &mut flat_idx, &mut |idx, em| {
        if idx == target {
            apply_fields(em, &edit.fields);
            if let Some(id) = swap_to {
                if let Some(sampler) = em.data.sampler0.as_mut() {
                    sampler.texture_id = id;
                }
            }
        }
    });
    Ok(())
}

pub(super) fn apply_fields(em: &mut effect_library::structs::Emitter, f: &EmitterFieldEdits) {
    let d = &mut em.data;
    for (id, value) in &f.attrs {
        if !crate::eff::fields::write(d, id, *value) {
            eprintln!(
                "[forge] warning: emitter '{}' carries an edit to unknown attribute '{id}' \
                 — skipped",
                d.display_name()
            );
        }
    }
    // Serializer prefers the cached EMTR blob; clearing it forces a re-encode of `data`.
    em.cached_binary = None;
}

/// Rebuild one emitter set's emitter list from a roster: which emitters play, in what order,
/// nested how.
///
/// Every slot clones an emitter of the set AS IT STANDS, so a roster is idempotent — the source
/// emitters are read into a flat list first and the set is then rebuilt from that, which is what
/// makes "duplicate emitter 2" mean the same thing however many times it is applied.
///
/// Children are re-parented by depth rather than kept with their original parent: the panel
/// presents the set as a flat, parent-first list, and that is the only ordering an edit can be
/// expressed against.
pub(super) fn apply_roster(
    ptcl: &mut effect_library::PtclFile,
    roster: &crate::eff::edits::EmitterRoster,
) -> Result<()> {
    let sets = &mut ptcl.emitter_list.emitter_sets;
    let set_named = |s: &effect_library::structs::EmitterSet| {
        !roster.set_name.is_empty() && s.name == roster.set_name
    };
    let set_idx = if sets.get(roster.set_idx).map(set_named).unwrap_or(false) {
        roster.set_idx
    } else {
        sets.iter().position(set_named).unwrap_or(roster.set_idx)
    };
    let sets_snapshot: Vec<effect_library::structs::EmitterSet> = if roster.slots.iter().any(|s| !s.source_set.is_empty()) {
        sets.clone()
    } else {
        Vec::new()
    };
    let set = sets.get_mut(set_idx).ok_or_else(|| {
        anyhow!(
            "emitter set '{}' (idx {}) not found — cannot apply its emitter list",
            roster.set_name,
            roster.set_idx
        )
    })?;

    // Flatten the source, parent-first, keeping each emitter's own subsections but dropping the
    // nesting — the roster restates it.
    let mut flat: Vec<effect_library::structs::Emitter> = Vec::new();
    fn flatten(
        src: &[effect_library::structs::Emitter],
        out: &mut Vec<effect_library::structs::Emitter>,
    ) {
        for em in src {
            let mut copy = em.clone();
            copy.children = Vec::new();
            out.push(copy);
            flatten(&em.children, out);
        }
    }
    flatten(&set.emitters, &mut flat);
    let names: Vec<String> = flat.iter().map(|em| em.data.display_name()).collect();

    // Other sets slots borrow from, flattened the same way.
    let mut borrowed: std::collections::HashMap<String, (Vec<effect_library::structs::Emitter>, Vec<String>)> =
        Default::default();
    for slot in &roster.slots {
        if slot.source_set.is_empty() || borrowed.contains_key(&slot.source_set) {
            continue;
        }
        let other = sets_snapshot
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(&slot.source_set))
            .ok_or_else(|| anyhow!("emitter list for '{}' borrows from set '{}', which the file does not have", roster.set_name, slot.source_set))?;
        let mut other_flat = Vec::new();
        flatten(&other.emitters, &mut other_flat);
        let other_names = other_flat.iter().map(|em| em.data.display_name()).collect();
        borrowed.insert(slot.source_set.clone(), (other_flat, other_names));
    }

    let mut built: Vec<effect_library::structs::Emitter> = Vec::new();
    let mut depths: Vec<u8> = Vec::new();
    for slot in &roster.slots {
        let (flat, names) = match borrowed.get(&slot.source_set) {
            Some((other_flat, other_names)) => (other_flat, other_names),
            None => (&flat, &names),
        };
        // Same resolution rule as an authored edit: the stored name at the stored index wins
        // outright, then the name anywhere, then the bare index.
        let named =
            |i: usize| !slot.source_name.is_empty() && names.get(i) == Some(&slot.source_name);
        let source = if named(slot.source_idx) {
            Some(slot.source_idx)
        } else if let Some(i) = (0..names.len()).find(|i| named(*i)) {
            Some(i)
        } else if slot.source_idx < flat.len() {
            Some(slot.source_idx)
        } else {
            None
        };
        let Some(source) = source else {
            eprintln!(
                "[forge] warning: emitter list for '{}' names a source emitter '{}' \
                 (idx {}) this set does not have — slot dropped",
                roster.set_name, slot.source_name, slot.source_idx
            );
            continue;
        };
        let mut emitter = flat[source].clone();
        if !slot.name.is_empty() && slot.name != names[source] {
            set_emitter_name(&mut emitter.data, &slot.name);
            // The cached EMTR blob still spells the old name; drop it so the rename is encoded.
            emitter.cached_binary = None;
        }
        built.push(emitter);
        depths.push(slot.depth);
    }

    if built.is_empty() && !roster.slots.is_empty() {
        return Err(anyhow!(
            "emitter list for '{}' resolved to no emitters at all",
            roster.set_name
        ));
    }

    set.emitters = nest_by_depth(built, &depths);
    Ok(())
}

/// Apply one entry's spawn-structure edit: which parts play at which frame, and which external
/// model comes with the effect.
///
/// This is header data, not particle data, so it is written against the whole `NamcoEffectFile`
/// rather than the PTCL. Both tables it touches are shared by every entry in the file and indexed
/// by 1-based handles, so nothing is edited in place: the variants are APPENDED and the entry is
/// repointed at the new run. The old run is left where it is — some other entry may still point
/// into it, and the file's own writer recomputes the counts from the vectors' lengths.
pub(super) fn apply_entry_edit(
    namco: &mut effect_library::NamcoEffectFile,
    edit: &crate::eff::edits::EntryEdit,
) -> Result<()> {
    let entry_idx = namco
        .entry_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(&edit.entry_name))
        .ok_or_else(|| {
            anyhow!(
                "spawn edit names entry '{}', which this eff does not have",
                edit.entry_name
            )
        })?;

    if let Some(set_name) = &edit.emitter_set {
        let set_id = if set_name.is_empty() {
            0
        } else {
            let idx = namco
                .ptcl_file
                .as_ref()
                .and_then(|p| {
                    p.emitter_list
                        .emitter_sets
                        .iter()
                        .position(|s| s.name == *set_name)
                })
                .ok_or_else(|| {
                    anyhow!(
                        "entry '{}' names primary emitter set '{}', which this eff does not have",
                        edit.entry_name,
                        set_name
                    )
                })?;
            u32::try_from(idx + 1)
                .map_err(|_| anyhow!("emitter set index {idx} does not fit a 32-bit id"))?
        };
        namco.entries[entry_idx].emitter_set_id = set_id;
    }

    if let Some(variants) = &edit.variants {
        // Resolve every set name BEFORE touching the file: a typo should leave the entry as it
        // was, not half-rewritten with the parts that happened to resolve.
        let resolved: Vec<(u16, u16, String)> = variants
            .iter()
            .map(|v| {
                let set_id = if v.set_name.is_empty() {
                    0u16
                } else {
                    let idx = namco
                        .ptcl_file
                        .as_ref()
                        .and_then(|p| {
                            p.emitter_list
                                .emitter_sets
                                .iter()
                                .position(|s| s.name == v.set_name)
                        })
                        .ok_or_else(|| {
                            anyhow!(
                                "part of '{}' names emitter set '{}', which this eff does not have",
                                edit.entry_name,
                                v.set_name
                            )
                        })?;
                    u16::try_from(idx + 1).map_err(|_| {
                        anyhow!("emitter set index {idx} does not fit an effect part's 16-bit id")
                    })?
                };
                Ok((v.start_frame, set_id, v.bone.clone()))
            })
            .collect::<Result<_>>()?;

        if resolved.is_empty() {
            namco.entries[entry_idx].variant_start_idx = 0;
            namco.entries[entry_idx].variant_count = 0;
        } else {
            let start = u16::try_from(namco.effect_variants.len() + 1)
                .map_err(|_| anyhow!("this eff already holds the most parts the format allows"))?;
            for (start_frame, emitter_set_id, bone) in resolved {
                namco
                    .effect_variants
                    .push(effect_library::namco_file::EffectVariant {
                        start_frame,
                        emitter_set_id,
                    });
                // The bone table runs parallel to the variant table — one name per part — so it
                // has to grow in lockstep or every later part reads the wrong bone.
                namco.external_bone_names.push(bone);
            }
            let count = namco.effect_variants.len() + 1 - start as usize;
            namco.entries[entry_idx].variant_start_idx = start;
            namco.entries[entry_idx].variant_count = count as u16;
        }
    }

    if let Some(model) = &edit.model {
        if model.name.is_empty() {
            namco.entries[entry_idx].external_model_idx = 0;
        } else {
            // A model row may be shared by several entries. Reusing a matching name with a
            // different flag would change all of them, even though this edit names one entry.
            // Reuse only an exact pair; otherwise append a private row for this entry.
            let idx = namco
                .external_model_names
                .iter()
                .zip(&namco.effect_models)
                .position(|(name, flag)| name == &model.name && *flag == model.flag)
                .unwrap_or_else(|| {
                    namco.external_model_names.push(model.name.clone());
                    namco.effect_models.push(model.flag);
                    namco.external_model_names.len() - 1
                });
            namco.entries[entry_idx].external_model_idx = idx as u32 + 1;
        }
    }
    Ok(())
}

/// Rename an emitter, writing whichever of the two name fields this file's version uses.
///
/// The emitter carries its name in a 64-byte slot below vfx version 40 and a 96-byte one above,
/// and `effect_library` exposes them as separate fields — `display_name` prefers the v40 one.
/// Both are written when both exist, so the name is the same whichever the writer picks. The cap
/// is one byte short of each slot so the string keeps its terminator.
pub(super) fn set_emitter_name(data: &mut effect_library::EmitterData, name: &str) {
    fn clip(name: &str, max: usize) -> String {
        name.char_indices()
            .take_while(|(i, c)| i + c.len_utf8() <= max)
            .map(|(_, c)| c)
            .collect()
    }
    if data.namev40.is_some() {
        data.namev40 = Some(clip(name, 95));
    }
    if data.name.is_some() || data.namev40.is_none() {
        data.name = Some(clip(name, 63));
    }
}

/// Rebuild a tree from a parent-first list and its depths. A slot deeper than the one before it
/// by more than one step is clamped to "child of the previous emitter", because there is no
/// emitter at the depth it asks for — that is a malformed roster, not a new level.
pub(super) fn nest_by_depth(
    emitters: Vec<effect_library::structs::Emitter>,
    depths: &[u8],
) -> Vec<effect_library::structs::Emitter> {
    let mut roots: Vec<effect_library::structs::Emitter> = Vec::new();
    // Path of indices from the root down to the emitter last placed at each depth.
    let mut path: Vec<usize> = Vec::new();
    for (emitter, &depth) in emitters.into_iter().zip(depths) {
        let depth = (depth as usize).min(path.len());
        path.truncate(depth);
        let mut level = &mut roots;
        for &step in &path {
            level = &mut level[step].children;
        }
        level.push(emitter);
        path.push(level.len() - 1);
    }
    roots
}
