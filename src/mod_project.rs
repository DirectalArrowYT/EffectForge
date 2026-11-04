//! The edit records the effect engine works with: transplants, field edits, texture additions.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EffMod {
    /// Source .eff path relative to the dump export root,
    /// e.g. "effect/fighter/mario/ef_mario.eff".
    pub source_rel: String,
    #[serde(default)]
    pub authored: Vec<AuthoredEdit>,
    /// Emitter sets copied in from a donor eff. Serialized as `one_slot` before the
    /// transplant/one-slot split; the alias keeps pre-split projects loadable.
    #[serde(default, alias = "one_slot")]
    pub transplants: Vec<TransplantOp>,
    /// Pool textures the user replaced with their own image.
    #[serde(default)]
    pub textures: Vec<TextureImport>,
    /// Pool textures the user ADDED, beyond the ones the eff shipped with.
    #[serde(default)]
    pub textures_added: Vec<TextureAddition>,
    /// Names of pool textures the user removed.
    #[serde(default)]
    pub textures_removed: Vec<String>,
    /// Emitter sets whose emitter LIST the user changed — emitters removed, duplicated or
    /// re-ordered. Recorded per set, and only for sets that differ from the source.
    #[serde(default)]
    pub rosters: Vec<EmitterRoster>,
    /// Effect entries whose spawn structure the user changed: which parts play at which frame,
    /// and which external model comes with them.
    #[serde(default)]
    pub entry_edits: Vec<EntryEdit>,
}

impl EffMod {
}

/// The emitter list one effect should end up with.
///
/// Declarative rather than a log of add/remove operations: each slot names an emitter of the
/// SOURCE set to clone, so applying the roster twice gives the same result as applying it once,
/// and a source eff that gained an emitter between sessions cannot shift the whole list by one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmitterRoster {
    pub set_name: String,
    /// The entry (kind) name that plays this set, for the UI and for carrier retargeting.
    #[serde(default)]
    pub entry_name: String,
    pub set_idx: usize,
    pub slots: Vec<EmitterSlot>,
}

/// One emitter in the result, and where its data comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmitterSlot {
    /// Flat parent-first index of the SOURCE emitter this slot clones.
    pub source_idx: usize,
    /// Name of that source emitter, preferred over the index when both are available.
    #[serde(default)]
    pub source_name: String,
    /// Name this emitter carries in the result. A duplicate is renamed so that authored edits,
    /// which address emitters by name, can still tell the two apart.
    #[serde(default)]
    pub name: String,
    /// Nesting depth in the result: 0 is a root emitter, 1 a child of the slot above it.
    #[serde(default)]
    pub depth: u8,
    /// Another emitter set in the same file to clone this slot from, by name. Empty means the
    /// set this roster rebuilds.
    ///
    /// How an effect is assembled from parts of others: a converted hit rebuilt on Smash emitters
    /// takes its glow from one effect, its spark lines from another and its shock sheet from a
    /// third, each already carrying shaders the file holds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_set: String,
}

/// Changes to what one effect entry SPAWNS — the eff header's side of an effect, as opposed to
/// the particle data the emitters carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryEdit {
    /// The entry (kind) name, e.g. `kirby_dash`. Entries are addressed by name only: the entry
    /// table's order is not stable across a transplant, which appends to it.
    pub entry_name: String,
    /// Replacement primary emitter set. `None` leaves it alone; `Some("")` removes it.
    /// Multi-part entries commonly have no primary set because their content lives in variants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emitter_set: Option<String>,
    /// Replacement part list, each with its own start frame. `None` leaves the entry's parts
    /// alone; `Some(empty)` makes it a plain single-part effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants: Option<Vec<VariantEdit>>,
    /// Replacement external model. `None` leaves it alone; `Some` with an empty name removes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelEdit>,
}

/// One part of a multi-part effect: an emitter set, when it starts, and what it hangs off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariantEdit {
    /// Frames after the effect begins before this part comes in.
    pub start_frame: u16,
    /// Emitter set this part plays, by name. Empty means "no set", which the format stores as 0.
    pub set_name: String,
    /// Bone the part attaches to. Empty keeps the effect's own attachment.
    #[serde(default)]
    pub bone: String,
}

/// The model an effect spawns alongside its particles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEdit {
    /// Model name as the eff's model table spells it. Empty removes the model.
    pub name: String,
    /// The model's spawn condition byte.
    #[serde(default)]
    pub flag: u8,
}

/// One pool texture replaced by an image of the user's.
///
/// Keyed by texture NAME, not pool index: the carrier and the exported eff both rebuild their
/// pools (pruning drops everything unreferenced), so an index recorded against the editor's
/// view names a different texture by the time it is applied. Names survive that.
///
/// The PNG is referenced by path rather than embedded, so the project file stays small and a
/// rebuild picks up whatever the image file says now — edit it in your paint program, send
/// again, and the new pixels ship without re-importing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureImport {
    pub texture_name: String,
    pub png_path: String,
    /// The PNG holds the texture's STORED channels rather than the editable form.
    ///
    /// Recorded per import because the two are not interchangeable: reading an editable image as
    /// raw (or the reverse) puts the shape in the wrong channel, which the game draws as a solid
    /// square. Defaults to false, so a project written before this field existed is read as
    /// editable — the only form there was.
    #[serde(default)]
    pub raw: bool,
}

/// A pool texture the user ADDED, on top of the ones the eff shipped with.
///
/// This is how one effect gets a texture of its own. A pool texture is shared by every emitter
/// that samples it, and the `ef_cmn_*` names are shared by dozens of effects inside a single eff,
/// so editing one to change a single effect changes all of them. Adding a private copy and
/// repointing just that emitter is the only way to alter one effect and leave the rest alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureAddition {
    /// The new pool texture's name — unique within the eff.
    pub texture_name: String,
    /// The existing pool texture this one is shaped like: it supplies the format, the channel
    /// swizzle and the required dimensions, and the pixels too when `png_path` is empty.
    pub template_name: String,
    /// Pixels from this PNG. Empty means "an exact copy of `template_name`", which is a rename
    /// rather than a re-encode and so loses nothing.
    #[serde(default)]
    pub png_path: String,
    /// The PNG holds stored channels rather than the editable form. See [`TextureImport::raw`].
    #[serde(default)]
    pub raw: bool,
}

/// One emitter's edited authored fields. Names are stored alongside indices; appliers
/// prefer the name match and fall back to the index with a warning (dump-revision drift).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthoredEdit {
    /// The EMITTER SET name (e.g. `P_KirbyDash`) — what `apply_authored` matches against
    /// `ptcl.emitter_list.emitter_sets[].name`.
    pub set_name: String,
    /// The ENTRY / KIND name (e.g. `kirby_dash`) — a DIFFERENT namespace: this is what the
    /// game spawns, what `TransplantOp::src_set_name` is resolved against (`entry_names`),
    /// and what an effect alias hashes. The two are related by
    /// `entries[i].emitter_set_id - 1 == set_idx`, not by position.
    ///
    /// Empty on projects saved before this field existed; callers that need a kind name must
    /// skip such edits rather than fall back to `set_name`, which would name a kind that does
    /// not exist and ship a carrier the game's loader can hang on.
    #[serde(default)]
    pub entry_name: String,
    pub set_idx: usize,
    pub emitter_name: String,
    pub emitter_idx: usize,
    pub fields: EmitterFieldEdits,
}

/// Absolute new values (pristine values are re-derivable from the source eff).
///
/// `attrs` is the general case and covers every field of the emitter; it is keyed by the stable
/// ids in [`crate::eff_attrs`]. The named fields below it predate that table and are kept for
/// projects saved before it existed — the editor re-records them as `attrs` the first time it
/// loads such a project, and both paths are applied (named first, so `attrs` wins a tie).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmitterFieldEdits {
    /// Attribute id → new value, e.g. `"emission.rate": 12.5`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub attrs: std::collections::BTreeMap<String, crate::eff_attrs::AttrValue>,
    /// Sparse byte edits to emitter child sections. The section index disambiguates repeated
    /// magic values; the magic prevents a changed source file from applying bytes to the wrong
    /// section. Storing only changed offsets keeps projects small and avoids embedding source
    /// subsection blobs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subsections: Vec<SubsectionEdit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission_rate: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emitter_scale: Option<[f32; 3]>,
    /// Color key rows: [r, g, b, time]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color0: Option<Vec<[f32; 4]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color1: Option<Vec<[f32; 4]>>,
    /// Alpha key rows: [value, time]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha0: Option<Vec<[f32; 2]>>,
    /// Pool texture NAME this emitter's `sampler0` should read instead of its original one.
    ///
    /// A name rather than the picker's index: the index is against the editor's view of the
    /// merged eff, and every path that ships these bytes rebuilds its own pool — the carrier
    /// prunes everything unreferenced, so indices there mean something else entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubsectionEdit {
    pub index: usize,
    pub magic: String,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub bytes: std::collections::BTreeMap<usize, u8>,
}

impl EmitterFieldEdits {

}


/// A TRANSPLANT: copy an emitter set from a donor eff into this fighter's eff, either
/// under a new entry name or replacing an existing entry in place.
///
/// `one_slot_slots` is the (optional) ONE-SLOT scoping layered on top of the transplant —
/// the transplant is the operation, the slot list narrows which costumes see it.
///
/// Was named `OneSlotOp` before the transplant/one-slot split. The struct name is invisible
/// to serde, but the renamed `one_slot_slots` field carries a `serde(alias)` for its old
/// on-disk name (`slots`) so projects saved before the split still load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransplantOp {
    pub new_entry_name: String,
    /// Donor .eff relative to the export root ("" = same file as the target).
    pub src_file_rel: String,
    pub src_set_name: String,
    pub src_set_idx: usize,
    /// ONE-SLOT scoping: costume slots this transplant applies to (0 = c00 …). Slot numbers
    /// are real costume indices and are NOT limited to 0–7 — added-costume mods use much
    /// larger ones. Empty = every costume: the transplant lands in the base
    /// ef_<fighter>.eff; otherwise it lands in ef_<fighter>_cXX.eff files, one per slot.
    /// Serialized as `slots` before the transplant/one-slot split.
    #[serde(default, alias = "slots", skip_serializing_if = "Vec::is_empty")]
    pub one_slot_slots: Vec<u8>,
    /// When set, the donor REPLACES this existing entry's emitter set(s) in place — every
    /// use switches, with no ACMD redirect needed — instead of being appended as a new
    /// entry. This is how a one-slot-scoped transplant is normally authored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace_entry: Option<String>,
}
