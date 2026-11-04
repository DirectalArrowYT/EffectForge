// Mod export: rebuild an ef_*.eff with the project's authored edits applied, using the
// effect_library crate (byte-exact writer). The in-tree parser (src/effects.rs) is the
// EDITOR's view; this module maps its edited fields onto effect_library's structs:
//
//   emission_rate → Emission.rate            lifetime      → ParticleData.life (i32)
//   scale         → ParticleScale.scale_x/y/z (ratio vs scale_x, preserves anisotropy)
//   color_scale   → EmitterStatic.color_scale emitter_scale → EmitterInfo.scale_x/y/z
//   color0/color1 → whichever source `effects.rs` READ for this emitter, at the same
//                   precedence: a Constant ParticleColor, else the EmitterStatic key table
//                   (key.x/y/z = r/g/b; times untouched), else the EmitterInfo static color
//   alpha0        → EmitterStatic.alpha0 keys (value in key.x), else a Constant
//                   ParticleColor.alpha0, else EmitterInfo.color0_a — same rule
//
// Writes are strictly scoped: one AuthoredEdit names ONE emitter set and ONE emitter, and
// touches only that emitter's LIVE key slots. Every other emitter, channel and inert key
// slot stays byte-identical to the source — see the
// `authored_color_edit_touches_only_the_targeted_emitter` regression test.
//
// Edited emitters get `cached_binary` cleared — the serializer prefers the cached EMTR
// blob and would otherwise silently drop the edits.

use anyhow::{anyhow, Context, Result};

use crate::mod_project::{AuthoredEdit, EffMod, EmitterFieldEdits, TextureImport, TransplantOp};


/// ONE-SLOT-scoped rebuild for export: `slot` None = the base file (only transplants with
/// no one-slot scoping); Some(s) = the `ef_*_cNN.eff` file (unscoped transplants + those
/// scoped to that slot — the slotted file replaces the base for that costume, so it must
/// carry both).
///
/// `slot` is a real costume index, NOT limited to the vanilla 0–7: the caller writes
/// `c{slot:02}`, so c08 and up (and three-digit slots) name their files correctly.
pub fn rebuild_eff_bytes_for_slot(
    src_bytes: &[u8],
    eff: &EffMod,
    donor_root: Option<&std::path::Path>,
    slot: Option<u8>,
) -> Result<Vec<u8>> {
    rebuild_eff_bytes_filtered(src_bytes, eff, donor_root, |op| match slot {
        None => op.one_slot_slots.is_empty(),
        Some(s) => op.one_slot_slots.is_empty() || op.one_slot_slots.contains(&s),
    })
}



/// Every texture GUID some surviving emitter samples.
fn sampled_texture_ids(ptcl: &effect_library::PtclFile) -> std::collections::HashSet<u64> {
    let mut used = std::collections::HashSet::new();
    for set in &ptcl.emitter_list.emitter_sets {
        visit_emitters_ref(&set.emitters, &mut |em| {
            let d = &em.data;
            for id in [
                d.sampler0.as_ref().map(|s| s.texture_id),
                d.sampler1.as_ref().map(|s| s.texture_id),
                d.sampler2.as_ref().map(|s| s.texture_id),
                d.sampler3.as_ref().map(|s| s.texture_id),
                d.sampler4.as_ref().map(|s| s.texture_id),
                d.sampler5.as_ref().map(|s| s.texture_id),
            ]
            .into_iter()
            .flatten()
            {
                used.insert(id);
            }
        });
    }
    used
}

/// Grow the pool with the textures the user added.
///
/// Runs BEFORE authored edits, because a texture swap resolves its target by NAME against the
/// descriptor table (`apply_authored`) — an addition applied afterwards would not exist yet and
/// the swap would be skipped with a warning. And AFTER pruning, so the pruner cannot drop a
/// texture nothing samples yet: the emitter that will use it is repointed by an edit that has not
/// run.
///
/// A missing template is a WARNING, not a failure — same rule as a missing authored edit. A
/// stale addition must not take a working carrier down with it.
fn apply_texture_additions(
    file: &mut effect_library::NamcoEffectFile,
    additions: &[crate::mod_project::TextureAddition],
    warnings: &mut Vec<String>,
) -> Result<()> {
    if additions.is_empty() {
        return Ok(());
    }
    let Some(textures) = file
        .ptcl_file
        .as_mut()
        .and_then(|ptcl| ptcl.texture_info.as_mut())
    else {
        for add in additions {
            warnings.push(format!("texture-add:{}", add.texture_name));
        }
        return Ok(());
    };

    for add in additions {
        let names: Vec<String> = textures
            .descriptors
            .iter()
            .map(|d| d.name.clone())
            .collect();
        if names.contains(&add.texture_name) {
            // Already present: a rebuild of a project that has been sent before.
            continue;
        }
        let Some(template) = names.iter().position(|n| *n == add.template_name) else {
            warnings.push(format!("texture-add:{}", add.texture_name));
            continue;
        };
        let Some(pool) = textures.binary_data.as_ref() else {
            warnings.push(format!("texture-add:{}", add.texture_name));
            continue;
        };
        let rebuilt = if add.png_path.is_empty() {
            crate::texture_import::duplicate_texture(pool, &names, template, &add.texture_name)
                .with_context(|| {
                    format!(
                        "duplicating '{}' as '{}'",
                        add.template_name, add.texture_name
                    )
                })?
        } else {
            let png = std::fs::read(&add.png_path).with_context(|| {
                format!(
                    "new texture '{}': cannot read {}",
                    add.texture_name, add.png_path
                )
            })?;
            let form = if add.raw {
                crate::texture_import::Form::Raw
            } else {
                crate::texture_import::Form::Editable
            };
            let (rebuilt, _) = crate::texture_import::add_texture_from_png(
                pool,
                &names,
                template,
                &add.texture_name,
                &png,
                form,
            )
            .with_context(|| format!("adding {} as '{}'", add.png_path, add.texture_name))?;
            rebuilt
        };
        let taken: Vec<u64> = textures.descriptors.iter().map(|d| d.id).collect();
        let id = crate::texture_import::unused_descriptor_id(&taken, &add.texture_name);
        textures
            .descriptors
            .push(effect_library::ptcl_file::TextureDescriptor {
                id,
                name: add.texture_name.clone(),
            });
        textures.binary_data = Some(rebuilt);
    }
    Ok(())
}

/// Drop the textures the user removed.
///
/// Runs LAST, so the swaps that moved emitters off the doomed texture have already been applied.
/// A texture something still samples is KEPT, with a warning: writing a pool without it would
/// leave that emitter addressing a GUID no descriptor holds, which renders as nothing at all.
/// The editor blocks the delete for the same reason, so reaching this is either a stale project
/// or a swap that could not be placed.
fn apply_texture_removals(
    file: &mut effect_library::NamcoEffectFile,
    removals: &[String],
    warnings: &mut Vec<String>,
) -> Result<()> {
    if removals.is_empty() {
        return Ok(());
    }
    let Some(ptcl) = file.ptcl_file.as_mut() else {
        return Ok(());
    };
    let still_used = sampled_texture_ids(ptcl);
    let Some(textures) = ptcl.texture_info.as_mut() else {
        return Ok(());
    };
    for name in removals {
        let names: Vec<String> = textures
            .descriptors
            .iter()
            .map(|d| d.name.clone())
            .collect();
        let Some(index) = names.iter().position(|n| n == name) else {
            continue; // already gone, or pruned — nothing to do
        };
        if still_used.contains(&textures.descriptors[index].id) {
            warnings.push(format!("texture-in-use:{name}"));
            continue;
        }
        let Some(pool) = textures.binary_data.as_ref() else {
            continue;
        };
        let rebuilt = crate::texture_import::remove_texture(pool, &names, index)
            .with_context(|| format!("removing texture '{name}'"))?;
        textures.descriptors.remove(index);
        textures.binary_data = Some(rebuilt);
    }
    Ok(())
}

/// Replace pool textures with the user's own images.
///
/// Runs LAST, on the final pool: pruning has already settled which textures survive and any
/// swap edits have already repointed samplers, so an import lands on the texture that will
/// actually be sampled. An import naming a texture this file does not hold is a WARNING, not
/// a failure — same rule as a missing authored edit. Transplants and imports are independent
/// features, and one stale import must not take a working carrier down with it.
fn apply_texture_imports(
    file: &mut effect_library::NamcoEffectFile,
    imports: &[TextureImport],
    warnings: &mut Vec<String>,
) -> Result<()> {
    if imports.is_empty() {
        return Ok(());
    }
    let Some(textures) = file
        .ptcl_file
        .as_mut()
        .and_then(|ptcl| ptcl.texture_info.as_mut())
    else {
        for import in imports {
            warnings.push(format!("texture:{}", import.texture_name));
        }
        return Ok(());
    };

    for import in imports {
        let Some(index) = textures
            .descriptors
            .iter()
            .position(|d| d.name == import.texture_name)
        else {
            warnings.push(format!("texture:{}", import.texture_name));
            continue;
        };
        let Some(pool) = textures.binary_data.as_ref() else {
            warnings.push(format!("texture:{}", import.texture_name));
            continue;
        };
        let png = std::fs::read(&import.png_path).with_context(|| {
            format!(
                "texture import for '{}': cannot read {}",
                import.texture_name, import.png_path
            )
        })?;
        let names: Vec<String> = textures
            .descriptors
            .iter()
            .map(|d| d.name.clone())
            .collect();
        let form = if import.raw {
            crate::texture_import::Form::Raw
        } else {
            crate::texture_import::Form::Editable
        };
        let (rebuilt, report) = crate::texture_import::replace_with_png(
            pool, &names, index, &png, form,
        )
        .with_context(|| {
            format!(
                "importing {} over '{}'",
                import.png_path, import.texture_name
            )
        })?;
        if let Some(original) = &report.format_substituted_from {
            // The user asked for THIS image on THIS texture and got it, but not in the format
            // the game shipped — worth saying, since a normal map re-encoded as colour will
            // look wrong in a way that has nothing to do with the image they picked.
            warnings.push(format!(
                "texture-format:{} was {original}, imported as {}",
                import.texture_name, report.format
            ));
        }
        textures.binary_data = Some(rebuilt);
    }
    Ok(())
}

/// Reduce a donor eff to the named entries plus only the resources those entries reference.
///
/// The plugin co-loads the donor eff whole so a foreign kind is resident in a match the donor's
/// character is not in. It used to be sent WHOLE: `ef_marx.eff` is 20 MB, and the transport
/// base64s it into a single JSON frame, so one Marx effect cost ~27 MB on the wire. Two donors
/// put ~43 MB through a socket the emulator reads in 8 KB chunks — the reported 30 s timeout.
///
/// Stripping was tried once and reverted, because a stripped mesh effect ("alucard_backdash")
/// spawned and rendered nothing. That is the exact signature of the BFRES relocation defect
/// since fixed and corpus-calibrated: the pool survived the trip structurally intact and was
/// unusable on hardware. The passes used here are the same three the carrier has always used,
/// and the carrier renders.
///
/// Entries and sets are kept and merely emptied, never deleted, so every id still resolves —
/// deleting an entry leaves the item's own spawn requests resolving against nothing.
pub fn strip_donor_eff_bytes(src: &[u8], keep_entries: &[&str]) -> Result<Vec<u8>> {
    let mut file = effect_library::NamcoEffectFile::load(src)
        .context("effect_library failed to parse the donor .eff")?;
    silence_carrier_native_effects(&mut file, keep_entries);
    clear_empty_emitter_set_references(&mut file);
    prune_unreferenced_resources(&mut file, &[])?;
    compact_shader_containers(&mut file)?;
    file.save()
        .context("effect_library failed to encode the stripped donor .eff")
}

/// Empty every emitter set the transplants did not create, leaving the carrier's own effects as
/// resolvable-but-silent entries.
///
/// The carrier is a game-owned assist EFF borrowed for its blessed load path — its native effects
/// are never played, but they are what holds the file open: Bomberman's four sets reference all
/// nineteen of its textures, 2.9 MB of a 3.4 MB base. Clearing their emitters makes those
/// textures, primitives and shader variations collectable by the passes that follow.
///
/// The entries and the sets themselves stay. The item still requests its own effects by name at
/// spawn, and a name that resolves to an empty set yields a valid handle that emits nothing —
/// whereas deleting the entry would leave those requests resolving against nothing at all.
fn silence_carrier_native_effects(
    carrier: &mut effect_library::NamcoEffectFile,
    transplanted: &[&str],
) {
    let mut keep = std::collections::HashSet::new();
    for (index, _) in carrier.entry_names.iter().enumerate().filter(|(_, name)| {
        transplanted
            .iter()
            .any(|new| new.eq_ignore_ascii_case(name))
    }) {
        // Every set the entry can spawn, not just `emitter_set_id` — a multi-part transplant
        // reaches its content only through its variants, and `apply_transplant_cross_file` leaves
        // `emitter_set_id` at the DONOR's value for those. Keying on that field alone kept a set
        // belonging to some unrelated carrier effect and cleared the transplant's real ones.
        entry_emitter_sets(carrier, index, &mut keep);
    }
    let Some(ptcl) = carrier.ptcl_file.as_mut() else {
        return;
    };
    for (index, set) in ptcl.emitter_list.emitter_sets.iter_mut().enumerate() {
        if !keep.contains(&index) {
            set.emitters.clear();
        }
    }
}

/// Collect the 0-based emitter-set indices entry `index` can spawn into `out`.
///
/// A single-part entry names one set through `emitter_set_id`. A multi-part entry names one per
/// variant AND keeps its own `emitter_set_id` live — 13 game entries point it at a populated set
/// that appears in no variant, so both are collected (see
/// `every_game_emitter_set_is_reachable_from_the_entry_table`). Ids are 1-based handles; 0 means
/// none.
fn entry_emitter_sets(
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
/// clearing the rest safe: the only way to produce an unreachable set is for the editor to orphan
/// one, and replace-mode transplants do exactly that.
fn live_emitter_sets(file: &effect_library::NamcoEffectFile) -> std::collections::HashSet<usize> {
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
/// [`silence_carrier_native_effects`] keeps the carrier's own: every id in the file still
/// resolves, and an empty set is a valid handle that emits nothing.
fn clear_unreachable_emitter_sets(file: &mut effect_library::NamcoEffectFile) {
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
/// makes the operation idempotent across repeated project rebuilds.
fn clear_empty_emitter_set_references(file: &mut effect_library::NamcoEffectFile) {
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
/// `keep_textures` names textures that must survive even though nothing samples them YET. The
/// carrier applies authored edits AFTER this pass (so an edit can never mask a bad transplant
/// clone), which means a texture-swap edit's TARGET is unreferenced at prune time and would be
/// thrown away right before the swap tried to point at it.
///
/// Whole resources go or stay; the ones that stay are not slimmed, and that is where the
/// remaining headroom is. A stripped Kirby donor is 3.12 MB, of which 2.84 MB is 13 textures this
/// pass has already proved are all sampled — and all 13 carry full mip chains (112 levels between
/// them), roughly a quarter of those bytes. Dropping mips is therefore the largest lever left, and
/// it is deliberately not pulled: mip count is part of what the sampler descriptors expect, so a
/// short chain is a change to what the GPU reads rather than to what the file merely holds. That
/// is the class of change this format punishes with a pool that validates and draws nothing, so it
/// needs on-hardware confirmation and belongs behind a user's choice, not in every build.
fn prune_unreferenced_resources(
    carrier: &mut effect_library::NamcoEffectFile,
    keep_textures: &[String],
) -> Result<()> {
    let Some(ptcl) = carrier.ptcl_file.as_mut() else {
        return Ok(());
    };
    let mut used_textures: Vec<u64> = Vec::new();
    for name in keep_textures {
        let id = ptcl
            .texture_info
            .as_ref()
            .and_then(|info| info.descriptors.iter().find(|d| d.name == *name))
            .map(|d| d.id);
        if let Some(id) = id {
            append_unique_resource_ids(&mut used_textures, [Some(id)]);
        }
    }
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
                .ok_or_else(|| anyhow!("carrier has texture descriptors but no BNTX"))?;
            let scratch = crate::scratch_dirs::app_scratch_dir("carrier-tex")?;
            let mut blobs = Vec::with_capacity(keep.len());
            for index in &keep {
                let name = textures.descriptors[*index].name.clone();
                let path = scratch.path().join(format!("{name}.bntx"));
                effect_library::bntx::export_single_texture(binary, *index, &name, &path)
                    .with_context(|| format!("extracting carrier texture '{name}'"))?;
                blobs.push(std::fs::read(&path)?);
            }
            textures.binary_data = match blobs.len() {
                0 => None,
                1 => blobs.pop(),
                _ => Some(
                    effect_library::bntx::merge_texture_files(&blobs)
                        .context("repacking the carrier's texture pool")?,
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
                .ok_or_else(|| anyhow!("carrier has primitive descriptors but no BFRES"))?;
            let mut blobs = Vec::with_capacity(keep.len());
            for index in &keep {
                blobs.push(
                    effect_library::bfres::export_single_model(binary, *index).with_context(
                        || {
                            format!(
                                "extracting carrier primitive {:#x}",
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
                        .context("repacking the carrier's primitive pool")?,
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
/// thing in the carrier. Keeping only what is referenced is what lets the carrier hold just the
/// effects the user asked to transplant.
///
/// Repacking variations was tried once before and produced handles with no pixels, which is why
/// the containers used to be transferred whole. That has since been diagnosed: `BnshFile::write`
/// emitted a relocation table listing slots that hold no offset, so the runtime turned each into
/// a pointer to the image base and every container the writer touched was corrupt. With the
/// writer fixed it reproduces the game's own pointer set exactly, so repacking is sound again.
fn compact_shader_containers(carrier: &mut effect_library::NamcoEffectFile) -> Result<()> {
    let Some(ptcl) = carrier.ptcl_file.as_mut() else {
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
/// 2.2% of a one-donor carrier, 0.2% of a three-donor one. That buys very little in exchange for
/// changing which compiled program an emitter binds to, in a format whose characteristic failure
/// is a pool that is structurally perfect and draws nothing on hardware.
fn subset_container(
    binary: Option<&mut Vec<u8>>,
    used: &std::collections::BTreeSet<i32>,
    kind: &str,
) -> Result<std::collections::HashMap<i32, i32>> {
    let Some(binary) = binary else {
        return Ok(std::collections::HashMap::new());
    };
    // An unused container is left alone rather than removed: the engine-global single-variation
    // compute container is the shape every carrier that has ever loaded was built with.
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
/// nothing to be gained by shipping and seeing. `subject` names what was being built, since the
/// carrier and the exported eff fail for different reasons and the fix differs.
///
/// Runs AFTER compaction and independently of it — compaction renumbers indices through a map it
/// built itself, and this checks the result against the container that actually shipped.
fn verify_shader_indices_resolve(
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




/// How a transplant's emitters relate to the destination's shader containers.
#[derive(Clone, Copy)]
enum ShaderStrategy {
    /// Append the donor's container to the destination's, relocating by the destination's count.
    Merge,
    /// The destination already holds this donor's variations at these bases, so the containers
    /// must not be touched again — only the copied emitters' indices are relocated. Merging a
    /// donor once per transplanted effect instead of once per file duplicated whole containers.
    AlreadyMerged {
        standard_base: i32,
        compute_base: i32,
    },
}

/// A negative index is the "no shader" sentinel; anything else must address a variation the
/// container actually holds.
fn shader_index_fits(index: i32, variations: usize) -> bool {
    index < 0 || (index as usize) < variations
}

fn rebuild_eff_bytes_filtered(
    src_bytes: &[u8],
    eff: &EffMod,
    donor_root: Option<&std::path::Path>,
    keep: impl Fn(&TransplantOp) -> bool,
) -> Result<Vec<u8>> {
    let mut namco = effect_library::NamcoEffectFile::load(src_bytes)
        .context("effect_library failed to parse the source .eff")?;
    let destination_is_fighter = eff
        .source_rel
        .to_ascii_lowercase()
        .starts_with("effect/fighter/");
    // Transplant ops FIRST (they only append sets, so pre-existing set indices are
    // stable), then authored edits — which may target a freshly cloned set by name
    // (editing the copy in the eff editor). Cross-fighter donors bake in here too: the
    // merged file replaces the player's own (resident) eff at boot, so it renders.
    for op in eff.transplants.iter().filter(|op| keep(op)) {
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
                ShaderStrategy::Merge,
            )?;
        }
    }
    // Strip the shader library down to what this file addresses, the same way the carrier does.
    // Cross-file transplants merge in each donor's ENTIRE BNSH — Pickel ships 370 variations and
    // one transplanted effect uses twelve — so without this a single transplant makes the
    // player's ef_*.eff carry a second fighter's whole shader library.
    //
    // Orphans go first: a replace-mode transplant leaves the set it repointed away from holding
    // live emitters, and compaction scans sets, so an orphan would keep its share of the donor
    // library reachable. Clearing them cannot touch anything the game shipped — every set in all
    // 328 game corpus files is reachable from the entry table.
    //
    // Unlike the carrier, native effects are NOT silenced: this file IS the fighter's own eff and
    // everything in it has to keep playing. Textures are not pruned either, and deliberately —
    // `apply_transplant_cross_file` copies only the textures the cloned emitters reference, so
    // there is no unreferenced-texture bloat here to collect, while a prune this late could drop
    // a pool texture that an authored swap or an import below is about to name.
    clear_unreachable_emitter_sets(&mut namco);
    compact_shader_containers(&mut namco)?;
    verify_shader_indices_resolve(&namco, "eff export")?;

    // Same order as the carrier build, and for the same reasons: added textures before the
    // authored edits (a swap resolves its target by name, so it has to exist first), replacements
    // after (they land on whichever texture will actually be sampled), removals last (the swaps
    // that moved emitters off them have run by then).
    //
    // Warnings go to stderr rather than a channel: the export UI reports the file it wrote, and
    // anything dropped here has already been surfaced by the live carrier build.
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
        eprintln!("[EFF-EXPORT] warning: {warning}");
    }
    namco
        .save()
        .context("effect_library failed to re-encode the .eff")
}

/// Duplicate a donor entry's emitter set(s) within the same eff under a new entry name —
/// the ACMD side then retargets the fighter's call to `new_entry_name`, leaving the
/// original entry vanilla for everyone else.
fn apply_transplant_same_file(
    namco: &mut effect_library::NamcoEffectFile,
    op: &TransplantOp,
) -> Result<()> {
    if op.replace_entry.is_none()
        && namco
            .entry_names
            .iter()
            .any(|n| n.eq_ignore_ascii_case(&op.new_entry_name))
    {
        anyhow::bail!(
            "transplant target name '{}' already exists",
            op.new_entry_name
        );
    }
    // Case-insensitive: the studio's donor list mixes original-case file entries with
    // lowercase live kinds (hash40 is over the lowercase name).
    let donor_idx = namco
        .entry_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(&op.src_set_name))
        .ok_or_else(|| {
            anyhow!(
                "transplant donor entry '{}' not found in this eff",
                op.src_set_name
            )
        })?;
    let donor = namco.entries[donor_idx].clone();

    let ptcl = namco
        .ptcl_file
        .as_mut()
        .ok_or_else(|| anyhow!("eff has no embedded PTCL"))?;
    let sets = &mut ptcl.emitter_list.emitter_sets;

    // NOTE: the on-disk entry/variant ids are 1-BASED handles (0 = none) — see
    // eff_lib's EffectHandle docs and `entry_set_id_is_one_based`. Reads subtract 1;
    // writes store `len` (= appended 0-based index + 1).
    let clone_set =
        |raw_id: u32, sets: &mut Vec<effect_library::structs::EmitterSet>| -> Result<u32> {
            let idx = (raw_id as usize)
                .checked_sub(1)
                .ok_or_else(|| anyhow!("donor entry has no emitter set (id 0)"))?;
            let src = sets
                .get(idx)
                .ok_or_else(|| anyhow!("donor emitter set id {raw_id} out of range"))?
                .clone();
            let mut new_set = src;
            new_set.name = op.new_entry_name.clone();
            sets.push(new_set);
            Ok(sets.len() as u32) // raw 1-based handle of the appended set
        };

    let mut new_entry = donor.clone();
    if donor.variant_count == 0 {
        new_entry.emitter_set_id = clone_set(donor.emitter_set_id, sets)?;
    } else {
        // Multi-part effect: clone every variant's set and append a new variant block.
        let start = (donor.variant_start_idx as usize)
            .checked_sub(1)
            .ok_or_else(|| anyhow!("donor variant start id 0"))?;
        let count = donor.variant_count as usize;
        let donor_variants: Vec<effect_library::namco_file::EffectVariant> = namco
            .effect_variants
            .get(start..start + count)
            .ok_or_else(|| anyhow!("donor variant range out of bounds"))?
            .to_vec();
        let new_start = namco.effect_variants.len() as u16 + 1; // raw 1-based
        for v in donor_variants {
            let new_set_id = clone_set(v.emitter_set_id as u32, sets)?;
            namco
                .effect_variants
                .push(effect_library::namco_file::EffectVariant {
                    start_frame: v.start_frame,
                    emitter_set_id: new_set_id as u16,
                });
        }
        new_entry.variant_start_idx = new_start;
    }
    finish_transplant_entry(namco, op, new_entry)
}

/// Append `new_entry` under the op's new name, or — replace mode — repoint an existing
/// entry at the cloned set(s) (its name and slot in the table stay; the old set is
/// orphaned but harmless). Replace is the costume-scoped semantic: every ACMD use of
/// the entry switches on that costume with no redirect needed.
fn finish_transplant_entry(
    namco: &mut effect_library::NamcoEffectFile,
    op: &TransplantOp,
    new_entry: effect_library::namco_file::EffectHeader,
) -> Result<()> {
    match &op.replace_entry {
        None => {
            namco.entries.push(new_entry);
            namco.entry_names.push(op.new_entry_name.clone());
        }
        Some(target) => {
            let ti = namco
                .entry_names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(target))
                .ok_or_else(|| {
                    anyhow!("transplant replace target entry '{target}' not found in this eff")
                })?;
            // Take the donor's header (kind/flags describe how its set plays) but keep
            // the entry's name so every existing ACMD call keeps resolving.
            namco.entries[ti] = new_entry;
        }
    }
    Ok(())
}

/// Copy a donor entry from ANOTHER eff into this one: clone the emitter set(s), transfer
/// the textures they reference (by GUID, merged into this file's BNTX), append the donor's
/// shader variations (BNSH merge) and remap the cloned emitters' shader indices. Primitives
/// (rare) are not transferable yet — referencing one bails with a clear error.
fn apply_transplant_cross_file(
    namco: &mut effect_library::NamcoEffectFile,
    donor: &effect_library::NamcoEffectFile,
    op: &TransplantOp,
    destination_is_fighter: bool,
    shader_strategy: ShaderStrategy,
) -> Result<()> {
    if op.replace_entry.is_none()
        && namco
            .entry_names
            .iter()
            .any(|n| n.eq_ignore_ascii_case(&op.new_entry_name))
    {
        anyhow::bail!(
            "transplant target name '{}' already exists",
            op.new_entry_name
        );
    }
    // Case-insensitive: live-kind donors arrive lowercase while file entries are
    // original-case (the hash40 name is the lowercase form).
    let donor_idx = donor
        .entry_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(&op.src_set_name))
        .ok_or_else(|| {
            anyhow!(
                "transplant donor entry '{}' not found in '{}'",
                op.src_set_name,
                op.src_file_rel
            )
        })?;
    let donor_entry = donor.entries[donor_idx].clone();
    let donor_ptcl = donor
        .ptcl_file
        .as_ref()
        .ok_or_else(|| anyhow!("donor eff has no embedded PTCL"))?;

    // Gather the donor emitter set(s) this entry uses (variants included).
    // Entry/variant ids are 1-BASED handles (0 = none) — see `entry_set_id_is_one_based`.
    let mut donor_sets: Vec<effect_library::structs::EmitterSet> = Vec::new();
    let mut primary_set_pos: Option<usize> = None;
    if let Some(idx) = (donor_entry.emitter_set_id as usize).checked_sub(1) {
        primary_set_pos = Some(donor_sets.len());
        donor_sets.push(
            donor_ptcl
                .emitter_list
                .emitter_sets
                .get(idx)
                .ok_or_else(|| anyhow!("donor primary emitter set out of range"))?
                .clone(),
        );
    }
    let variant_set_start = donor_sets.len();
    let mut variant_frames: Vec<u16> = Vec::new();
    let mut variant_bones: Vec<String> = Vec::new();
    if donor_entry.variant_count == 0 {
        if primary_set_pos.is_none() {
            anyhow::bail!("donor entry has neither a primary emitter set nor parts");
        }
    } else {
        let start = (donor_entry.variant_start_idx as usize)
            .checked_sub(1)
            .ok_or_else(|| anyhow!("donor variant start id 0"))?;
        let count = donor_entry.variant_count as usize;
        for (variant_offset, v) in donor
            .effect_variants
            .get(start..start + count)
            .ok_or_else(|| anyhow!("donor variant range out of bounds"))?
            .iter()
            .enumerate()
        {
            let idx = (v.emitter_set_id as usize)
                .checked_sub(1)
                .ok_or_else(|| anyhow!("donor variant has no emitter set (id 0)"))?;
            donor_sets.push(
                donor_ptcl
                    .emitter_list
                    .emitter_sets
                    .get(idx)
                    .ok_or_else(|| anyhow!("donor variant set out of range"))?
                    .clone(),
            );
            variant_frames.push(v.start_frame);
            variant_bones.push(
                donor
                    .external_bone_names
                    .get(start + variant_offset)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
    }

    // Collect the resources the donor emitters reference.
    let mut tex_ids: Vec<u64> = Vec::new();
    let mut uses_shader = false;
    let mut uses_compute = false;
    let mut prim_ids: Vec<u64> = Vec::new();
    for set in &donor_sets {
        visit_emitters_ref(&set.emitters, &mut |em| {
            let d = &em.data;
            append_unique_resource_ids(
                &mut tex_ids,
                [
                    d.sampler0.as_ref().map(|s| s.texture_id),
                    d.sampler1.as_ref().map(|s| s.texture_id),
                    d.sampler2.as_ref().map(|s| s.texture_id),
                    d.sampler3.as_ref().map(|s| s.texture_id),
                    d.sampler4.as_ref().map(|s| s.texture_id),
                    d.sampler5.as_ref().map(|s| s.texture_id),
                ],
            );
            uses_shader |= d.shader_references.shader_index >= 0
                || d.shader_references.user_shader_index1 >= 0
                || d.shader_references.user_shader_index2 >= 0;
            uses_compute |= d.shader_references.compute_shader_index >= 0;
            for pid in [
                d.particle_data.primitive_id,
                d.particle_data.primitive_ex_id,
                d.shape_info.primitive_index,
            ] {
                // 0 and u64::MAX are "no primitive" sentinels (see descriptor_index_for_id).
                if pid != 0 && pid != u64::MAX && !prim_ids.contains(&pid) {
                    prim_ids.push(pid);
                }
            }
        });
    }

    // Primitives: the PRMA section = descriptor table (GUID + per-model attribute
    // indices) + a multi-model BFRES container, with descriptor index i ↔ model index i
    // (verified against the crate's dumper/creator). Emitters reference primitives by
    // GUID, so appending donor models + their descriptors in lockstep needs no emitter
    // rewrites. Cases:
    // Each missing donor primitive is extracted as a single-model BFRES and appended.
    // This deliberately avoids copying the donor's whole PRMA when the destination pool
    // is empty: the runtime carrier should retain only resources the selected effect uses.
    if !prim_ids.is_empty() {
        let dest_has = |id: u64| {
            namco
                .ptcl_file
                .as_ref()
                .and_then(|p| p.primitive_info.as_ref())
                .map(|pi| pi.descriptors.iter().any(|d| d.id == id))
                .unwrap_or(false)
        };
        // Some vanilla emitters retain nonzero values in inactive primitive fields (and some
        // refer to globally-owned primitives). If the donor itself has no descriptor for an
        // ID, there is no local model to transplant; vanilla already resolves or ignores it.
        // Only donor-owned descriptors belong in the merged PRMA.
        let donor_prim = donor_ptcl.primitive_info.as_ref();
        let missing: Vec<u64> = prim_ids
            .iter()
            .copied()
            .filter(|id| {
                !dest_has(*id)
                    && donor_prim
                        .map(|p| p.descriptors.iter().any(|d| d.id == *id))
                        .unwrap_or(false)
            })
            .collect();
        if !missing.is_empty() {
            let donor_prim = donor_prim.expect("missing IDs were filtered through donor PRMA");
            let dest_ptcl = namco
                .ptcl_file
                .as_mut()
                .ok_or_else(|| anyhow!("target eff has no embedded PTCL"))?;
            if dest_ptcl.primitive_info.is_none() {
                let mut empty = donor_prim.clone();
                empty.descriptors.clear();
                empty.binary_data = None;
                dest_ptcl.primitive_info = Some(empty);
            }
            let donor_bin = donor_prim
                .binary_data
                .as_ref()
                .ok_or_else(|| anyhow!("donor PRMA has descriptors but no BFRES binary"))?;
            let dest_prim = dest_ptcl.primitive_info.as_mut().unwrap();
            let dest_count = dest_prim.descriptors.len();
            let mut files: Vec<Vec<u8>> = dest_prim
                .binary_data
                .as_ref()
                .filter(|b| !b.is_empty())
                .cloned()
                .into_iter()
                .collect();
            for id in &missing {
                let idx =
                    effect_library::bfres::descriptor_index_for_id(&donor_prim.descriptors, *id)
                        .ok_or_else(|| anyhow!("donor primitive {id:#x} has no descriptor"))?;
                let blob = effect_library::bfres::export_single_model(donor_bin, idx)
                    .with_context(|| format!("extracting donor primitive {id:#x}"))?;
                files.push(blob);
                dest_prim
                    .descriptors
                    .push(donor_prim.descriptors[idx].clone());
            }
            let merged = if files.len() == 1 {
                files.pop().unwrap()
            } else {
                effect_library::bfres::ResFile::merge_model_files(&files)
                    .context("merging donor primitives into the target PRMA")?
            };
            // Sanity: descriptor index i must still map to model index i — a model
            // NAME collision would make the container replace-instead-of-append and
            // silently desync every primitive after it.
            let last = dest_count + missing.len() - 1;
            if effect_library::bfres::export_single_model(&merged, last).is_err()
                || effect_library::bfres::export_single_model(&merged, last + 1).is_ok()
            {
                anyhow::bail!(
                    "transplant '{}': donor primitive model name collides with the target's \
                     (PRMA merge would desync) — not merged",
                    op.new_entry_name
                );
            }
            dest_prim.binary_data = Some(merged);
        }
    }

    // Textures: copy missing GUIDs (descriptor + single-texture BNTX merged into ours).
    {
        let dest_ptcl = namco
            .ptcl_file
            .as_mut()
            .ok_or_else(|| anyhow!("target eff has no embedded PTCL"))?;
        let donor_tex = donor_ptcl.texture_info.as_ref();
        let dest_tex = dest_ptcl
            .texture_info
            .as_mut()
            .ok_or_else(|| anyhow!("target eff has no texture section"))?;
        let mut new_blobs: Vec<Vec<u8>> = Vec::new();
        for id in &tex_ids {
            if dest_tex.descriptors.iter().any(|d| d.id == *id) {
                continue;
            }
            let donor_tex = donor_tex
                .ok_or_else(|| anyhow!("donor eff has no texture section but emitters sample"))?;
            let idx = donor_tex
                .descriptors
                .iter()
                .position(|d| d.id == *id)
                .ok_or_else(|| anyhow!("donor texture {id:#x} has no descriptor"))?;
            let name = donor_tex.descriptors[idx].name.clone();
            let donor_bin = donor_tex
                .binary_data
                .as_ref()
                .ok_or_else(|| anyhow!("donor texture section has no binary"))?;
            // bntx extraction is file-path based — round-trip through the scratch dir.
            let tmp = crate::scratch_dirs::app_scratch_dir("transplant-tex")?;
            let path = tmp.path().join(format!("{name}.bntx"));
            effect_library::bntx::export_single_texture(donor_bin, idx, &name, &path)
                .with_context(|| format!("extracting donor texture '{name}'"))?;
            new_blobs.push(std::fs::read(&path)?);
            dest_tex
                .descriptors
                .push(effect_library::ptcl_file::TextureDescriptor { id: *id, name });
        }
        if !new_blobs.is_empty() {
            let mut files: Vec<Vec<u8>> = Vec::with_capacity(new_blobs.len() + 1);
            if let Some(existing) = dest_tex.binary_data.as_ref().filter(|b| !b.is_empty()) {
                files.push(existing.clone());
            }
            files.extend(new_blobs);
            dest_tex.binary_data = Some(if files.len() == 1 {
                files.pop().unwrap()
            } else {
                effect_library::bntx::merge_texture_files(&files)
                    .context("merging donor textures into the target BNTX")?
            });
        }
    }

    // Donor shader containers are appended whole here and the copied emitters relocated onto
    // them. Narrowing to the variations actually referenced is a separate, later pass
    // (`compact_shader_containers`) so it can see every transplant at once; doing it per-donor
    // would have to guess which of the destination's own variations stay reachable.
    let mut shader_index_base = 0i32;
    let mut compute_index_base = 0i32;
    if uses_shader || uses_compute {
        let dest_ptcl = namco.ptcl_file.as_mut().unwrap();
        let dest_shader = dest_ptcl
            .shader_info
            .as_mut()
            .ok_or_else(|| anyhow!("target eff has no shader section"))?;
        let donor_shader = donor_ptcl
            .shader_info
            .as_ref()
            .ok_or_else(|| anyhow!("donor eff has no shader section"))?;
        if uses_shader {
            let dest_bin = dest_shader.binary_data.clone().unwrap_or_default();
            let donor_bin = donor_shader
                .binary_data
                .as_ref()
                .ok_or_else(|| anyhow!("donor shader section has no binary"))?;
            match shader_strategy {
                ShaderStrategy::AlreadyMerged { standard_base, .. } => {
                    shader_index_base = standard_base;
                }
                ShaderStrategy::Merge => {
                    if dest_bin.is_empty() {
                        dest_shader.binary_data = Some(donor_bin.clone());
                    } else {
                        shader_index_base = effect_library::bnsh::BnshFile::read(&dest_bin)
                            .context("reading destination shader variations")?
                            .variations
                            .len() as i32;
                        dest_shader.binary_data = Some(
                            effect_library::bnsh::merge_variation_files(&[
                                dest_bin,
                                donor_bin.clone(),
                            ])
                            .context("merging the donor shader container")?,
                        );
                    }
                }
            }
        }
        if uses_compute {
            let dest_bin = dest_shader.compute_binary.clone().unwrap_or_default();
            let donor_bin = donor_shader
                .compute_binary
                .as_ref()
                .ok_or_else(|| anyhow!("donor compute-shader section has no binary"))?;
            match shader_strategy {
                ShaderStrategy::AlreadyMerged { compute_base, .. } => {
                    compute_index_base = compute_base;
                }
                ShaderStrategy::Merge => {
                    if dest_bin.is_empty() {
                        dest_shader.compute_binary = Some(donor_bin.clone());
                    } else {
                        compute_index_base = effect_library::bnsh::BnshFile::read(&dest_bin)
                            .context("reading destination compute-shader variations")?
                            .variations
                            .len() as i32;
                        dest_shader.compute_binary = Some(
                            effect_library::bnsh::merge_variation_files(&[
                                dest_bin,
                                donor_bin.clone(),
                            ])
                            .context("merging the donor compute-shader container")?,
                        );
                    }
                }
            }
        }
    }

    // Shader-remap the donor emitters (shaders are index-referenced; textures/primitives are
    // GUID-referenced against the merged pools, so they need no remap).
    let remap = |set: &mut effect_library::structs::EmitterSet| {
        visit_emitters(&mut set.emitters, &mut 0, &mut |_, em| {
            let s = &mut em.data.shader_references;
            remap_shader_indices(
                &mut s.shader_index,
                &mut s.user_shader_index1,
                &mut s.user_shader_index2,
                &mut s.compute_shader_index,
                shader_index_base,
                compute_index_base,
            );
            // `custom_shader_index` is NOT an index into the BNSH variation array. Its values
            // are small custom-shader modes (typically 0, 4, or 8) shared by emitters whose
            // real `shader_index` spans the whole file. Relocating it by the destination's
            // variation count makes otherwise valid effects invisible (Daisy's petals exposed
            // this by changing mode 0 into the invalid value 38 in the Bomberman carrier).
            em.cached_binary = None; // indices moved → cached EMTR blob is stale
        });
    };

    // LIVE REPLACE-IN-PLACE: replacing a single-set target with a single-set donor overwrites
    // the target entry's OWN emitter set in place — the entry name + set index are unchanged,
    // so `req(name)` (already registered at fighter-load) keeps resolving and a mid-match
    // reparse rebuilds that set with the donor's content → renders LIVE with NO re-entry and
    // no new-kind registration (the wall that makes appended `_os` entries NOT-FOUND live).
    if let Some(target) = &op.replace_entry {
        if donor_entry.variant_count == 0 {
            let ti = namco
                .entry_names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(target))
                .ok_or_else(|| anyhow!("replace target entry '{target}' not found in this eff"))?;
            let tgt = &namco.entries[ti];
            if tgt.variant_count == 0 {
                let tset = (tgt.emitter_set_id as usize)
                    .checked_sub(1)
                    .ok_or_else(|| anyhow!("replace target '{target}' has no emitter set"))?;
                let mut donor_set = donor_sets.into_iter().next().unwrap();
                remap(&mut donor_set);
                let dest_ptcl = namco.ptcl_file.as_mut().unwrap();
                let sets = &mut dest_ptcl.emitter_list.emitter_sets;
                let keep_name = sets
                    .get(tset)
                    .map(|s| s.name.clone())
                    .ok_or_else(|| anyhow!("replace target set {tset} out of range"))?;
                donor_set.name = keep_name; // keep the target set's name; only its content changes
                sets[tset] = donor_set;
                return Ok(());
            }
        }
        // Fall through to append for multi-variant targets/donors (registration caveat applies).
    }

    // Append path (new named entry). NOTE: an APPENDED entry does NOT register as a spawnable
    // kind mid-match (`req` NOT-FOUND) — it only becomes visible on a fresh fighter load. For
    // live cross-fighter, prefer replace-in-place above.
    let dest_ptcl = namco.ptcl_file.as_mut().unwrap();
    let sets = &mut dest_ptcl.emitter_list.emitter_sets;
    let mut new_set_ids: Vec<u32> = Vec::new();
    for mut set in donor_sets {
        set.name = op.new_entry_name.clone();
        remap(&mut set);
        sets.push(set);
        new_set_ids.push(sets.len() as u32); // raw 1-based handle
    }

    let mut new_entry = donor_entry;
    // External models are referenced through parallel name/flag tables. Transfer the selected
    // row and repoint the cloned entry; retaining the donor's numeric handle would index an
    // unrelated row in the carrier, while dropping it made model edits export-only.
    new_entry.external_model_idx = (new_entry.external_model_idx as usize)
        .checked_sub(1)
        .and_then(|index| {
            let name = donor.external_model_names.get(index)?.clone();
            let flag = donor.effect_models.get(index).copied().unwrap_or(0);
            let dest = namco
                .external_model_names
                .iter()
                .zip(&namco.effect_models)
                .position(|(n, f)| *n == name && *f == flag)
                .unwrap_or_else(|| {
                    namco.external_model_names.push(name);
                    namco.effect_models.push(flag);
                    namco.external_model_names.len() - 1
                });
            Some(dest as u32 + 1)
        })
        .unwrap_or(0);
    // Non-fighter donors (assists/items — e.g. ef_alucard) commonly use type 0 in the
    // kind u16's high byte. A FIGHTER destination needs 0x01xx: the fighter spawn path
    // rejects type-0 entries. An assist-owned carrier needs the inverse normalization:
    // fighter-type 0x01xx entries are not registered by the assist loader.
    new_entry.kind = destination_entry_kind(new_entry.kind, destination_is_fighter);
    if new_entry.variant_count == 0 {
        new_entry.emitter_set_id = new_set_ids[primary_set_pos.unwrap_or(0)];
    } else {
        new_entry.emitter_set_id = primary_set_pos
            .map(|position| new_set_ids[position])
            .unwrap_or(0);
        let new_start = namco.effect_variants.len() as u16 + 1; // raw 1-based
        for ((frame, set_id), bone) in variant_frames
            .iter()
            .zip(&new_set_ids[variant_set_start..])
            .zip(&variant_bones)
        {
            namco
                .effect_variants
                .push(effect_library::namco_file::EffectVariant {
                    start_frame: *frame,
                    emitter_set_id: *set_id as u16,
                });
            namco.external_bone_names.push(bone.clone());
        }
        new_entry.variant_start_idx = new_start;
    }
    finish_transplant_entry(namco, op, new_entry)
}

fn destination_entry_kind(kind: u16, destination_is_fighter: bool) -> u16 {
    if destination_is_fighter {
        if kind & 0xff00 == 0 {
            kind | 0x0100
        } else {
            kind
        }
    } else {
        // Assist/item owners register type-0 entries. Leaving a fighter donor as 0x01xx
        // makes load_effects succeed while omitting the entry from the live kind table.
        kind & 0x00ff
    }
}

fn remap_shader_indices(
    shader: &mut i32,
    user1: &mut i32,
    user2: &mut i32,
    compute: &mut i32,
    shader_base: i32,
    compute_base: i32,
) {
    for index in [shader, user1, user2] {
        if *index >= 0 {
            *index += shader_base;
        }
    }
    if *compute >= 0 {
        *compute += compute_base;
    }
}

fn append_unique_resource_ids<const N: usize>(out: &mut Vec<u64>, ids: [Option<u64>; N]) {
    for id in ids.into_iter().flatten() {
        // 0 and u64::MAX are the EFF "no resource" sentinels.
        if id != 0 && id != u64::MAX && !out.contains(&id) {
            out.push(id);
        }
    }
}


/// Read-only recursive emitter visit (children after parent).
fn visit_emitters_ref<F: FnMut(&effect_library::structs::Emitter)>(
    emitters: &[effect_library::structs::Emitter],
    f: &mut F,
) {
    for em in emitters {
        f(em);
        visit_emitters_ref(&em.children, f);
    }
}

fn apply_authored(ptcl: &mut effect_library::PtclFile, edit: &AuthoredEdit) -> Result<()> {
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
                    "[EFF-EXPORT] warning: texture swap on '{}' wants '{wanted}', which this \
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
            "[EFF-EXPORT] warning: set name mismatch ('{}' vs '{}') — applying by index {}",
            set.name, edit.set_name, set_idx
        );
    }

    // Resolve the target emitter FIRST over a read-only, flat parent-first traversal (the
    // same order the editor enumerates), then write to exactly that one index. Same rule as
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
            "[EFF-EXPORT] warning: emitter '{}' of set '{}' moved from index {} to {}",
            edit.emitter_name, set.name, edit.emitter_idx, i
        );
        Some(i)
    } else if edit.emitter_idx < names.len() {
        if !edit.emitter_name.is_empty() {
            eprintln!(
                "[EFF-EXPORT] warning: emitter '{}' not found by name in set '{}' — using index {}",
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

/// Depth-first, parent-before-children traversal with a running flat index.
fn visit_emitters<F: FnMut(usize, &mut effect_library::structs::Emitter)>(
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

fn apply_fields(em: &mut effect_library::structs::Emitter, f: &EmitterFieldEdits) {
    let d = &mut em.data;
    if let Some(v) = f.emission_rate {
        d.emission.rate = v;
    }
    if let Some(v) = f.lifetime {
        d.particle_data.life = v.round() as i32;
    }
    if let Some(v) = f.scale {
        // The editor's scalar tracks ParticleScale.scale_x; apply the ratio to all axes so
        // authored anisotropy (scale_y/z ≠ scale_x) is preserved.
        let base = d.particle_scale.scale_x;
        if base.abs() > 1e-6 {
            let r = v / base;
            d.particle_scale.scale_x = v;
            d.particle_scale.scale_y *= r;
            d.particle_scale.scale_z *= r;
        } else {
            d.particle_scale.scale_x = v;
            d.particle_scale.scale_y = v;
            d.particle_scale.scale_z = v;
        }
    }
    if let Some(v) = f.color_scale {
        d.emitter_static.color_scale = v;
    }
    if let Some(v) = f.emitter_scale {
        d.emitter_info.scale_x = v[0];
        d.emitter_info.scale_y = v[1];
        d.emitter_info.scale_z = v[2];
    }
    // Color precedence mirrors the editor's read path (`effects.rs`): a
    // ParticleColor "Constant" type overrides the key table; an empty key table falls
    // back to the EmitterInfo static color. Write to whichever source the editor showed.
    if let Some(rows) = &f.color0 {
        if matches!(
            d.particle_color.color0_type,
            effect_library::ColorType::Constant
        ) {
            if let Some(row) = rows.first() {
                d.particle_color.color0_r = row[0];
                d.particle_color.color0_g = row[1];
                d.particle_color.color0_b = row[2];
            }
        } else if d.emitter_static.num_color0_keys > 0 {
            // Only the first `num_color0_keys` slots are live; the rest of the fixed-size
            // table is inert padding the editor never showed. A stale project file carrying
            // more rows than this emitter has keys must not spill into those slots.
            let live = d.emitter_static.num_color0_keys as usize;
            for (k, row) in d.emitter_static.color0.keys.iter_mut().take(live).zip(rows) {
                k.x = row[0];
                k.y = row[1];
                k.z = row[2];
                k.time = row[3];
            }
        } else if let Some(row) = rows.first() {
            d.emitter_info.color0_r = row[0];
            d.emitter_info.color0_g = row[1];
            d.emitter_info.color0_b = row[2];
        }
    }
    if let Some(rows) = &f.color1 {
        if matches!(
            d.particle_color.color1_type,
            effect_library::ColorType::Constant
        ) {
            if let Some(row) = rows.first() {
                d.particle_color.color1_r = row[0];
                d.particle_color.color1_g = row[1];
                d.particle_color.color1_b = row[2];
            }
        } else if d.emitter_static.num_color1_keys > 0 {
            let live = d.emitter_static.num_color1_keys as usize;
            for (k, row) in d.emitter_static.color1.keys.iter_mut().take(live).zip(rows) {
                k.x = row[0];
                k.y = row[1];
                k.z = row[2];
                k.time = row[3];
            }
        } else if let Some(row) = rows.first() {
            d.emitter_info.color1_r = row[0];
            d.emitter_info.color1_g = row[1];
            d.emitter_info.color1_b = row[2];
        }
    }
    // Alpha follows the SAME source-precedence rule as the colors above — `alpha_keys` in
    // `effects.rs` reads the key table first, then a Constant ParticleColor alpha, then the
    // EmitterInfo static alpha. Writing unconditionally into the key table dropped every
    // alpha edit on an emitter with no alpha keys (the game ignores the inert table) while
    // scribbling over slots the editor never showed.
    if let Some(rows) = &f.alpha0 {
        let live = d.emitter_static.num_alpha0_keys as usize;
        if live > 0 {
            for (k, row) in d.emitter_static.alpha0.keys.iter_mut().take(live).zip(rows) {
                k.x = row[0];
                k.time = row[1];
            }
        } else if matches!(
            d.particle_color.alpha0_type,
            effect_library::ColorType::Constant
        ) {
            if let Some(row) = rows.first() {
                d.particle_color.alpha0 = row[0];
            }
        } else if let Some(row) = rows.first() {
            d.emitter_info.color0_a = row[0];
        }
    }
    // General attributes go on LAST, so that where one names the same field as a legacy record
    // above — `emission.rate` against `emission_rate`, say — the newer, more specific edit is
    // the one that survives. A project written by the current editor only carries these.
    for (id, value) in &f.attrs {
        if !crate::eff_attrs::write(d, id, *value) {
            eprintln!(
                "[EFF-EXPORT] warning: emitter '{}' carries an edit to unknown attribute '{id}' \
                 — skipped",
                d.display_name()
            );
        }
    }
    for edit in &f.subsections {
        let Some(section) = em.subsections.get_mut(edit.index) else {
            eprintln!(
                "[EFF-EXPORT] warning: emitter '{}' has no subsection index {} — skipped",
                d.display_name(),
                edit.index
            );
            continue;
        };
        if section.magic != edit.magic {
            eprintln!(
                "[EFF-EXPORT] warning: emitter '{}' subsection {} is '{}' rather than '{}' — skipped",
                d.display_name(), edit.index, section.magic, edit.magic
            );
            continue;
        }
        for (&offset, &value) in &edit.bytes {
            if let Some(byte) = section.data.get_mut(offset) {
                *byte = value;
            }
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
/// Children are re-parented by depth rather than kept with their original parent: the editor
/// presents the set as a flat, parent-first list, and that is the only ordering an edit can be
/// expressed against.
fn apply_roster(
    ptcl: &mut effect_library::PtclFile,
    roster: &crate::mod_project::EmitterRoster,
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
                "[EFF-EXPORT] warning: emitter list for '{}' names a source emitter '{}' \
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
fn apply_entry_edit(
    namco: &mut effect_library::NamcoEffectFile,
    edit: &crate::mod_project::EntryEdit,
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
fn set_emitter_name(data: &mut effect_library::EmitterData, name: &str) {
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
fn nest_by_depth(
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
