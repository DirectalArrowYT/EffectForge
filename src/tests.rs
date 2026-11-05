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
    let doc = open(&path);
    let (keyed, report) = doc.borrow_on_effect("PIT_GUARDIAN_SHIELD", "particle_color.alpha0_type", 2.0).unwrap();
    assert_eq!(report, [
        "tsubu1: already",
        "thunder1: shader 144 from PIT_FLY_MIRACLE_B / Feather1",
        "shield_aura: shader 5 from PIT_ATK_AIR_N / sword_outLine",
        "shield: shader 49 from PIT_PA_MAX / ring2_b",
        "shield_light: no donor",
        "wing: no donor",
        "flare1_add: already",
        "ring1: shader 5 from PIT_ATK_AIR_N / sword_outLine",
        "flashline2: shader 5 from PIT_ATK_AIR_N / sword_outLine",
        "flashline1a: shader 5 from PIT_ATK_AIR_N / sword_outLine",
        "flashline1b: shader 5 from PIT_ATK_AIR_N / sword_outLine",
        "flashline2_add: shader 155 from PIT_FINAL_ATTACK / flare1",
        "ring1_add: shader 155 from PIT_FINAL_ATTACK / flare1",
        "rainbow1: already",
        "rainbow2: already",
    ]);
    assert_eq!(fnv(&keyed.bytes), 0x679c0b8147837749);
    let (stretched, report) = doc.borrow_on_effect("PIT_GUARDIAN_SHIELD", "particle_data.billboard_type", 5.0).unwrap();
    assert_eq!(report, [
        "tsubu1: no donor",
        "thunder1: shader 37 from PIT_PA_FLY_ARROW / arrow_flare",
        "shield_aura: shader 169 from PIT_FINAL_END / light1",
        "shield: no donor",
        "shield_light: no donor",
        "wing: no donor",
        "flare1_add: shader 169 from PIT_FINAL_END / light1",
        "ring1: shader 169 from PIT_FINAL_END / light1",
        "flashline2: shader 169 from PIT_FINAL_END / light1",
        "flashline1a: shader 169 from PIT_FINAL_END / light1",
        "flashline1b: shader 169 from PIT_FINAL_END / light1",
        "flashline2_add: no donor",
        "ring1_add: no donor",
        "rainbow1: no donor",
        "rainbow2: no donor",
    ]);
    assert_eq!(fnv(&stretched.bytes), 0xe712aa5b613e444e);
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
const FIELDS: u64 = 0xe01372aeb90903b3;
