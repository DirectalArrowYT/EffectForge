//! Texture pool edits: textures added, replaced by an image, or removed.

use anyhow::{Context, Result};

use super::visit_emitters_ref;
use crate::eff::edits::TextureImport;

/// Every texture GUID some surviving emitter samples.
pub(super) fn sampled_texture_ids(ptcl: &effect_library::PtclFile) -> std::collections::HashSet<u64> {
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
/// stale addition must not take a working file down with it.
pub(super) fn apply_texture_additions(
    file: &mut effect_library::NamcoEffectFile,
    additions: &[crate::eff::edits::TextureAddition],
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
            // Already present: a rebuild of edits that were applied before.
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
            crate::eff::textures::duplicate_texture(pool, &names, template, &add.texture_name)
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
                crate::eff::textures::Form::Raw
            } else {
                crate::eff::textures::Form::Editable
            };
            let (rebuilt, _) = crate::eff::textures::add_texture_from_png(
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
        let id = crate::eff::textures::unused_descriptor_id(&taken, &add.texture_name);
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
/// Reaching this means either a stale edit
/// or a swap that could not be placed.
pub(super) fn apply_texture_removals(
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
        let rebuilt = crate::eff::textures::remove_texture(pool, &names, index)
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
/// features, and one stale import must not take a working file down with it.
pub(super) fn apply_texture_imports(
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
            crate::eff::textures::Form::Raw
        } else {
            crate::eff::textures::Form::Editable
        };
        let (rebuilt, report) = crate::eff::textures::replace_with_png(
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
