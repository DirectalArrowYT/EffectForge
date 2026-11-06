//! Forge's own tests: `cargo test --release`.
//!
//! They read the game's own effect files, which don't change, so each output can be pinned by a hash. The file
//! goldens were made with the Effect Forge exe from before the 2026-11-04 cleanup (the code Visionary first gave it),
//! so a rewrite of the writer or the simulation has to give the same bytes as that.
//!
//! Files: `FORGE_TEST_PIT` (default the extracted arc's fighter/pit/ef_pit.eff) and `FORGE_TEST_COMMON`
//! (system/common/ef_common.eff). A test whose file is missing says so and passes.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::draw::sim::{spawns_moving, EmitterSim};
use crate::engine::{self, Doc, Pending};
use crate::eff::fields::{self, AttrValue};

/// Doc::open writes a working copy named after the file, so tests that open one take turns.
static OPEN: Mutex<()> = Mutex::new(());

fn file(var: &str, default: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var(var).unwrap_or_else(|_| default.to_string()));
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipped: {} is not there (set {var})", path.display());
        None
    }
}

fn pit() -> Option<PathBuf> {
    file("FORGE_TEST_PIT", "D:/Documents/SmashArc/effect/fighter/pit/ef_pit.eff")
}

fn common() -> Option<PathBuf> {
    file("FORGE_TEST_COMMON", "D:/Documents/SmashArc/effect/system/common/ef_common.eff")
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn open(path: &PathBuf) -> Doc {
    Doc::open(path).expect("the file opens")
}

#[test]
fn set_gives_the_same_bytes() {
    let Some(path) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let doc = open(&path).set_on_effect("PIT_GUARDIAN_SHIELD", "particle_data.life", "30").unwrap();
    assert_eq!(fnv(&doc.bytes), 0x2790c92a3172fbf0);
    let (_, set) = doc.find("PIT_GUARDIAN_SHIELD").unwrap();
    for emitter in doc.emitters(set) {
        assert_eq!(engine::attr(emitter, "particle_data.life").map(|v| v.as_f32()), Some(30.0), "{}", emitter.name);
    }
}

#[test]
fn borrow_gives_the_same_bytes_and_donors() {
    let Some(path) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    // the whole game is searched, whichever test runs first
    if engine::game_root().is_none() {
        std::env::set_var("FORGE_GAME_ROOT", game_root_above(&path).expect("the game folder"));
    }
    let doc = open(&path);
    let (keyed, report) = doc.borrow_on_effect("PIT_GUARDIAN_SHIELD", None, "particle_color.alpha0_type", 2.0).unwrap();
    assert_eq!(report, [
        "tsubu1: already",
        "thunder1: shader from effect/system/common/ef_common.eff / SYS_HIT_DEAD / flash1_alpha",
        "shield_aura: shader from effect/fighter/buddy/ef_buddy.eff / BUDDY_ATTACK_LW_SMOKE / flare1",
        "shield: shader from effect/fighter/bayonetta/ef_bayonetta.eff / BAYONETTA_FINAL_GAUGE_FINISH / magicL1",
        "shield_light: shader from effect/fighter/palutena/ef_palutena.eff / PALUTENA_MIRROR / mirror1_alpha; took texture_anim0.is_scroll, combiner.primitive_color_input_type",
        "wing: shader from effect/fighter/dolly/ef_dolly.eff / DOLLY_WAVE / waveBase; took texture_anim0.is_scroll, texture_anim1.is_scale, combiner.texture1_color_blend, combiner.tex_alpha1_input_type",
        "flare1_add: already",
        "ring1: shader from effect/fighter/elight/ef_elight.eff / ELIGHT_PHOTON_FLASH / ring1",
        "flashline2: shader from effect/fighter/kirby/ef_kirby.eff / KIRBY_AIR_N / ring1",
        "flashline1a: shader from PIT_GUARDIAN_SHIELD / flashline2",
        "flashline1b: shader from PIT_GUARDIAN_SHIELD / flashline2",
        "flashline2_add: shader from effect/system/common/ef_common.eff / SYS_REFLECTION / 3lineFlash_f; took texture_anim0.pattern_anim_type",
        "ring1_add: shader from effect/fighter/miifighter/ef_miifighter.eff / MIIFIGHTER_SPINKICK_FLASH / ring; took texture_anim0.pattern_anim_type, combiner.alpha_combiner_process",
        "rainbow1: already",
        "rainbow2: already",
    ]);
    assert_eq!(fnv(&keyed.bytes), 0x67d31e67547ec959);
    let (stretched, report) = doc.borrow_on_effect("PIT_GUARDIAN_SHIELD", None, "particle_data.billboard_type", 5.0).unwrap();
    assert_eq!(report, [
        "tsubu1: shader from effect/fighter/captain/ef_captain.eff / CAPTAIN_FINAL_LINE_A / Line3; took texture_anim0.is_scroll, texture_anim0.is_scale, combiner.color_combiner_process",
        "thunder1: shader from effect/fighter/dedede/ef_dedede.eff / DEDEDE_JETHAMMER_HOLD / fireCenter2",
        "shield_aura: shader from effect/fighter/lucario/ef_lucario.eff / LUCARIO_FINAL_BEAM / beam_Line2",
        "shield: shader from effect/fighter/edge/ef_edge.eff / EDGE_OCTASLASH_LINE / lightline2B",
        "shield_light: shader from effect/fighter/krool/ef_krool.eff / KROOL_SMASH_S4 / line2_b; took texture_anim0.is_scroll, combiner.tex_alpha0_input_type, combiner.tex_alpha1_input_type, combiner.primitive_color_input_type",
        "wing: shader from effect/fighter/diddy/ef_diddy.eff / DIDDY_POPGUN / muzzleflash1_back; took texture_anim1.is_scroll, combiner.texture1_color_blend, combiner.tex_alpha1_input_type",
        "flare1_add: shader from effect/fighter/bayonetta/ef_bayonetta.eff / BAYONETTA_FINAL_GAUGE_MOON / line1",
        "ring1: shader from effect/fighter/ganon/ef_ganon.eff / GANON_MAJINKEN_FLASH / lightning1",
        "flashline2: shader from effect/fighter/chrom/ef_chrom.eff / CHROM_COUNTER_SUCCESS / zigzagline1",
        "flashline1a: shader from PIT_GUARDIAN_SHIELD / flashline2",
        "flashline1b: shader from PIT_GUARDIAN_SHIELD / flashline2",
        "flashline2_add: shader from effect/fighter/jack/ef_jack.eff / JACK_DOYLE_BOMB / flashLine1_f",
        "ring1_add: shader from PIT_GUARDIAN_SHIELD / flashline2_add; took texture_anim0.is_scale",
        "rainbow1: shader from effect/fighter/miiswordsman/ef_miiswordsman.eff / MIISWORDSMAN_ROCKET_STAB / stab3_add; took texture_anim0.is_scale",
        "rainbow2: shader from PIT_GUARDIAN_SHIELD / rainbow1; took texture_anim0.is_scale",
    ]);
    assert_eq!(fnv(&stretched.bytes), 0x246f4bd80bdc60a3);
}

/// The app sets a value first and finds the shader once it is in; that lands where a borrow straight away does.
#[test]
fn values_then_reshade_matches_a_borrow() {
    let Some(path) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if engine::game_root().is_none() {
        std::env::set_var("FORGE_GAME_ROOT", game_root_above(&path).expect("the game folder"));
    }
    let doc = open(&path);
    let (entry, set) = doc.find("PIT_GUARDIAN_SHIELD").unwrap();
    for (id, value) in [("particle_color.alpha0_type", 2.0), ("particle_data.billboard_type", 5.0)] {
        for layer in [1, 7] {
            let (direct, report) = doc.borrow_on_effect(&entry, Some(layer), id, value).unwrap();
            let mut pending = Pending::new();
            let like = engine::attr(&doc.emitters(set)[layer], id).unwrap();
            pending.entry((set, layer)).or_default().insert(id.into(), engine::typed(like, value));
            let (later, found) = doc.apply_values(&pending).unwrap().reshade(&entry, set, layer, &[(id, value)], &[]).unwrap();
            let found = found.expect("a shader");
            assert!(report[0].contains(&found.from), "{id} on layer {layer}: {} vs {}", report[0], found.from);
            let shader = |d: &Doc| engine::attr(&d.emitters(set)[layer], "shader_references.shader_index").unwrap().as_f32();
            assert_eq!(shader(&direct), shader(&later), "{id} on layer {layer}");
        }
    }
}

/// A layer set to turn whose shader can't read the start angle is pointed out, and borrowing the turn gives it a
/// shader that reads it, even when the flag was already on.
#[test]
fn a_turn_borrows_a_shader_that_reads_it() {
    let Some(path) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if engine::game_root().is_none() {
        std::env::set_var("FORGE_GAME_ROOT", game_root_above(&path).expect("the game folder"));
    }
    let doc = open(&path);
    let (entry, set) = doc.find("PIT_GUARDIAN_SHIELD").unwrap();
    let id = "particle_data.is_rotate_x";
    let reads = |d: &Doc, layer: usize| {
        let shader = engine::attr(&d.emitters(set)[layer], "shader_references.shader_index").unwrap().as_f32();
        d.variations()[shader as usize].inputs.iter().any(|n| n == engine::ROTATION_INPUT)
    };
    let layer = (0..doc.emitters(set).len()).find(|&l| !reads(&doc, l)).expect("a layer whose shader doesn't turn");
    let mut pending = Pending::new();
    let like = engine::attr(&doc.emitters(set)[layer], id).unwrap();
    pending.entry((set, layer)).or_default().insert(id.into(), engine::typed(like, 1.0));
    let flagged = doc.apply_values(&pending).unwrap();
    assert!(flagged.turn_ignored(&flagged.emitters(set)[layer]));
    let (turned, report) = flagged.borrow_on_effect(&entry, Some(layer), id, 1.0).unwrap();
    assert!(!report[0].ends_with("already"), "{}", report[0]);
    assert!(reads(&turned, layer), "{}", report[0]);
    assert!(!turned.turn_ignored(&turned.emitters(set)[layer]));
}

/// A picture's shape comes from the channel asked for, colour gives way to white when asked, and a shape that is
/// solid or empty is pointed out.
#[test]
fn texture_shape_comes_from_the_channel_asked_for() {
    use engine::{texture_pixels, AlphaFrom, TextureFrom};
    let picture = image::RgbaImage::from_fn(2, 1, |x, _| image::Rgba(if x == 0 { [200, 10, 0, 255] } else { [0, 90, 30, 255] }));
    let shaped = |alpha, white| texture_pixels(picture.clone(), TextureFrom { alpha, white });
    let (own, notes) = shaped(AlphaFrom::Own, false);
    assert_eq!(own.as_raw(), picture.as_raw());
    assert_eq!(notes.len(), 1, "an opaque picture is pointed out");
    let (green, notes) = shaped(AlphaFrom::Green, true);
    assert_eq!(green.as_raw(), &[255, 255, 255, 10, 255, 255, 255, 90]);
    assert!(notes.is_empty());
    let (bright, _) = shaped(AlphaFrom::Brightness, false);
    assert_eq!(bright.as_raw(), &[200, 10, 0, 200, 0, 90, 30, 90]);
    let (_, notes) = texture_pixels(image::RgbaImage::new(2, 2), TextureFrom { alpha: AlphaFrom::Red, white: false });
    assert!(notes[0].contains("empty"));
    assert_eq!(AlphaFrom::parse("g"), Some(AlphaFrom::Green));
    assert_eq!(AlphaFrom::parse("lum"), Some(AlphaFrom::Brightness));
    assert_eq!(AlphaFrom::parse("q"), None);
}

/// Every value of every layer reads back as written after a rebuild that changes one of them.
#[test]
fn a_rebuild_keeps_every_other_value() {
    let Some(path) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let before = open(&path);
    let (entry, set) = before.find("PIT_SWORD").unwrap();
    let mut pending = Pending::new();
    pending.entry((set, 0)).or_default().insert("particle_scale.scale_x".into(), AttrValue::Float(0.5));
    let after = before.apply_values(&pending).unwrap();
    assert_eq!(before.effects(), after.effects());
    for (name, set_idx) in before.effects() {
        for (i, (a, b)) in before.emitters(set_idx).iter().zip(after.emitters(set_idx)).enumerate() {
            for (attr, (x, y)) in fields::table().iter().zip(a.attrs.iter().zip(&b.attrs)) {
                if name == entry && i == 0 && attr.id == "particle_scale.scale_x" {
                    assert_eq!(y.map(|v| v.as_f32()), Some(0.5));
                } else {
                    assert_eq!(x.map(|v| v.as_f32().to_bits()), y.map(|v| v.as_f32().to_bits()), "{name} / {} {}", a.name, attr.id);
                }
            }
        }
    }
}

/// The particles every layer of ef_common spawns over its first 90 frames, still and moving, pinned by a hash.
#[test]
fn spawns_are_unchanged() {
    let Some(path) = common() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let doc = open(&path);
    let moving = |t: f32| t * 2.5;
    let mut text = String::new();
    let mut count = 0;
    for (name, set) in doc.effects() {
        for (i, emitter) in doc.emitters(set).iter().enumerate() {
            let sim = EmitterSim::read(emitter);
            for age in [0.0, 1.0, 4.0, 15.0, 40.0, 90.0] {
                let seed = fnv(format!("{name}/{i}").as_bytes());
                for travelled in [None, Some(&moving as &dyn Fn(f32) -> f32)] {
                    let spawns = spawns_moving(&sim, age, seed, travelled);
                    count += spawns.len();
                    text += &format!("{name}/{i}@{age}:{spawns:?}\n");
                }
            }
        }
    }
    println!("{count} spawns, hash {:#x}", fnv(text.as_bytes()));
    assert!(count > 10_000);
    assert_eq!(fnv(text.as_bytes()), SPAWNS);
}
const SPAWNS: u64 = 0xffeeb80362f78a05;

/// What the translator makes of one file's shaders, as text: per variation and stage, its bindings and WGSL, and
/// the stages it could not translate.
fn shader_digest(eff: &[u8]) -> (crate::draw::shaders::Library, String) {
    use crate::draw::mx::wgsl::Stage;
    let library = crate::draw::shaders::Library::translate(eff).unwrap();
    let mut variations: Vec<u32> = library.vertex.keys().chain(library.fragment.keys()).copied().collect();
    variations.sort();
    variations.dedup();
    let mut text = format!("tried {}
", library.tried);
    for v in variations {
        for stage in [Stage::Vertex, Stage::Fragment] {
            match library.program(v, stage) {
                Some(p) => text += &format!(
                    "v{v} {stage:?} cbufs {:#x} in {:#x} units {:?} c1 {:016x} attributes {:?}
{}
",
                    p.cbufs, p.inputs, p.units, fnv(&p.constants), p.attributes, p.wgsl
                ),
                None => text += &format!("v{v} {stage:?} not translated
"),
            }
        }
    }
    (library, text)
}

/// Every shader variation of a fighter's file and of the common file translates, and to the same WGSL as when the
/// translator was taken in.
#[test]
fn shaders_translate_the_same() {
    let (Some(fighter), Some(shared)) = (pit(), common()) else { return };
    let mut all = String::new();
    for (path, complete) in [(fighter, true), (shared, false)] {
        let (library, text) = shader_digest(&std::fs::read(&path).unwrap());
        println!("{}: {} of {} stages translated", path.display(), library.translated(), library.tried);
        assert!(library.tried > 0);
        assert!(!complete || library.translated() == library.tried);
        all += &text;
    }
    assert_eq!(fnv(all.as_bytes()), SHADERS);
}
const SHADERS: u64 = 0x62fa264239432958;

/// Opt in with `FORGE_TEST_SHADER_DIR` (a folder searched for .eff files) and `FORGE_TEST_SHADER_OUT` (a file):
/// writes one line per file, its path and the hash of its [`shader_digest`], to compare against an older build.
#[test]
fn shader_corpus() {
    let (Ok(dir), Ok(out)) = (std::env::var("FORGE_TEST_SHADER_DIR"), std::env::var("FORGE_TEST_SHADER_OUT")) else { return };
    fn walk(dir: &std::path::Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|e| e == "eff") {
                found.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(std::path::Path::new(&dir), &mut files);
    files.sort();
    let lines: Vec<String> = files
        .iter()
        .map(|path| {
            let eff = std::fs::read(path).unwrap();
            let digest = std::panic::catch_unwind(|| shader_digest(&eff).1).map_or_else(|_| "panicked".to_string(), |t| format!("{:016x}", fnv(t.as_bytes())));
            format!("{} {digest}", path.display())
        })
        .collect();
    std::fs::write(&out, lines.join("
") + "
").unwrap();
    println!("{} files", lines.len());
}

/// Every field a recipe sets is a real field, and every recipe sets something.
#[test]
fn recipes_name_real_fields() {
    let list: Vec<serde_json::Value> = serde_json::from_str(include_str!("app/recipes.json")).unwrap();
    assert!(!list.is_empty());
    for recipe in &list {
        let values = recipe["values"].as_object().expect("values");
        assert!(!values.is_empty(), "{}", recipe["name"]);
        for id in values.keys().chain(recipe["modes"].as_object().into_iter().flat_map(|m| m.keys())) {
            assert!(fields::index_of(id).is_some(), "{}: {id}", recipe["name"]);
        }
    }
}

/// The fields the panel borrows shaders for are real fields.
#[test]
fn baked_modes_are_real_fields() {
    for id in engine::BAKED_MODES.iter().chain(["shader_references.shader_index"].iter()) {
        assert!(fields::index_of(id).is_some(), "{id}");
    }
}

/// Every emitter's c9 block in ef_pit and ef_common (and `FORGE_TEST_BLOCKS`, `;`-separated, if set), pinned by a hash.
#[test]
fn blocks_are_unchanged() {
    let mut files: Vec<PathBuf> = [pit(), common()].into_iter().flatten().collect();
    let extra = std::env::var("FORGE_TEST_BLOCKS").ok();
    files.extend(extra.iter().flat_map(|s| s.split(';')).map(PathBuf::from));
    if files.is_empty() {
        return;
    }
    let mut text = Vec::new();
    let mut count = 0;
    for path in &files {
        let file = effect_library::NamcoEffectFile::load(&std::fs::read(path).unwrap()).unwrap();
        let ptcl = file.ptcl_file.as_ref().unwrap();
        fn walk(list: &[effect_library::Emitter], version: u16, out: &mut Vec<u8>, count: &mut usize) {
            for e in list {
                if let Some(block) = crate::draw::block::emitter_block(e, version) {
                    out.extend(block);
                    *count += 1;
                }
                walk(&e.children, version, out, count);
            }
        }
        for set in &ptcl.emitter_list.emitter_sets {
            walk(&set.emitters, ptcl.vfx_version, &mut text, &mut count);
        }
    }
    println!("{count} blocks, hash {:#x}", fnv(&text));
    if extra.is_none() {
        assert!(count > 1000);
        assert_eq!(fnv(&text), BLOCKS);
    }
}
const BLOCKS: u64 = 0x78b5d9d39362d018;

/// What the texture module reads out of every pool texture in ef_pit and ef_common, pinned by a hash.
#[test]
fn textures_read_the_same() {
    use crate::eff::textures::{decode_for_sampling, decode_preview, describe, Form};
    let files: Vec<PathBuf> = [pit(), common()].into_iter().flatten().collect();
    if files.is_empty() {
        return;
    }
    let mut text = Vec::new();
    let mut count = 0;
    for path in &files {
        let doc = open(path);
        let pool = doc.loaded.texture_pool.clone().unwrap_or_default();
        for (i, t) in doc.loaded.ptcl.bntx_textures.iter().enumerate() {
            let d = describe(&pool, i, &t.tex_name).ok();
            text.extend(format!("{} {:?}\n", t.tex_name, d.as_ref().map(|d| (d.width, d.height, d.mipmaps, &d.format, d.convertible))).bytes());
            match decode_for_sampling(&pool, i, &t.tex_name) {
                Ok(s) => {
                    text.push(s.srgb as u8);
                    for level in &s.levels {
                        text.extend(format!("{}x{}", level.width(), level.height()).bytes());
                        text.extend(level.as_raw());
                    }
                }
                Err(_) => text.extend(b"no sampling"),
            }
            for (form, edge) in [(Form::Raw, None), (Form::Editable, Some(64))] {
                match decode_preview(&pool, i, &t.tex_name, form, edge) {
                    Ok(image) => {
                        text.extend(format!("{}x{}", image.width(), image.height()).bytes());
                        text.extend(image.as_raw());
                    }
                    Err(_) => text.extend(b"no preview"),
                }
            }
            count += 1;
        }
    }
    println!("{count} textures, hash {:#x}", fnv(&text));
    assert!(count > 100);
    assert_eq!(fnv(&text), TEXTURES_READ);
}
const TEXTURES_READ: u64 = 0xab28207941b082e3;

/// Replacing, adding, copying and removing pool textures in ef_pit: one small texture of each format, through an
/// edited picture of itself in both forms. The rebuilt pools and what each import reports, pinned by a hash.
#[test]
fn texture_edits_give_the_same_pool() {
    use crate::eff::textures::*;
    let Some(path) = pit() else { return };
    let doc = open(&path);
    let pool = doc.loaded.texture_pool.clone().unwrap();
    let names: Vec<String> = doc.loaded.ptcl.bntx_textures.iter().map(|t| t.tex_name.clone()).collect();
    let mut seen = Vec::new();
    let mut text = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let Ok(d) = describe(&pool, i, name) else { continue };
        if seen.contains(&d.format) || d.width * d.height > 128 * 128 {
            continue;
        }
        seen.push(d.format.clone());
        text.extend(format!("{name} {}\n", d.format).bytes());
        for form in [Form::Editable, Form::Raw] {
            let png = decode_preview(&pool, i, name, form, None).ok().map(|mut image| {
                // Paint the top-left quarter black and see-through, the way an edit would.
                for (x, y, px) in image.enumerate_pixels_mut() {
                    if x < d.width / 2 && y < d.height / 2 {
                        *px = image::Rgba([0, 0, 0, 0]);
                    }
                }
                let mut out = std::io::Cursor::new(Vec::new());
                image.write_to(&mut out, image::ImageFormat::Png).unwrap();
                out.into_inner()
            });
            let Some(png) = png else {
                text.extend(b"no picture\n");
                continue;
            };
            for result in [replace_with_png(&pool, &names, i, &png, form), add_texture_from_png(&pool, &names, i, "forge_new", &png, form)] {
                match result {
                    Ok((rebuilt, report)) => {
                        text.extend(format!("{} {:?} {:#x}\n", report.format, report.format_substituted_from, fnv(&rebuilt)).bytes())
                    }
                    Err(_) => text.extend(b"failed\n"),
                }
            }
        }
        text.extend(format!("{:?}\n", duplicate_texture(&pool, &names, i, "forge_copy").map(|p| fnv(&p)).ok()).bytes());
        text.extend(format!("{:?}\n", remove_texture(&pool, &names, i).map(|p| fnv(&p)).ok()).bytes());
    }
    text.extend(format!("{:?}\n", duplicate_texture(&pool, &names, 0, &names[1]).is_err()).bytes());
    text.extend(format!("{} {}\n", unique_texture_name(&names, "forge_a"), unique_texture_name(&["forge_a_v2".into()], "forge_a_v2")).bytes());
    text.extend(format!("{:#x} {:#x}\n", unused_descriptor_id(&[], "forge_a"), unused_descriptor_id(&[unused_descriptor_id(&[], "x")], "x")).bytes());
    println!("{}", String::from_utf8_lossy(&text));
    println!("{} formats, hash {:#x}", seen.len(), fnv(&text));
    assert!(seen.len() > 1);
    assert_eq!(fnv(&text), TEXTURE_EDITS);
}
const TEXTURE_EDITS: u64 = 0x1fb77a886849b645;

/// The primitives of ef_pit and ef_common: the listing, each one's glb and the mesh read back from it, and new
/// primitives made from them, pinned by a hash. `FORGE_TEST_GLB` (a folder) also reads every glb in it, printed only.
#[test]
fn meshes_are_unchanged() {
    use crate::eff::mesh_io::*;
    let files: Vec<PathBuf> = [pit(), common()].into_iter().flatten().collect();
    if files.is_empty() {
        return;
    }
    let mut text = Vec::new();
    let mut count = 0;
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let listing = list_primitives(&bytes).unwrap();
        let mut meshes = Vec::new();
        for p in &listing {
            text.extend(format!("{} {:#x} {} {} {:?}\n", p.name, p.id, p.vertices, p.triangles, p.used_by).bytes());
            let glb = export_primitive_glb(&bytes, &p.name).unwrap();
            text.extend(format!("{:#x}\n", fnv(&glb)).bytes());
            let mesh = from_gltf(&glb).unwrap();
            text.extend(format!("{mesh:?}\n").bytes());
            meshes.push(mesh);
            count += 1;
        }
        // Each used primitive replaced, for its first user, by the next primitive's mesh; then one mesh with every
        // attribute on, and one with none.
        let used: Vec<_> = listing.iter().enumerate().filter(|(_, p)| !p.used_by.is_empty()).take(6).collect();
        for (n, (i, p)) in used.iter().enumerate() {
            let user = &p.used_by[0];
            let (entry, emitter) = user.split_once('/').unwrap();
            let target = [(entry.to_string(), emitter.to_string())];
            let mut mesh = meshes[(i + 1) % meshes.len()].clone();
            if n == 1 {
                mesh.colors = mesh.positions.iter().map(|p| [p[0].abs().min(1.0), 0.5, 2.0, -1.0]).collect();
                mesh.uv1 = mesh.positions.iter().map(|p| [p[1], p[2]]).collect();
                mesh.normals = mesh.positions.iter().map(|p| [p[0], 0.0, p[2]]).collect();
            }
            if n == 2 {
                (mesh.colors, mesh.uv1, mesh.normals) = (Vec::new(), Vec::new(), Vec::new());
            }
            match add_primitive_for_emitters(&bytes, &p.name, &format!("forge_mesh_{n}"), &mesh, &target) {
                Ok(out) => text.extend(format!("{user} {:#x}\n", fnv(&out)).bytes()),
                Err(e) => text.extend(format!("{user} {e:#}\n").bytes()),
            }
        }
        if let Some((_, p)) = used.first() {
            let bad = add_primitive_for_emitters(&bytes, &p.name, &p.name, &meshes[0], &[]).is_err();
            let wrong = add_primitive_for_emitters(&bytes, &p.name, "forge_x", &meshes[0], &[("none".into(), "none".into())]).is_err();
            text.extend(format!("{bad} {wrong}\n").bytes());
        }
    }
    println!("{count} primitives, hash {:#x}", fnv(&text));
    if let Ok(dir) = std::env::var("FORGE_TEST_GLB") {
        let mut glbs: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
        glbs.sort();
        let mut text = Vec::new();
        for glb in glbs.iter().filter(|p| p.extension().is_some_and(|e| e == "glb")) {
            text.extend(format!("{:?}\n", from_gltf(&std::fs::read(glb).unwrap()).map_err(|e| e.to_string())).bytes());
        }
        println!("{} glbs, hash {:#x}", glbs.len(), fnv(&text));
    }
    assert!(count > 10);
    assert_eq!(fnv(&text), MESHES);
}
const MESHES: u64 = 0x0c3c67db8a5efcb8;

/// The field table: every row's id, place and kind, every value it reads from every emitter of ef_pit and
/// ef_common, and what each row reads back after being set to edge values on a few emitters, pinned by a hash.
#[test]
fn fields_are_unchanged() {
    let table = fields::table();
    let mut text = Vec::new();
    for a in table {
        text.extend(format!("{} | {} | {} | {} | {:?}\n", a.id, a.group, a.label, a.doc, a.kind).bytes());
    }
    text.extend(format!("{:?}\n", fields::GROUPS).bytes());
    println!("{} rows, table hash {:#x}", table.len(), fnv(&text));
    let files: Vec<PathBuf> = [pit(), common()].into_iter().flatten().collect();
    let mut tried = 0;
    for path in &files {
        let file = effect_library::NamcoEffectFile::load(&std::fs::read(path).unwrap()).unwrap();
        let mut all = Vec::new();
        fn walk<'a>(list: &'a [effect_library::Emitter], out: &mut Vec<&'a effect_library::EmitterData>) {
            for e in list {
                out.push(&e.data);
                walk(&e.children, out);
            }
        }
        for set in &file.ptcl_file.as_ref().unwrap().emitter_list.emitter_sets {
            walk(&set.emitters, &mut all);
        }
        for d in &all {
            text.extend(format!("{:?}\n", fields::read_all(d)).bytes());
        }
        // Set each row on emitters with and without the optional blocks.
        let picks: Vec<_> = [all.iter().position(|d| d.sampler2.is_some() && d.texture_anim0.is_some()), all.iter().position(|d| d.sampler1.is_none()), Some(0)]
            .into_iter()
            .flatten()
            .collect();
        let values = [AttrValue::Int(-5), AttrValue::Int(300), AttrValue::UInt(u64::MAX), AttrValue::Float(3.6), AttrValue::Float(-70000.5), AttrValue::Int(1), AttrValue::Int(i64::MIN)];
        for &p in &picks {
            assert!(!fields::write(&mut all[p].clone(), "no.such", AttrValue::Int(0)));
            for a in table {
                for v in values {
                    let mut d = all[p].clone();
                    assert!(fields::write(&mut d, a.id, v));
                    text.extend(format!("{:?} ", (a.get)(&d)).bytes());
                    tried += 1;
                }
                text.push(b'\n');
            }
        }
    }
    println!("{tried} sets, hash {:#x}", fnv(&text));
    assert!(tried > 1000);
    assert_eq!(fnv(&text), FIELDS);
}
const FIELDS: u64 = 0xe330901adb819c1b;

/// The folder holding the game's `effect` folder, above `file`.
fn game_root_above(file: &std::path::Path) -> Option<PathBuf> {
    file.ancestors().find(|dir| dir.join("effect").is_dir()).map(|dir| dir.to_path_buf())
}

fn kinds(bytes: &[u8]) -> std::collections::HashMap<String, u16> {
    let file = effect_library::NamcoEffectFile::load(bytes).expect("the file reads");
    file.entry_names.iter().cloned().zip(file.entries.iter().map(|e| e.kind)).collect()
}

/// Every piece of the kit makes an effect in a fighter's file that has the piece's layers, and only those, with the
/// fighter type the fighter's spawns need.
#[test]
fn kit_pieces_make_effects() {
    let Some(pit) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let root = game_root_above(&pit);
    let doc = open(&pit);
    for (i, piece) in engine::kit().iter().enumerate() {
        let name = format!("kit_test_{i}");
        let op = crate::eff::edits::TransplantOp { new_entry_name: name.clone(), ..Default::default() };
        let made = doc.from_piece(piece, op, root.clone()).unwrap_or_else(|e| panic!("{}: {e:#}", piece.name));
        let (_, set) = made.effects().into_iter().find(|(n, _)| *n == name).expect("the new effect");
        let layers: Vec<String> = made.emitters(set).iter().filter(|e| e.depth == 0).map(|e| e.name.clone()).collect();
        assert_eq!(layers, piece.layers, "{}", piece.name);
        assert_ne!(kinds(&made.bytes)[&name] & 0xff00, 0, "{} has no fighter type", piece.name);
    }
}

/// Layers added to an effect come after its own, renamed when the effect already has the name, and removing one
/// takes out only it.
#[test]
fn layers_add_and_remove() {
    let Some(pit) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let root = game_root_above(&pit);
    let piece = |name: &str| engine::kit().iter().find(|p| p.name == name).expect("a piece of the kit");
    let doc = open(&pit);
    let (entry, set) = doc.effects().into_iter().find(|(_, set)| doc.emitters(*set).len() > 1).expect("an effect with layers");
    let before: Vec<String> = doc.emitters(set).iter().map(|e| e.name.clone()).collect();
    let add = |doc: &Doc, name: &str| {
        let op = crate::eff::edits::TransplantOp { into_entry: Some(entry.clone()), ..Default::default() };
        doc.from_piece(piece(name), op, root.clone()).unwrap()
    };
    let doc = add(&add(&doc, "Hit sparks"), "Hit sparks");
    let after: Vec<String> = doc.emitters(set).iter().map(|e| e.name.clone()).collect();
    let spark = &piece("Hit sparks").layers[0];
    let expected: Vec<String> = before.iter().cloned().chain([spark.clone(), format!("{spark}_2")]).collect();
    assert_eq!(after, expected);
    let doc = doc.remove_layer(&entry, set, before.len()).unwrap();
    let left: Vec<String> = doc.emitters(set).iter().map(|e| e.name.clone()).collect();
    let expected: Vec<String> = before.iter().cloned().chain([format!("{spark}_2")]).collect();
    assert_eq!(left, expected);
    // the effect's own layers kept every value
    let reopened = open(&pit);
    for (i, em) in reopened.emitters(set).iter().enumerate() {
        assert_eq!(em.attrs, doc.emitters(set)[i].attrs, "{}", em.name);
    }
}

/// A duplicated layer comes right after the layer and the layers under it, with them, renamed, and with their values;
/// the effect's own layers keep theirs.
#[test]
fn layer_duplicates() {
    let Some(pit) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let doc = open(&pit);
    // a layer with one under it, if the file has one, so the copy takes both
    let (entry, set, layer) = doc.effects().into_iter()
        .find_map(|(n, set)| {
            let ems = doc.emitters(set);
            (0..ems.len().saturating_sub(1)).find(|&i| ems[i + 1].depth > ems[i].depth).map(|i| (n.clone(), set, i))
        })
        .or_else(|| doc.effects().into_iter().find(|(_, s)| doc.emitters(*s).len() > 1).map(|(n, s)| (n, s, 0)))
        .expect("an effect with layers");
    let before = doc.emitters(set).to_vec();
    let end = (layer + 1..before.len()).find(|&i| before[i].depth <= before[layer].depth).unwrap_or(before.len());
    let (made, at) = doc.duplicate_layer(&entry, set, layer).unwrap();
    assert_eq!(at, end);
    let after = made.emitters(set);
    assert_eq!(after.len(), before.len() + end - layer);
    for i in 0..before.len() {
        let j = if i < end { i } else { i + end - layer };
        assert_eq!(after[j].name, before[i].name);
        assert_eq!(after[j].attrs, before[i].attrs, "{}", before[i].name);
    }
    for i in layer..end {
        let copy = &after[end + i - layer];
        assert_eq!(copy.name, format!("{}_2", before[i].name));
        assert_eq!(copy.depth, before[i].depth);
        assert_eq!(copy.attrs, before[i].attrs, "{}", copy.name);
    }
}

/// A renamed effect keeps its layers and its values, and an export takes the effects of a skipped folder out entirely
/// (name and set) and leaves every other effect as it was.
#[test]
fn rename_and_export_skipping_a_folder() {
    let Some(pit) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let doc = open(&pit);
    let effects = doc.effects();
    let (old, set) = effects.iter().find(|(_, set)| doc.emitters(*set).len() > 1).cloned().expect("an effect with layers");
    assert!(doc.rename(&old, "has space").is_err());
    let other = &effects.iter().find(|(n, _)| *n != old).unwrap().0;
    assert!(doc.rename(&old, other).is_err());
    let renamed = doc.rename(&old, "forge_renamed").unwrap();
    let found = renamed.effects().into_iter().find(|(n, _)| n == "forge_renamed").expect("the new name");
    assert_eq!(found.1, set);
    assert!(!renamed.effects().iter().any(|(n, _)| *n == old));
    for (i, em) in renamed.emitters(set).iter().enumerate() {
        assert_eq!(em.attrs, doc.emitters(set)[i].attrs, "{}", em.name);
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("forge_project_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut project = crate::project::Project::create(&dir.join("p.forge"), &renamed.bytes, Some(dir.join("out.eff"))).unwrap();
    project.add_folder("old").unwrap();
    project.move_to("forge_renamed", Some(0));
    assert!(project.left_out().is_empty());
    project.set_skipped(0, true).unwrap();
    assert_eq!(project.folders[0].name, "UNUSED_old");
    assert_eq!(project.left_out(), vec!["forge_renamed".to_string()]);
    project.save().unwrap();
    let project = crate::project::Project::load(&project.path).unwrap();
    let skipped = crate::project::export(&std::fs::read(project.eff_path()).unwrap(), &project.left_out(), &dir.join("out.eff")).unwrap();
    assert_eq!(skipped, 1);
    let out = open(&dir.join("out.eff"));
    let file = effect_library::NamcoEffectFile::load(&out.bytes).unwrap();
    assert!(!file.entry_names.iter().any(|n| n == "forge_renamed"), "the skipped effect's name is gone");
    assert_eq!(file.entry_names.len(), renamed.effects().len() - 1);
    let sets = |d: &Doc| d.loaded.ptcl.emitter_sets.len();
    assert!(sets(&out) < sets(&renamed), "its set is gone");
    for (name, set) in renamed.effects() {
        if name == "forge_renamed" {
            continue;
        }
        let (_, out_set) = out.effects().into_iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name} stays"));
        let (a, b) = (renamed.emitters(set), out.emitters(out_set));
        assert_eq!(a.len(), b.len(), "{name}");
        for (x, y) in a.iter().zip(b) {
            assert_eq!(x.name, y.name, "{name}");
        }
    }
    let mut back = project.clone();
    back.set_skipped(0, false).unwrap();
    assert_eq!(back.folders[0].name, "old");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Turning a layer's trail on leaves it on a shader that doesn't keep its particles where they were born; the fix puts
/// it on one that does (from the game's own files if need be) and changes nothing else about any layer.
#[test]
fn trail_shaders_follow_the_trail() {
    let Some(pit) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if engine::game_root().is_none() {
        std::env::set_var("FORGE_GAME_ROOT", game_root_above(&pit).expect("the game folder"));
    }
    let doc = open(&pit);
    assert!(doc.trail_mismatches(None).is_empty(), "the game's own file breaks the rule");
    let follow = |em: &crate::eff::model::EmitterDef, id: &str| engine::attr(em, id).map(|v| v.as_i64());
    let (entry, set, layer) = doc.effects().into_iter()
        .find_map(|(n, set)| {
            let ems = doc.emitters(set);
            (0..ems.len()).find(|&i| follow(&ems[i], "emitter_info.follow_type") == Some(0)).map(|i| (n.clone(), set, i))
        })
        .expect("a layer that rides its bone");
    let mut pending = Pending::new();
    pending.entry((set, layer)).or_default().insert("emitter_info.follow_type".into(), AttrValue::Int(1));
    let trailed = doc.apply_values(&pending).unwrap();
    assert_eq!(trailed.trail_mismatches(Some(&entry)).len(), 1);
    let (fixed, found) = trailed.fix_trail_shaders(Some(&entry)).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].fix.is_some(), "no shader for {}", found[0].name);
    assert!(fixed.trail_mismatches(None).is_empty());
    let shader = fields::table().iter().position(|a| a.id == "shader_references.shader_index").unwrap();
    for (name, set) in trailed.effects() {
        let (before, after) = (trailed.emitters(set), fixed.emitters(set));
        assert_eq!(before.len(), after.len(), "{name}");
        for (b, a) in before.iter().zip(after) {
            assert_eq!(b.name, a.name);
            for (i, (x, y)) in b.attrs.iter().zip(&a.attrs).enumerate() {
                if i != shader {
                    assert_eq!(x, y, "{name} {} {}", b.name, fields::table()[i].id);
                }
            }
        }
    }
}

/// Every value a kit piece sets is a real field.
#[test]
fn kit_values_are_real_fields() {
    for piece in engine::kit() {
        for id in piece.set.keys() {
            assert!(fields::table().iter().any(|a| a.id == id), "{}: no field {id}", piece.name);
        }
    }
}

/// A blank piece makes one plain layer with the piece's values, and the model one draws the model given to it.
#[test]
fn blank_pieces_start_plain() {
    let Some(pit) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if engine::game_root().is_none() {
        std::env::set_var("FORGE_GAME_ROOT", game_root_above(&pit).expect("the game folder"));
    }
    let doc = open(&pit);
    let quad = crate::eff::mesh_io::MeshData {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        colors: Vec::new(),
        uv0: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        uv1: Vec::new(),
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    for piece in engine::kit().iter().filter(|p| p.blank.is_some()) {
        let model = piece.is_model().then_some(&quad);
        let made = doc.new_effect("BLANK_TEST", piece, model).unwrap_or_else(|e| panic!("{}: {e:#}", piece.name));
        let (_, set) = made.find("BLANK_TEST").unwrap();
        let layers = made.emitters(set);
        assert_eq!(layers.len(), 1, "{}", piece.name);
        for (id, want) in &piece.set {
            let got = engine::attr(&layers[0], id).map(|v| v.as_f32());
            assert_eq!(got, Some(*want), "{} {id}", piece.name);
        }
        if piece.is_model() {
            let mesh = made.mesh(made.primitive_of("BLANK_TEST", &layers[0].name).expect("a model")).unwrap();
            assert_eq!(mesh.positions.len(), 4, "{}", piece.name);
        }
    }
}

/// Switching a draw style on puts the layer on a shader drawn that way and changes only the style, the shader, what
/// the shader needs and the style's tuning; switching it off again leaves it off, and no layer comes or goes.
#[test]
fn draw_styles_change_only_the_style() {
    let Some(pit) = pit() else { return };
    let _turn = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if engine::game_root().is_none() {
        std::env::set_var("FORGE_GAME_ROOT", game_root_above(&pit).expect("the game folder"));
    }
    let piece = engine::kit().iter().find(|p| p.blank.as_deref() == Some("sprite")).expect("a blank sprite");
    let doc = open(&pit).new_effect("BLANK_TEST", piece, None).unwrap();
    let (entry, set) = doc.find("BLANK_TEST").unwrap();
    let soft = engine::STYLES.iter().find(|s| s.field == "particle_color.is_soft_particle").unwrap();
    let (on, found) = doc.restyle(&entry, set, 0, soft, soft.on).unwrap();
    let found = found.expect("a soft shader in the game");
    let (before, after) = (&doc.emitters(set)[0], &on.emitters(set)[0]);
    assert_eq!(on.emitters(set).len(), 1);
    assert_eq!(engine::attr(after, soft.field).map(|v| v.as_f32()), Some(soft.on));
    let may_change = |id: &str| id == soft.field || id == "shader_references.shader_index" || found.taken.iter().any(|t| t == id)
        || soft.params.contains(&id) || id.starts_with("emitter_static.amplitude_") || id.starts_with("emitter_static.cycle_")
        || id.starts_with("emitter_static.num_") || id.contains(".keys[0].") || id.starts_with("particle_color.color") || id.starts_with("particle_color.alpha");
    for (i, (x, y)) in before.attrs.iter().zip(&after.attrs).enumerate() {
        let id = fields::table()[i].id;
        if !may_change(id) {
            assert_eq!(x, y, "{id} changed");
        }
    }
    for (name, s) in doc.effects() {
        if name != entry {
            assert_eq!(doc.emitters(s).len(), on.emitters(s).len(), "{name}");
        }
    }
    let (off, _) = on.restyle(&entry, set, 0, soft, 0.0).unwrap();
    assert_eq!(engine::attr(&off.emitters(set)[0], soft.field).map(|v| v.as_f32()), Some(0.0));
    assert_eq!(off.emitters(set).len(), 1);
}
