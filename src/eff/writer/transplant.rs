//! Transplants: an effect's emitter sets copied in, from this file or a donor file.

use anyhow::{anyhow, Context, Result};

use super::{visit_emitters, visit_emitters_ref};
use super::prune::append_unique_resource_ids;
use crate::eff::edits::TransplantOp;

/// Append the donor's shader container to the destination's, and return the index its first variation now has.
pub(super) fn merge_container(dest: &mut Option<Vec<u8>>, donor: &[u8], what: &str) -> Result<i32> {
    let dest_bin = dest.clone().unwrap_or_default();
    if dest_bin.is_empty() {
        *dest = Some(donor.to_vec());
        return Ok(0);
    }
    let base = effect_library::bnsh::BnshFile::read(&dest_bin)
        .with_context(|| format!("reading destination {what} variations"))?
        .variations
        .len() as i32;
    *dest = Some(
        effect_library::bnsh::merge_variation_files(&[dest_bin, donor.to_vec()])
            .with_context(|| format!("merging the donor {what} container"))?,
    );
    Ok(base)
}

/// The emitters of `emitters` that `names` names, each with its children, in file order. An emitter named as well as
/// its parent comes along with the parent; one whose parent is not named comes out as a top-level emitter.
fn pick_emitters(
    emitters: Vec<effect_library::structs::Emitter>,
    names: &[String],
    picked: &mut Vec<effect_library::structs::Emitter>,
) {
    for mut em in emitters {
        if names.iter().any(|n| em.data.display_name().eq_ignore_ascii_case(n)) {
            picked.push(em);
        } else {
            pick_emitters(std::mem::take(&mut em.children), names, picked);
        }
    }
}

/// Narrow `sets` to the op's `only_emitters`. Fails when none of them is there.
fn narrow_to_picked(op: &TransplantOp, sets: &mut [effect_library::structs::EmitterSet]) -> Result<()> {
    if op.only_emitters.is_empty() {
        return Ok(());
    }
    let mut found = 0;
    for set in sets.iter_mut() {
        let mut picked = Vec::new();
        pick_emitters(std::mem::take(&mut set.emitters), &op.only_emitters, &mut picked);
        found += picked.len();
        set.emitters = picked;
    }
    if found == 0 {
        anyhow::bail!(
            "'{}' has none of the layers {:?}",
            op.src_set_name,
            op.only_emitters
        );
    }
    Ok(())
}

/// Give `em` and its children names `taken` does not hold yet (`<name>_2`, `_3`, ...), and add them to it.
fn rename_unused(em: &mut effect_library::structs::Emitter, taken: &mut Vec<String>) {
    let name = em.data.display_name();
    if taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        let fresh = (2..)
            .map(|n| format!("{name}_{n}"))
            .find(|n| !taken.iter().any(|t| t.eq_ignore_ascii_case(n)))
            .unwrap();
        super::emitters::set_emitter_name(&mut em.data, &fresh);
        em.cached_binary = None;
        taken.push(fresh);
    } else {
        taken.push(name);
    }
    for child in &mut em.children {
        rename_unused(child, taken);
    }
}

/// Every emitter name in `emitters`, children included.
fn emitter_names(emitters: &[effect_library::structs::Emitter]) -> Vec<String> {
    let mut names = Vec::new();
    visit_emitters_ref(emitters, &mut |em| names.push(em.data.display_name()));
    names
}

/// Add `donor_sets`' emitters to the set of the one-part effect `target`, as more layers.
fn add_layers(
    namco: &mut effect_library::NamcoEffectFile,
    target: &str,
    donor_sets: Vec<effect_library::structs::EmitterSet>,
) -> Result<()> {
    let ti = namco
        .entry_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(target))
        .ok_or_else(|| anyhow!("no effect '{target}' to add layers to"))?;
    let entry = &namco.entries[ti];
    if entry.variant_count != 0 {
        anyhow::bail!("'{target}' plays in parts; layers can only be added to a one-part effect");
    }
    let set_idx = (entry.emitter_set_id as usize)
        .checked_sub(1)
        .ok_or_else(|| anyhow!("'{target}' has no emitter set"))?;
    let set = namco
        .ptcl_file
        .as_mut()
        .ok_or_else(|| anyhow!("eff has no embedded PTCL"))?
        .emitter_list
        .emitter_sets
        .get_mut(set_idx)
        .ok_or_else(|| anyhow!("'{target}' names emitter set {set_idx}, which is out of range"))?;
    let mut taken = emitter_names(&set.emitters);
    for donor in donor_sets {
        for mut em in donor.emitters {
            rename_unused(&mut em, &mut taken);
            set.emitters.push(em);
        }
    }
    Ok(())
}

/// Duplicate a donor entry's emitter set(s) within the same eff under a new entry name —
/// the ACMD side then retargets the fighter's call to `new_entry_name`, leaving the
/// original entry vanilla for everyone else.
pub(super) fn apply_transplant_same_file(
    namco: &mut effect_library::NamcoEffectFile,
    op: &TransplantOp,
) -> Result<()> {
    if op.replace_entry.is_none()
        && op.into_entry.is_none()
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

    if let Some(target) = &op.into_entry {
        let sets = &namco
            .ptcl_file
            .as_ref()
            .ok_or_else(|| anyhow!("eff has no embedded PTCL"))?
            .emitter_list
            .emitter_sets;
        let mut donor_sets = Vec::new();
        for raw in entry_set_ids(namco, &donor)? {
            donor_sets.push(
                sets.get(raw as usize - 1)
                    .ok_or_else(|| anyhow!("donor emitter set id {raw} out of range"))?
                    .clone(),
            );
        }
        narrow_to_picked(op, &mut donor_sets)?;
        return add_layers(namco, target, donor_sets);
    }

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
            narrow_to_picked(op, std::slice::from_mut(&mut new_set))?;
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

/// The raw (1-based) emitter set ids an entry plays: its own set, then its parts'.
fn entry_set_ids(
    namco: &effect_library::NamcoEffectFile,
    entry: &effect_library::namco_file::EffectHeader,
) -> Result<Vec<u32>> {
    let mut ids = Vec::new();
    if entry.emitter_set_id != 0 {
        ids.push(entry.emitter_set_id);
    }
    if entry.variant_count != 0 {
        let start = (entry.variant_start_idx as usize)
            .checked_sub(1)
            .ok_or_else(|| anyhow!("donor variant start id 0"))?;
        let parts = namco
            .effect_variants
            .get(start..start + entry.variant_count as usize)
            .ok_or_else(|| anyhow!("donor variant range out of bounds"))?;
        ids.extend(parts.iter().filter(|v| v.emitter_set_id != 0).map(|v| v.emitter_set_id as u32));
    }
    Ok(ids)
}

/// Append `new_entry` under the op's new name, or — replace mode — repoint an existing
/// entry at the cloned set(s) (its name and slot in the table stay; the old set is
/// orphaned but harmless). Replace is the costume-scoped semantic: every ACMD use of
/// the entry switches on that costume with no redirect needed.
pub(super) fn finish_transplant_entry(
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
pub(super) fn apply_transplant_cross_file(
    namco: &mut effect_library::NamcoEffectFile,
    donor: &effect_library::NamcoEffectFile,
    op: &TransplantOp,
    destination_is_fighter: bool,
) -> Result<()> {
    if op.replace_entry.is_none()
        && op.into_entry.is_none()
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

    // Only the chosen layers, so only their textures, models and shaders come along.
    narrow_to_picked(op, &mut donor_sets)?;

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
    // is empty: the file should gain only the resources the selected effect uses.
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
            let donor_bin = donor_shader
                .binary_data
                .as_ref()
                .ok_or_else(|| anyhow!("donor shader section has no binary"))?;
            shader_index_base = merge_container(&mut dest_shader.binary_data, donor_bin, "shader")?;
        }
        if uses_compute {
            let donor_bin = donor_shader
                .compute_binary
                .as_ref()
                .ok_or_else(|| anyhow!("donor compute-shader section has no binary"))?;
            compute_index_base = merge_container(&mut dest_shader.compute_binary, donor_bin, "compute-shader")?;
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
            // this by changing mode 0 into the invalid value 38).
            em.cached_binary = None; // indices moved → cached EMTR blob is stale
        });
    };

    if let Some(target) = &op.into_entry {
        for set in &mut donor_sets {
            remap(set);
        }
        return add_layers(namco, target, donor_sets);
    }

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
    // unrelated row in the destination, while dropping it would lose the model.
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
    // rejects type-0 entries. A non-fighter destination needs the inverse: fighter-type
    // 0x01xx entries are not registered by the assist loader.
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

pub(super) fn destination_entry_kind(kind: u16, destination_is_fighter: bool) -> u16 {
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

pub(super) fn remap_shader_indices(
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
