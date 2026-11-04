# Smash effect runtime (nw::eft2) — reverse-engineering notes

Findings from Ghidra on `main` from Smash **13.0.1**, the version the mods run against. All
addresses are image addresses in the flat image produced by `tools/nso_to_bin.py`, which loads
at base 0, so a Ghidra address is also a skyline offset.

## Getting the binary

1. `exefs/main` (42.9 MB NSO) of the 13.0.1 update, from your own dump of the game.
2. `python tools/nso_to_bin.py main smash_main.bin` → 122 MB flat image (text at 0, rodata at
   0x39c8000, data at 0x4f16000, then bss). Contains the string `13.0.1`.
3. `python tools/nso_symbols.py main smash_main.bin --csv main_symbols.csv` → 4,918 dynamic
   symbols, 4,888 of them functions. Nearly all are `app::lua_bind` thunks; the effect runtime
   itself is static and unnamed.
4. Ghidra headless import (raw binary, AARCH64:LE:64:v8A, base 0), `ApplySmashSymbols.py` as a
   pre-script, analysis with **Create Address Tables, Aggressive Instruction Finder and
   Decompiler Parameter ID disabled** — 15 minutes. With them on it ran 11 hours in the first
   analyzer alone and never finished.

## Where the code lives

The effect runtime sits roughly in `0x60000..0xc0000` (561 functions).

| what | address |
| --- | --- |
| resource loader (sizes the GPU memory) | `FUN_0009af30` |
| emitter parser (walks `EMTR`, builds runtime structs) | inside the loader, from `0x9afa0` |
| per-emitter sub-block dispatcher | `FUN_0008cae0` |
| emitter instance init | `FUN_000872a0` |
| animation driver (runs the per-life blocks) | `FUN_00095370` |
| colour animation eval (`FCOL`) | `FUN_000948f0` |
| `FCOV` / `FSPN` / `FPAD` / `FCLN` evals | `FUN_00094c30` / `FUN_000944b0` / `FUN_000950c0` / `FUN_00098450` |
| constant-buffer binding, graphics stages | `FUN_0009f710` |
| constant-buffer binding, compute stage | `FUN_0009f870` |
| set-constant-buffer helper (slot, stage, gpu address, size) | `FUN_00033430` |
| shader slot table reset (fills with -1) | `FUN_000b8e60` |

`app::lua_bind::EffectModule__*_impl` (92 symbols, from `0x02017000`) are one-line wrappers
that dispatch through the module vtable at `boma + 0x140` — `req` is slot `+0x68`, `kill_kind`
is `+0xe0`. They lead to the game's effect module, not to this runtime.

## Runtime emitter struct — 0x3E0 bytes (992)

The loader counts `EMTR` blocks, allocates `count * 0x3E0 + 0x20`, constructs each with
`FUN_0008a5a0`, and links them through `+0x110` (next). `+0x08` is the index, `+0x10` points at
the emitter's resource block, `+0x18` at the emitter data (`resource + 0x50`).

`FUN_0008cae0` walks the emitter's tagged sub-blocks and stores each pointer:

| tag | offset | tag | offset | tag | offset |
| --- | --- | --- | --- | --- | --- |
| `FRND` | `+0x248` | `EAES` | `+0x290` | `EASL` | `+0x2e8` |
| `FRN1` | `+0x250` | `EAER` | `+0x298` | `EASS` | `+0x2f0` |
| `FSPN` | `+0x260` | `EAET` | `+0x2a0` | `CSDP` | `+0x300` |
| `FCOL` | `+0x268` | `EATR` | `+0x2b8` | `CADP` | `+0x310` |
| `FCOV` | `+0x270` | `EAPL` | `+0x2c0` | `CUDP` | `+0x318` |
| `FPAD` | `+0x278` | `EAA0` | `+0x2c8` | `EP01`…`EP04` | `+0x338` |
| `FCLN` | `+0x280` | `EAA1` | `+0x2d0` | (their type, 1–4) | `+0x330` |
| `FCSF` | `+0x288` | `EADV` | `+0x2e0` | | |

`FRN1` and `FPAD` also set a flag at `+0x03`, which controls whether a second GPU block is
allocated. `EP01`–`EP04` are mutually exclusive: one pointer slot, with the variant number
beside it. Anything not in this table is skipped by the engine.

## GPU buffers per emitter

The loader sizes them (aligned up to the pool's alignment):

- **`0x750` (1872 bytes)** always — the emitter's constant buffer, bound from the pool at
  emitter `+0x20` with the offset at `+0x28`.
- **child block** when `CSDP` is present, sized by the value at `+0x308`.
- **`0x120` (288 bytes)** when the `+0x03` flag is set (`FRN1`/`FPAD`), bound from offset `+0x40`.

Both binding functions bind the same three things: a `0xc0` view/camera buffer, the emitter's
`0x750` buffer, and (compute path) the `0x120` buffer, plus structured buffers of particle
attributes with a `0x10` stride. Slot indices come from a per-shader table; `-1` means the
shader does not take that buffer.

Note the file's own static region is larger than `0x750` — 0xa70 in a sampled emitter — so the
engine **builds** the constant buffer rather than uploading the file's bytes verbatim.

## Naming an offset

`tools/emitter_field_offsets.csv` has the byte offset of all 750 emitter attributes inside an
emitter data block, so any offset the engine reads can be named. It comes from Visionary's
`where_each_attribute_lives` test (`src/eff_export.rs`): set one attribute, re-serialise that
emitter alone, diff the bytes. Re-run it with `VISIONARY_ATTRS="*"` to regenerate.

Two details that matter when reading the output: diffing the whole rebuilt file is useless
(every byte moves), and a single probe value can leave bytes untouched -- 1.0 -> 0.25 only
alters a float's top byte -- so it probes each attribute twice and unions what moves.

### Smash's emitter copy loop, named

`FUN_00087530` copies from the emitter data (the runtime struct's `+0x10`) into the live
emitter. All 18 of its reads resolve, and they come in coherent groups -- each `_z` is the last
of an xyz triple being copied as a vector:

| instance | file | attribute |
| --- | --- | --- |
| `+0x620` | `+0x778` | `emitter_info.trans_z` |
| `+0x614` | `+0x790` | `emitter_info.rotate_z` |
| `+0x608` | `+0x7a8` | `emitter_info.scale_z` |
| `+0x624`/`+0x628`/`+0x62c` | `+0x7ac`/`+0x7b0`/`+0x7b4` | `emitter_info.color0_r/g/b` |
| `+0x654` | `+0x7b8` | `emitter_info.color0_a` |
| `+0x630`/`+0x634`/`+0x638` | `+0x7bc`/`+0x7c0`/`+0x7c4` | `emitter_info.color1_r/g/b` |
| `+0x660` | `+0x7c8` | `emitter_info.color1_a` |
| `+0x63c` | `+0x800` | `emission.rate` |
| `+0x69c` | `+0x814` | `emission.gravity_scale` |
| `+0x698` | `+0x870` | `shape_info.volume_form_scale_z` |
| `+0x648` | `+0x8b8` | `particle_data.life` (int, converted to float) |
| `+0x66c` | `+0x964` | `particle_velocity.all_direction` |
| `+0x678` | `+0x968` | `particle_velocity.designated_dir_scale` |
| `+0x68c` | `+0x9c8` | `particle_scale.scale_z` |

Alpha is kept away from its own rgb (`color0_a` at `+0x654`, rgb at `+0x624`), so colour and
alpha stay separate paths all the way to the shader.

## What the 0x750 buffer holds

Attribute groups sit in the file like this: `emitter_static` runs `+0x50..~+0x750` (428 of the
750 attributes), and the next group, `emitter_info`, starts at `+0x751`. So **the emitter data's
first 0x750 bytes are its 0x50-byte header plus the whole `emitter_static` struct** -- exactly
the size of the constant buffer, and the reason those attributes are named "static" at all.

Supporting evidence: nothing in the effect runtime reads `emitter_static.color_scale` (`+0x3b3`)
or the combiner fields (`color_combiner_process` `+0x8f8`, `tex_alpha0_input_type` `+0x903`,
`blend_type` `+0x89e`) on the CPU at all -- consistent with the static half being handed to the
GPU wholesale and the rest driving pipeline state. `shader_references.shader_index` (`+0x914`)
IS read, by `FUN_0008b350`, which is the shader selection.

The boundary is exact, and the layout is fixed: probing emitters from three unrelated effects
(SYS_HIT_NORMAL/flare1_bk, SYS_TURN_SMOKE/impact1_b, SYS_QUAKE/ring1) gives identical offsets
for every field. So `emitter_static` is a fixed 0x700-byte struct at `+0x50`, the header is the
0x50 before it, and together they are precisely the 0x750 the engine binds.

**What this means in practice: a uniform offset in the emitter's constant buffer IS the byte
offset in the file.** Editing `emitter_static.*` edits the GPU constant buffer directly, which
is why those attributes are the ones whose changes show up without the engine touching them --
and why Visionary's preview can feed its renderer the same bytes the game feeds the shader.

Still inference rather than a read of the copy instruction, but four things agree: the sizes
match exactly, the group boundary lands exactly on 0x750, no CPU code reads any field inside
that window, and the runtime keeps pointers to both the data start (`+0x10`) and data+0x50
(`+0x18`) -- the buffer and the struct inside it.

## Open question

Which bytes of that `0x750` block correspond to which file fields. That is the mapping that
would let Visionary feed its preview exactly what the game feeds the shader. The fill has not
been found: the loader only *sizes* the memory, and the assignment of `+0x28`/`+0x40` happens
somewhere else. Tried and ruled out: the loader's second pass (sizes child emitters the same
way), the emitter constructor `FUN_0008a5a0` (zeroes those fields), the emitter-set pass
`FUN_0009a840` and its caller, every function in `0x60000..0xc0000` that writes both `+0x20`
and `+0x28` (including as `[4]`/`[5]` array indices), and any 0x750-sized memcpy or 0x75-count
copy loop -- none exists in that range. The likely remaining candidates are a per-frame fill in
the emitter update (`FUN_0008d770`, `FUN_0008e590`, `FUN_0008fbe0`) or code outside the range.

The file's own `shader_cbuf` section would describe the buffer layout directly, but
effect_library stubs it out (`ShaderCbuf::read_list` returns an empty list), so reading it
means parsing that section first.

Worth knowing for context: each `.eff` embeds its own compiled shaders (`ShaderInfo` in the
ptcl file), so what `color_combiner_process` and `tex_alpha0_input_type` actually *compute*
lives in those shader binaries, not in `main`. `main` only packs the values and binds them.
