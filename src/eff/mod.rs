//! The `.eff` file: reading it (`model`), every layer value by name (`fields`), writing it back (`writer`), the edit
//! records the writer applies (`edits`), its models (`mesh_io`) and its textures (`textures`).

pub mod edits;
pub mod fields;
pub mod mesh_io;
pub mod model;
pub mod textures;
pub mod writer;
