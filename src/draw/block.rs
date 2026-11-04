//! The emitter block: the 0x750-byte constant buffer every effect shader reads as c9.
//!
//! It is the emitter's stored data from byte 0 (header 0x50, name at +0x10, then `emitter_static`), with the fields the
//! game's runtime fills in when it loads the emitter. Built here from the .eff alone, it matched every block
//! the game bound in the recomp captures of October 2026 (918 of them), byte for byte.

pub const BLOCK_LEN: usize = 0x750;

/// The six animation tables: (offset of the key count, offset of key 0). Eight keys of (x, y, z, time) each.
const TABLES: [(usize, usize); 6] = [
    (0x60, 0x3c0), // colour 0
    (0x64, 0x440), // alpha 0
    (0x68, 0x4c0), // colour 1
    (0x6c, 0x540), // alpha 1
    (0x70, 0x600), // scale
    (0x74, 0x680), // param
];
/// A fixed (type 0) colour / alpha's constant, particle_color.color0 / alpha0 / color1 / alpha1: (offset, components).
const CONSTANTS: [Option<(usize, usize)>; 6] = [
    Some((0x9a0, 3)), // colour 0 rgb
    Some((0x9ac, 1)), // alpha 0
    Some((0x9b0, 3)), // colour 1 rgb
    Some((0x9bc, 1)), // alpha 1
    None,
    None,
];
/// particle_color.{color0, alpha0, color1, alpha1}_type for the first four tables: 0 fixed, 1 random, 2 animated.
const COLOUR_TYPE: [usize; 4] = [0x99c, 0x99e, 0x99d, 0x99f];

/// tex_scroll_anim0..2.uv_scale_x / _y.
const UV_SCALE: [usize; 3] = [0x300, 0x350, 0x3a0];

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn put_f32(b: &mut [u8], at: usize, v: f32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// The emitter's c9 block, from its stored data. `None` when the emitter has no stored binary.
pub fn emitter_block(emitter: &effect_library::Emitter) -> Option<Vec<u8>> {
    let stored = emitter.binary_data.as_ref().or(emitter.cached_binary.as_ref())?;
    let mut b = stored.get(..BLOCK_LEN)?.to_vec();

    // Animation tables. The keys past the count repeat the last key, at its time plus the slot index. A fixed colour
    // (type 0) is its constant in key 0 and zeros; a fixed alpha is its constant in key 0's x, padded the same way.
    for (t, &(count_at, keys_at)) in TABLES.iter().enumerate() {
        let fixed = COLOUR_TYPE.get(t).is_some_and(|&at| stored.get(at) == Some(&0));
        let n = (u32::from_le_bytes(b[count_at..count_at + 4].try_into().unwrap()) as usize).clamp(1, 8);
        if fixed {
            let (from, comps) = CONSTANTS[t].unwrap();
            if comps == 3 {
                b[keys_at..keys_at + 0x80].fill(0);
                for c in 0..3 {
                    put_f32(&mut b, keys_at + 4 * c, f32_at(stored, from + 4 * c));
                }
                continue;
            }
            put_f32(&mut b, keys_at, f32_at(stored, from));
        }
        let last = keys_at + 16 * (n - 1);
        let (xyz, time) = (b[last..last + 12].to_vec(), f32_at(&b, last + 12));
        for k in n..8 {
            let at = keys_at + 16 * k;
            b[at..at + 12].copy_from_slice(&xyz);
            put_f32(&mut b, at + 12, time + k as f32);
        }
    }

    // Colour / alpha / scale loops: the loop rate (an int in particle_data) as a float where the loop is on, and 1.0
    // where its start is random.
    for i in 0..5 {
        if stored[0x8d8 + i] != 0 {
            let rate = i32::from_le_bytes(stored[0x8e4 + 4 * i..0x8e8 + 4 * i].try_into().unwrap());
            put_f32(&mut b, 0x80 + 4 * i, rate as f32);
        }
        if stored[0x8dd + i] != 0 {
            put_f32(&mut b, 0x94 + 4 * i, 1.0);
        }
    }

    // Particle rotation: an axis that does not rotate (particle_data.is_rotate_x/y/z) has its initial value, random
    // range, speed and speed range zeroed.
    for axis in 0..3 {
        if stored[0x8b0 + axis] == 0 {
            for base in [0x700, 0x710, 0x720, 0x730] {
                put_f32(&mut b, base + 4 * axis, 0.0);
            }
        }
    }

    // Texture animations 0..2 (tex_scroll_anim at 0x2c0 + 0x50 n, tex_pattern_anim at 0x110 + 0x90 n, settings in
    // texture_anim at 0xa50 + 0x10 n: pattern type, is_scroll, is_rotate, is_scale, repeat). UV scale: 2 on an axis
    // that repeats (repeat bit 0 = u, bit 1 = v), else 1. A scroll, rotation or scale the animation does not do is
    // zeroed (its scale set to 1).
    for n in 0..3 {
        let settings = 0xa50 + 0x10 * n;
        let scroll = 0x2c0 + 0x50 * n;
        let repeat = stored[settings + 4];
        put_f32(&mut b, UV_SCALE[n], if repeat & 1 != 0 { 2.0 } else { 1.0 });
        put_f32(&mut b, UV_SCALE[n] + 4, if repeat & 2 != 0 { 2.0 } else { 1.0 });
        if stored[settings + 1] == 0 {
            for at in [0x00, 0x04, 0x08, 0x0c, 0x10, 0x14] {
                put_f32(&mut b, scroll + at, 0.0);
            }
        }
        if stored[settings + 2] == 0 {
            for at in [0x30, 0x34, 0x38] {
                put_f32(&mut b, scroll + at, 0.0);
            }
        }
        if stored[settings + 3] == 0 {
            for (at, v) in [(0x18, 0.0), (0x1c, 0.0), (0x20, 1.0), (0x24, 1.0), (0x28, 0.0), (0x2c, 0.0)] {
                put_f32(&mut b, scroll + at, v);
            }
        }
        // A type 4 pattern animation plays num_random patterns, the table's first slots counting 0, 1, 2, ...
        let pattern = 0x110 + 0x90 * n;
        if stored[settings] == 4 {
            let count = f32_at(stored, pattern + 8);
            put_f32(&mut b, pattern, count);
            for i in 0..(count.max(0.0) as usize).min(32) {
                let at = pattern + 0x10 + 4 * i;
                b[at..at + 4].copy_from_slice(&(i as u32).to_le_bytes());
            }
        }
    }
    // flags1 / flags2: switches the runtime derives from the emitter's settings for the shaders.
    let mut flags1: u32 = 0;
    flags1 |= (stored[0x9e7] as u32 >> 3) & 7; // particle_fluctuation wave type 8 / 16 / 32
    flags1 |= ((stored[0x7f1] != 0) as u32) << 3; // emission.is_world_gravity
    for n in 0..3 {
        let settings = 0xa50 + 0x10 * n;
        let pattern_type = stored[settings] as u32;
        if (1..=4).contains(&pattern_type) {
            flags1 |= 1 << (4 + 4 * n as u32 + pattern_type - 1);
        }
        flags1 |= ((stored[settings + 5] != 0) as u32) << (19 + 2 * n); // inv_rand_u
        flags1 |= ((stored[settings + 6] != 0) as u32) << (20 + 2 * n); // inv_rand_v
        flags1 |= ((stored[settings + 7] != 0) as u32) << (25 + n); // is_pat_anim_loop_random
    }
    for axis in 0..3 {
        flags1 |= ((stored[0x8ad + axis] != 0) as u32) << (16 + axis); // particle_data.rot_rev_rand_x/y/z
    }
    match stored[0x8ac] {
        0 => flags1 |= 1 << 29, // particle_data.offset_type
        1 => flags1 |= 1 << 30,
        _ => {}
    }
    let mut flags2: u32 = (stored[0x8b4] != 0) as u32; // particle_data.is_texture_common_random
    match stored[0x753] {
        0 => flags2 |= 1 << 9, // emitter_info.follow_type
        1 => flags2 |= 1 << 11,
        2 => flags2 |= 1 << 10,
        _ => {}
    }
    // flags2 bits 1..8: the emitter's field subsections (bit 6, FCOL by the order of the runtime's field table, is not
    // in any captured emitter).
    for (bit, tag) in [(1, "FRND"), (2, "FPAD"), (3, "FMAG"), (4, "FCOV"), (5, "FSPN"), (6, "FCOL"), (7, "FCLN"), (8, "FRN1")] {
        if emitter.subsections.iter().any(|x| x.magic == tag) {
            flags2 |= 1 << bit;
        }
    }
    b[0x50..0x54].copy_from_slice(&flags1.to_le_bytes());
    b[0x54..0x58].copy_from_slice(&flags2.to_le_bytes());
    Some(b)
}
