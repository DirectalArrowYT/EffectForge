//! Drawing an effect in the preview with the game's own shaders: the particles over time (`sim`), the per-layer
//! constant block (`block`), the shader inputs built from the file (`inputs`), the translated shaders (`shaders`) and
//! the GPU side (`gpu`).

pub mod block;
pub mod gpu;
pub mod inputs;
pub mod mx;
pub mod shaders;
pub mod sim;
