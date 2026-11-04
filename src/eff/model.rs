//! What Forge reads out of an `.eff` when it opens one: the effects the game can spawn, the layers (emitters) each
//! plays and every value of each layer, the texture pool, and what the game's shaders need to draw each layer.

use std::path::Path;

use anyhow::{Context, Result};
use effect_library::ptcl_file::TextureDescriptor;
use effect_library::{Emitter, NamcoEffectFile};

use crate::draw::inputs::GameEmitter;
use crate::eff::fields::AttrValue;

/// One layer of an effect.
#[derive(Debug, Clone)]
pub struct EmitterDef {
    pub name: String,
    /// Every value, in the order of [`crate::eff::fields::table`]. `None` where the layer has no such block at all
    /// (no third sampler, say), which is not the same as zero.
    pub attrs: Vec<Option<AttrValue>>,
    /// The magic of each of its extra sections (`CSDP`, `FRND`, ...).
    pub sections: Vec<String>,
    /// 0 for a top-level layer, 1 for its child, and so on.
    pub depth: u8,
    /// The pool textures its first and second samplers read. `None` when it samples nothing the pool has.
    pub texture_index: Option<u32>,
    pub texture_index1: Option<u32>,
}

/// An emitter set: the layers one effect plays, parents before their children.
#[derive(Debug, Clone)]
pub struct EmitterSet {
    pub name: String,
    pub emitters: Vec<EmitterDef>,
}

/// A texture in the pool. Width and height are 0 when its header could not be read.
#[derive(Debug, Clone)]
pub struct TextureInfo {
    pub tex_name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default)]
pub struct PtclFile {
    pub emitter_sets: Vec<EmitterSet>,
    pub bntx_textures: Vec<TextureInfo>,
}

/// An effect as the game spawns it, by name, and the emitter set it plays (`None` when its parts are all variants).
#[derive(Debug, Clone, Default)]
pub struct EffEntryInfo {
    pub name: String,
    pub set_idx: Option<usize>,
}

pub struct LoadedEffect {
    pub ptcl: PtclFile,
    /// Every effect, in the file's order.
    pub entries: Vec<EffEntryInfo>,
    /// The texture pool (a BNTX) as stored, for decoding a texture to edit it.
    pub texture_pool: Option<Vec<u8>>,
    /// Per set, per layer in `ptcl`'s order: what drawing it with the game's shaders needs. `None` for a layer that
    /// path cannot draw.
    pub game: Vec<Vec<Option<GameEmitter>>>,
}

pub fn load_effect(path: &Path) -> Result<LoadedEffect> {
    let bytes = std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let file = NamcoEffectFile::load(&bytes).with_context(|| format!("failed to parse {}", path.display()))?;
    // Ids in the entry table count from 1; 0 is none.
    let entries = file
        .entry_names
        .iter()
        .zip(&file.entries)
        .map(|(name, entry)| EffEntryInfo { name: name.clone(), set_idx: (entry.emitter_set_id as usize).checked_sub(1) })
        .collect();
    let Some(ptcl) = file.ptcl_file.as_ref() else {
        return Ok(LoadedEffect { ptcl: PtclFile::default(), entries, texture_pool: None, game: Vec::new() });
    };
    let pool = ptcl.texture_info.as_ref();
    let descriptors = pool.map(|i| i.descriptors.as_slice()).unwrap_or_default();
    let pool_bytes = pool.and_then(|i| i.binary_data.as_deref());

    let bntx_textures = descriptors
        .iter()
        .enumerate()
        .map(|(index, texture)| {
            let size = pool_bytes.and_then(|p| crate::eff::textures::describe(p, index, &texture.name).ok());
            TextureInfo {
                tex_name: texture.name.clone(),
                width: size.as_ref().map_or(0, |d| d.width),
                height: size.as_ref().map_or(0, |d| d.height),
            }
        })
        .collect();

    let sets = &ptcl.emitter_list.emitter_sets;
    let emitter_sets = sets
        .iter()
        .map(|set| {
            let mut emitters = Vec::new();
            each_emitter(&set.emitters, 0, &mut |e, depth| emitters.push(layer(e, descriptors, depth)));
            EmitterSet { name: set.name.clone(), emitters }
        })
        .collect();
    let game = sets
        .iter()
        .map(|set| {
            let mut out = Vec::new();
            each_emitter(&set.emitters, 0, &mut |e, _| out.push(GameEmitter::read(e, descriptors, ptcl.vfx_version)));
            out
        })
        .collect();

    Ok(LoadedEffect {
        ptcl: PtclFile { emitter_sets, bntx_textures },
        entries,
        texture_pool: pool.and_then(|i| i.binary_data.clone()),
        game,
    })
}

/// Every emitter of a tree, parents before their children, with its depth.
fn each_emitter(list: &[Emitter], depth: u8, f: &mut dyn FnMut(&Emitter, u8)) {
    for e in list {
        f(e, depth);
        each_emitter(&e.children, depth.saturating_add(1), f);
    }
}

fn layer(emitter: &Emitter, textures: &[TextureDescriptor], depth: u8) -> EmitterDef {
    let d = &emitter.data;
    let texture = |sampler: &Option<effect_library::TextureSampler>| {
        let id = sampler.as_ref()?.texture_id;
        textures.iter().position(|t| t.id == id).map(|i| i as u32)
    };
    EmitterDef {
        name: d.display_name(),
        attrs: crate::eff::fields::read_all(d),
        sections: emitter.subsections.iter().map(|s| s.magic.clone()).collect(),
        depth,
        texture_index: texture(&d.sampler0),
        texture_index1: texture(&d.sampler1),
    }
}
