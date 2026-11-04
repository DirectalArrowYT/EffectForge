//! The translator for the game's effect shaders: Maxwell machine code in, WGSL out. Brought over from Visionary's
//! src/mx on 2026-11-04 (its tests, over all 17,885 effect shader stages in the game, stay there).

#[allow(dead_code)]
pub mod decode;
#[allow(dead_code)]
pub mod wgsl;
