# Porting an Unreal particle system

How the parts of an Unreal (Cascade, and mostly Niagara too) emitter map onto a Forge layer. Every id here works in the
panel's **All values**, in `effect_forge value <eff> <out.eff> <EFFECT> <layer> <id=value>...`, and in a batch file.
The panel's own names (`size`, `colour`, `opacity`, `glow`, `turn-z`, ...) go through
`effect_forge set <eff> <out.eff> <EFFECT> [layer] <name=value>...`, which lists them when given none.

## Start from the right layer

Don't build each emitter from a blank one. Pick a kit layer that already draws the way the emitter does (the shader is
compiled in, see the README's "Shaders decide some looks"), then change its numbers:

| Unreal emitter | Kit layer to start from |
|---|---|
| Sprite, still texture, facing the camera | a spark or flash |
| Sprite with an animated SubUV sheet | fire or smoke |
| Sprite with a random SubUV cell per particle | smoke |
| Velocity-aligned sprite (`PSA_Velocity`) | a speed line, or a hit spark if it is also a sheet |
| Mesh facing the camera | a flash drawn on a mesh, or **Blank model** with billboard 0 (the model's front, +Z, turns to the camera) |
| Mesh in the world's orientation | wind or a ring |
| Mesh that turns with its bone | a ring or impact that follows its bone |
| Ribbon or anim trail | **Trail** on a sprite layer (the README's "Trails") |

**Behave like...** then takes motion, life and fade from a game layer in one go, and you can change any of them after.

## Units

- **Distance:** Unreal centimetres × 0.1 gives Smash units. A 100 cm sphere is radius 10.
- **Time:** seconds × 60 gives frames.
- **Speed:** cm/s × 0.1 / 60 gives units a frame.
- **Acceleration:** cm/s² × 0.1 / 3600 gives units a frame².
- **Turns:** Unreal rotation modules count whole turns (1 = 360°) and turns a second. Forge's `rotate_*` ids take
  radians and radians a frame. The panel's `turn-*` and `spin-*` take degrees and degrees a frame.
- **Axes:** Unreal is X forward, Y right, Z up. Smash is X forward, Y up, Z sideways. Swap Y and Z on every vector.
  A turn about Unreal's X, Y or Z is a turn about Smash's X, Z or Y, and swapping two axes mirrors the sense of the X
  and Z turns: negate them.
- **Size:** the game's largest effects sit near 60 units. An emitter sized in the thousands of centimetres is meant
  to fill the screen, and in Smash it should be capped.

## Module by module

| Unreal module | Forge id(s) | Notes |
|---|---|---|
| Required: duration, loops, delay | `emission.start`, `emission.duration`, `emission.is_one_time` | Loops 1 → one time. Loops 0 → `is_one_time` 0 (runs until the script kills it). |
| Spawn rate | `emission.rate` (a frame), `emission.rate_random` | Rate a second ÷ 60. Keep it to about 2 a frame per layer; more runs the pool dry with nothing to show for it. |
| Spawn burst | `emission.duration` 1, `emission.rate` = count | Up to about 30. A burst that fires every cycle of a looping emitter is `emission.interval` = cycle frames − 1. |
| Lifetime | `particle_data.life`, `particle_data.life_random` | Random is percent either side of the mean. A lifetime of 0 in Cascade means "as long as the emitter runs", not one frame. |
| Initial size | `particle_scale.scale_x/y/z`, `scale_random_*` | Mean × 0.1. A velocity-aligned sprite's Y is its length. |
| Size by life / size scale | **Over their life**: `emitter_static.scale_anim.keys[i].x/y/z/time`, `num_scale_keys` | At most 8 keys. Time runs 0 to 1 over the life. |
| Initial colour, colour over life | `particle_color.color0_*`, `emitter_static.color0.keys[i]`, `num_color0_keys` | Set `particle_color.color0_type` 2 even for one key. Some shaders read every key, and a lone fixed colour draws black on them. |
| Colour scale over life | multiply it into the colour keys | Unreal's HDR colours (up to 30 or more) clip to the same white in Smash. Keep keys near 1 and use `emitter_static.color_scale` (Brightness) a little over 1 for the glow. |
| Alpha over life | `particle_color.alpha0_type` 2, `emitter_static.alpha0.keys[i]`, `num_alpha0_keys` | `alpha0_type` is compiled into the shader. Setting it in the panel borrows one that reads it. |
| Material blend | `render_state.blend_type` (Normal / Glow (adds) / Subtract) | Additive → Glow. Translucent → Normal. |
| Local space on / off | `emitter_info.follow_type` | Local space rides the bone. World space leaves particles behind as the bone moves; for a real trail use **Trail**. |
| Sphere / cylinder location | `shape_info.volume_type`, `volume_radius_*` | Sphere filled or surface only. **When it emits** names every shape. |
| Disc / ring location (a sphere or cylinder with one axis off) | `volume_type` Circle (the rim) or Filled circle, `volume_radius_*` | Flat in Smash's X/Z. Radius × 0.1. |
| Sphere "velocity" (outward) | `particle_velocity.all_direction` | Radius × velocity scale, as a speed (× 0.1 / 60). |
| Initial velocity | `particle_velocity.designated_dir_x/y/z` (unit), `designated_dir_scale` (speed), `diffusion_*` (spread) | Mean direction normalised, swapped to Smash axes. |
| Velocity cone / radial | `all_direction`, `diffusion_*` | |
| Const acceleration, gravity | `emitter_static.gravity_scale`, `gravity_dir_*` | Also set the `emission.` copies. |
| Drag | `emitter_static.air_res` | Kept speed a frame: e^(−drag ÷ 60). 1 is no drag. |
| Initial rotation | `emitter_static.rotate_init_z`, `rotate_init_rand_z` | Turns × 2π. Or `turn-z` / `roll-random` in degrees. |
| Rotation rate | `emitter_static.rotate_add_z`, `rotate_add_rand_z` | Turns a second × 2π ÷ 60. Or `spin-z`. |
| Mesh rotation and rate | `rotate_init_*`, `rotate_add_*` on all three axes, `particle_data.is_rotate_*` | Axes as under Units. |
| Emitter rotation (Required) | `emitter_info.rotate_x/y/z` | Radians. Axes as under Units. |
| SubUV sheet | `emitter_static.tex_scroll_anim0.uv_div_x/y` | Columns and rows. Set `texture_anim0.repeat` 0 too: some kit layers repeat their texture twice, which draws 2 × 2 cells. |
| SubUV index over life | `texture_anim0.pattern_anim_type` 2, `emitter_static.tex_pattern_anim0.table[i]`, `.num`, `.frequency` | Write the cells in play order, up to 32. The table plays once and holds the end. `num` = steps written, `frequency` = life ÷ steps. |
| Random SubUV cell | `texture_anim0.pattern_anim_type` 4, `tex_pattern_anim0.num` = cells | |
| Second texture (mask, noise) | `texture second` / **Swap texture 2…** | Only on a layer whose shader reads two textures. |
| Texture tiling (UV scale) | `emitter_static.tex_scroll_anim0.scale_x/y`, `sampler0.wrap_u/v` Repeat | Tiling more than once needs Repeat, or the edge pixels smear. Blanks start on Repeat. |

## Textures

Swap the art in with **Swap texture…** (or `effect_forge texture`) and choose where its shape comes from (alpha, one
channel, or brightness). Unreal materials often keep the art in one parameter and masks, gradients or noise in the
rest. Port the one named like `Texture` or `Base` first. A gradient that is opaque everywhere draws as a solid square:
that is a control map, not the art.

Dynamic material parameters (`*_Tex_R`, `*_Tex_G`, `*_Tex_B`, a mask switch) pick which channel holds the shape: swap
the texture with `shape=r|g|b` to match. A colour gradient in the material (dark stop to bright stop) maps onto
**Colour** (where the texture is bright) and **Colour 2** (where it is dark): set `colour=` to the bright stop and
`colour2=` to the dark one.

## Curves

Every curve (opacity, colour, size over life) holds at most 8 keys, each at a time from 0 (birth) to 1 (death). The
shorthand sets a whole curve and its type at once, and finds a shader that reads it:

    effect_forge set <eff> <out.eff> <EFFECT> 1 fade=0:0,0.1:1,0.8:1,1:0 size-keys=0:0.5,1:2 colour-keys=0:#ffffff,1:#ff6020

Don't give a model layer both fade keys and size keys: one set up like that drew nothing in game. `caps` and `set`
warn of it.

## Models

Export the Unreal mesh to `.glb` (Y up), then fit it as you add it:

    effect_forge model <eff> <out.eff> <EFFECT> <layer> mesh.glb turn-x=90 size=26 centre

The options are listed in the README's "Fitting a model". Unreal meshes come in centimetres, so a 300 cm ring is 300
across until scaled. `size=` sets its largest side directly. `layers` prints each model's size.

## Limits to plan around

- **Layers:** 16 per effect. Ways of spawning that share a window (a burst plus a rate) can be one layer at the
  summed rate.
- **Keys:** 8 per curve. Keep the corners: a fade that holds and then drops needs its corner key more than evenly
  spaced ones.
- **Emission:** about 2 particles a frame per layer, 30 in one burst.
- **Shaders:** billboard type, alpha type, follow type and draw style are compiled in. Set them in the panel, which
  borrows a matching shader, rather than only typing the number.
- **Scripts:** Forge doesn't write the move's script. The script spawns the effect by name and picks its bone.
