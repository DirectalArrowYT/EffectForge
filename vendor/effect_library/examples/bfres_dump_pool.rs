//! Write an `.eff`'s BFRES pool and our re-save of it to disk for byte-level inspection.
//!
//! Usage: cargo run --release --example bfres_dump_pool -- <eff> <out-prefix>

use effect_library::bfres::{export_single_model, ResFile};
use effect_library::NamcoEffectFile;

fn main() {
    let mut args = std::env::args().skip(1);
    let eff = args.next().expect("usage: <eff> <out-prefix>");
    let prefix = args.next().expect("usage: <eff> <out-prefix>");

    let bytes = std::fs::read(&eff).expect("read eff");
    let file = NamcoEffectFile::load(&bytes).expect("parse eff");
    let prim = file
        .ptcl_file
        .as_ref()
        .and_then(|p| p.primitive_info.as_ref())
        .expect("eff has primitives");
    let pool = prim.binary_data.clone().expect("eff has a BFRES pool");
    let ours = ResFile::canonicalize(&pool).expect("re-save");

    // Also the split-and-merge path, which is what the app actually performs.
    let blobs: Vec<Vec<u8>> = (0..prim.descriptors.len())
        .map(|i| export_single_model(&pool, i).expect("extract model"))
        .collect();
    let merged = ResFile::merge_model_files(&blobs).expect("merge");

    std::fs::write(format!("{prefix}.orig.bfres"), &pool).expect("write orig");
    std::fs::write(format!("{prefix}.ours.bfres"), &ours).expect("write ours");
    std::fs::write(format!("{prefix}.merged.bfres"), &merged).expect("write merged");
    println!(
        "orig {} B -> resave {} B -> merged {} B ({} models)",
        pool.len(),
        ours.len(),
        merged.len(),
        prim.descriptors.len()
    );
}
