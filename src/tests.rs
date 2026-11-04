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

/// Every shader variation of a fighter's file translates.
#[test]
fn every_pit_shader_translates() {
    let Some(path) = pit() else { return };
    let dir = std::env::temp_dir().join("effect_forge_test_shaders");
    let _ = std::fs::remove_dir_all(&dir);
    let n = crate::draw::shaders::dump_variations(&std::fs::read(&path).unwrap(), &dir).unwrap();
    let (ok, all) = crate::draw::shaders::translate_dir(&dir).unwrap();
    println!("{n} stages dumped, {ok} of {all} translated");
    assert!(all > 0);
    assert_eq!(ok, all);
    let _ = std::fs::remove_dir_all(&dir);
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
