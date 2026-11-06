//! Keeping a file to what it uses: unreachable emitter sets, unreferenced textures and models, and shader variations
//! no emitter addresses.

use anyhow::{anyhow, Context, Result};

use super::{visit_emitters, visit_emitters_ref};

/// Empty every emitter set the `keep_entries` cannot spawn, leaving the other effects as
/// resolvable-but-silent entries. Clearing their emitters makes their textures, primitives and
/// shader variations collectable by the passes that follow.
///
/// The entries and the sets themselves stay: a name that resolves to an empty set yields a valid
/// handle that emits nothing, whereas deleting the entry would leave requests for it resolving
/// against nothing at all.
pub(super) fn empty_other_effects(file: &mut effect_library::NamcoEffectFile, keep_entries: &[&str]) {
    let mut keep = std::collections::HashSet::new();
    for (index, _) in file.entry_names.iter().enumerate().filter(|(_, name)| {
        keep_entries
            .iter()
            .any(|new| new.eq_ignore_ascii_case(name))
    }) {
        // Every set the entry can spawn, not just `emitter_set_id` — a multi-part effect
        // reaches its content only through its variants, and `apply_transplant_cross_file` leaves
        // `emitter_set_id` at the DONOR's value for those.
        entry_emitter_sets(file, index, &mut keep);
    }
    let Some(ptcl) = file.ptcl_file.as_mut() else {
        return;
    };
    for (index, set) in ptcl.emitter_list.emitter_sets.iter_mut().enumerate() {
        if !keep.contains(&index) {
            set.emitters.clear();
        }
    }
}

/// Take the effects named in `drop` out of the file entirely: entry, name, the parts (and their bone names) no other
/// effect shares, and every emitter set no remaining effect can spawn. The 1-based handles that stay (an entry's set
/// and first part, a part's set) are renumbered to match.
pub(super) fn remove_entries(file: &mut effect_library::NamcoEffectFile, drop: &[&str]) -> Result<()> {
    let gone: Vec<bool> = file.entry_names.iter().map(|n| drop.iter().any(|d| d.eq_ignore_ascii_case(n))).collect();
    if !gone.contains(&true) {
        return Ok(());
    }
    // the parts the remaining entries still use
    let mut part_used = vec![false; file.effect_variants.len()];
    for (entry, _) in file.entries.iter().zip(&gone).filter(|(_, g)| !**g) {
        if entry.variant_count == 0 {
            continue;
        }
        let start = (entry.variant_start_idx as usize).saturating_sub(1);
        for used in part_used.iter_mut().skip(start).take(entry.variant_count as usize) {
            *used = true;
        }
    }
    let mut part_new = vec![0usize; part_used.len()];
    let mut next = 0;
    for (i, used) in part_used.iter().enumerate() {
        if *used {
            next += 1;
            part_new[i] = next;
        }
    }
    let mut keep_parts = part_used.iter();
    file.effect_variants.retain(|_| *keep_parts.next().unwrap());
    if file.external_bone_names.len() == part_used.len() {
        let mut keep_bones = part_used.iter();
        file.external_bone_names.retain(|_| *keep_bones.next().unwrap());
    }
    let mut keep_entries = gone.iter();
    file.entries.retain(|_| !*keep_entries.next().unwrap());
    let mut keep_names = gone.iter();
    file.entry_names.retain(|_| !*keep_names.next().unwrap());
    for entry in &mut file.entries {
        if entry.variant_count > 0 {
            let start = (entry.variant_start_idx as usize).saturating_sub(1);
            entry.variant_start_idx = *part_new.get(start).ok_or_else(|| anyhow!("a part out of range"))? as u16;
        }
    }

    // the sets the remaining effects can spawn; the rest go and the handles close up
    let live = live_emitter_sets(file);
    let Some(ptcl) = file.ptcl_file.as_mut() else {
        return Ok(());
    };
    let sets = ptcl.emitter_list.emitter_sets.len();
    let mut set_new = vec![0u32; sets];
    let mut next = 0;
    for (i, slot) in set_new.iter_mut().enumerate() {
        if live.contains(&i) {
            next += 1;
            *slot = next;
        }
    }
    let mut index = 0;
    ptcl.emitter_list.emitter_sets.retain(|_| {
        index += 1;
        live.contains(&(index - 1))
    });
    let renumber = |id: u32| -> u32 { if id == 0 { 0 } else { set_new.get(id as usize - 1).copied().unwrap_or(0) } };
    for entry in &mut file.entries {
        entry.emitter_set_id = renumber(entry.emitter_set_id);
    }
    for part in &mut file.effect_variants {
        part.emitter_set_id = renumber(part.emitter_set_id as u32) as u16;
    }
    Ok(())
}

/// Collect the 0-based emitter-set indices entry `index` can spawn into `out`.
///
/// A single-part entry names one set through `emitter_set_id`. A multi-part entry names one per
/// variant AND keeps its own `emitter_set_id` live — 13 game entries point it at a populated set
/// that appears in no variant, so both are collected (see
/// `every_game_emitter_set_is_reachable_from_the_entry_table`). Ids are 1-based handles; 0 means
/// none.
pub(super) fn entry_emitter_sets(
    file: &effect_library::NamcoEffectFile,
    index: usize,
    out: &mut std::collections::HashSet<usize>,
) {
    let Some(entry) = file.entries.get(index) else {
        return;
    };
    out.extend((entry.emitter_set_id as usize).checked_sub(1));
    if entry.variant_count == 0 {
        return;
    }
    let start = (entry.variant_start_idx as usize).saturating_sub(1);
    out.extend(
        file.effect_variants
            .iter()
            .skip(start)
            .take(entry.variant_count as usize)
            .filter_map(|variant| (variant.emitter_set_id as usize).checked_sub(1)),
    );
}

/// The 0-based indices of every emitter set some entry can spawn.
///
/// Emitter sets are addressable ONLY through the entry table, so a set outside this index can
/// never play. Every set in every one of the 328 game files is reachable, which is what makes
/// clearing the rest safe: the only way to produce an unreachable set is for an edit to orphan
/// one, and replace-mode transplants do exactly that.
pub(super) fn live_emitter_sets(file: &effect_library::NamcoEffectFile) -> std::collections::HashSet<usize> {
    let mut live = std::collections::HashSet::new();
    for index in 0..file.entries.len() {
        entry_emitter_sets(file, index, &mut live);
    }
    live
}

/// Empty every emitter set no entry can reach.
///
/// Replace mode repoints an entry at freshly cloned sets and leaves the set it used to name
/// behind, emitters intact (`finish_transplant_entry`). Nothing can spawn that set again, but the
/// passes that collect resources scan sets rather than entries, so an orphan goes on pinning its
/// textures and its share of the donor's shader library. Clearing orphans first is what lets
/// those passes stay simple and still see the truth.
///
/// The sets themselves are kept and merely emptied, exactly as
/// [`empty_other_effects`] keeps the others: every id in the file still
/// resolves, and an empty set is a valid handle that emits nothing.
pub(super) fn clear_unreachable_emitter_sets(file: &mut effect_library::NamcoEffectFile) {
    let live = live_emitter_sets(file);
    let Some(ptcl) = file.ptcl_file.as_mut() else {
        return;
    };
    for (index, set) in ptcl.emitter_list.emitter_sets.iter_mut().enumerate() {
        if !live.contains(&index) {
            set.emitters.clear();
        }
    }
}

/// An empty ESET is not a safe game-side no-op: the runtime's set loader can consume the next
/// ESET's first emitter when the referenced set has no children.  Entries and variants use
/// 1-based set handles, with zero already defined as "none", so detach those handles while
/// retaining the ESET and entry name.  Retaining the tables preserves all other indices and
/// makes the operation idempotent across repeated rebuilds.
pub(super) fn clear_empty_emitter_set_references(file: &mut effect_library::NamcoEffectFile) {
    let Some(ptcl) = file.ptcl_file.as_ref() else {
        return;
    };
    let empty: std::collections::HashSet<u32> = ptcl
        .emitter_list
        .emitter_sets
        .iter()
        .enumerate()
        .filter(|(_, set)| set.emitters.is_empty())
        .map(|(index, _)| index as u32 + 1)
        .collect();
    if empty.is_empty() {
        return;
    }
    for entry in &mut file.entries {
        if empty.contains(&entry.emitter_set_id) {
            entry.emitter_set_id = 0;
        }
    }
    for variant in &mut file.effect_variants {
        if empty.contains(&(variant.emitter_set_id as u32)) {
            variant.emitter_set_id = 0;
        }
    }
}

/// Drop every texture and primitive no surviving emitter samples.
///
/// Both pools are GUID-keyed, so dropping an entry moves nothing: the emitters that remain keep
/// referring to exactly the resources they already did. Shader variations are index-keyed and so
/// need renumbering — [`compact_shader_containers`] handles those separately.
///
/// Whole resources go or stay; the ones that stay are not slimmed, and that is where the
/// remaining headroom is. A stripped Kirby donor is 3.12 MB, of which 2.84 MB is 13 textures this
/// pass has already proved are all sampled — and all 13 carry full mip chains (112 levels between
/// them), roughly a quarter of those bytes. Dropping mips is therefore the largest lever left, and
/// it is deliberately not pulled: mip count is part of what the sampler descriptors expect, so a
/// short chain is a change to what the GPU reads rather than to what the file merely holds. That
/// is the class of change this format punishes with a pool that validates and draws nothing, so it
/// needs on-hardware confirmation and belongs behind a user's choice, not in every build.
pub(super) fn prune_unreferenced_resources(file: &mut effect_library::NamcoEffectFile) -> Result<()> {
    let Some(ptcl) = file.ptcl_file.as_mut() else {
        return Ok(());
    };
    let mut used_textures: Vec<u64> = Vec::new();
    let mut used_primitives: Vec<u64> = Vec::new();
    for set in &ptcl.emitter_list.emitter_sets {
        visit_emitters_ref(&set.emitters, &mut |em| {
            let d = &em.data;
            append_unique_resource_ids(
                &mut used_textures,
                [
                    d.sampler0.as_ref().map(|s| s.texture_id),
                    d.sampler1.as_ref().map(|s| s.texture_id),
                    d.sampler2.as_ref().map(|s| s.texture_id),
                    d.sampler3.as_ref().map(|s| s.texture_id),
                    d.sampler4.as_ref().map(|s| s.texture_id),
                    d.sampler5.as_ref().map(|s| s.texture_id),
                ],
            );
            append_unique_resource_ids(
                &mut used_primitives,
                [
                    Some(d.particle_data.primitive_id),
                    Some(d.particle_data.primitive_ex_id),
                    Some(d.shape_info.primitive_index),
                ],
            );
        });
    }

    if let Some(textures) = ptcl.texture_info.as_mut() {
        // Descriptor order matches BNTX texture order: the serializer sorts the descriptor table
        // and reorders the archive to match (`reorder_and_save`), so index i addresses both.
        let keep: Vec<usize> = (0..textures.descriptors.len())
            .filter(|index| used_textures.contains(&textures.descriptors[*index].id))
            .collect();
        if keep.len() < textures.descriptors.len() {
            let binary = textures
                .binary_data
                .as_ref()
                .ok_or_else(|| anyhow!("the file has texture descriptors but no BNTX"))?;
            let scratch = crate::scratch_dirs::app_scratch_dir("prune-tex")?;
            let mut blobs = Vec::with_capacity(keep.len());
            for index in &keep {
                let name = textures.descriptors[*index].name.clone();
                let path = scratch.path().join(format!("{name}.bntx"));
                effect_library::bntx::export_single_texture(binary, *index, &name, &path)
                    .with_context(|| format!("extracting texture '{name}'"))?;
                blobs.push(std::fs::read(&path)?);
            }
            textures.binary_data = match blobs.len() {
                0 => None,
                1 => blobs.pop(),
                _ => Some(
                    effect_library::bntx::merge_texture_files(&blobs)
                        .context("repacking the texture pool")?,
                ),
            };
            let kept: std::collections::HashSet<usize> = keep.into_iter().collect();
            let mut index = 0;
            textures.descriptors.retain(|_| {
                index += 1;
                kept.contains(&(index - 1))
            });
        }
    }

    {
        let Some(primitives) = ptcl.primitive_info.as_mut() else {
            return Ok(());
        };
        let keep: Vec<usize> = (0..primitives.descriptors.len())
            .filter(|index| used_primitives.contains(&primitives.descriptors[*index].id))
            .collect();
        if keep.len() < primitives.descriptors.len() {
            let binary = primitives
                .binary_data
                .as_ref()
                .ok_or_else(|| anyhow!("the file has primitive descriptors but no BFRES"))?;
            let mut blobs = Vec::with_capacity(keep.len());
            for index in &keep {
                blobs.push(
                    effect_library::bfres::export_single_model(binary, *index).with_context(
                        || {
                            format!(
                                "extracting primitive {:#x}",
                                primitives.descriptors[*index].id
                            )
                        },
                    )?,
                );
            }
            primitives.binary_data = match blobs.len() {
                0 => None,
                1 => blobs.pop(),
                _ => Some(
                    effect_library::bfres::ResFile::merge_model_files(&blobs)
                        .context("repacking the primitive pool")?,
                ),
            };
            let kept: std::collections::HashSet<usize> = keep.into_iter().collect();
            let mut index = 0;
            primitives.descriptors.retain(|_| {
                index += 1;
                kept.contains(&(index - 1))
            });
        }
    }
    Ok(())
}

/// Drop every shader variation no emitter addresses, then renumber the emitters onto the
/// compacted containers.
///
/// A donor's BNSH holds its whole fighter's shader library — Pickel ships 370 variations and a
/// single transplanted effect uses twelve of them — and once merged that library is the largest
/// thing in the file. Keeping only what is referenced is what keeps a transplant from bringing a
/// whole second library with it.
///
/// Repacking variations was tried once before and produced handles with no pixels, which is why
/// the containers used to be transferred whole. That has since been diagnosed: `BnshFile::write`
/// emitted a relocation table listing slots that hold no offset, so the runtime turned each into
/// a pointer to the image base and every container the writer touched was corrupt. With the
/// writer fixed it reproduces the game's own pointer set exactly, so repacking is sound again.
pub(super) fn compact_shader_containers(file: &mut effect_library::NamcoEffectFile) -> Result<()> {
    let Some(ptcl) = file.ptcl_file.as_mut() else {
        return Ok(());
    };
    let mut used_standard: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
    let mut used_compute: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
    for set in &ptcl.emitter_list.emitter_sets {
        visit_emitters_ref(&set.emitters, &mut |em| {
            let refs = &em.data.shader_references;
            for index in [
                refs.shader_index,
                refs.user_shader_index1,
                refs.user_shader_index2,
            ] {
                if index >= 0 {
                    used_standard.insert(index);
                }
            }
            if refs.compute_shader_index >= 0 {
                used_compute.insert(refs.compute_shader_index);
            }
        });
    }

    let (standard_map, compute_map) = {
        let Some(shader) = ptcl.shader_info.as_mut() else {
            return Ok(());
        };
        (
            subset_container(shader.binary_data.as_mut(), &used_standard, "shader")?,
            subset_container(
                shader.compute_binary.as_mut(),
                &used_compute,
                "compute-shader",
            )?,
        )
    };

    for set in &mut ptcl.emitter_list.emitter_sets {
        let name = set.name.clone();
        let mut failed = None;
        visit_emitters(&mut set.emitters, &mut 0, &mut |_, em| {
            let refs = &mut em.data.shader_references;
            for (index, map, kind) in [
                (&mut refs.shader_index, &standard_map, "shader"),
                (&mut refs.user_shader_index1, &standard_map, "shader"),
                (&mut refs.user_shader_index2, &standard_map, "shader"),
                (
                    &mut refs.compute_shader_index,
                    &compute_map,
                    "compute-shader",
                ),
            ] {
                if *index < 0 || map.is_empty() {
                    continue;
                }
                match map.get(index) {
                    Some(new) => *index = *new,
                    None => failed = Some((kind, *index)),
                }
            }
            // `custom_shader_index` is deliberately untouched: it is a small custom-shader mode,
            // not an index into the variation array.
            em.cached_binary = None; // indices moved → the cached EMTR blob is stale
        });
        if let Some((kind, index)) = failed {
            anyhow::bail!(
                "compacting shader containers: '{name}' addresses {kind} variation {index}, \
                 which the merged container does not hold"
            );
        }
    }
    Ok(())
}

/// Rewrite `binary` keeping only the listed variations, returning the old-index → new-index map.
/// An empty map means the container was left exactly as it was and its indices still apply.
///
/// Identical variations are NOT folded together, though they exist: a donor's own library is full
/// of them (225 of Pickel's 370 are byte-identical to another), and the map returned here is
/// exactly the mechanism that could collapse them. It is not worth it. Compaction runs first and
/// leaves 12–27 variations, of which 0–6 are duplicates, and folding those saves a flat ~8.8 KB —
/// 2.2% of a one-donor file, 0.2% of a three-donor one. That buys very little in exchange for
/// changing which compiled program an emitter binds to, in a format whose characteristic failure
/// is a pool that is structurally perfect and draws nothing on hardware.
pub(super) fn subset_container(
    binary: Option<&mut Vec<u8>>,
    used: &std::collections::BTreeSet<i32>,
    kind: &str,
) -> Result<std::collections::HashMap<i32, i32>> {
    let Some(binary) = binary else {
        return Ok(std::collections::HashMap::new());
    };
    // An unused container is left alone rather than removed: the engine-global single-variation
    // compute container is the shape every file the game has loaded was built with.
    if used.is_empty() || binary.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut file = effect_library::bnsh::BnshFile::read(binary)
        .with_context(|| format!("reading the merged {kind} container"))?;
    // Already exactly 0..n with nothing to drop — leave the bytes untouched rather than putting
    // a game-authored container through the writer for no gain.
    if used.len() == file.variations.len()
        && used.iter().enumerate().all(|(i, old)| *old as usize == i)
    {
        return Ok(std::collections::HashMap::new());
    }
    let mut kept = Vec::with_capacity(used.len());
    let mut map = std::collections::HashMap::with_capacity(used.len());
    for old in used {
        let variation = file
            .variations
            .get(*old as usize)
            .ok_or_else(|| anyhow!("an emitter addresses {kind} variation {old}, which the merged container does not hold"))?
            .clone();
        map.insert(*old, kept.len() as i32);
        kept.push(variation);
    }
    file.variations = kept;
    *binary = file.write();
    Ok(map)
}

/// Refuse to encode a file in which some emitter addresses a shader variation its containers do
/// not hold.
///
/// This is the last line before bytes reach the game, and it is deliberately a hard failure:
/// an out-of-range variation index hangs the game's loader rather than drawing wrong, so there is
/// nothing to be gained by shipping and seeing. `subject` names what was being built.
///
/// Runs AFTER compaction and independently of it — compaction renumbers indices through a map it
/// built itself, and this checks the result against the container that actually shipped.
pub(super) fn verify_shader_indices_resolve(
    file: &effect_library::NamcoEffectFile,
    subject: &str,
) -> Result<()> {
    let shader = file
        .ptcl_file
        .as_ref()
        .and_then(|ptcl| ptcl.shader_info.as_ref());
    let count = |binary: Option<&Vec<u8>>| match binary {
        Some(bnsh) => effect_library::bnsh::BnshFile::read(bnsh)
            .map(|file| file.variations.len())
            .unwrap_or(0),
        None => 0,
    };
    let (standard_variations, compute_variations) = match shader {
        Some(shader) => (
            count(shader.binary_data.as_ref()),
            count(shader.compute_binary.as_ref()),
        ),
        None => (0, 0),
    };
    let Some(ptcl) = file.ptcl_file.as_ref() else {
        return Ok(());
    };
    for set in &ptcl.emitter_list.emitter_sets {
        let mut bad: Option<(&'static str, i32, usize)> = None;
        visit_emitters_ref(&set.emitters, &mut |em| {
            let refs = &em.data.shader_references;
            for index in [
                refs.shader_index,
                refs.user_shader_index1,
                refs.user_shader_index2,
            ] {
                if !shader_index_fits(index, standard_variations) {
                    bad = Some(("shader", index, standard_variations));
                }
            }
            if !shader_index_fits(refs.compute_shader_index, compute_variations) {
                bad = Some((
                    "compute-shader",
                    refs.compute_shader_index,
                    compute_variations,
                ));
            }
        });
        if let Some((kind, index, available)) = bad {
            anyhow::bail!(
                "{subject} safety: '{}' addresses {kind} variation {index}, but the container \
                 only holds {available}. Shipping that would hang the game's loader, so nothing \
                 was written.",
                set.name
            );
        }
    }
    Ok(())
}

/// A negative index is the "no shader" sentinel; anything else must address a variation the
/// container actually holds.
pub(super) fn shader_index_fits(index: i32, variations: usize) -> bool {
    index < 0 || (index as usize) < variations
}

pub(super) fn append_unique_resource_ids<const N: usize>(out: &mut Vec<u64>, ids: [Option<u64>; N]) {
    for id in ids.into_iter().flatten() {
        // 0 and u64::MAX are the EFF "no resource" sentinels.
        if id != 0 && id != u64::MAX && !out.contains(&id) {
            out.push(id);
        }
    }
}
