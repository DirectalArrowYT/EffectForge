//! Particle simulation, evaluated closed-form at any frame.
//!
//! The editor scrubs. A conventional particle system steps state forward a frame at a time and
//! cannot answer "what does frame 30 look like" without having run frames 0..29 first, so
//! dragging the playhead backwards either shows the wrong thing or forces a replay from the
//! start of the move. Everything here is therefore a pure function of the particle's age, and
//! every random quantity comes from hashing the particle's identity rather than from a running
//! generator. Scrub anywhere, get the same answer, in the same time.
//!
//! The rules themselves -- when an emitter fires, how many particles a firing makes, what each
//! random percentage does, how drag and gravity step -- are the game's own, read out of the
//! particle library in the 13.0.3 executable (NintendoWare Vfx, see
//! `research/decomp/ssbu-re`). Where a comment gives an address, that is the function the rule
//! was read from. The game steps a frame at a time; the forms here are the sums of those steps,
//! so they agree with it on whole frames and interpolate between them.
//!
//! That constraint rules out anything path-dependent — collision, turbulence that integrates,
//! drag applied per step — and those are the parts this deliberately does not model. What it
//! does cover is what the overwhelming majority of Smash effects are made of: an emitter
//! producing particles at a rate for a duration, each living a fixed life, moving under an
//! initial velocity and gravity, changing size and colour across that life.

use crate::eff_attrs::AttrValue;
use crate::effects::{ColorKey, EmitterDef};

/// Deterministic per-particle randomness.
///
/// Not a sequence: a hash. The nth particle's jitter has to be the same value whether the
/// viewport arrived at this frame by playing forward, scrubbing backwards, or opening the move
/// cold, and a stateful generator gives three different answers.
fn hashed(seed: u64, salt: u64) -> f32 {
    let mut x = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    // 24 bits is plenty for jitter and keeps the value exactly representable.
    ((x >> 40) as f32) / ((1u32 << 24) as f32)
}

/// Symmetric jitter in −1..1.
fn hashed_signed(seed: u64, salt: u64) -> f32 {
    hashed(seed, salt) * 2.0 - 1.0
}

/// The emitter fields the simulation reads, pulled out of the attribute table once.
///
/// Resolved by attribute id rather than by index: the table is generated and its ordering is
/// not a stable interface, so an index captured here would silently start reading a different
/// field the next time a row is inserted.
#[derive(Debug, Clone)]
pub struct EmitterSim {
    /// Particles per firing, not per frame: the emitter fires every `interval + 1` frames and
    /// each firing makes this many at once (0x8d340). Fractions carry over to the next firing.
    pub rate: f32,
    /// Percentage a firing's count may fall short by. One-sided, as every percentage here is.
    pub rate_random: f32,
    /// Frames skipped between firings. Zero fires every frame.
    pub interval: f32,
    /// Up to this many extra whole frames added to each gap (0x88750).
    pub interval_random: f32,
    pub emission_start: f32,
    pub emission_duration: f32,
    /// Whether the emitter stops after `emission_duration`. An emitter without this set keeps
    /// firing for as long as the effect lives, whatever its duration says. `None` when the file
    /// does not carry the flag, which falls back to "stops if it has a duration".
    pub is_one_time: Option<bool>,
    /// Radius of a random shell around the spawn point: a unit direction times this, not a
    /// box.
    pub position_random: f32,
    /// Whether gravity pulls along the world's axes rather than the emitter's own.
    pub world_gravity: bool,
    /// Per-frame multiplier on velocity. 1 is no drag.
    pub air_res: f32,
    /// Symmetric spread on how far a particle travels per frame for its velocity: each one
    /// moves at `1 +- this` of the common pace for its whole life.
    pub momentum_random: f32,
    pub gravity: glam::Vec3,
    pub life: f32,
    pub life_random: f32,
    pub all_direction: f32,
    pub designated_dir: glam::Vec3,
    pub designated_dir_scale: f32,
    pub diffusion: glam::Vec3,
    pub velocity_random: f32,
    /// Half-angle, in degrees, of a cone around the designated direction that the directed
    /// speed is spread over. Zero keeps every particle exactly on the direction.
    pub diffusion_dir_angle: f32,
    /// Speed outward from the emitter's Y axis, taken from where in the shape the particle
    /// was born.
    pub xz_diffusion: f32,
    pub scale: glam::Vec3,
    /// Particle size over its own life: (frame, xyz). This is the PARTICLE's scale, distinct
    /// from `scale` above which is the emitter's — reading the emitter's as the particle's
    /// gives every particle in the game the same size, because emitters are nearly all 1.0.
    pub scale_keys: Vec<(f32, glam::Vec3)>,
    /// How the emitter says its texture is cut into cells, if it says at all.
    ///
    /// This is the authority when it is set: an explosion's 4x4 sheet is declared here and
    /// nowhere else, and inferring a grid from a square texture's proportions returns "one
    /// cell", which draws the whole sheet on every particle.
    pub uv_div: (u32, u32),
    /// The texture's own animation inside the particle, from `tex_scroll_anim0`: an offset
    /// and a zoom, each with a per-frame rate.
    ///
    /// This is what sweeps an attack arc. SYS_ATTACK_ARC_D scrolls V by 0.27 and zooms it
    /// 1.4x, both moving every frame, which slides a window across a slash frame so the arc
    /// is drawn from one end to the other. Without it the frame sits still, whole, and tapers
    /// at both ends.
    pub uv_scroll: glam::Vec2,
    pub uv_scroll_add: glam::Vec2,
    pub uv_scale: glam::Vec2,
    pub uv_scale_add: glam::Vec2,
    /// The shape particles are born in, as the game's own table at 0x4f48c30 orders them:
    /// 0 point, 1 circle, 2 circle-same-divide, 3 filled circle, 4 sphere,
    /// 5 sphere-same-divide, 6 sphere-same-divide-64, 7 filled sphere, 8 cylinder,
    /// 9 filled cylinder, 10 box, 11 filled box, 12 line, 13 line-same-divide, 14 rectangle,
    /// 15 primitive.
    ///
    /// Half the corpus is a point and the rest is mostly circles and spheres. The "same
    /// divide" variants space their particles evenly around the sweep instead of scattering
    /// them, which is what makes a shockwave ring a ring rather than a smear.
    pub volume_type: i64,
    /// Half-extent of that shape per axis.
    pub volume_radius: glam::Vec3,
    /// How much of the shape is used, in radians: longitude around, latitude from the pole.
    /// 2pi and pi -- the whole thing -- on nearly every emitter, but an arc of a ring is a
    /// sweep of less.
    pub sweep_longitude: f32,
    pub sweep_latitude: f32,
    pub sweep_start: f32,
    /// How a sphere is cut down: 0 by longitude (a wedge), 1 by latitude (a cap, around
    /// `latitude_dir`).
    pub arc_type: i64,
    /// Which way a latitude cap points: +X, -X, +Y, -Y, +Z, -Z.
    pub latitude_dir: i64,
    /// For the line shapes: how long, and where its centre sits.
    pub line_length: f32,
    pub line_center: f32,
    /// How much of a filled shape's radius is filled, from the rim inwards: 1.0 fills it to
    /// the middle, 0.2 leaves a hole 0.8 of the radius wide, which is how a ring of fire keeps
    /// its hole.
    pub caliber_ratio: f32,
    /// Start the sweep at a random angle each firing instead of at `sweep_start`.
    pub sweep_start_random: bool,
    /// Angular jitter, radians either way, on the evenly divided circle's positions.
    pub surface_pos_rand: f32,
    /// How many positions the evenly divided circle and line have, and the percentage a firing
    /// may fall short of that. A firing makes `rate` particles for each position.
    pub num_divide_circle: (u32, f32),
    pub num_divide_line: (u32, f32),
    /// The particle's size in world units, before its curve and the emitter's scale.
    ///
    /// This is the one that carries the data: across ef_common it runs 0.1 to 300, while the
    /// emitter's own scale is 1.0 on all but two of 1348 emitters. Building a size without it
    /// draws every particle in the game about a unit across, whatever the effect asked for.
    pub particle_scale: glam::Vec3,
    /// Per-particle spread on that size, as a PERCENTAGE -- 30 means a particle lands
    /// anywhere in 0.7x to 1.3x. Better than half the corpus sets one, and without it a
    /// cloud is a stack of identically sized puffs.
    pub scale_random: glam::Vec3,
    pub billboard_type: i64,
    /// How the emitter's particles blend with what is behind them.
    ///
    /// 0 is normal alpha, 1 additive, 2 subtractive -- the format's own numbering, with 0 the
    /// default as a format's zero usually is. Across two real fighter files this splits about
    /// 73 / 25 / 3, so an assumption that everything is additive is wrong for roughly three
    /// quarters of every emitter in the game, and wrong in the direction that makes smoke and
    /// dust glow like fire.
    pub blend_type: i64,
    /// Brightness multiplier on the emitter's colour, straight out of the emitter's constant
    /// buffer (`emitter_static.color_scale`). Values run about 1.0 to 5.0 in the game's own
    /// effects, so ignoring it draws everything dimmer and flatter than it plays -- and the
    /// ones that clip to white in game, which is what makes a hit spark read as a flash,
    /// never clip here at all.
    pub color_scale: f32,
    /// Which texture channel carries the particle's shape: 0 the texture's alpha, 1 its red,
    /// 2 nothing (fully opaque).
    ///
    /// Confirmed in game rather than inferred: a decal set to 1 with black art disappeared,
    /// because its shape lives in alpha and its red is zero everywhere.
    pub alpha_input: i64,
    /// Where the particle's colour comes from: 0 the texture's RGB, 1 the emitter's colour
    /// alone, with the texture contributing only shape.
    pub color_input: i64,
    /// How the emitter's textures are meant to be read. 2 is the indirect path, where the
    /// first texture displaces the lookup into the second rather than being drawn itself --
    /// every emitter using it in ef_common binds a second texture, and it is how fire is built.
    pub shader_type: i64,
    /// How a plain second texture folds into the first: 0 multiply, 1 add, 2 subtract.
    pub tex1_blend: i64,
    /// The particle's own roll about the quad's normal, and how it changes.
    ///
    /// `init` is where the particle starts, `init_rand` a symmetric spread on that, `add` the
    /// per-frame velocity and `add_rand` a spread on that. Radians, as the emitter's own
    /// rotation is.
    ///
    /// Reading these is what tells a spinning smoke puff apart from an attack arc. Both are
    /// one quad with a texture on it; the puff carries a full turn of `init_rand` so every
    /// one in the cloud sits differently, and the arc carries zeros because the crescent is
    /// painted at the angle it is meant to be seen at. Substituting a random roll for the
    /// data made the puffs look right and left the arc at a random angle every time -- and no
    /// orientation setting could correct it, because the error was per-particle.
    pub rotate_init: glam::Vec3,
    pub rotate_init_rand: glam::Vec3,
    pub rotate_add: glam::Vec3,
    pub rotate_add_rand: glam::Vec3,
    /// Which axes the emitter actually animates, from `particle_data.is_rotate_*`.
    ///
    /// Not decoration. Across ef_common Z is enabled on 49% of emitters, Y on 14% and X on
    /// 5%, so an implementation that assumes the roll is always about Z covers about half the
    /// game and silently drops the rest -- including SYS_ATTACK_ARC, which sweeps about Y.
    pub rotate_enabled: [bool; 3],
    /// Per-frame multiplier on rotation velocity -- a resistance, not a rate. The arc carries
    /// 0.99 and its cylinder 0.98, so the sweep slows as it goes.
    ///
    /// Zero means the emitter never set it, not "stop instantly": treating an unset field as
    /// total damping would freeze every rotation in the game.
    pub rotate_regist: f32,
    /// How particles keep to the emitter once born: 0 they move with it, 1 they stay where they were born (what every
    /// trail in the game uses), 2 they follow its position but not its turn.
    pub follow_type: i64,
    /// Fire by how far the emitter has moved rather than by the clock: `rate` particles every `unit` of travel. A
    /// standing emitter makes none. Every trail in the game is built this way.
    pub distance: Option<Distance>,
    /// Number of cells the emitter's texture is divided into. Zero or one means the texture is
    /// a single image and the particle uses all of it.
    pub pattern_cells: u32,
    /// Cell index to show at each step of the pattern animation.
    pub pattern_table: Vec<i32>,
    /// Frames each pattern step lasts.
    pub pattern_frequency: f32,
    /// Cells the emitter's texture divides into, worked out from the texture's real size. Set
    /// by the caller, which is the side that knows the texture; the emitter data alone does
    /// not say.
    pub sheet_cells: u32,
    /// Name hash of the primitive this emitter draws, when it draws one. Matched against the
    /// file's descriptor table -- it is not an index, and treating it as one finds the wrong
    /// mesh or none.
    pub primitive_id: Option<u64>,
    /// The emitter's own offset from the effect's origin, and its orientation.
    ///
    /// Ignored until now, which stacked every emitter of an effect at one point: SYS_TURN_SMOKE
    /// separates its five emitters by up to 2 units front-to-back, and collapsing them together
    /// turns a cloud into a single blob.
    pub translation: glam::Vec3,
    /// Euler rotation, radians. EDGE_ATTACK_DASH_HIT turns two of its emitters 90 degrees.
    pub rotation: glam::Vec3,
    pub color0: Vec<ColorKey>,
    pub alpha0: Vec<ColorKey>,
}

fn attr(emitter: &EmitterDef, slots: &Slots, which: Option<usize>) -> Option<f32> {
    let _ = slots;
    match emitter.attrs.get(which?).and_then(|value| value.as_ref())? {
        AttrValue::Int(v) => Some(*v as f32),
        AttrValue::UInt(v) => Some(*v as f32),
        AttrValue::Float(v) => Some(*v),
    }
}

/// Attribute-table positions, looked up once per load rather than per emitter.
pub struct Slots {
    rate: Option<usize>,
    rate_random: Option<usize>,
    interval: Option<usize>,
    interval_random: Option<usize>,
    emission_start: Option<usize>,
    emission_duration: Option<usize>,
    is_one_time: Option<usize>,
    is_world_gravity: Option<usize>,
    position_random: Option<usize>,
    air_res: Option<usize>,
    momentum_random: Option<usize>,
    diffusion_dir_angle: Option<usize>,
    xz_diffusion: Option<usize>,
    sweep_start_random: Option<usize>,
    surface_pos_rand: Option<usize>,
    num_divide_circle: [Option<usize>; 2],
    num_divide_line: [Option<usize>; 2],
    gravity_scale: Option<usize>,
    gravity_dir: [Option<usize>; 3],
    life: Option<usize>,
    life_random: Option<usize>,
    all_direction: Option<usize>,
    designated_dir: [Option<usize>; 3],
    designated_dir_scale: Option<usize>,
    diffusion: [Option<usize>; 3],
    velocity_random: Option<usize>,
    scale: [Option<usize>; 3],
    uv_div: [Option<usize>; 2],
    uv_scroll: [Option<usize>; 2],
    uv_scroll_add: [Option<usize>; 2],
    uv_scale: [Option<usize>; 2],
    uv_scale_add: [Option<usize>; 2],
    volume_type: Option<usize>,
    volume_radius: [Option<usize>; 3],
    volume_form_scale: [Option<usize>; 3],
    sweep_longitude: Option<usize>,
    sweep_latitude: Option<usize>,
    sweep_start: Option<usize>,
    arc_type: Option<usize>,
    latitude_dir: Option<usize>,
    line_length: Option<usize>,
    line_center: Option<usize>,
    caliber_ratio: Option<usize>,
    particle_scale: [Option<usize>; 3],
    scale_random: [Option<usize>; 3],
    primitive_id: Option<usize>,
    translation: [Option<usize>; 3],
    rotation: [Option<usize>; 3],
    pattern_cells: Option<usize>,
    pattern_frequency: Option<usize>,
    pattern_table: Vec<Option<usize>>,
    num_scale_keys: Option<usize>,
    scale_keys: Vec<[Option<usize>; 4]>,
    billboard_type: Option<usize>,
    color_scale: Option<usize>,
    alpha_input: Option<usize>,
    color_input: Option<usize>,
    shader_type: Option<usize>,
    tex1_blend: Option<usize>,
    blend_type: Option<usize>,
    rotate_init: [Option<usize>; 3],
    rotate_init_rand: [Option<usize>; 3],
    rotate_add: [Option<usize>; 3],
    rotate_add_rand: [Option<usize>; 3],
    is_rotate: [Option<usize>; 3],
    rotate_regist: Option<usize>,
    follow_type: Option<usize>,
    distance: [Option<usize>; 4],
}

impl Slots {
    pub fn new() -> Self {
        let table = crate::eff_attrs::table();
        let at = |id: &str| table.iter().position(|attr| attr.id == id);
        Self {
            rate: at("emission.rate"),
            rate_random: at("emission.rate_random"),
            interval: at("emission.interval"),
            interval_random: at("emission.interval_random"),
            is_one_time: at("emission.is_one_time"),
            is_world_gravity: at("emission.is_world_gravity"),
            air_res: at("emitter_static.air_res"),
            momentum_random: at("particle_data.momentum_random"),
            diffusion_dir_angle: at("particle_velocity.diffusion_dir_angle"),
            xz_diffusion: at("particle_velocity.xz_diffusion"),
            sweep_start_random: at("shape_info.sweep_start_random"),
            arc_type: at("shape_info.arc_type"),
            latitude_dir: at("shape_info.volume_latitude_dir"),
            surface_pos_rand: at("shape_info.volume_surface_pos_rand"),
            num_divide_circle: [
                at("shape_info.num_divide_circle"),
                at("shape_info.num_divide_circle_random"),
            ],
            num_divide_line: [
                at("shape_info.num_divide_line"),
                at("shape_info.num_divide_line_random"),
            ],
            emission_start: at("emission.start"),
            emission_duration: at("emission.duration"),
            position_random: at("emission.position_random"),
            gravity_scale: at("emission.gravity_scale"),
            gravity_dir: [
                at("emission.gravity_dir_x"),
                at("emission.gravity_dir_y"),
                at("emission.gravity_dir_z"),
            ],
            life: at("particle_data.life"),
            life_random: at("particle_data.life_random"),
            all_direction: at("particle_velocity.all_direction"),
            designated_dir: [
                at("particle_velocity.designated_dir_x"),
                at("particle_velocity.designated_dir_y"),
                at("particle_velocity.designated_dir_z"),
            ],
            designated_dir_scale: at("particle_velocity.designated_dir_scale"),
            diffusion: [
                at("particle_velocity.diffusion_x"),
                at("particle_velocity.diffusion_y"),
                at("particle_velocity.diffusion_z"),
            ],
            velocity_random: at("particle_velocity.vel_random"),
            scale: [
                at("emitter_info.scale_x"),
                at("emitter_info.scale_y"),
                at("emitter_info.scale_z"),
            ],
            primitive_id: at("particle_data.primitive_id"),
            translation: [
                at("emitter_info.trans_x"),
                at("emitter_info.trans_y"),
                at("emitter_info.trans_z"),
            ],
            rotation: [
                at("emitter_info.rotate_x"),
                at("emitter_info.rotate_y"),
                at("emitter_info.rotate_z"),
            ],
            pattern_cells: at("emitter_static.tex_pattern_anim0.num"),
            pattern_frequency: at("emitter_static.tex_pattern_anim0.frequency"),
            pattern_table: (0..32)
                .map(|i| at(&format!("emitter_static.tex_pattern_anim0.table[{i}]")))
                .collect(),
            num_scale_keys: at("emitter_static.num_scale_keys"),
            scale_keys: (0..8)
                .map(|i| {
                    [
                        at(&format!("emitter_static.scale_anim.keys[{i}].x")),
                        at(&format!("emitter_static.scale_anim.keys[{i}].y")),
                        at(&format!("emitter_static.scale_anim.keys[{i}].z")),
                        at(&format!("emitter_static.scale_anim.keys[{i}].time")),
                    ]
                })
                .collect(),
            uv_div: [
                at("emitter_static.tex_scroll_anim0.uv_div_x"),
                at("emitter_static.tex_scroll_anim0.uv_div_y"),
            ],
            uv_scroll: [
                at("emitter_static.tex_scroll_anim0.scroll_x"),
                at("emitter_static.tex_scroll_anim0.scroll_y"),
            ],
            uv_scroll_add: [
                at("emitter_static.tex_scroll_anim0.scroll_add_x"),
                at("emitter_static.tex_scroll_anim0.scroll_add_y"),
            ],
            uv_scale: [
                at("emitter_static.tex_scroll_anim0.scale_x"),
                at("emitter_static.tex_scroll_anim0.scale_y"),
            ],
            uv_scale_add: [
                at("emitter_static.tex_scroll_anim0.scale_add_x"),
                at("emitter_static.tex_scroll_anim0.scale_add_y"),
            ],
            volume_type: at("shape_info.volume_type"),
            volume_radius: [
                at("shape_info.volume_radius_x"),
                at("shape_info.volume_radius_y"),
                at("shape_info.volume_radius_z"),
            ],
            volume_form_scale: [
                at("shape_info.volume_form_scale_x"),
                at("shape_info.volume_form_scale_y"),
                at("shape_info.volume_form_scale_z"),
            ],
            sweep_longitude: at("shape_info.sweep_longitude"),
            sweep_latitude: at("shape_info.sweep_latitude"),
            sweep_start: at("shape_info.sweep_start"),
            line_length: at("shape_info.line_length"),
            line_center: at("shape_info.line_center"),
            caliber_ratio: at("shape_info.caliber_ratio"),
            particle_scale: [
                at("particle_scale.scale_x"),
                at("particle_scale.scale_y"),
                at("particle_scale.scale_z"),
            ],
            scale_random: [
                at("particle_scale.scale_random_x"),
                at("particle_scale.scale_random_y"),
                at("particle_scale.scale_random_z"),
            ],
            billboard_type: at("particle_data.billboard_type"),
            color_scale: at("emitter_static.color_scale"),
            alpha_input: at("combiner.tex_alpha0_input_type"),
            color_input: at("combiner.tex_color0_input_type"),
            shader_type: at("combiner.shader_type"),
            tex1_blend: at("combiner.texture1_color_blend"),
            blend_type: at("render_state.blend_type"),
            rotate_init: [
                at("emitter_static.rotate_init_x"),
                at("emitter_static.rotate_init_y"),
                at("emitter_static.rotate_init_z"),
            ],
            rotate_init_rand: [
                at("emitter_static.rotate_init_rand_x"),
                at("emitter_static.rotate_init_rand_y"),
                at("emitter_static.rotate_init_rand_z"),
            ],
            rotate_add: [
                at("emitter_static.rotate_add_x"),
                at("emitter_static.rotate_add_y"),
                at("emitter_static.rotate_add_z"),
            ],
            rotate_add_rand: [
                at("emitter_static.rotate_add_rand_x"),
                at("emitter_static.rotate_add_rand_y"),
                at("emitter_static.rotate_add_rand_z"),
            ],
            is_rotate: [
                at("particle_data.is_rotate_x"),
                at("particle_data.is_rotate_y"),
                at("particle_data.is_rotate_z"),
            ],
            rotate_regist: at("emitter_static.rotate_regist"),
            follow_type: at("emitter_info.follow_type"),
            distance: [
                at("emission.is_emit_dist_enabled"),
                at("emission.emitter_dist_unit"),
                at("emission.emitter_dist_min"),
                at("emission.emitter_dist_max"),
            ],
        }
    }
}

impl Default for Slots {
    fn default() -> Self {
        Self::new()
    }
}

impl EmitterSim {
    pub fn read(emitter: &EmitterDef, slots: &Slots) -> Self {
        let get = |which: Option<usize>| attr(emitter, slots, which);
        let vec3 = |which: &[Option<usize>; 3]| {
            glam::Vec3::new(
                get(which[0]).unwrap_or(0.0),
                get(which[1]).unwrap_or(0.0),
                get(which[2]).unwrap_or(0.0),
            )
        };
        let gravity_scale = get(slots.gravity_scale).unwrap_or(0.0);
        Self {
            // A rate of zero would emit nothing at all, which is almost never what the data
            // means — it means the field was not populated for this emitter.
            primitive_id: slots.primitive_id.and_then(|slot| {
                match emitter.attrs.get(slot).and_then(|value| value.as_ref()) {
                    Some(AttrValue::UInt(v)) => Some(*v),
                    Some(AttrValue::Int(v)) if *v > 0 => Some(*v as u64),
                    _ => None,
                }
            }),
            translation: vec3(&slots.translation),
            rotation: vec3(&slots.rotation),
            rate: get(slots.rate).unwrap_or(1.0).max(0.0),
            rate_random: get(slots.rate_random).unwrap_or(0.0),
            interval: get(slots.interval).unwrap_or(0.0).max(0.0),
            interval_random: get(slots.interval_random).unwrap_or(0.0).max(0.0),
            emission_start: get(slots.emission_start).unwrap_or(0.0),
            emission_duration: get(slots.emission_duration).unwrap_or(0.0),
            is_one_time: get(slots.is_one_time).map(|v| v != 0.0),
            world_gravity: get(slots.is_world_gravity).unwrap_or(0.0) != 0.0,
            position_random: get(slots.position_random).unwrap_or(0.0),
            // Zero is a real setting -- a handful of emitters use it to throw a particle one
            // step and leave it to gravity -- so only a missing field means "no drag".
            air_res: get(slots.air_res).unwrap_or(1.0).max(0.0),
            momentum_random: get(slots.momentum_random).unwrap_or(0.0),
            diffusion_dir_angle: get(slots.diffusion_dir_angle).unwrap_or(0.0),
            xz_diffusion: get(slots.xz_diffusion).unwrap_or(0.0),
            sweep_start_random: get(slots.sweep_start_random).unwrap_or(0.0) != 0.0,
            arc_type: get(slots.arc_type).unwrap_or(0.0) as i64,
            // Unset is +Y, the pole the cap is built around.
            latitude_dir: get(slots.latitude_dir).unwrap_or(2.0) as i64,
            surface_pos_rand: get(slots.surface_pos_rand).unwrap_or(0.0),
            num_divide_circle: (
                get(slots.num_divide_circle[0]).unwrap_or(1.0).max(1.0) as u32,
                get(slots.num_divide_circle[1]).unwrap_or(0.0),
            ),
            num_divide_line: (
                get(slots.num_divide_line[0]).unwrap_or(1.0).max(1.0) as u32,
                get(slots.num_divide_line[1]).unwrap_or(0.0),
            ),
            gravity: vec3(&slots.gravity_dir) * gravity_scale,
            life: get(slots.life).unwrap_or(20.0).max(1.0),
            life_random: get(slots.life_random).unwrap_or(0.0),
            all_direction: get(slots.all_direction).unwrap_or(0.0),
            designated_dir: vec3(&slots.designated_dir),
            designated_dir_scale: get(slots.designated_dir_scale).unwrap_or(0.0),
            diffusion: vec3(&slots.diffusion),
            velocity_random: get(slots.velocity_random).unwrap_or(0.0),
            color_scale: get(slots.color_scale).unwrap_or(1.0).max(0.0),
            alpha_input: get(slots.alpha_input).unwrap_or(0.0) as i64,
            color_input: get(slots.color_input).unwrap_or(0.0) as i64,
            shader_type: get(slots.shader_type).unwrap_or(0.0) as i64,
            tex1_blend: get(slots.tex1_blend).unwrap_or(0.0) as i64,
            scale: {
                let scale = vec3(&slots.scale);
                // An all-zero scale draws nothing. Treated as "unset" rather than "invisible",
                // because an emitter that renders nothing at all is not a shape the data uses.
                if scale.length_squared() <= f32::EPSILON {
                    glam::Vec3::ONE
                } else {
                    scale
                }
            },
            uv_scroll: glam::Vec2::new(
                get(slots.uv_scroll[0]).unwrap_or(0.0),
                get(slots.uv_scroll[1]).unwrap_or(0.0),
            ),
            uv_scroll_add: glam::Vec2::new(
                get(slots.uv_scroll_add[0]).unwrap_or(0.0),
                get(slots.uv_scroll_add[1]).unwrap_or(0.0),
            ),
            uv_scale: {
                let scale = glam::Vec2::new(
                    get(slots.uv_scale[0]).unwrap_or(1.0),
                    get(slots.uv_scale[1]).unwrap_or(1.0),
                );
                // A zero zoom is "unset", not "collapse the texture to one texel".
                glam::Vec2::new(
                    if scale.x.abs() <= f32::EPSILON { 1.0 } else { scale.x },
                    if scale.y.abs() <= f32::EPSILON { 1.0 } else { scale.y },
                )
            },
            uv_scale_add: glam::Vec2::new(
                get(slots.uv_scale_add[0]).unwrap_or(0.0),
                get(slots.uv_scale_add[1]).unwrap_or(0.0),
            ),
            uv_div: {
                let axis = |slot| get(slot).unwrap_or(1.0).round().clamp(1.0, 64.0) as u32;
                (axis(slots.uv_div[0]), axis(slots.uv_div[1]))
            },
            volume_type: get(slots.volume_type).unwrap_or(0.0) as i64,
            // The shape's own per-axis stretch multiplies its radius (0x8dc80).
            volume_radius: vec3(&slots.volume_radius)
                * glam::Vec3::new(
                    get(slots.volume_form_scale[0]).unwrap_or(1.0),
                    get(slots.volume_form_scale[1]).unwrap_or(1.0),
                    get(slots.volume_form_scale[2]).unwrap_or(1.0),
                ),
            sweep_longitude: get(slots.sweep_longitude).unwrap_or(std::f32::consts::TAU),
            sweep_latitude: get(slots.sweep_latitude).unwrap_or(std::f32::consts::PI),
            sweep_start: get(slots.sweep_start).unwrap_or(0.0),
            line_length: get(slots.line_length).unwrap_or(0.0),
            line_center: get(slots.line_center).unwrap_or(0.0),
            caliber_ratio: get(slots.caliber_ratio).unwrap_or(1.0),
            particle_scale: {
                let scale = vec3(&slots.particle_scale);
                // An unset base is 1, so the curve and emitter scale still decide the size,
                // rather than the particle collapsing to nothing.
                if scale.length_squared() <= f32::EPSILON {
                    glam::Vec3::ONE
                } else {
                    scale
                }
            },
            scale_random: vec3(&slots.scale_random),
            scale_keys: {
                let count = get(slots.num_scale_keys).unwrap_or(0.0).max(0.0) as usize;
                slots
                    .scale_keys
                    .iter()
                    .take(count.min(8))
                    .filter_map(|key| {
                        Some((
                            get(key[3])?,
                            glam::Vec3::new(get(key[0])?, get(key[1])?, get(key[2])?),
                        ))
                    })
                    .collect()
            },
            pattern_cells: get(slots.pattern_cells).unwrap_or(0.0).max(0.0) as u32,
            pattern_frequency: get(slots.pattern_frequency).unwrap_or(1.0).max(1.0),
            sheet_cells: 0,
            pattern_table: slots
                .pattern_table
                .iter()
                .filter_map(|slot| get(*slot).map(|v| v as i32))
                .collect(),
            billboard_type: emitter
                .attrs
                .get(slots.billboard_type.unwrap_or(usize::MAX))
                .and_then(|value| value.as_ref())
                .map(|value| match value {
                    AttrValue::Int(v) => *v,
                    AttrValue::UInt(v) => *v as i64,
                    AttrValue::Float(v) => *v as i64,
                })
                .unwrap_or(0),
            blend_type: emitter
                .attrs
                .get(slots.blend_type.unwrap_or(usize::MAX))
                .and_then(|value| value.as_ref())
                .map(|value| match value {
                    AttrValue::Int(v) => *v,
                    AttrValue::UInt(v) => *v as i64,
                    AttrValue::Float(v) => *v as i64,
                })
                .unwrap_or(0),
            rotate_init: vec3(&slots.rotate_init),
            rotate_init_rand: vec3(&slots.rotate_init_rand),
            rotate_add: vec3(&slots.rotate_add),
            rotate_add_rand: vec3(&slots.rotate_add_rand),
            rotate_enabled: [
                get(slots.is_rotate[0]).unwrap_or(0.0) != 0.0,
                get(slots.is_rotate[1]).unwrap_or(0.0) != 0.0,
                get(slots.is_rotate[2]).unwrap_or(0.0) != 0.0,
            ],
            rotate_regist: get(slots.rotate_regist).unwrap_or(0.0),
            follow_type: get(slots.follow_type).unwrap_or(0.0) as i64,
            distance: (get(slots.distance[0]).unwrap_or(0.0) != 0.0).then(|| Distance {
                unit: get(slots.distance[1]).unwrap_or(1.0).max(0.01),
                min: get(slots.distance[2]).unwrap_or(0.0).max(0.0),
                max: get(slots.distance[3]).unwrap_or(0.0).max(0.0),
            }),
            color0: emitter.color0.clone(),
            alpha0: emitter.alpha0_keys.clone(),
        }
    }

    /// How many frames this emitter keeps firing for.
    ///
    /// Only a one-time emitter stops (0x8dc80): it fires while its clock is short of
    /// `start + duration`, so a duration of 1 is a single firing. Anything else runs until the
    /// effect is removed. A one-time emitter that has made nothing yet keeps trying past its
    /// end, so even a duration of 0 fires once -- see [`bursts`].
    fn emission_span(&self) -> f32 {
        match self.is_one_time {
            Some(true) => self.emission_duration.max(0.0),
            Some(false) => f32::INFINITY,
            None if self.emission_duration > 0.0 => self.emission_duration,
            None => f32::INFINITY,
        }
    }

    /// How many positions an evenly divided shape has on this firing. `None` for every other
    /// shape.
    fn divisions(&self, unit: f32) -> Option<u32> {
        let (count, random) = match self.volume_type {
            2 => self.num_divide_circle,
            13 => self.num_divide_line,
            _ => return None,
        };
        let short = (unit * random * 0.01 * count as f32) as u32;
        Some(count.saturating_sub(short).max(1))
    }
}

/// Distance emission's settings.
#[derive(Debug, Clone, Copy)]
pub struct Distance {
    /// Travel between firings.
    pub unit: f32,
    /// A frame's travel below this counts for nothing; above `max` (when set) it counts as `max`, so a teleport
    /// doesn't pour out a wall of particles.
    pub min: f32,
    pub max: f32,
}

/// One firing of an emitter: when, and which particles it made.
struct Burst {
    /// Frames after the emitter's start.
    time: f32,
    /// Index of its first particle among all the emitter has made.
    first: u64,
    count: u32,
    /// Positions of an evenly divided shape on this firing.
    divisions: u32,
    /// The firing's own random number, shared by its particles (a random sweep start).
    unit: f32,
}

/// Every firing up to `until` frames after the emitter's start, oldest first.
///
/// The game's loop (0x8d340), run forward: when the gap has elapsed, add the rate -- less a
/// random percentage -- to a running total, emit its whole part, keep the fraction, and draw
/// the next gap. Walked from the start because both the fraction and a random gap depend on
/// everything before them; it is a few additions per firing.
fn bursts(sim: &EmitterSim, until: f32, seed: u64) -> Vec<Burst> {
    let span = sim.emission_span();
    let mut out = Vec::new();
    let mut time = 0.0f32;
    let mut saving = 0.0f32;
    let mut first = 0u64;
    let mut index = 0u64;
    // Past its end a one-time emitter still fires if it has not managed a particle yet,
    // which is what guarantees a rate below one, or a duration of zero, shows something.
    while time <= until && (time < span || out.is_empty()) && out.len() < MAX_BURSTS {
        let unit = hashed(seed, index ^ 0xb0_0000);
        saving += sim.rate * (1.0 - sim.rate_random / 100.0 * hashed(seed, index ^ 0xb1_0000));
        saving = saving.max(0.0);
        let whole = saving as u32;
        saving -= whole as f32;
        let divisions = sim.divisions(unit);
        let count = whole.saturating_mul(divisions.unwrap_or(1));
        if count > 0 {
            out.push(Burst {
                time,
                first,
                count,
                divisions: divisions.unwrap_or(1),
                unit,
            });
            first += count as u64;
        }
        let extra = (hashed(seed, index ^ 0xb2_0000) * sim.interval_random).floor();
        time += sim.interval + 1.0 + extra;
        index += 1;
    }
    out
}

/// Firings of a distance emitter on a moving emitter: `travelled(t)` is how far it has gone by frame `t` after its
/// start. Walked a frame at a time, as the game measures each frame's move.
fn bursts_by_distance(sim: &EmitterSim, distance: Distance, until: f32, seed: u64, travelled: &dyn Fn(f32) -> f32) -> Vec<Burst> {
    let span = sim.emission_span();
    let mut out = Vec::new();
    let (mut first, mut saving, mut gone) = (0u64, 0.0f32, 0.0f32);
    let mut frame = 0.0f32;
    while frame <= until && frame < span && out.len() < MAX_BURSTS {
        let mut step = travelled(frame + 1.0) - travelled(frame);
        if step < distance.min {
            step = 0.0;
        }
        if distance.max > 0.0 {
            step = step.min(distance.max);
        }
        gone += step;
        let units = (gone / distance.unit).floor();
        gone -= units * distance.unit;
        let index = frame as u64;
        saving += units * sim.rate * (1.0 - sim.rate_random / 100.0 * hashed(seed, index ^ 0xb1_0000));
        let whole = saving.max(0.0) as u32;
        saving -= whole as f32;
        if whole > 0 {
            let unit = hashed(seed, index ^ 0xb0_0000);
            let divisions = sim.divisions(unit).unwrap_or(1);
            let count = whole.saturating_mul(divisions);
            out.push(Burst { time: frame, first, count, divisions, unit });
            first += count as u64;
        }
        frame += 1.0;
    }
    out
}

/// Where a particle is after `age` frames, relative to its spawn point, and its velocity then.
///
/// The game's step (0x95370) is, each frame: move by the velocity, scale the velocity by the
/// drag, add gravity. Summed over `n` whole frames that is a geometric series, written out
/// here; between frames the two neighbouring sums are blended, which keeps a scrubbed
/// half-frame on the path the game's own frames lie on.
fn travel(v0: glam::Vec3, gravity: glam::Vec3, drag: f32, age: f32) -> (glam::Vec3, glam::Vec3) {
    let undamped = (drag - 1.0).abs() < 1e-6;
    let at = |n: f32| {
        if undamped {
            v0 * n + gravity * (n * (n - 1.0) * 0.5)
        } else {
            let sum = (1.0 - drag.powf(n)) / (1.0 - drag);
            v0 * sum + gravity * ((n - sum) / (1.0 - drag))
        }
    };
    let whole = age.max(0.0).floor();
    let part = age.max(0.0) - whole;
    let offset = at(whole).lerp(at(whole + 1.0), part);
    let velocity = if undamped {
        v0 + gravity * age
    } else {
        let decay = drag.powf(age);
        v0 * decay + gravity * ((1.0 - decay) / (1.0 - drag))
    };
    (offset, velocity)
}






/// Cap on particles produced by one emitter in one evaluation.
///
/// A continuous emitter with a high rate, evaluated late in a long move, describes an unbounded
/// number of particles; the viewport has to stay interactive while scrubbing, and beyond a few
/// hundred quads per emitter nothing further is distinguishable on screen.
const MAX_PER_EMITTER: usize = 256;
/// Firings walked per evaluation. Past this an emitter has been running for hours.
const MAX_BURSTS: usize = 100_000;

/// A uniformly random unit vector.
fn unit_vector(seed: u64, id: u64, salt: u64) -> glam::Vec3 {
    let y = hashed_signed(seed, id ^ salt);
    let turn = hashed(seed, id ^ (salt + 1)) * std::f32::consts::TAU;
    let ring = (1.0 - y * y).max(0.0).sqrt();
    glam::Vec3::new(ring * turn.cos(), y, ring * turn.sin())
}

/// Where in the emitter's shape a particle is born, and the direction the shape throws it.
///
/// The game's shape functions return both: a position, and a velocity that is this direction
/// times the emitter's all-direction speed. `slot` is the particle's place within its firing,
/// which is what the evenly divided shapes space themselves by.
///
/// These are the game's own formulas (0x90c00 to 0x92e00), with two exceptions: the evenly
/// divided spheres place particles from a lookup table and are scattered here instead, and
/// the box, rectangle and primitive shapes are not modelled and spawn at the emitter.
fn volume_spawn(
    sim: &EmitterSim,
    burst: &Burst,
    slot: u32,
    id: u64,
    seed: u64,
) -> (glam::Vec3, glam::Vec3) {
    let unit = |salt: u64| hashed(seed, id ^ salt);
    let radius = sim.volume_radius;
    let start = if sim.sweep_start_random {
        burst.unit * std::f32::consts::TAU
    } else {
        sim.sweep_start
    };
    // The sweep is centred on its start, not begun there.
    let scattered = start + sim.sweep_longitude * (unit(0x81) - 0.5);
    // Angle 0 lies along +Z and the sweep turns toward +X.
    let on_circle = |angle: f32, reach: f32| {
        let dir = glam::Vec3::new(angle.sin(), 0.0, angle.cos());
        (
            glam::Vec3::new(dir.x * radius.x, 0.0, dir.z * radius.z) * reach,
            dir,
        )
    };

    match sim.volume_type {
        1 => on_circle(scattered, 1.0),
        2 => {
            // An arc puts a particle on each end, so it has one gap fewer than positions; a
            // full turn would stack its two ends, so it keeps every gap.
            let open = (sim.sweep_longitude - std::f32::consts::TAU).abs() > 1e-4;
            let gaps = if open && burst.divisions > 1 {
                burst.divisions - 1
            } else {
                burst.divisions
            };
            let angle = start + sim.sweep_longitude / gaps as f32 * slot as f32
                - sim.sweep_longitude * 0.5
                + sim.surface_pos_rand * hashed_signed(seed, id ^ 0x87);
            on_circle(angle, 1.0)
        }
        3 | 9 => {
            // Evenly over the ring between the hole and the rim.
            let inner = 1.0 - sim.caliber_ratio;
            let pick = unit(0x82);
            let reach = (pick + (1.0 - pick) * inner * inner).max(0.0).sqrt();
            let (mut position, dir) = on_circle(scattered, reach);
            // On an ellipse the throw leans toward the long axis.
            let mut thrown = dir * reach;
            if radius.x > radius.z {
                thrown.z *= radius.z / radius.x;
            } else if radius.z > 0.0 {
                thrown.x *= radius.x / radius.z;
            }
            if sim.volume_type == 9 {
                position.y = hashed_signed(seed, id ^ 0x86) * radius.y;
            }
            (position, thrown.normalize_or_zero())
        }
        8 => {
            let (mut position, dir) = on_circle(scattered, 1.0);
            position.y = hashed_signed(seed, id ^ 0x86) * radius.y;
            (position, dir)
        }
        4 | 5 | 6 | 7 => {
            // Height is drawn evenly, which is what covers a sphere evenly. A longitude cut
            // keeps the full height and narrows the turn; a latitude cut keeps the full turn
            // and stops the height at the cap's edge.
            let (turn, height) = if sim.arc_type == 1 {
                let edge = sim.sweep_latitude.cos();
                (
                    unit(0x81) * std::f32::consts::TAU,
                    1.0 - unit(0x83) * (1.0 - edge),
                )
            } else {
                (scattered, hashed_signed(seed, id ^ 0x83))
            };
            let ring = (1.0 - height * height).max(0.0).sqrt();
            let mut dir = glam::Vec3::new(ring * turn.sin(), height, ring * turn.cos());
            if sim.arc_type == 1 {
                let pole = match sim.latitude_dir {
                    0 => glam::Vec3::X,
                    1 => glam::Vec3::NEG_X,
                    3 => glam::Vec3::NEG_Y,
                    4 => glam::Vec3::Z,
                    5 => glam::Vec3::NEG_Z,
                    _ => glam::Vec3::Y,
                };
                dir = glam::Quat::from_rotation_arc(glam::Vec3::Y, pole) * dir;
            }
            // A filled sphere reaches from its hole to its rim (0x922c0).
            let reach = if sim.volume_type == 7 {
                unit(0x84).sqrt() * sim.caliber_ratio + 1.0 - sim.caliber_ratio
            } else {
                1.0
            };
            (dir * radius * reach, dir)
        }
        12 | 13 => {
            // Along Z. `line_center` slides the line: 0 centres it on the emitter.
            let fraction = if sim.volume_type == 13 {
                if burst.divisions > 1 {
                    slot as f32 / (burst.divisions - 1) as f32
                } else {
                    0.5
                }
            } else {
                unit(0x81)
            };
            let length = sim.line_length;
            let z = length * fraction - 0.5 * (length + length * sim.line_center);
            (glam::Vec3::new(0.0, 0.0, z), glam::Vec3::Z)
        }
        // A point, and the shapes not modelled: at the emitter, thrown in a random direction.
        _ => (glam::Vec3::ZERO, unit_vector(seed, id, 0x21)),
    }
}

/// A particle as it is born: its life, where it appears and how it is thrown, all in the emitter's frame. Everything
/// after birth is a function of these and its age, which is what both the CPU evaluation here and the game's own
/// vertex shaders (fed by [`spawns`]) do with them.
struct Initial {
    life: f32,
    spawn: glam::Vec3,
    velocity: glam::Vec3,
    /// The particle's own pace (momentum_random), applied to everything it travels.
    pace: f32,
}

fn initial(sim: &EmitterSim, burst: &Burst, slot: u32, id: u64, seed: u64) -> Initial {
    // A whole percentage of the life, taken off: 30 means anywhere from 70% to the
    // full life, in steps of 1% (0x908d8). Never added.
    let short = (hashed(seed, id ^ 0x11) * sim.life_random.max(0.0)).floor();
    let life = (sim.life * (1.0 - short / 100.0)).max(1.0);

    let (born, thrown) = volume_spawn(sim, burst, slot, id, seed);
    let mut velocity = thrown * sim.all_direction;
    if sim.xz_diffusion != 0.0 {
        // Away from the emitter's Y axis. A particle born on the axis has no "away",
        // so it gets a random one.
        let mut outward = glam::Vec3::new(born.x, 0.0, born.z);
        if outward.length_squared() <= f32::EPSILON {
            outward = glam::Vec3::new(
                hashed_signed(seed, id ^ 0x24),
                0.0,
                hashed_signed(seed, id ^ 0x25),
            );
        }
        velocity += outward.normalize_or_zero() * sim.xz_diffusion;
    }
    // The directed part: straight along the designated direction, or spread over a
    // cone about it. The cone is sampled about +Y and turned onto the direction.
    let directed = if sim.diffusion_dir_angle == 0.0 {
        sim.designated_dir
    } else {
        let floor = 1.0 - sim.diffusion_dir_angle / 90.0;
        let turn = hashed(seed, id ^ 0x26) * std::f32::consts::TAU;
        let y = floor + (1.0 - floor) * hashed(seed, id ^ 0x27);
        let ring = (1.0 - y * y).max(0.0).sqrt();
        let sample = glam::Vec3::new(ring * turn.cos(), y, ring * turn.sin());
        let length = sim.designated_dir.length();
        if length > f32::EPSILON {
            glam::Quat::from_rotation_arc(glam::Vec3::Y, sim.designated_dir / length)
                * sample
                * length
        } else {
            glam::Vec3::ZERO
        }
    };
    velocity += directed * sim.designated_dir_scale;
    // A percentage taken off the speed, never added (0x8fd7c): 50 means anywhere from
    // half speed to full. It scales the shape's throw and the directed part together.
    velocity *= 1.0 - sim.velocity_random / 100.0 * hashed(seed, id ^ 0x31);
    // The per-axis spread goes on afterwards, at full strength.
    velocity += sim.diffusion
        * glam::Vec3::new(
            hashed_signed(seed, id ^ 0x41),
            hashed_signed(seed, id ^ 0x42),
            hashed_signed(seed, id ^ 0x43),
        );

    let spawn = born + unit_vector(seed, id, 0x51) * sim.position_random;
    let pace = 1.0 + sim.momentum_random * hashed_signed(seed, id ^ 0x54);
    Initial { life, spawn, velocity, pace }
}

/// A live particle as the game's effect shaders take it (eff_gpu): the vertex inputs `sysLocalPosAttr` (position,
/// life), `sysLocalVecAttr` (velocity, birth), `sysScaleAttr`, `sysRandomAttr` and `sysInitRotateAttr`.
///
/// Measured against the game's own streams (the recomp capture): position and velocity are the particle's CURRENT
/// ones, in the emitter's frame (a CPU emitter moves its particles on the CPU, drag and gravity included, and the
/// shader animates size, colour and turn from the clock); life is the file's life less a whole percentage, birth is
/// a whole frame of the emitter's clock, the scale is the particle scale less one percentage for all axes, the
/// randoms are four uniforms in 0..1, and the initial rotation is the emitter's plus up to its random range (never
/// less).
#[derive(Debug, Clone, Copy)]
pub struct Spawn {
    pub position: glam::Vec3,
    pub life: f32,
    pub velocity: glam::Vec3,
    /// Frames on the effect's clock (the same clock as `evaluate`'s `age_frames`) when it was born.
    pub birth: f32,
    pub scale: glam::Vec3,
    pub random: [f32; 4],
    pub rotation: glam::Vec3,
}


/// [`spawns`] for an emitter that moves: `travelled(t)`, how far it has gone by frame `t` of the effect, drives a
/// distance emitter. Without it a distance emitter fires on the clock instead, so a still preview shows something.
pub fn spawns_moving(sim: &EmitterSim, age_frames: f32, seed: u64, travelled: Option<&dyn Fn(f32) -> f32>) -> Vec<Spawn> {
    let mut out = Vec::new();
    if age_frames < sim.emission_start {
        return out;
    }
    let since_start = age_frames - sim.emission_start;
    let firings = match (sim.distance, travelled) {
        (Some(distance), Some(travelled)) => {
            bursts_by_distance(sim, distance, since_start, seed, &|t| travelled(t + sim.emission_start))
        }
        _ => bursts(sim, since_start, seed),
    };
    'bursts: for burst in firings.iter().rev() {
        let age = since_start - burst.time;
        if age >= sim.life.max(1.0) {
            break;
        }
        for slot in (0..burst.count).rev() {
            if out.len() >= MAX_PER_EMITTER {
                break 'bursts;
            }
            let id = burst.first + slot as u64;
            let Initial { life, spawn, velocity, pace } = initial(sim, burst, slot, id, seed);
            if age >= life {
                continue;
            }
            // Where it is now and how fast it is going, as evaluate has it.
            let (travelled, heading) = travel(velocity, glam::Vec3::ZERO, sim.air_res, age);
            let (fallen, falling) = travel(glam::Vec3::ZERO, sim.gravity, sim.air_res, age);
            let shrink = 1.0 - sim.scale_random.x / 100.0 * hashed(seed, id ^ 0x91);
            let mut rotation = glam::Vec3::ZERO;
            for axis in 0..3 {
                if sim.rotate_enabled[axis] {
                    rotation[axis] = sim.rotate_init[axis] + sim.rotate_init_rand[axis] * hashed(seed, id ^ (0x61 + axis as u64));
                }
            }
            out.push(Spawn {
                position: spawn + (travelled + fallen) * pace,
                life,
                velocity: heading + falling,
                birth: sim.emission_start + burst.time,
                scale: sim.particle_scale * shrink.max(0.0),
                random: [0xa1u64, 0xa2, 0xa3, 0xa4].map(|salt| hashed(seed, id ^ salt)),
                rotation,
            });
        }
    }
    out
}

