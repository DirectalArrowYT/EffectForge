//! Every value of an emitter Forge can show and set, as one table.
//!
//! A row names one field of `effect_library::EmitterData` by its path (`emission.rate`, `sampler0.wrap_u`), which
//! is also its id in a batch or recipe file, so a path must not change. It carries the group the panel shows it
//! under, a label, a description and the kind of control, and reads and writes the field. Names and descriptions
//! follow LilyLavender's EffectResearch (`documentation/EmitterData.md`); fields it marks as padding are left out.
//!
//! A field in a block the emitter may not have (a sampler, a texture animation, the combiner) reads as `None` when
//! the block is absent, which the panel hides, and setting it does nothing.

use effect_library::{ColorType, EmitterData, WrapMode};
use serde::{Deserialize, Serialize};

/// A value as a batch file holds it: a plain number (`{"emission.rate": 12.5}`). The field's kind says how it is
/// stored, so an integer field set from a float rounds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttrValue {
    Int(i64),
    UInt(u64),
    Float(f32),
}

impl AttrValue {
    pub fn as_f32(self) -> f32 {
        match self {
            Self::Int(v) => v as f32,
            Self::UInt(v) => v as f32,
            Self::Float(v) => v,
        }
    }

    pub fn as_i64(self) -> i64 {
        match self {
            Self::Int(v) => v,
            Self::UInt(v) => v.min(i64::MAX as u64) as i64,
            Self::Float(v) => v.round() as i64,
        }
    }

    pub fn as_u64(self) -> u64 {
        match self {
            Self::Int(v) => v.max(0) as u64,
            Self::UInt(v) => v,
            Self::Float(v) => v.round().clamp(0.0, u64::MAX as f32) as u64,
        }
    }

    pub fn as_bool(self) -> bool {
        self.as_i64() != 0
    }
}

/// The control a row gets.
#[derive(Debug, Clone, Copy)]
pub enum AttrKind {
    /// `speed` is what one pixel of drag adds.
    Float { speed: f32 },
    Int,
    UInt,
    /// On or off.
    Flag,
    /// A small number whose values have names; the stored value is the index.
    Enum(&'static [&'static str]),
}

pub struct Attr {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub doc: &'static str,
    pub kind: AttrKind,
    pub get: fn(&EmitterData) -> Option<AttrValue>,
    pub set: fn(&mut EmitterData, AttrValue),
}

/// A stored field read and written as a row of a kind. Integers are read as `Int` or (for `UInt`) `UInt`, and a
/// value past what the field holds is set to the nearest end; a flag is set to 0 or 1.
trait Field {
    fn get(&self, kind: AttrKind) -> Option<AttrValue>;
    fn set(&mut self, kind: AttrKind, value: AttrValue);
}

macro_rules! integer_fields {
    ($($t:ty),*) => {$(
        impl Field for $t {
            fn get(&self, kind: AttrKind) -> Option<AttrValue> {
                Some(match kind {
                    AttrKind::UInt => AttrValue::UInt(*self as u64),
                    _ => AttrValue::Int(*self as i64),
                })
            }
            fn set(&mut self, kind: AttrKind, value: AttrValue) {
                *self = match kind {
                    AttrKind::UInt => value.as_u64().min(<$t>::MAX as u64) as $t,
                    AttrKind::Flag => value.as_bool() as $t,
                    _ => (value.as_i64() as i128).clamp(<$t>::MIN as i128, <$t>::MAX as i128) as $t,
                };
            }
        }
    )*};
}
integer_fields!(i8, i16, i32, i64, u8, u16, u32, u64);

impl Field for f32 {
    fn get(&self, _: AttrKind) -> Option<AttrValue> {
        Some(AttrValue::Float(*self))
    }
    fn set(&mut self, _: AttrKind, value: AttrValue) {
        *self = value.as_f32();
    }
}

impl Field for bool {
    fn get(&self, _: AttrKind) -> Option<AttrValue> {
        Some(AttrValue::Int(*self as i64))
    }
    fn set(&mut self, _: AttrKind, value: AttrValue) {
        *self = value.as_bool();
    }
}

macro_rules! enum_fields {
    ($($t:ty),*) => {$(
        impl Field for $t {
            fn get(&self, _: AttrKind) -> Option<AttrValue> {
                Some(AttrValue::Int(self.as_u8() as i64))
            }
            fn set(&mut self, _: AttrKind, value: AttrValue) {
                *self = <$t>::from_u8(value.as_i64().clamp(0, 255) as u8);
            }
        }
    )*};
}
enum_fields!(ColorType, WrapMode);

/// A field the file may not have.
impl<T: Field> Field for Option<T> {
    fn get(&self, kind: AttrKind) -> Option<AttrValue> {
        self.as_ref()?.get(kind)
    }
    fn set(&mut self, kind: AttrKind, value: AttrValue) {
        if let Some(field) = self {
            field.set(kind, value);
        }
    }
}

fn float(speed: f32) -> AttrKind {
    AttrKind::Float { speed }
}
fn pick(names: &'static [&'static str]) -> AttrKind {
    AttrKind::Enum(names)
}
const INT: AttrKind = AttrKind::Int;
const UINT: AttrKind = AttrKind::UInt;
const FLAG: AttrKind = AttrKind::Flag;

/// One row. Forms of the path:
/// - `a.b.c`: a field of the emitter; the id is the path.
/// - `?block.a`: a field of an optional block.
/// - `@cmb f` / `@wide f`: combiner field `f` (id `combiner.f`). The combiner's layout depends on the file's
///   version; the blend bytes are in all three, the `@wide` fields in all but version 36's.
/// - `id => (path tokens)`: an explicit id, for rows made in a loop.
macro_rules! row {
    ($g:expr, [$k:expr] ? $block:ident . $first:ident $(. $rest:ident)*, $label:expr, $doc:expr) => {
        Attr {
            id: concat!(stringify!($block), ".", stringify!($first) $(, ".", stringify!($rest))*),
            group: $g, label: $label, doc: $doc, kind: $k,
            get: |d| d.$block.as_ref().and_then(|b| Field::get(&b.$first $(.$rest)*, $k)),
            set: |d, v| if let Some(b) = d.$block.as_mut() { Field::set(&mut b.$first $(.$rest)*, $k, v) },
        }
    };
    ($g:expr, [$k:expr] @cmb $f:ident, $label:expr, $doc:expr) => {
        Attr {
            id: concat!("combiner.", stringify!($f)),
            group: $g, label: $label, doc: $doc, kind: $k,
            get: |d| match d.combiner.as_ref()? {
                Combiner::Legacy(c) => Field::get(&c.$f, $k),
                Combiner::V36(c) => Field::get(&c.$f, $k),
                Combiner::V40(c) => Field::get(&c.$f, $k),
            },
            set: |d, v| match d.combiner.as_mut() {
                Some(Combiner::Legacy(c)) => Field::set(&mut c.$f, $k, v),
                Some(Combiner::V36(c)) => Field::set(&mut c.$f, $k, v),
                Some(Combiner::V40(c)) => Field::set(&mut c.$f, $k, v),
                None => {}
            },
        }
    };
    ($g:expr, [$k:expr] @wide $f:ident, $label:expr, $doc:expr) => {
        Attr {
            id: concat!("combiner.", stringify!($f)),
            group: $g, label: $label, doc: $doc, kind: $k,
            get: |d| match d.combiner.as_ref()? {
                Combiner::Legacy(c) => Field::get(&c.$f, $k),
                Combiner::V40(c) => Field::get(&c.$f, $k),
                Combiner::V36(_) => None,
            },
            set: |d, v| match d.combiner.as_mut() {
                Some(Combiner::Legacy(c)) => Field::set(&mut c.$f, $k, v),
                Some(Combiner::V40(c)) => Field::set(&mut c.$f, $k, v),
                _ => {}
            },
        }
    };
    ($g:expr, [$k:expr] $first:ident $(. $rest:ident)*, $label:expr, $doc:expr) => {
        Attr {
            id: concat!(stringify!($first) $(, ".", stringify!($rest))*),
            group: $g, label: $label, doc: $doc, kind: $k,
            get: |d| Field::get(&d.$first $(.$rest)*, $k),
            set: |d, v| Field::set(&mut d.$first $(.$rest)*, $k, v),
        }
    };
    ($g:expr, [$k:expr] $id:expr => ($($path:tt)+), $label:expr, $doc:expr) => {
        Attr {
            id: $id, group: $g, label: $label, doc: $doc, kind: $k,
            get: |d| Field::get(&d.$($path)+, $k),
            set: |d, v| Field::set(&mut d.$($path)+, $k, v),
        }
    };
}

/// Rows of one group, pushed onto `v`. Each path is in parentheses so a row has one shape.
macro_rules! rows {
    ($v:ident, $g:expr; $([$k:expr] ($($path:tt)+), $label:expr, $doc:expr);* $(;)?) => {
        $($v.push(row!($g, [$k] $($path)+, $label, $doc));)*
    };
}

use effect_library::EmitterCombinerVariant as Combiner;

// Groups, in the order the panel stacks them, named after the sections of the emitter binary.
pub const G_COMMON: &str = "Common";
pub const G_EMISSION: &str = "Emission";
pub const G_SHAPE: &str = "Emitter shape";
pub const G_INFO: &str = "Emitter transform & colour";
pub const G_INFO_FLAGS: &str = "Emitter behaviour";
pub const G_PARTICLE: &str = "Particle";
pub const G_PTCL_VELOCITY: &str = "Particle velocity";
pub const G_PTCL_COLOR: &str = "Particle colour";
pub const G_PTCL_SCALE: &str = "Particle scale";
pub const G_PTCL_ROTATE: &str = "Particle rotation";
pub const G_MOTION: &str = "Gravity & air resistance";
pub const G_FLUCTUATION: &str = "Fluctuation";
pub const G_ANIM_RATES: &str = "Animation keys & loops";
pub const G_COLOR_KEYS: &str = "Colour keyframes";
pub const G_ALPHA_KEYS: &str = "Alpha keyframes";
pub const G_SCALE_KEYS: &str = "Scale keyframes";
pub const G_PARAM_KEYS: &str = "Shader coefficient keyframes";
pub const G_RENDER: &str = "Render state";
pub const G_COMBINER: &str = "Combiner";
pub const G_ALPHA_FX: &str = "Soft / fresnel / distance alpha";
pub const G_INHERIT: &str = "Child inheritance";
pub const G_SHADER: &str = "Shader references";
pub const G_SAMPLER0: &str = "Sampler 0";
pub const G_SAMPLER1: &str = "Sampler 1";
pub const G_SAMPLER2: &str = "Sampler 2";
pub const G_TEXANIM0: &str = "Texture 0 animation";
pub const G_TEXANIM1: &str = "Texture 1 animation";
pub const G_TEXANIM2: &str = "Texture 2 animation";
pub const G_TEXPAT0: &str = "Texture 0 pattern anim";
pub const G_TEXPAT1: &str = "Texture 1 pattern anim";
pub const G_TEXPAT2: &str = "Texture 2 pattern anim";
pub const G_TEXUV0: &str = "Texture 0 UV anim";
pub const G_TEXUV1: &str = "Texture 1 UV anim";
pub const G_TEXUV2: &str = "Texture 2 UV anim";

pub const GROUPS: &[&str] = &[
    G_COMMON, G_EMISSION, G_SHAPE, G_INFO, G_INFO_FLAGS, G_PARTICLE, G_PTCL_VELOCITY, G_PTCL_COLOR, G_PTCL_SCALE,
    G_PTCL_ROTATE, G_MOTION, G_FLUCTUATION, G_ANIM_RATES, G_COLOR_KEYS, G_ALPHA_KEYS, G_SCALE_KEYS, G_PARAM_KEYS,
    G_RENDER, G_COMBINER, G_ALPHA_FX, G_INHERIT, G_SHADER, G_SAMPLER0, G_SAMPLER1, G_SAMPLER2, G_TEXANIM0,
    G_TEXANIM1, G_TEXANIM2, G_TEXPAT0, G_TEXPAT1, G_TEXPAT2, G_TEXUV0, G_TEXUV1, G_TEXUV2,
];

pub const WRAP_MODES: &[&str] = &["Mirror", "Repeat", "ClampEdge", "MirrorOnce"];
pub const COLOR_TYPES: &[&str] = &["Constant", "Random", "Animated 8-key"];
/// `render_state.blend_type`: how a particle's colour meets what is behind it.
pub const BLEND_TYPES: &[&str] = &["Normal (over, by opacity)", "Glow (adds)", "Darken (subtracts)"];
/// `emitter_info.follow_type`.
pub const FOLLOW_TYPES: &[&str] = &["Moves with the bone", "Stays where born", "Follows position only"];
/// `shape_info.volume_type`: where in the layer particles appear.
pub const VOLUME_TYPES: &[&str] = &[
    "Point", "Circle", "Circle (even spacing)", "Filled circle", "Sphere", "Sphere (even)", "Sphere (even, 64)",
    "Filled sphere", "Cylinder", "Filled cylinder", "Box", "Filled box", "Line", "Line (even spacing)", "Rectangle",
    "Model surface",
];
/// `shape_info.arc_type`: how a sphere's sweep cuts it down.
pub const ARC_TYPES: &[&str] = &["Wedge (by longitude)", "Cap (by latitude)"];
/// `shape_info.volume_latitude_dir`: which way a latitude cap points.
pub const AXIS_DIRECTIONS: &[&str] = &["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

/// The value a name of an enum row stands for: its index, the name read without case, or the number itself.
pub fn enum_value(kind: AttrKind, text: &str) -> Option<f32> {
    let AttrKind::Enum(names) = kind else { return text.parse().ok() };
    let text = text.trim();
    names.iter().position(|n| n.eq_ignore_ascii_case(text) || n.split(" (").next().is_some_and(|short| short.eq_ignore_ascii_case(text)))
        .map(|i| i as f32).or_else(|| text.parse().ok())
}

/// A row's value as text: the name, for an enum row, after the number.
pub fn show_value(kind: AttrKind, value: f32) -> String {
    match kind {
        AttrKind::Enum(names) => match names.get(value as usize).filter(|_| value >= 0.0 && value.fract() == 0.0) {
            Some(name) => format!("{value} ({name})"),
            None => format!("{value}"),
        },
        _ => format!("{value}"),
    }
}

fn build() -> Vec<Attr> {
    let mut v = Vec::with_capacity(1100);
    rows!(v, G_COMMON;
        [UINT] (flag), "Flag", "Emitter flag word";
        [UINT] (random_seed), "Random seed", "Random seed for the emitter";
        [UINT] (emitter_static.flags1), "Static flags 1", "Emitter static flag word 1";
        [UINT] (emitter_static.flags2), "Static flags 2", "Emitter static flag word 2";
        [UINT] (emitter_static.flags3), "Static flags 3", "Emitter static flag word 3";
        [UINT] (emitter_static.flags4), "Static flags 4", "Emitter static flag word 4";
    );
    rows!(v, G_EMISSION;
        [FLAG] (emission.is_one_time), "One time", "Whether to only play once";
        [FLAG] (emission.is_world_gravity), "World gravity", "Whether to apply gravity in world coordinates";
        [FLAG] (emission.is_emit_dist_enabled), "Distance emission", "Whether to use distance emission";
        [FLAG] (emission.is_world_oriented_velocity), "World oriented velocity", "Whether to apply the specified oriented initial velocity in world coordinates";
        [INT] (emission.start), "Start", "Emission start frame";
        [INT] (emission.timing), "Timing", "Emission start timing";
        [INT] (emission.duration), "Duration", "Emission time";
        [float(0.05)] (emission.rate), "Rate", "Emission rate";
        [float(0.05)] (emission.rate_random), "Rate random", "Discharge rate random";
        [INT] (emission.interval), "Interval", "Emission interval";
        [float(0.05)] (emission.interval_random), "Interval random", "Emission interval random";
        [float(0.01)] (emission.position_random), "Position random", "Initial position randomness";
        [float(0.01)] (emission.gravity_scale), "Gravity scale", "Gravity scale";
        [float(0.01)] (emission.gravity_dir_x), "Gravity dir X", "Gravity direction X";
        [float(0.01)] (emission.gravity_dir_y), "Gravity dir Y", "Gravity direction Y";
        [float(0.01)] (emission.gravity_dir_z), "Gravity dir Z", "Gravity direction Z";
        [float(0.01)] (emission.emitter_dist_unit), "Distance unit", "Emission interval (distance)";
        [float(0.01)] (emission.emitter_dist_min), "Distance min", "Minimum translation distance allowed per frame";
        [float(0.01)] (emission.emitter_dist_max), "Distance max", "Maximum translation distance allowed per frame";
        [float(0.01)] (emission.emitter_dist_marg), "Distance margin", "Threshold for traverse distance truncation";
        [INT] (emission.emitter_dist_particles_max), "Distance particles max", "Maximum particle emissions when using distance emission";
    );
    rows!(v, G_SHAPE;
        [pick(VOLUME_TYPES)] (shape_info.volume_type), "Volume type", "Where in the layer particles appear";
        [INT] (shape_info.sweep_start_random), "Sweep start random", "Arc width randomness";
        [pick(ARC_TYPES)] (shape_info.arc_type), "Arc type", "How a sphere's sweep cuts it: a wedge by longitude, or a cap round the latitude direction";
        [FLAG] (shape_info.is_volume_latitude_enabled), "Volume latitude", "Unused";
        [INT] (shape_info.volume_tbl_index), "Volume table index", "Sphere volume table index";
        [INT] (shape_info.volume_tbl_index64), "Volume table index 64", "Sphere 64 volume table index";
        [pick(AXIS_DIRECTIONS)] (shape_info.volume_latitude_dir), "Latitude direction", "Which way a sphere's latitude cap points (arc type Cap)";
        [FLAG] (shape_info.is_gpu_emitter), "GPU emitter", "Whether to enable the GPU emitter";
        [float(0.01)] (shape_info.sweep_longitude), "Sweep longitude", "Value to use for calculating arc";
        [float(0.01)] (shape_info.sweep_latitude), "Sweep latitude", "Latitude to use for calculating arc";
        [float(0.01)] (shape_info.sweep_start), "Sweep start", "Arc width (start)";
        [float(0.01)] (shape_info.volume_surface_pos_rand), "Surface position random", "Random position on emitter shape surface";
        [float(0.01)] (shape_info.caliber_ratio), "Caliber ratio", "Caliber ratio";
        [float(0.01)] (shape_info.line_center), "Line center", "Line center";
        [float(0.01)] (shape_info.line_length), "Line length", "Line length";
        [float(0.01)] (shape_info.volume_radius_x), "Volume radius X", "Volume radius";
        [float(0.01)] (shape_info.volume_radius_y), "Volume radius Y", "Volume radius";
        [float(0.01)] (shape_info.volume_radius_z), "Volume radius Z", "Volume radius";
        [float(0.01)] (shape_info.volume_form_scale_x), "Form scale X", "Emitter scale";
        [float(0.01)] (shape_info.volume_form_scale_y), "Form scale Y", "Emitter scale";
        [float(0.01)] (shape_info.volume_form_scale_z), "Form scale Z", "Emitter scale";
        [INT] (shape_info.prim_emit_type), "Primitive emit type", "Emitter type when a primitive was specified";
        [UINT] (shape_info.primitive_index), "Primitive index", "Primitive index — only meaningful when this eff holds that primitive";
        [INT] (shape_info.num_divide_circle), "Divide circle", "Number of equilateral circular segments";
        [INT] (shape_info.num_divide_circle_random), "Divide circle random", "Random number of equilateral circular segments";
        [INT] (shape_info.num_divide_line), "Divide line", "Number of equal length line segment divisions";
        [INT] (shape_info.num_divide_line_random), "Divide line random", "Random number of equal length line segment divisions";
        [FLAG] (shape_info.is_on_another_binary_volume_primitive), "External volume primitive", "Whether the emitter shape primitive is stored in another binary";
    );
    rows!(v, G_INFO_FLAGS;
        [FLAG] (emitter_info.is_particle_draw), "Draw particles", "Draw particles";
        [INT] (emitter_info.sort_type), "Sort type", "Particle sort type";
        [INT] (emitter_info.calc_type), "Calc type", "Behavior calculation type";
        [pick(FOLLOW_TYPES)] (emitter_info.follow_type), "Follow type", "How particles move with the bone the layer is on. Compiled into the shader: change it from the Follow control, which finds a shader for it";
        [FLAG] (emitter_info.is_fade_emit), "Fade emit", "Whether to stop emitting in the finalization process";
        [FLAG] (emitter_info.is_fade_alpha_fade), "Fade alpha", "Whether to apply alpha fade in the finalization process";
        [FLAG] (emitter_info.is_scale_fade), "Scale fade", "Whether to enable scale-fade";
        [INT] (emitter_info.random_seed_type), "Random seed type", "Random seed type";
        [FLAG] (emitter_info.is_update_matrix_by_emit), "Update matrix by emit", "Updates the matrix at each emission";
        [FLAG] (emitter_info.test_always), "Test always", "Whether to always test";
        [FLAG] (emitter_info.interpolate_emission_amount), "Interpolate emission", "Whether to interpolate the emission amount";
        [FLAG] (emitter_info.is_alpha_fade_in), "Alpha fade in", "Whether to apply alpha fade-in";
        [FLAG] (emitter_info.is_scale_fade_in), "Scale fade in", "Whether to enable scale fade-in";
        [INT] (emitter_info.random_seed), "Random seed", "Random number seed";
        [INT] (emitter_info.draw_path), "Draw path", "Rendering pass";
        [INT] (emitter_info.alpha_fade_time), "Alpha fade time", "Alpha fadeout duration";
        [INT] (emitter_info.fade_in_time), "Fade in time", "Fade-in duration";
    );
    rows!(v, G_INFO;
        [float(0.01)] (emitter_info.trans_x), "Position X", "Emitter position X";
        [float(0.01)] (emitter_info.trans_y), "Position Y", "Emitter position Y";
        [float(0.01)] (emitter_info.trans_z), "Position Z", "Emitter position Z";
        [float(0.01)] (emitter_info.trans_rand_x), "Position random X", "Matrix translation X randomness";
        [float(0.01)] (emitter_info.trans_rand_y), "Position random Y", "Matrix translation Y randomness";
        [float(0.01)] (emitter_info.trans_rand_z), "Position random Z", "Matrix translation Z randomness";
        [float(0.01)] (emitter_info.rotate_x), "Rotation X", "Emitter rotation X";
        [float(0.01)] (emitter_info.rotate_y), "Rotation Y", "Emitter rotation Y";
        [float(0.01)] (emitter_info.rotate_z), "Rotation Z", "Emitter rotation Z";
        [float(0.01)] (emitter_info.rotate_rand_x), "Rotation random X", "Matrix rotation X randomness";
        [float(0.01)] (emitter_info.rotate_rand_y), "Rotation random Y", "Matrix rotation Y randomness";
        [float(0.01)] (emitter_info.rotate_rand_z), "Rotation random Z", "Matrix rotation Z randomness";
        [float(0.01)] (emitter_info.scale_x), "Scale X", "Emitter scale X";
        [float(0.01)] (emitter_info.scale_y), "Scale Y", "Emitter scale Y";
        [float(0.01)] (emitter_info.scale_z), "Scale Z", "Emitter scale Z";
        [float(0.01)] (emitter_info.color0_r), "Colour 0 R", "Emitter color 0";
        [float(0.01)] (emitter_info.color0_g), "Colour 0 G", "Emitter color 0";
        [float(0.01)] (emitter_info.color0_b), "Colour 0 B", "Emitter color 0";
        [float(0.01)] (emitter_info.color0_a), "Colour 0 A", "Emitter color 0";
        [float(0.01)] (emitter_info.color1_r), "Colour 1 R", "Emitter color 1";
        [float(0.01)] (emitter_info.color1_g), "Colour 1 G", "Emitter color 1";
        [float(0.01)] (emitter_info.color1_b), "Colour 1 B", "Emitter color 1";
        [float(0.01)] (emitter_info.color1_a), "Colour 1 A", "Emitter color 1";
        [float(0.5)] (emitter_info.emission_range_near), "Emission range near", "Emission range near distance";
        [float(0.5)] (emitter_info.emission_range_far), "Emission range far", "Emission range far distance";
        [float(0.5)] (emitter_info.emission_ratio_far), "Emission ratio far", "Emission ratio at far distance";
    );
    rows!(v, G_PARTICLE;
        [FLAG] (particle_data.infinite_life), "Infinite life", "Infinite lifespan";
        [FLAG] (particle_data.is_triming), "Trimming", "Trimming";
        [INT] (particle_data.billboard_type), "Billboard type", "How a particle faces: 0 faces the camera (a model turns so its front, +Z, faces it); 3 draws a model as it is, turned only by the layer; 5 and 6 stretch along its motion. The game's files also use 1, 2, 4 and 7. Compiled into the shader: setting it finds a shader built for it";
        [INT] (particle_data.rot_type), "Rotation type", "Rotation type";
        [INT] (particle_data.offset_type), "Offset type", "Camera depth offset type";
        [FLAG] (particle_data.rot_rev_rand_x), "Rotation reverse random X", "Random X for rotation direction";
        [FLAG] (particle_data.rot_rev_rand_y), "Rotation reverse random Y", "Random Y for rotation direction";
        [FLAG] (particle_data.rot_rev_rand_z), "Rotation reverse random Z", "Random Z for rotation direction";
        [FLAG] (particle_data.is_rotate_x), "Rotate X", "Whether to use rotate X";
        [FLAG] (particle_data.is_rotate_y), "Rotate Y", "Whether to use rotate Y";
        [FLAG] (particle_data.is_rotate_z), "Rotate Z", "Whether to use rotate Z";
        [INT] (particle_data.primitive_scale_type), "Primitive scale type", "Primitive scale application type";
        [INT] (particle_data.is_texture_common_random), "Common texture random", "Common texture randomization";
        [INT] (particle_data.connect_ptcl_scale_and_z_offset), "Link scale to Z offset", "Relate particle scale and Z offset";
        [FLAG] (particle_data.enable_avoid_z_fighting), "Avoid Z-fighting", "Whether to enter an offset to avoid z-fighting";
        [INT] (particle_data.life), "Life", "Lifetime, in frames";
        [INT] (particle_data.life_random), "Life random", "Life randomness";
        [float(0.01)] (particle_data.momentum_random), "Momentum random", "Momentum random";
        [INT] (particle_data.primitive_vertex_info_flags), "Primitive vertex flags", "Bit flag for the data held by the primitive's vertices";
        [UINT] (particle_data.primitive_id), "Primitive ID", "Index of the primitive to use — only meaningful when this eff holds it";
        [UINT] (particle_data.primitive_ex_id), "Trim primitive ID", "Index of the trimming primitive to use";
    );
    rows!(v, G_ANIM_RATES;
        [FLAG] (particle_data.loop_color0), "Loop colour 0", "Color 0 animation loop";
        [FLAG] (particle_data.loop_alpha0), "Loop alpha 0", "Alpha animation loop";
        [FLAG] (particle_data.loop_color1), "Loop colour 1", "Color 1 animation loop";
        [FLAG] (particle_data.loop_alpha1), "Loop alpha 1", "Alpha 1 animation loop";
        [FLAG] (particle_data.scale_loop), "Loop scale", "Scale animation loop";
        [FLAG] (particle_data.loop_random_color0), "Loop random colour 0", "Initial position randomness of the colour 0 animation";
        [FLAG] (particle_data.loop_random_alpha0), "Loop random alpha 0", "Initial position randomness of the alpha 0 animation";
        [FLAG] (particle_data.loop_random_color1), "Loop random colour 1", "Initial position randomness of the colour 1 animation";
        [FLAG] (particle_data.loop_random_alpha1), "Loop random alpha 1", "Initial position randomness of the alpha 1 animation";
        [FLAG] (particle_data.scale_loop_random), "Loop random scale", "Initial position randomness of the scale animation";
    );
    rows!(v, G_PARTICLE;
        [FLAG] (particle_data.prim_flag1), "External primitive", "Whether the primitive is stored in another binary";
        [FLAG] (particle_data.prim_flag2), "External trim primitive", "Whether the trimming primitive is stored in another binary";
    );
    rows!(v, G_ANIM_RATES;
        [INT] (particle_data.color0_loop_rate), "Colour 0 loop rate", "Colour 0 loop frame rate, as a percent of one cycle's lifespan";
        [INT] (particle_data.alpha0_loop_rate), "Alpha 0 loop rate", "Alpha 0 loop frame rate, as a percent of one cycle's lifespan";
        [INT] (particle_data.color1_loop_rate), "Colour 1 loop rate", "Colour 1 loop frame rate, as a percent of one cycle's lifespan";
        [INT] (particle_data.alpha1_loop_rate), "Alpha 1 loop rate", "Alpha 1 loop frame rate, as a percent of one cycle's lifespan";
        [INT] (particle_data.scale_loop_rate), "Scale loop rate", "Scale loop frame rate, as a percent of one cycle's lifespan";
    );
    rows!(v, G_PTCL_VELOCITY;
        [float(0.01)] (particle_velocity.all_direction), "All direction", "All-direction initial velocity";
        [float(0.01)] (particle_velocity.designated_dir_scale), "Designated dir scale", "Specified direction velocity";
        [float(0.01)] (particle_velocity.designated_dir_x), "Designated dir X", "Specified direction X";
        [float(0.01)] (particle_velocity.designated_dir_y), "Designated dir Y", "Specified direction Y";
        [float(0.01)] (particle_velocity.designated_dir_z), "Designated dir Z", "Specified direction Z";
        [float(0.5)] (particle_velocity.diffusion_dir_angle), "Diffusion angle", "Specified direction dispersion angle";
        [float(0.01)] (particle_velocity.xz_diffusion), "XZ diffusion", "Y axis diffusion speed";
        [float(0.01)] (particle_velocity.diffusion_x), "Diffusion X", "Diffusion initial velocity X";
        [float(0.01)] (particle_velocity.diffusion_y), "Diffusion Y", "Diffusion initial velocity Y";
        [float(0.01)] (particle_velocity.diffusion_z), "Diffusion Z", "Diffusion initial velocity Z";
        [float(0.01)] (particle_velocity.vel_random), "Velocity random", "Velocity randomness";
        [float(0.01)] (particle_velocity.em_vel_inherit), "Emitter velocity inherit", "Inherited emitter velocity ratio";
    );
    rows!(v, G_PTCL_COLOR;
        [INT] (particle_color.is_soft_particle), "Soft particle", "Soft edges: 0 off; the game's files use 1 and 2 (compiled into the shader, so switch it with Draw style)";
        [INT] (particle_color.is_fresnel_alpha), "Fresnel alpha", "Rim fade: 0 off; the game's files use 1 and 2 (compiled into the shader, so switch it with Draw style)";
        [FLAG] (particle_color.is_near_dist_alpha), "Near distance alpha", "Near distance alpha";
        [FLAG] (particle_color.is_far_dist_alpha), "Far distance alpha", "Far distance alpha";
        [FLAG] (particle_color.is_decal), "Decal", "Decals";
        [pick(COLOR_TYPES)] (particle_color.color0_type), "Colour 0 type", "Colour 0 behaviour type. This decides WHERE the colour is read from, so changing it changes which of the colour controls has any effect.";
        [pick(COLOR_TYPES)] (particle_color.color1_type), "Colour 1 type", "Colour 1 behaviour type";
        [pick(COLOR_TYPES)] (particle_color.alpha0_type), "Alpha 0 type", "Alpha 0 behaviour type";
        [pick(COLOR_TYPES)] (particle_color.alpha1_type), "Alpha 1 type", "Alpha 1 behaviour type";
        [float(0.01)] (particle_color.color0_r), "Colour 0 R", "Colour 0 red component";
        [float(0.01)] (particle_color.color0_g), "Colour 0 G", "Colour 0 green component";
        [float(0.01)] (particle_color.color0_b), "Colour 0 B", "Colour 0 blue component";
        [float(0.01)] (particle_color.alpha0), "Alpha 0", "Alpha 0";
        [float(0.01)] (particle_color.color1_r), "Colour 1 R", "Colour 1 red component";
        [float(0.01)] (particle_color.color1_g), "Colour 1 G", "Colour 1 green component";
        [float(0.01)] (particle_color.color1_b), "Colour 1 B", "Colour 1 blue component";
        [float(0.01)] (particle_color.alpha1), "Alpha 1", "Alpha 1";
    );
    rows!(v, G_PTCL_SCALE;
        [float(0.01)] (particle_scale.scale_x), "Scale X", "Base scale";
        [float(0.01)] (particle_scale.scale_y), "Scale Y", "Base scale";
        [float(0.01)] (particle_scale.scale_z), "Scale Z", "Base scale";
        [float(0.01)] (particle_scale.scale_random_x), "Scale random X", "Base scale randomness";
        [float(0.01)] (particle_scale.scale_random_y), "Scale random Y", "Base scale randomness";
        [float(0.01)] (particle_scale.scale_random_z), "Scale random Z", "Base scale randomness";
        [FLAG] (particle_scale.enable_scaling_by_camera_dist_near), "Scale by camera (near)", "Whether to enable near camera distance scaling";
        [FLAG] (particle_scale.enable_scaling_by_camera_dist_far), "Scale by camera (far)", "Whether to enable far camera distance scaling";
        [FLAG] (particle_scale.enable_add_scale_y), "Add scale Y", "Y velocity scaling";
        [FLAG] (particle_scale.enable_link_fovy_to_scale_value), "Link FOV to scale", "Relate angle of view to scale restrictions";
        [float(0.01)] (particle_scale.scale_min), "Scale limit near", "Particle scaling limit distance (near)";
        [float(0.01)] (particle_scale.scale_max), "Scale limit far", "Particle scaling limit distance (far)";
    );
    rows!(v, G_PTCL_ROTATE;
        [float(0.01)] (emitter_static.rotate_init_x), "Initial rotation X", "Initial rotation value X";
        [float(0.01)] (emitter_static.rotate_init_y), "Initial rotation Y", "Initial rotation value Y";
        [float(0.01)] (emitter_static.rotate_init_z), "Initial rotation Z", "Initial rotation value Z";
        [float(0.01)] (emitter_static.rotate_init_rand_x), "Initial rotation random X", "Initial random rotation X";
        [float(0.01)] (emitter_static.rotate_init_rand_y), "Initial rotation random Y", "Initial random rotation Y";
        [float(0.01)] (emitter_static.rotate_init_rand_z), "Initial rotation random Z", "Initial random rotation Z";
        [float(0.01)] (emitter_static.rotate_add_x), "Rotation velocity X", "Rotation velocity X";
        [float(0.01)] (emitter_static.rotate_add_y), "Rotation velocity Y", "Rotation velocity Y";
        [float(0.01)] (emitter_static.rotate_add_z), "Rotation velocity Z", "Rotation velocity Z";
        [float(0.01)] (emitter_static.rotate_regist), "Rotation attenuation", "Rotation attenuation rate";
        [float(0.01)] (emitter_static.rotate_add_rand_x), "Rotation velocity random X", "Rotation velocity randomness X";
        [float(0.01)] (emitter_static.rotate_add_rand_y), "Rotation velocity random Y", "Rotation velocity randomness Y";
        [float(0.01)] (emitter_static.rotate_add_rand_z), "Rotation velocity random Z", "Rotation velocity randomness Z";
        [float(0.5)] (emitter_static.scale_limit_dist_near), "Scale limit distance near", "Scale limit distance in front of the camera (near)";
        [float(0.5)] (emitter_static.scale_limit_dist_far), "Scale limit distance far", "Scale limit distance in front of the camera (far)";
    );
    rows!(v, G_MOTION;
        [float(0.01)] (emitter_static.gravity_dir_x), "Gravity X", "Gravity X";
        [float(0.01)] (emitter_static.gravity_dir_y), "Gravity Y", "Gravity Y";
        [float(0.01)] (emitter_static.gravity_dir_z), "Gravity Z", "Gravity Z";
        [float(0.01)] (emitter_static.gravity_scale), "Gravity scale", "Gravity scale";
        [float(0.01)] (emitter_static.air_res), "Air resistance", "Air resistance";
        [float(0.01)] (emitter_static.center_x), "Centre X", "Particle centre";
        [float(0.01)] (emitter_static.center_y), "Centre Y", "Particle centre";
        [float(0.01)] (emitter_static.offset), "Offset", "Particle offset";
        [float(0.01)] (emitter_static.add_vel_to_scale), "Add velocity to scale", "Add velocity to scale";
    );
    rows!(v, G_FLUCTUATION;
        [float(0.01)] (emitter_static.amplitude_x), "Amplitude X", "Fluctuation amplitude X";
        [float(0.01)] (emitter_static.amplitude_y), "Amplitude Y", "Fluctuation amplitude Y";
        [float(0.01)] (emitter_static.cycle_x), "Cycle X", "Fluctuation cycle X";
        [float(0.01)] (emitter_static.cycle_y), "Cycle Y", "Fluctuation cycle Y";
        [float(0.01)] (emitter_static.phase_rnd_x), "Random phase X", "Fluctuation random phase X";
        [float(0.01)] (emitter_static.phase_rnd_y), "Random phase Y", "Fluctuation random phase Y";
        [float(0.01)] (emitter_static.phase_init_x), "Initial phase X", "Fluctuation initial phase X";
        [float(0.01)] (emitter_static.phase_init_y), "Initial phase Y", "Fluctuation initial phase Y";
        [FLAG] (?particle_fluctuation.is_apply_alpha), "Apply to alpha", "Whether to apply the fluctuation to alpha";
        [FLAG] (?particle_fluctuation.is_applay_scale), "Apply to scale", "Whether to apply the fluctuation to scaling";
        [FLAG] (?particle_fluctuation.is_applay_scale_y), "Apply to scale Y", "Set the y axis individually";
        [FLAG] (?particle_fluctuation.is_wave_type), "Wave type", "Fluctuation waveform type";
        [FLAG] (?particle_fluctuation.is_phase_random_x), "Phase random X", "Dependent randomness X";
        [FLAG] (?particle_fluctuation.is_phase_random_y), "Phase random Y", "Fluctuation randomness Y";
    );
    rows!(v, G_ANIM_RATES;
        [float(0.01)] (emitter_static.color_scale), "Colour scale", "Colour scaling";
        [INT] (emitter_static.num_color0_keys), "Colour 0 key count", "How many of the eight colour 0 keyframes are live";
        [INT] (emitter_static.num_alpha0_keys), "Alpha 0 key count", "How many of the eight alpha 0 keyframes are live";
        [INT] (emitter_static.num_color1_keys), "Colour 1 key count", "How many of the eight colour 1 keyframes are live";
        [INT] (emitter_static.num_alpha1_keys), "Alpha 1 key count", "How many of the eight alpha 1 keyframes are live";
        [INT] (emitter_static.num_scale_keys), "Scale key count", "How many of the eight scale keyframes are live";
        [INT] (emitter_static.num_param_keys), "Shader coefficient key count", "How many of the eight shader-coefficient keyframes are live";
        [float(0.01)] (emitter_static.color0_loop_rate), "Colour 0 loop rate", "Colour 0 loop frame rate, as a percent of one cycle's lifespan";
        [float(0.01)] (emitter_static.alpha0_loop_rate), "Alpha 0 loop rate", "Alpha 0 loop frame rate, as a percent of one cycle's lifespan";
        [float(0.01)] (emitter_static.color1_loop_rate), "Colour 1 loop rate", "Colour 1 loop frame rate, as a percent of one cycle's lifespan";
        [float(0.01)] (emitter_static.alpha1_loop_rate), "Alpha 1 loop rate", "Alpha 1 loop frame rate, as a percent of one cycle's lifespan";
        [float(0.01)] (emitter_static.scale_loop_rate), "Scale loop rate", "Scale loop frame rate, as a percent of one cycle's lifespan";
        [float(0.01)] (emitter_static.color0_loop_random), "Colour 0 loop random", "Initial position randomness of the colour 0 animation";
        [float(0.01)] (emitter_static.alpha0_loop_random), "Alpha 0 loop random", "Initial position randomness of the alpha 0 animation";
        [float(0.01)] (emitter_static.color1_loop_random), "Colour 1 loop random", "Initial position randomness of the colour 1 animation";
        [float(0.01)] (emitter_static.alpha1_loop_random), "Alpha 1 loop random", "Initial position randomness of the alpha 1 animation";
        [float(0.01)] (emitter_static.scale_loop_random), "Scale loop random", "Initial position randomness of the scale animation";
        [float(0.01)] (emitter_static.coefficient0), "Shader coefficient 0", "Shader coefficient 0";
        [float(0.01)] (emitter_static.coefficient1), "Shader coefficient 1", "Shader coefficient 1";
    );
    rows!(v, G_RENDER;
        [FLAG] (render_state.is_blend_enable), "Blend", "Blend";
        [FLAG] (render_state.is_depth_test), "Depth test", "Depth test";
        [INT] (render_state.depth_func), "Depth function", "Depth test pass conditions";
        [FLAG] (render_state.is_depth_mask), "Depth mask", "Depth mask";
        [FLAG] (render_state.is_alpha_test), "Alpha test", "Alpha test";
        [INT] (render_state.alpha_func), "Alpha function", "Alpha test pass conditions";
        [pick(BLEND_TYPES)] (render_state.blend_type), "Blend type", "How a particle's colour meets what is behind it";
        [INT] (render_state.display_side), "Display side", "Which face is drawn";
        [float(0.01)] (render_state.alpha_threshold), "Alpha threshold", "Alpha threshold";
    );
    rows!(v, G_ALPHA_FX;
        [float(0.01)] (emitter_static.soft_edge_param1), "Soft edge 1", "Soft particles";
        [float(0.01)] (emitter_static.soft_edge_param2), "Soft edge 2", "Soft particles";
        [float(0.01)] (emitter_static.fresnel_alpha_param1), "Fresnel alpha 1", "Fresnel alpha";
        [float(0.01)] (emitter_static.fresnel_alpha_param2), "Fresnel alpha 2", "Fresnel alpha";
        [float(0.01)] (emitter_static.near_dist_alpha_param1), "Near distance alpha 1", "Near distance alpha";
        [float(0.01)] (emitter_static.near_dist_alpha_param2), "Near distance alpha 2", "Near distance alpha";
        [float(0.01)] (emitter_static.far_dist_alpha_param1), "Far distance alpha 1", "Far distance alpha";
        [float(0.01)] (emitter_static.far_dist_alpha_param2), "Far distance alpha 2", "Far distance alpha";
        [float(0.01)] (emitter_static.decal_param1), "Decal 1", "Decals";
        [float(0.01)] (emitter_static.decal_param2), "Decal 2", "Decals";
        [float(0.01)] (emitter_static.alpha_threshold), "Alpha test threshold", "Threshold value for the alpha test";
        [float(0.01)] (emitter_static.soft_partcile_dist), "Soft particle distance", "Start-to-fade distance";
        [float(0.01)] (emitter_static.soft_particle_volume), "Soft particle volume", "Soft particle volume value";
    );
    rows!(v, G_COMBINER;
        [INT] (@cmb color_combiner_process), "Colour process", "Colour calculation formula type";
        [INT] (@cmb alpha_combiner_process), "Alpha process", "Alpha calculation formula type";
        [FLAG] (@cmb texture1_color_blend), "Blend texture 1 colour", "Combines the texture1 colour with the colour in the top row";
        [FLAG] (@cmb texture2_color_blend), "Blend texture 2 colour", "Combines the texture2 colour with the colour in the top row";
        [FLAG] (@cmb primitive_color_blend), "Blend primitive colour", "Combines the primitive colour with the colour in the top row";
        [FLAG] (@cmb texture1_alpha_blend), "Blend texture 1 alpha", "Combines the texture1 alpha with the alpha in the top row";
        [FLAG] (@cmb texture2_alpha_blend), "Blend texture 2 alpha", "Combines the texture2 alpha with the alpha in the top row";
        [FLAG] (@cmb primitive_alpha_blend), "Blend primitive alpha", "Combines the primitive alpha with the alpha in the top row";
        [INT] (@wide tex_color0_input_type), "Texture colour 0 input", "Texture colour 0 input type";
        [INT] (@wide tex_color1_input_type), "Texture colour 1 input", "Texture colour 1 input type";
        [INT] (@wide tex_color2_input_type), "Texture colour 2 input", "Texture colour 2 input type";
        [INT] (@wide tex_alpha0_input_type), "Texture alpha 0 input", "Texture alpha 0 input type";
        [INT] (@wide tex_alpha1_input_type), "Texture alpha 1 input", "Texture alpha 1 input type";
        [INT] (@wide tex_alpha2_input_type), "Texture alpha 2 input", "Texture alpha 2 input type";
        [INT] (@wide primitive_color_input_type), "Primitive colour input", "Primitive colour input type";
        [INT] (@wide primitive_alpha_input_type), "Primitive alpha input", "Primitive alpha input type";
        [INT] (@wide shader_type), "Shader type", "Shader type";
        [INT] (@wide apply_alpha), "Apply alpha", "Refraction shader, or apply the alpha value";
        [FLAG] (@wide is_distortion_by_camera_distance), "Distortion by camera distance", "Whether to enhance the distortion with the camera's distance";
    );
    rows!(v, G_INHERIT;
        [FLAG] (child_inheritance.velocity), "Velocity", "Inherited speed";
        [FLAG] (child_inheritance.scale), "Scale", "Inherited scale";
        [FLAG] (child_inheritance.rotate), "Rotation", "Inherited rotation";
        [FLAG] (child_inheritance.color_scale), "Colour scale", "Inherited colour scale";
        [FLAG] (child_inheritance.color0), "Colour 0", "Inherited colour 0";
        [FLAG] (child_inheritance.color1), "Colour 1", "Inherited colour 1";
        [FLAG] (child_inheritance.alpha0), "Alpha 0", "Inherited alpha 0";
        [FLAG] (child_inheritance.alpha1), "Alpha 1", "Inherited alpha 1";
        [FLAG] (child_inheritance.draw_path), "Draw path", "Inherited draw path";
        [FLAG] (child_inheritance.pre_draw), "Pre-draw", "Draw before the inherited class";
        [FLAG] (child_inheritance.alpha0_each_frame), "Alpha 0 each frame", "Whether to inherit alpha0 every frame";
        [FLAG] (child_inheritance.alpha1_each_frame), "Alpha 1 each frame", "Whether to inherit alpha1 every frame";
        [FLAG] (child_inheritance.enable_emitter_particle), "Emitter per particle", "Whether to generate an emitter for each particle";
        [float(0.01)] (child_inheritance.velocity_rate), "Velocity rate", "Inherited velocity ratio";
        [float(0.01)] (child_inheritance.scale_rate), "Scale rate", "Inherited scale ratio";
    );
    rows!(v, G_SHADER;
        [INT] (action.action_index), "Custom action index", "Selected custom action index";
        [INT] (shader_references.type_), "Shader type", "Shader type";
        [INT] (shader_references.shader_index), "Shader index", "Shader index to use. Pointing this outside the variations this eff ships is what makes a file fail to load.";
        [INT] (shader_references.compute_shader_index), "Compute shader index", "Compute shader index to use";
        [INT] (shader_references.user_shader_index1), "User shader index 1", "User shader index 1";
        [INT] (shader_references.user_shader_index2), "User shader index 2", "User shader index 2";
        [INT] (shader_references.custom_shader_index), "Custom shader index", "Custom shader index";
        [UINT] (shader_references.custom_shader_flag), "Custom shader flag", "Custom shader option flag bits";
        [UINT] (shader_references.custom_shader_switch), "Custom shader switch", "Custom shader option switch-selection bits";
        [INT] (shader_references.extra_shader_index2), "Extra shader index", "Index of the shader generated by the effect combiner";
    );

    macro_rules! sampler {
        ($g:expr, $s:ident) => {
            rows!(v, $g;
                [pick(WRAP_MODES)] (?$s.wrap_u), "Wrap U", "U wrap mode";
                [pick(WRAP_MODES)] (?$s.wrap_v), "Wrap V", "V wrap mode";
                [INT] (?$s.filter), "Filter", "Filter mode";
                [FLAG] (?$s.is_sphere_map), "Sphere map", "Whether a sphere map is used";
                [float(0.1)] (?$s.max_lod), "Max LOD", "Effective mip level (0.0 to 15.99)";
                [float(0.1)] (?$s.lod_bias), "LOD bias", "Mip level bias";
                [INT] (?$s.mip_level_limit), "Mip level limit", "Restrict the mipmap level";
                [FLAG] (?$s.is_density_fixed_u), "Fix density U", "Fix texture density option U";
                [FLAG] (?$s.is_density_fixed_v), "Fix density V", "Fix texture density option V";
                [FLAG] (?$s.is_square_rgb), "Square RGB", "Whether to square the texture's RGB values when reading them";
                [UINT] (?$s.texture_id), "Texture GUID", "GUID of the texture this sampler reads. The Texture panel is the safe way to change this — a GUID the pool has no descriptor for samples nothing.";
                [FLAG] (?$s.is_on_another_binary), "External texture", "Whether the texture is stored in another binary";
            );
        };
    }
    sampler!(G_SAMPLER0, sampler0);
    sampler!(G_SAMPLER1, sampler1);
    sampler!(G_SAMPLER2, sampler2);

    macro_rules! texture_anim {
        ($g:expr, $a:ident) => {
            rows!(v, $g;
                [INT] (?$a.pattern_anim_type), "Pattern anim type", "Pattern animation type";
                [FLAG] (?$a.is_scroll), "Scroll", "Enable or disable UV scrolling animation";
                [FLAG] (?$a.is_rotate), "Rotate", "Enable or disable UV rotation animation";
                [FLAG] (?$a.is_scale), "Scale", "Enable or disable UV scaling animation";
                [INT] (?$a.repeat), "Repeat", "Repetition count";
                [INT] (?$a.inv_rand_u), "Invert random U", "U invert randomness";
                [INT] (?$a.inv_rand_v), "Invert random V", "V invert randomness";
                [FLAG] (?$a.is_pat_anim_loop_random), "Pattern loop random", "Texture pattern animation loop start randomness";
                [INT] (?$a.uv_channel), "UV channel", "Primitive UV channel";
                [FLAG] (?$a.is_crossfade), "Crossfade", "Enable or disable crossfade";
            );
        };
    }
    texture_anim!(G_TEXANIM0, texture_anim0);
    texture_anim!(G_TEXANIM1, texture_anim1);
    texture_anim!(G_TEXANIM2, texture_anim2);

    macro_rules! pattern_anim {
        ($g:expr, $a:ident) => {
            rows!(v, $g;
                [float(1.0)] (emitter_static.$a.num), "Pattern table count", "Number of pattern tables";
                [float(1.0)] (emitter_static.$a.frequency), "Pattern frequency", "Pattern frequency";
                [float(1.0)] (emitter_static.$a.num_random), "Random pattern count", "Number of patterns for random patterns";
                [float(1.0)] (emitter_static.$a.pad), "Loop count", "Number of surface animation loops";
            );
        };
    }
    pattern_anim!(G_TEXPAT0, tex_pattern_anim0);
    pattern_anim!(G_TEXPAT1, tex_pattern_anim1);
    pattern_anim!(G_TEXPAT2, tex_pattern_anim2);

    macro_rules! uv_anim {
        ($g:expr, $a:ident) => {
            rows!(v, $g;
                [float(0.001)] (emitter_static.$a.scroll_add_x), "Scroll add X", "X value to add when scrolling";
                [float(0.001)] (emitter_static.$a.scroll_add_y), "Scroll add Y", "Y value to add when scrolling";
                [float(0.01)] (emitter_static.$a.scroll_x), "Scroll X", "X initial scroll value";
                [float(0.01)] (emitter_static.$a.scroll_y), "Scroll Y", "Y initial scroll value";
                [float(0.01)] (emitter_static.$a.scroll_random_x), "Scroll random X", "X random initial scroll value";
                [float(0.01)] (emitter_static.$a.scroll_random_y), "Scroll random Y", "Y random initial scroll value";
                [float(0.001)] (emitter_static.$a.scale_add_x), "Scale add X", "X value to add when scaling";
                [float(0.001)] (emitter_static.$a.scale_add_y), "Scale add Y", "Y value to add when scaling";
                [float(0.01)] (emitter_static.$a.scale_x), "Scale X", "X initial scale value";
                [float(0.01)] (emitter_static.$a.scale_y), "Scale Y", "Y initial scale value";
                [float(0.01)] (emitter_static.$a.scale_random_x), "Scale random X", "X random initial scale";
                [float(0.01)] (emitter_static.$a.scale_random_y), "Scale random Y", "Y random initial scale";
                [float(0.001)] (emitter_static.$a.rotation_add), "Rotation add", "Value to add when rotating";
                [float(0.01)] (emitter_static.$a.rotation), "Rotation", "Initial rotation value";
                [float(0.01)] (emitter_static.$a.rotation_random), "Rotation random", "Random initial rotation value";
                [float(1.0)] (emitter_static.$a.rotation_type), "Rotation type", "Random rotation type";
                [float(0.01)] (emitter_static.$a.uv_scale_x), "UV scale X", "X UV scale value";
                [float(0.01)] (emitter_static.$a.uv_scale_y), "UV scale Y", "Y UV scale value";
                [float(1.0)] (emitter_static.$a.uv_div_x), "UV divisions X", "Number of horizontal divisions";
                [float(1.0)] (emitter_static.$a.uv_div_y), "UV divisions Y", "Number of vertical divisions";
            );
        };
    }
    uv_anim!(G_TEXUV0, tex_scroll_anim0);
    uv_anim!(G_TEXUV1, tex_scroll_anim1);
    uv_anim!(G_TEXUV2, tex_scroll_anim2);

    // Fixed-size arrays get one row per element, so each pattern slot and keyframe component has its own id.
    macro_rules! pattern_slots {
        ($g:expr, $a:ident; $($i:tt)+) => {
            rows!(v, $g; $(
                [INT] (concat!("emitter_static.", stringify!($a), ".table[", stringify!($i), "]") => (emitter_static.$a.table[$i])),
                    concat!("Pattern slot ", stringify!($i)), "Texture index stored in this pattern-animation slot";
            )+);
        };
    }
    pattern_slots!(G_TEXPAT0, tex_pattern_anim0; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31);
    pattern_slots!(G_TEXPAT1, tex_pattern_anim1; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31);
    pattern_slots!(G_TEXPAT2, tex_pattern_anim2; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31);

    macro_rules! keys {
        ($g:expr, $a:ident; $($i:tt)+) => {
            rows!(v, $g; $(
                [float(0.01)] (concat!("emitter_static.", stringify!($a), ".keys[", stringify!($i), "].x") => (emitter_static.$a.keys[$i].x)),
                    concat!("Key ", stringify!($i), " X"), "Keyframe X component";
                [float(0.01)] (concat!("emitter_static.", stringify!($a), ".keys[", stringify!($i), "].y") => (emitter_static.$a.keys[$i].y)),
                    concat!("Key ", stringify!($i), " Y"), "Keyframe Y component";
                [float(0.01)] (concat!("emitter_static.", stringify!($a), ".keys[", stringify!($i), "].z") => (emitter_static.$a.keys[$i].z)),
                    concat!("Key ", stringify!($i), " Z"), "Keyframe Z component";
                [float(0.01)] (concat!("emitter_static.", stringify!($a), ".keys[", stringify!($i), "].time") => (emitter_static.$a.keys[$i].time)),
                    concat!("Key ", stringify!($i), " time"), "When the key is reached, through the particle's life: 0 at birth, 1 at death";
            )+);
        };
    }
    keys!(G_COLOR_KEYS, color0; 0 1 2 3 4 5 6 7);
    keys!(G_COLOR_KEYS, color1; 0 1 2 3 4 5 6 7);
    keys!(G_ALPHA_KEYS, alpha0; 0 1 2 3 4 5 6 7);
    keys!(G_ALPHA_KEYS, alpha1; 0 1 2 3 4 5 6 7);
    keys!(G_SCALE_KEYS, scale_anim; 0 1 2 3 4 5 6 7);
    keys!(G_PARAM_KEYS, param_anim; 0 1 2 3 4 5 6 7);

    // The user shader defines are byte vectors; a byte past the end reads as absent.
    macro_rules! define_bytes {
        ($a:ident, $n:literal; $($i:tt)+) => {$(
            v.push(Attr {
                id: concat!("shader_references.", stringify!($a), "[", stringify!($i), "]"),
                group: G_SHADER,
                label: concat!("User define ", $n, " byte ", stringify!($i)),
                doc: "Raw byte in the fixed-size user shader definition",
                kind: INT,
                get: |d| Field::get(d.shader_references.$a.get($i)?, INT),
                set: |d, value| {
                    if let Some(byte) = d.shader_references.$a.get_mut($i) {
                        Field::set(byte, INT, value);
                    }
                },
            });
        )+};
    }
    define_bytes!(user_shader_define1, "1"; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15);
    define_bytes!(user_shader_define2, "2"; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15);

    v
}

static TABLE: std::sync::OnceLock<Vec<Attr>> = std::sync::OnceLock::new();
static BY_ID: std::sync::OnceLock<std::collections::HashMap<&'static str, usize>> = std::sync::OnceLock::new();

/// Every row, in a fixed order. Editors keep per-emitter values by index into this, so it never changes while
/// the program runs.
pub fn table() -> &'static [Attr] {
    TABLE.get_or_init(build)
}

pub fn index_of(id: &str) -> Option<usize> {
    BY_ID.get_or_init(|| table().iter().enumerate().map(|(i, a)| (a.id, i)).collect()).get(id).copied()
}

/// Every row's value on one emitter, in table order.
pub fn read_all(data: &EmitterData) -> Vec<Option<AttrValue>> {
    table().iter().map(|a| (a.get)(data)).collect()
}

/// Set one row by id. Returns false for an id the table does not have, so an edit that would not apply is not
/// lost quietly.
pub fn write(data: &mut EmitterData, id: &str, value: AttrValue) -> bool {
    let Some(i) = index_of(id) else { return false };
    (table()[i].set)(data, value);
    true
}
