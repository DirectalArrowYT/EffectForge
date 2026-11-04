//! The emitter block: the 0x750-byte constant buffer every effect shader reads as c9.
//!
//! It is the emitter as stored, header and `emitter_static`, after the changes the game makes when it loads the
//! emitter: animation tables padded out to eight keys, fixed colours written in as keys, loops turned into rates,
//! switched-off rotations and texture animations zeroed, and two words of flags the shaders branch on. Forge makes
//! those changes to a copy of the parsed `emitter_static` and writes it back out. Built this way from the .eff alone,
//! the block matched every one the game bound in the recomp captures of October 2026 (918 of them), byte for byte.

use effect_library::{AnimationKeyTable, Emitter, EmitterStatic, TexPatAnim, TexScrollAnim};

pub const BLOCK_LEN: usize = 0x750;
/// The emitter's header (flags, seed, name) comes before `emitter_static` and goes in as stored.
const HEADER_LEN: usize = 0x50;

/// The emitter's c9 block. `None` when the emitter has no stored binary.
pub fn emitter_block(emitter: &Emitter, version: u16) -> Option<Vec<u8>> {
    let stored = emitter.binary_data.as_ref().or(emitter.cached_binary.as_ref())?;
    if stored.len() < BLOCK_LEN {
        return None;
    }
    let mut s = emitter.data.emitter_static.clone();
    as_loaded(&mut s, emitter);
    let mut block = stored[..HEADER_LEN].to_vec();
    s.write(&mut block, version).ok()?;
    block.truncate(BLOCK_LEN);
    (block.len() == BLOCK_LEN).then_some(block)
}

/// What the game changes in `emitter_static` when it loads the emitter.
fn as_loaded(s: &mut EmitterStatic, emitter: &Emitter) {
    let d = &emitter.data;
    let colour = &d.particle_color;
    let particle = &d.particle_data;

    // Colours and alphas: a fixed one (type 0) is its constant in key 0. A fixed colour's table is then that one key
    // and zeros; every other table has the keys past its count repeat the last, one frame apart.
    let fixed = |t: &effect_library::ColorType| t.as_u8() == 0;
    let colour_tables = [
        (&mut s.color0, s.num_color0_keys, fixed(&colour.color0_type).then_some([colour.color0_r, colour.color0_g, colour.color0_b])),
        (&mut s.color1, s.num_color1_keys, fixed(&colour.color1_type).then_some([colour.color1_r, colour.color1_g, colour.color1_b])),
    ];
    for (table, count, constant) in colour_tables {
        match constant {
            Some([r, g, b]) => {
                for key in &mut table.keys {
                    (key.x, key.y, key.z, key.time) = (0.0, 0.0, 0.0, 0.0);
                }
                (table.keys[0].x, table.keys[0].y, table.keys[0].z) = (r, g, b);
            }
            None => pad(table, count),
        }
    }
    for (table, count, constant) in [
        (&mut s.alpha0, s.num_alpha0_keys, fixed(&colour.alpha0_type).then_some(colour.alpha0)),
        (&mut s.alpha1, s.num_alpha1_keys, fixed(&colour.alpha1_type).then_some(colour.alpha1)),
    ] {
        if let Some(a) = constant {
            table.keys[0].x = a;
        }
        pad(table, count);
    }
    pad(&mut s.scale_anim, s.num_scale_keys);
    pad(&mut s.param_anim, s.num_param_keys);

    // Loops: the rate as a float where the loop is on, and 1 where it starts at a random point.
    let loops = [
        (particle.loop_color0, particle.color0_loop_rate, particle.loop_random_color0, &mut s.color0_loop_rate, &mut s.color0_loop_random),
        (particle.loop_alpha0, particle.alpha0_loop_rate, particle.loop_random_alpha0, &mut s.alpha0_loop_rate, &mut s.alpha0_loop_random),
        (particle.loop_color1, particle.color1_loop_rate, particle.loop_random_color1, &mut s.color1_loop_rate, &mut s.color1_loop_random),
        (particle.loop_alpha1, particle.alpha1_loop_rate, particle.loop_random_alpha1, &mut s.alpha1_loop_rate, &mut s.alpha1_loop_random),
        (particle.scale_loop, particle.scale_loop_rate, particle.scale_loop_random, &mut s.scale_loop_rate, &mut s.scale_loop_random),
    ];
    for (on, rate, random, rate_slot, random_slot) in loops {
        if on {
            *rate_slot = rate as f32;
        }
        if random {
            *random_slot = 1.0;
        }
    }

    // Rotation: an axis the particle does not turn about has no start angle, spread, speed or speed spread.
    let turns = [particle.is_rotate_x, particle.is_rotate_y, particle.is_rotate_z != 0];
    let axes = [
        [&mut s.rotate_init_x, &mut s.rotate_init_rand_x, &mut s.rotate_add_x, &mut s.rotate_add_rand_x],
        [&mut s.rotate_init_y, &mut s.rotate_init_rand_y, &mut s.rotate_add_y, &mut s.rotate_add_rand_y],
        [&mut s.rotate_init_z, &mut s.rotate_init_rand_z, &mut s.rotate_add_z, &mut s.rotate_add_rand_z],
    ];
    for (turns, values) in turns.into_iter().zip(axes) {
        if !turns {
            values.into_iter().for_each(|v| *v = 0.0);
        }
    }

    // Texture animations 0..2. UV scale 2 on an axis that repeats, else 1; a scroll, rotation or scale the animation
    // does not do is zeroed (scale to 1); a random pattern animation (type 4) plays its first num_random cells.
    let settings = [&d.texture_anim0, &d.texture_anim1, &d.texture_anim2].map(AnimSettings::of);
    let anims: [(&mut TexScrollAnim, &mut TexPatAnim); 3] = [
        (&mut s.tex_scroll_anim0, &mut s.tex_pattern_anim0),
        (&mut s.tex_scroll_anim1, &mut s.tex_pattern_anim1),
        (&mut s.tex_scroll_anim2, &mut s.tex_pattern_anim2),
    ];
    for (a, (scroll, pattern)) in settings.iter().zip(anims) {
        scroll.uv_scale_x = if a.repeat & 1 != 0 { 2.0 } else { 1.0 };
        scroll.uv_scale_y = if a.repeat & 2 != 0 { 2.0 } else { 1.0 };
        if !a.scroll {
            (scroll.scroll_add_x, scroll.scroll_add_y, scroll.scroll_x, scroll.scroll_y) = (0.0, 0.0, 0.0, 0.0);
            (scroll.scroll_random_x, scroll.scroll_random_y) = (0.0, 0.0);
        }
        if !a.rotate {
            (scroll.rotation_add, scroll.rotation, scroll.rotation_random) = (0.0, 0.0, 0.0);
        }
        if !a.scale {
            (scroll.scale_add_x, scroll.scale_add_y, scroll.scale_x, scroll.scale_y) = (0.0, 0.0, 1.0, 1.0);
            (scroll.scale_random_x, scroll.scale_random_y) = (0.0, 0.0);
        }
        if a.pattern_type == 4 {
            pattern.num = pattern.num_random;
            let cells = (pattern.num_random.max(0.0) as usize).min(pattern.table.len());
            for (i, cell) in pattern.table.iter_mut().take(cells).enumerate() {
                *cell = i as i32;
            }
        }
    }

    // flags1 and flags2: switches the runtime derives from the emitter's settings for the shaders.
    let bit = |on: bool, at: u32| (on as u32) << at;
    let mut flags1 = d.particle_fluctuation.as_ref().map_or(0, |f| (f.is_wave_type as u32 >> 3) & 7); // wave 8/16/32
    flags1 |= bit(d.emission.is_world_gravity, 3);
    for (n, a) in settings.iter().enumerate() {
        let n = n as u32;
        if (1..=4).contains(&a.pattern_type) {
            flags1 |= 1 << (4 + 4 * n + a.pattern_type as u32 - 1);
        }
        flags1 |= bit(a.inv_rand_u, 19 + 2 * n) | bit(a.inv_rand_v, 20 + 2 * n);
        flags1 |= bit(a.loop_random, 25 + n);
    }
    flags1 |= bit(particle.rot_rev_rand_x, 16) | bit(particle.rot_rev_rand_y, 17) | bit(particle.rot_rev_rand_z, 18);
    flags1 |= match particle.offset_type {
        0 => 1 << 29,
        1 => 1 << 30,
        _ => 0,
    };
    let mut flags2 = (particle.is_texture_common_random != 0) as u32;
    flags2 |= match d.emitter_info.follow_type {
        0 => 1 << 9,
        1 => 1 << 11,
        2 => 1 << 10,
        _ => 0,
    };
    // Bits 1..8: which field subsections the emitter has, in the order of the runtime's field table. (FCOL, bit 6, is
    // in no captured emitter.)
    for (at, tag) in [(1, "FRND"), (2, "FPAD"), (3, "FMAG"), (4, "FCOV"), (5, "FSPN"), (6, "FCOL"), (7, "FCLN"), (8, "FRN1")] {
        flags2 |= bit(emitter.subsections.iter().any(|x| x.magic == tag), at);
    }
    (s.flags1, s.flags2) = (flags1, flags2);
}

/// Keys past the table's count (taken as 1..=8) repeat the last one, its time plus the key's index.
fn pad(table: &mut AnimationKeyTable, count: u32) {
    let n = (count as usize).clamp(1, 8);
    let last = table.keys[n - 1];
    for (k, key) in table.keys.iter_mut().enumerate().skip(n) {
        *key = last;
        key.time = last.time + k as f32;
    }
}

/// One texture animation's switches. An emitter without the block has them all off.
#[derive(Default)]
struct AnimSettings {
    pattern_type: u8,
    scroll: bool,
    rotate: bool,
    scale: bool,
    repeat: u8,
    inv_rand_u: bool,
    inv_rand_v: bool,
    loop_random: bool,
}

impl AnimSettings {
    fn of(anim: &Option<effect_library::TextureAnim>) -> Self {
        anim.as_ref().map_or_else(Self::default, |a| Self {
            pattern_type: a.pattern_anim_type,
            scroll: a.is_scroll,
            rotate: a.is_rotate,
            scale: a.is_scale,
            repeat: a.repeat,
            inv_rand_u: a.inv_rand_u != 0,
            inv_rand_v: a.inv_rand_v != 0,
            loop_random: a.is_pat_anim_loop_random != 0,
        })
    }
}
