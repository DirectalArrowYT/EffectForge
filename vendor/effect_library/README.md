# effect_library

Rust library and CLI for Nintendo Switch VFX effect files (`.eff`, `.ptcl`). Decompiles archives into editable JSON/text assets and re-encodes them with byte-for-byte parity against the reference C# exporter.

**Crates.io:** [`effect_library`](https://crates.io/crates/effect_library) **`1.1.4`**

## Build

From the repo root or from this directory:

```bash
cargo build --release --bin effect_converter
```

## CLI usage

```bash
effect_converter dump /path/to/ef_mario.eff /path/to/output
effect_converter build /path/to/output/ef_mario /path/to/ef_mario_NEW.eff
```

Install the crate from crates.io (binary name: `effect_converter`):

```bash
cargo install effect_library
```

## Library API

```toml
[dependencies]
effect_library = "1.1.4"
```

```rust
use effect_library::{Creator, Dumper, NamcoEffectFile, PtclFile};
use std::fs;

// Load and dump
let namco = NamcoEffectFile::load(&fs::read("ef_mario.eff")?)?;
Dumper::dump_namco(&namco, "output/ef_mario")?;

// Rebuild
let rebuilt = Creator::create_namco_from_folder("output/ef_mario")?
    .expect("effect has Base.ptcl");
fs::write("ef_mario_NEW.eff", rebuilt.save()?)?;

// PTCL only
let ptcl = PtclFile::load(&fs::read("Base.ptcl")?)?;
let bytes = ptcl.save();
```

Submodules `bfres`, `bntx`, and `bnsh` expose load/save helpers for embedded asset pools — including `bfres::export_single_model_with_session` (one parse reused across many exports), `bfres::merge_model_files`, and `bntx::build_single_texture_bntx_public`.

## What's new in 1.1.4

Empty emitter sets keep their `NONE` child offset when saved. The game's ESET reader can
otherwise follow that offset into the next set and inherit emitters that do not belong there.

## What's new in 1.1.3

1.1.3 is a maintenance release for current Rust toolchains. It removes strict
Clippy warnings and modernizes equivalent standard-library operations without
changing the file formats, public API, or intended output.

## What's new in 1.1.1

BFRES and BNSH now write relocation tables the game's own loader accepts, not merely tables that parse. Measured against 296 game BFRES pools, the container body is byte-exact for all 296 and 293 relocate the identical pointer slots, so exported and merged models keep rendering in-game.

Dumping and rebuilding also preserves more of the emitter: `DepthMode`, `PassInfo`, `UnknownV36` and `Namev40` now reach the JSON, and the combiner is reconstructed from the file's vfx version instead of being guessed from the JSON. Both affect version 36 and above; a version 22 dump is unchanged.

See the [repository README](../README.md#whats-new-in-111) for the full list.

## Verification

See the [repository README](../README.md#verification) for comparison scripts. Run from this directory:

```bash
python3 scripts/batch_eff_roundtrip.py
python3 scripts/speedtest_roundtrip.py --csharp
```

Requires local `.eff` files (see `scripts/compare_setup.py`).

Writer fidelity is checked against real containers rather than against itself:

```bash
EFFECT_LIBRARY_EFF_CORPUS=/path/to/export/effect cargo test --test bfres_corpus_fidelity
```

The tests skip without that variable.

## Credits

- [EffectLibrary](https://github.com/KillzXGaming/EffectLibrary)
- [Joob's EffectLibrary fork](https://github.com/joobert/EffectLibrary)
- [eff_lib](https://github.com/ultimate-research/eff_lib/tree/main)
