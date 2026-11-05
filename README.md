# Effect Forge

Make and edit Smash Ultimate effects without touching scripts.

    target/release/effect_forge.exe [file.eff] [EFFECT_NAME]

**File > Open…** an `.eff`, pick an effect and a layer, change values (they apply as you let go), then **Save to
mod** on the top bar. The first save keeps a `.forge-backup` beside the file.

**Options** has the theme (Dark, or Ember: purple with orange) and **Auto save**, on by default, which saves a few
seconds after the last change. Outside a project that writes the opened `.eff` itself. Both are kept in
`forge_data/settings.json`.

Saved edits live in the `.eff` only: running `bake_slotted.py` rebuilds that file and loses them, unless the effect is
kept as tuned.

## Making an effect

Type a name under the search box and pick **New effect**: the menu is a starter kit of Smash's own layers (sparks,
flashes, rings, fire, smoke, trails, beams, slashes, bits; `src/kit.json`). The layer is copied in with its texture,
model and shader, so it draws in game exactly as it does in Smash. **Add layer** beside the layer list adds more from
the kit (a name the effect already has gets `_2`), **Remove layer** takes the selected one out with any under it. Then
shape each layer with the panel and **Save to mod**.

The kit is read from the game's own effect files: the first time, the menu asks for the dumped game folder (later: **File > Game files folder…**) that holds
the `effect` folder (or set `FORGE_GAME_ROOT`). Only what the chosen layers use comes along.

A new effect is in the saved file only, so a rebuild of a generated file loses it: **Keep my tuning** on it keeps it
over rebuilds like any tuned effect. Copied into a fighter's file (one under `effect/fighter/`), an effect gets the
fighter type its spawns need. The move's script then spawns it by name; Forge does not write scripts.

## Projects

**File > New project…** starts a project from the open file: `<name>.forge` (the folders and where it exports) and
`<name>.project.eff` beside it, a copy of the file holding every effect. Work in the project and **Save project**; the
file in the mod is untouched until **Export**, which writes it to the file the project came from, or **File > Export to…**
anywhere (the project remembers it). The first export keeps a `.forge-backup` of what was there. **Open** takes a
`.forge` too.

In a project the effect list is sorted into folders: name one and **New folder**, then drag effects onto a folder (or
right-click one: **Move to**). Ctrl-click picks more than one and shift-click picks a run of them; dragging or moving
any picked effect takes them all. Right-click a folder to rename or remove it; removing a folder keeps its effects. Tick a
folder to skip it on export: the tick adds `UNUSED_` to its name, and every folder named `UNUSED_...` is left out. A
skipped effect keeps its name in the exported file but plays nothing, and whatever only it used is dropped.

**Rename** beside the effect's name renames it (letters, digits and `_`). Scripts that play it need the new name.

## Keeping tuned effects

**Keep my tuning** copies the effect, as it is now, into the repo's tuned file:
`<repo>/effects_tuned/<name>_tuned.eff`, listed in `<name>_tuned.json` (for a mod's `ef_pit_c01.eff`, `effects_tuned/ef_pit_c01_tuned.eff`).
Tuned effects are starred in the list. `bake_slotted.py` puts every listed effect back over the generated one after
each rebuild (`effect_forge merge-tuned <tuned.eff> <built.eff>`). Open the tuned file to work on them there; its
**Put into …** button writes them into the mod file without a rebuild.

## Trails

Smash builds trails (Roy's and Ike's fire) from distance emission plus particles that stay where they were born. The
**Trail** section turns both on for a layer. A trail only draws when the move's script attaches the effect to a bone
that moves (`EFFECT_FOLLOW` on a hand). Tick **Swing** under the preview to move the effect round and see it.

## Seeing a layer's model

Under the preview, **Outline** draws a yellow line round the selected layer. **Ghost** draws the selected layer's
model over everything as a flat see-through shape, whatever its texture, so you can see where the mesh actually
sits; the slider beside it sets how solid it is. Both are preview only and never reach the file.

Build: `cargo build --release`. Everything it needs is in this folder: the game's effect shaders are translated to
WGSL in-process (`src/draw/mx`), with no Python, recomp or glslang.
`src/eff/` (the file), `src/draw/` (the preview's game shaders), `scratch_dirs.rs` and `vendor/effect_library` started
as copies of Visionary's. They are Forge's own now: Visionary's tests, its capture reader and everything Forge never
calls were removed on 2026-11-04 (about 10,700 lines), and then the engine (`src/eff`, `src/draw`, the shader
translator in `src/draw/mx` included) was rewritten module by module, each output checked byte for byte against the
code it replaced (`cargo test --release` pins them).

## Shaders decide some looks

Each layer draws with a shader compiled for it, and some fields are compiled in: the billboard type (stretching along
motion is type 5) and the opacity type (fixed, or keyed over life). Changing those values alone does nothing on screen
or in game. So the panel borrows a shader: it finds a layer in the same file with the same texture setup and blend that
already draws that way, and points this layer at its shader (`shader_references.shader_index`). The panel says whose
shader a layer uses, with **Use its own** to undo it. The fade presets, Taper, **Stretch along motion** and
**Behave like...** all borrow when they need to. **Behave like...** copies how one of Smash's own layers moves, lives
and fades (`src/app/recipes.json`).

`effect_forge stage <base eff> <arc root> <builds dir> <manifest> <out eff>` is the whole-file bake.
`effect_forge check <eff> <png> <EFFECT>` runs every kind of edit on a copy of a file (the effect's first layer needs a model).

## Logs and checks

- `cargo test --release`: the set and borrow edits must give the same bytes as the Forge of before the 2026-11-04
  cleanup, every other value must survive a rebuild, the particles spawned for all of ef_common are pinned by a hash, every
  shader of ef_pit must translate, recipes may only name real fields, and every piece of the kit must make an effect. They read the extracted arc
  (`FORGE_TEST_PIT`, `FORGE_TEST_COMMON`).

- Crashes, GPU errors and the effect on screen go to `%APPDATA%/EffectForge/forge.log`.
- `effect_forge sweep <file.eff>` draws every effect early, midway and late, makes one edit, and logs anything that fails.
- `effect_forge shot <file.eff> <EFFECT> <out.png> <frame> [layer number, o<number> to outline it, g<number> to ghost its model, or -] [camera turn] [camera tilt] [half view] [swing]` saves the window's own frame and closes.
- `effect_forge attrs <file.eff> <id,id,...> [id=value]` prints values across a whole file, to see how the game's own effects use a field.
- `effect_forge dump <file.eff> <out.jsonl>` writes every layer's values, one JSON line per layer.
- `effect_forge borrow <file.eff> <out.eff> <EFFECT> <field> <value>` borrows a shader for every layer of an effect, as the panel does, and prints whose.
- `effect_forge stage-effect <file.eff> <EFFECT>` is the Keep my tuning button.
- `effect_forge kit` lists the starter kit; `new-effect <file.eff> <out.eff> <NAME> <piece>`, `add-layer <file.eff>
  <out.eff> <EFFECT> <piece>` and `remove-layer <file.eff> <out.eff> <EFFECT> <layer number>` are its buttons.
- `effect_forge rename <file.eff> <out.eff> <OLD> <NEW>` is the Rename button. `project <p.forge> <from.eff> [export
  to]` starts a project, `folder <p.forge> <folder> <EFFECT...>` puts effects in a folder, and `export <p.forge> [to]`
  exports as the window does.
- `effect_forge layers <file.eff> [EFFECT]` lists each effect's type and layers, with their textures and models.
