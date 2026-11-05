//! The translator for the game's effect shaders: Maxwell machine code in, WGSL out.
//!
//! `decode` reads the machine words into `isa` instructions, `flow` works out the jumps, `emit` writes each
//! instruction as WGSL statements and `wgsl` puts them in a module with its bindings and entry point.

pub mod decode;
mod emit;
mod flow;
pub mod isa;
pub mod wgsl;
