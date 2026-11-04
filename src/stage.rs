//! Stage a manifest of effect builds onto an .eff file: `effect_forge stage`. Gives the same bytes as
//! Visionary's `stage_carrier_from_manifest` test, which bake_slotted.py still drives.
#![allow(unused_variables, unused_mut)]

use std::path::Path;
use std::time::Instant;

pub fn stage_manifest(carrier: &str, root: &Path, builds_dir: &Path, manifest_rel: &str, out: &Path) {
    let started = Instant::now();
    macro_rules! println {
        ($($arg:tt)*) => { std::println!("[{:6.1}s] {}", started.elapsed().as_secs_f32(), format!($($arg)*)) };
    }
    use crate::eff::fields::AttrValue;
    use crate::eff::edits::{
        AuthoredEdit, EffMod, EmitterFieldEdits, TextureAddition, TextureImport, TransplantOp,
    };
    let builds_dir = builds_dir.to_path_buf();
    let root = root.to_path_buf();
    let manifest_path = builds_dir.join(manifest_rel);
    let mut manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("manifest reads"),
    )
    .expect("manifest parses");
    println!("manifest {}", manifest_path.display());
    // Generated builds (from the converter) keep their own files; their arrays join the
    // manifest's, so a converted effect is staged exactly like a hand-written one.
    let builds: Vec<String> = manifest["builds"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    for build in &builds {
        let part: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(builds_dir.join(build)).unwrap_or_else(|e| panic!("build {build}: {e}")),
        )
        .unwrap_or_else(|e| panic!("build {build} parses: {e}"));
        for (key, value) in part.as_object().expect("a build is an object") {
            let Some(items) = value.as_array() else { continue };
            let slot = manifest
                .as_object_mut()
                .unwrap()
                .entry(key.clone())
                .or_insert_with(|| serde_json::json!([]));
            slot.as_array_mut()
                .unwrap_or_else(|| panic!("manifest '{key}' is not a list"))
                .extend(items.iter().cloned());
        }
    }
    println!("builds: {}", builds.len());
    let manifest = manifest;
    let text = |v: &serde_json::Value, key: &str| -> String {
        v[key].as_str().unwrap_or_else(|| panic!("manifest: missing '{key}' in {v}")).to_string()
    };
    let list = |key: &str| manifest[key].as_array().cloned().unwrap_or_default();
    let carrier_rel = text(&manifest, "carrier_rel");
    let moves: Vec<(String, String, String)> =
        list("transplants").iter().map(|t| (text(t, "from"), text(t, "src"), text(t, "new"))).collect();
    let swaps: Vec<(String, String)> =
        list("textures").iter().map(|t| (text(t, "name"), text(t, "png"))).collect();
    let keep: Vec<String> =
        list("keep").iter().filter_map(|v| v.as_str().map(String::from)).collect();
    // A JSON integer is an Int, anything else numeric a Float; the setters convert either.
    let to_attr = |v: &serde_json::Value| -> AttrValue {
        match v.as_i64() {
            Some(i) => AttrValue::Int(i),
            None => AttrValue::Float(v.as_f64().expect("attribute value is a number") as f32),
        }
    };
    struct Retexture {
        entry: String,
        emitter: String,
        texture: String,
        template: String,
        png: String,
        attrs: Vec<(String, AttrValue)>,
    }
    let retextures: Vec<Retexture> = list("retexture")
        .iter()
        .map(|r| Retexture {
            entry: text(r, "entry"),
            emitter: text(r, "emitter"),
            texture: text(r, "texture"),
            template: text(r, "template"),
            png: text(r, "png"),
            attrs: r["attrs"]
                .as_object()
                .map(|o| o.iter().map(|(k, v)| (k.clone(), to_attr(v))).collect())
                .unwrap_or_default(),
        })
        .collect();
    struct Recolor {
        entries: Vec<String>,
        rgb: [f32; 3],
        /// Saturation floor: 0 keeps whites white; higher tints white-hot cores too.
        min_sat: f32,
        /// Brightness factor for normal-blend emitters -- how a body is inked dark while its
        /// additive glow keeps the hue. 1 leaves them alone.
        normal_value: f32,
        /// Ceiling on on-screen brightness, colour x the emitter's color_scale: an HDR key
        /// clips every channel toward white, so a boosted purple reads as white.
        max_brightness: f32,
        exclude: std::collections::HashMap<String, Vec<String>>,
        /// When an entry is listed here, only these of its emitters are recoloured.
        only: std::collections::HashMap<String, Vec<String>>,
    }
    let strings = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };
    let recolors: Vec<Recolor> = list("recolor")
        .iter()
        .map(|r| {
            let rgb = r["rgb"].as_array().expect("recolor: rgb");
            Recolor {
                entries: strings(&r["entries"]),
                rgb: [0, 1, 2].map(|i| rgb[i].as_f64().expect("recolor: rgb is numeric") as f32),
                min_sat: r["min_sat"].as_f64().unwrap_or(0.0) as f32,
                normal_value: r["normal_value"].as_f64().unwrap_or(1.0) as f32,
                max_brightness: r["max_brightness"].as_f64().map_or(f32::INFINITY, |v| v as f32),
                exclude: r["exclude"]
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), strings(v))).collect())
                    .unwrap_or_default(),
                only: r["only"]
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), strings(v))).collect())
                    .unwrap_or_default(),
            }
        })
        .collect();

    // (set index, set name, flat emitter names) for one entry.
    fn locate(bytes: &[u8], entry: &str) -> (usize, String, Vec<String>) {
        let namco = effect_library::NamcoEffectFile::load(bytes).expect("eff parses");
        let at = namco
            .entry_names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(entry))
            .unwrap_or_else(|| panic!("entry {entry} is missing"));
        let set_idx = (namco.entries[at].emitter_set_id as usize)
            .checked_sub(1)
            .unwrap_or_else(|| panic!("entry {entry} has no primary set"));
        let ptcl = namco.ptcl_file.as_ref().expect("ptcl");
        let set = &ptcl.emitter_list.emitter_sets[set_idx];
        let mut names = Vec::new();
        visit_emitters_ref(&set.emitters, &mut |em| names.push(em.data.display_name()));
        (set_idx, set.name.clone(), names)
    }

    let original = std::fs::read(&carrier).expect("carrier reads");
    let kept: Vec<(String, Vec<String>)> =
        keep.iter().map(|e| (e.clone(), locate(&original, e).2)).collect();

    // Phase 1: bring the effects in.
    let phase1 = EffMod {
        source_rel: carrier_rel.clone(),
        transplants: moves
            .iter()
            .map(|(from, src, new)| TransplantOp {
                new_entry_name: new.clone(),
                src_file_rel: from.clone(),
                src_set_name: src.clone(),
                src_set_idx: 0,
                one_slot_slots: Vec::new(),
                replace_entry: None,
            })
            .collect(),
        ..Default::default()
    };
    let cloned = crate::eff::writer::rebuild_eff_bytes_for_slot(&original, &phase1, Some(&root), None)
        .expect("phase 1: the effects come in");

    /// A colour moved toward `hue`, keeping its brightness AND its saturation: a white
    /// highlight stays white and a grey stays grey, and only what was already coloured
    /// changes colour. Brightness is the channel maximum, which also carries HDR keys (>1).
    /// `min_sat` raises the saturation to at least that, so whites take a pale tint of the
    /// hue too; `value` scales the brightness.
    fn toward_hue(c: [f32; 3], hue: [f32; 3], min_sat: f32, value: f32) -> [f32; 3] {
        let v = c[0].max(c[1]).max(c[2]);
        if v <= 1e-6 {
            return c;
        }
        let sat = ((v - c[0].min(c[1]).min(c[2])) / v).max(min_sat);
        let top = hue[0].max(hue[1]).max(hue[2]).max(1e-6);
        [0, 1, 2].map(|i| v * value * ((1.0 - sat) + sat * hue[i] / top))
    }
    let cloned_namco =
        effect_library::NamcoEffectFile::load(&cloned).expect("phase 1 output parses");
    let cloned_ptcl = cloned_namco.ptcl_file.as_ref().expect("ptcl");
    let mut recolor_edits: Vec<AuthoredEdit> = Vec::new();
    for r in &recolors {
        for entry in &r.entries {
            let (set_idx, set_name, names) = locate(&cloned, entry);
            let mut flat = Vec::new();
            visit_emitters_ref(
                &cloned_ptcl.emitter_list.emitter_sets[set_idx].emitters,
                &mut |em| flat.push(em.data.clone()),
            );
            let skip = r.exclude.get(entry).cloned().unwrap_or_default();
            for (idx, data) in flat.iter().enumerate() {
                if skip.contains(&names[idx]) {
                    continue;
                }
                if r.only.get(entry).is_some_and(|keep| !keep.contains(&names[idx])) {
                    continue;
                }
                let read = |id: &str| {
                    crate::eff::fields::index_of(id)
                        .and_then(|i| (crate::eff::fields::table()[i].get)(data))
                        .map(|v| v.as_f32())
                };
                // Additive black is invisible, so only a normal-blend emitter can be inked.
                let value = if read("render_state.blend_type") == Some(0.0) { r.normal_value } else { 1.0 };
                let boost = read("emitter_static.color_scale").unwrap_or(1.0).max(1e-6);
                let mut triplets: Vec<[String; 3]> = vec![
                    ["r", "g", "b"].map(|c| format!("particle_color.color0_{c}")),
                    ["r", "g", "b"].map(|c| format!("particle_color.color1_{c}")),
                ];
                // Only the live keys: the rest of the eight-slot table is inert padding.
                for slot in ["color0", "color1"] {
                    let live = read(&format!("emitter_static.num_{slot}_keys")).unwrap_or(0.0).max(0.0) as usize;
                    for k in 0..live.min(8) {
                        triplets.push(["x", "y", "z"].map(|c| format!("emitter_static.{slot}.keys[{k}].{c}")));
                    }
                }
                let mut attrs = std::collections::BTreeMap::new();
                for ids in &triplets {
                    let (Some(x), Some(y), Some(z)) = (read(&ids[0]), read(&ids[1]), read(&ids[2])) else {
                        continue;
                    };
                    let mut out = toward_hue([x, y, z], r.rgb, r.min_sat, value);
                    let peak = out[0].max(out[1]).max(out[2]) * boost;
                    if peak > r.max_brightness {
                        out = out.map(|c| c * r.max_brightness / peak);
                    }
                    for (k, (old, new)) in [x, y, z].into_iter().zip(out).enumerate() {
                        if (old - new).abs() > 1e-4 {
                            attrs.insert(ids[k].clone(), AttrValue::Float(new));
                        }
                    }
                }
                if !attrs.is_empty() {
                    recolor_edits.push(AuthoredEdit {
                        set_name: set_name.clone(),
                        entry_name: entry.clone(),
                        set_idx,
                        emitter_name: names[idx].clone(),
                        emitter_idx: idx,
                        fields: EmitterFieldEdits { attrs, ..Default::default() },
                    });
                }
            }
        }
    }

    // Retiming: named attributes multiplied on every emitter of the listed effects. A longer
    // particle life plays the whole keyframed animation slower -- keys sit at fractions of
    // life -- and slower velocities keep it covering the same ground.
    let mut tune_edits: Vec<AuthoredEdit> = Vec::new();
    for t in list("tune") {
        let mul: Vec<(String, f32)> = t["mul"]
            .as_object()
            .map(|o| {
                o.iter().map(|(k, v)| (k.clone(), v.as_f64().expect("tune: factor is numeric") as f32)).collect()
            })
            .unwrap_or_default();
        for entry in strings(&t["entries"]) {
            let (set_idx, set_name, names) = locate(&cloned, &entry);
            let mut flat = Vec::new();
            visit_emitters_ref(
                &cloned_ptcl.emitter_list.emitter_sets[set_idx].emitters,
                &mut |em| flat.push(em.data.clone()),
            );
            for (idx, data) in flat.iter().enumerate() {
                let mut attrs = std::collections::BTreeMap::new();
                for (id, k) in &mul {
                    let got = crate::eff::fields::index_of(id)
                        .and_then(|i| (crate::eff::fields::table()[i].get)(data))
                        .unwrap_or_else(|| panic!("tune: {entry}/{}: {id} unreadable", names[idx]));
                    let scaled = match got {
                        AttrValue::Int(v) => AttrValue::Int(((v as f32) * k).round() as i64),
                        AttrValue::Float(v) => AttrValue::Float(v * k),
                        AttrValue::UInt(_) => panic!("tune: {id} is an id, not a quantity"),
                    };
                    attrs.insert(id.clone(), scaled);
                }
                if !attrs.is_empty() {
                    tune_edits.push(AuthoredEdit {
                        set_name: set_name.clone(),
                        entry_name: entry.clone(),
                        set_idx,
                        emitter_name: names[idx].clone(),
                        emitter_idx: idx,
                        fields: EmitterFieldEdits { attrs, ..Default::default() },
                    });
                }
            }
        }
    }
    println!("tune: {} emitters retimed", tune_edits.len());

    // Inked art keeps its black core in the colour and the silhouette in alpha. A
    // normal-blend emitter reading alpha from RED cuts the core away and shows only the
    // light rim, so on the listed textures it reads the texture's own alpha instead.
    // (Additive emitters keep red: their black would be invisible either way.)
    let inked: std::collections::HashSet<String> =
        strings(&manifest["alpha_from_alpha"]["textures"]).into_iter().collect();
    let texture_name_of: std::collections::HashMap<u64, String> = cloned_ptcl
        .texture_info
        .as_ref()
        .map(|info| info.descriptors.iter().map(|d| (d.id, d.name.clone())).collect())
        .unwrap_or_default();
    // Entries a roster rebuilds lose these emitters, and an edit addressed to one would land
    // by index on whatever the roster put there instead.
    let rostered: Vec<String> = list("rosters")
        .iter()
        .filter_map(|r| r["entry"].as_str().map(|e| e.to_ascii_uppercase()))
        .collect();
    for (_, _, new) in &moves {
        if rostered.contains(&new.to_ascii_uppercase()) {
            continue;
        }
        let (set_idx, set_name, names) = locate(&cloned, new);
        let mut flat = Vec::new();
        visit_emitters_ref(&cloned_ptcl.emitter_list.emitter_sets[set_idx].emitters, &mut |em| {
            flat.push(em.data.clone())
        });
        for (idx, data) in flat.iter().enumerate() {
            let read = |id: &str| {
                crate::eff::fields::index_of(id)
                    .and_then(|i| (crate::eff::fields::table()[i].get)(data))
                    .map(|v| v.as_f32())
            };
            let samples_inked = data
                .sampler0
                .as_ref()
                .and_then(|s| texture_name_of.get(&s.texture_id))
                .is_some_and(|name| inked.contains(name));
            if samples_inked
                && read("combiner.tex_alpha0_input_type") == Some(1.0)
                && read("render_state.blend_type") == Some(0.0)
            {
                tune_edits.push(AuthoredEdit {
                    set_name: set_name.clone(),
                    entry_name: new.clone(),
                    set_idx,
                    emitter_name: names[idx].clone(),
                    emitter_idx: idx,
                    fields: EmitterFieldEdits {
                        attrs: [("combiner.tex_alpha0_input_type".to_string(), AttrValue::Int(0))].into(),
                        ..Default::default()
                    },
                });
                println!("alpha from alpha: {new}/{}", names[idx]);
            }
        }
    }

    // Assembled effects: each slot clones an emitter of another entry. Resolved to set names
    // here, where phase 1's sets exist; applied by the rebuild before any authored edit, so
    // edits below address the emitters by the names their slots give them.
    let mut rosters: Vec<crate::eff::edits::EmitterRoster> = Vec::new();
    let mut roster_names: std::collections::HashMap<String, Vec<String>> = Default::default();
    for r in list("rosters") {
        let entry = text(&r, "entry");
        let (set_idx, set_name, _) = locate(&cloned, &entry);
        let mut slots = Vec::new();
        let mut names = Vec::new();
        for s in r["slots"].as_array().expect("roster: slots") {
            let from = text(s, "from").replace("{p}", manifest["prefix"].as_str().unwrap_or("SHIGA"));
            let emitter = text(s, "emitter");
            let (_, from_set, from_names) = locate(&cloned, &from);
            let source_idx = from_names
                .iter()
                .position(|n| n == &emitter)
                .unwrap_or_else(|| panic!("roster {entry}: {from} has no emitter {emitter}: {from_names:?}"));
            let name = s["name"].as_str().map(String::from).unwrap_or_else(|| emitter.clone());
            names.push(name.clone());
            slots.push(crate::eff::edits::EmitterSlot {
                source_idx,
                source_name: emitter,
                name,
                depth: 0,
                source_set: from_set,
            });
        }
        assert!(!slots.is_empty(), "roster {entry} has no slots");
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "roster {entry} repeats a name: {names:?}");
        roster_names.insert(entry.to_ascii_uppercase(), names);
        rosters.push(crate::eff::edits::EmitterRoster {
            set_name,
            entry_name: entry,
            set_idx,
            slots,
        });
    }
    let names_after = |entry: &str| -> Vec<String> {
        roster_names
            .get(&entry.to_ascii_uppercase())
            .cloned()
            .unwrap_or_else(|| locate(&cloned, entry).2)
    };
    let mut edit_list: Vec<AuthoredEdit> = Vec::new();
    let mut edit_textures: Vec<(String, String, String)> = Vec::new();
    for e in list("edits") {
        let entry = text(&e, "entry");
        let emitter = text(&e, "emitter");
        let (set_idx, set_name, _) = locate(&cloned, &entry);
        let names = names_after(&entry);
        let emitter_idx = names
            .iter()
            .position(|n| n == &emitter)
            .unwrap_or_else(|| panic!("edit {entry}: no emitter {emitter}: {names:?}"));
        let texture = e["texture"].as_str().map(String::from);
        if let Some(texture) = &texture {
            if !edit_textures.iter().any(|(t, _, _)| t == texture) {
                edit_textures.push((texture.clone(), text(&e, "template"), text(&e, "png")));
            }
        }
        edit_list.push(AuthoredEdit {
            set_name,
            entry_name: entry,
            set_idx,
            emitter_name: emitter,
            emitter_idx,
            fields: EmitterFieldEdits {
                texture_name: texture,
                attrs: e["attrs"]
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), to_attr(v))).collect())
                    .unwrap_or_default(),
                ..Default::default()
            },
        });
    }
    // Aliases: effects authored as copies of one another point at one emitter set rather
    // than carrying their own, so the name still spawns and the file holds it once.
    let entry_edits: Vec<crate::eff::edits::EntryEdit> = list("entry_edits")
        .iter()
        .map(|e| crate::eff::edits::EntryEdit {
            entry_name: text(e, "entry"),
            emitter_set: e["emitter_set"].as_str().map(String::from),
            variants: None,
            model: None,
        })
        .collect();
    println!(
        "assembled: {} rosters, {} emitter edits, {} textures, {} aliases",
        rosters.len(), edit_list.len(), edit_textures.len(), entry_edits.len()
    );

    // Phase 2: the art, addressed to where phase 1 put things.
    let mut added: Vec<TextureAddition> = Vec::new();
    for (texture, template, png) in &edit_textures {
        let png = std::path::Path::new(png);
        added.push(TextureAddition {
            texture_name: texture.clone(),
            template_name: template.clone(),
            png_path: if png.is_absolute() { png.to_path_buf() } else { builds_dir.join(png) }
                .to_string_lossy()
                .into_owned(),
            raw: false,
        });
    }
    for r in &retextures {
        if !added.iter().any(|t| t.texture_name == r.texture) {
            added.push(TextureAddition {
                texture_name: r.texture.clone(),
                template_name: r.template.clone(),
                png_path: builds_dir.join(&r.png).to_string_lossy().into_owned(),
                raw: false,
            });
        }
    }
    let retexture_edits: Vec<AuthoredEdit> = retextures
        .iter()
        .map(|r| {
            let (set_idx, set_name, names) = locate(&cloned, &r.entry);
            AuthoredEdit {
                set_name,
                entry_name: r.entry.clone(),
                set_idx,
                emitter_name: r.emitter.clone(),
                emitter_idx: names
                    .iter()
                    .position(|n| n == &r.emitter)
                    .unwrap_or_else(|| panic!("{} has no emitter {}: {names:?}", r.entry, r.emitter)),
                fields: EmitterFieldEdits {
                    texture_name: Some(r.texture.clone()),
                    attrs: r.attrs.iter().cloned().collect(),
                    ..Default::default()
                },
            }
        })
        .collect();
    // Recolours first, so a retexture's explicit colours win on any emitter both touch.
    let mut authored = recolor_edits.clone();
    authored.extend(retexture_edits);
    authored.extend(tune_edits);
    authored.extend(edit_list);
    let phase2 = EffMod {
        source_rel: carrier_rel.clone(),
        textures: swaps
            .iter()
            .map(|(name, png)| TextureImport {
                texture_name: name.clone(),
                png_path: builds_dir.join(png).to_string_lossy().into_owned(),
                raw: false,
            })
            .collect(),
        textures_added: added,
        authored,
        rosters,
        entry_edits,
        ..Default::default()
    };
    let mut built = crate::eff::writer::rebuild_eff_bytes_for_slot(&cloned, &phase2, None, None)
        .expect("phase 2: the art applies");

    // Meshes last: a new primitive per source mesh, pointed at by the emitters that draw it. Builds
    // name a mesh by mesh and template, so the same source mesh from many effects is one
    // primitive with all their emitters as targets.
    let mut mesh_list: Vec<serde_json::Value> = Vec::new();
    for m in list("meshes") {
        match mesh_list.iter_mut().find(|seen| seen["name"] == m["name"]) {
            Some(seen) => {
                let more = m["targets"].as_array().cloned().unwrap_or_default();
                seen["targets"].as_array_mut().expect("mesh: targets").extend(more);
            }
            None => mesh_list.push(m),
        }
    }
    for m in mesh_list {
        let template = text(&m, "template");
        let name = text(&m, "name");
        let glb_path = builds_dir.join(text(&m, "glb"));
        let glb = std::fs::read(&glb_path).unwrap_or_else(|e| panic!("mesh {}: {e}", glb_path.display()));
        let mesh = crate::eff::mesh_io::from_gltf(&glb).unwrap_or_else(|e| panic!("mesh {name}: {e}"));
        let targets: Vec<(String, String)> = m["targets"]
            .as_array()
            .expect("mesh: targets")
            .iter()
            .map(|t| (t[0].as_str().unwrap().to_string(), t[1].as_str().unwrap().to_string()))
            .collect();
        built = crate::eff::mesh_io::add_primitive_for_emitters(&built, &template, &name, &mesh, &targets)
            .unwrap_or_else(|e| panic!("mesh {name}: {e:#}"));
        println!("mesh {name}: {} verts for {} emitters", mesh.positions.len(), targets.len());
    }

    // One character per file: everything outside its prefixes is emptied and pruned.
    let keep_prefixes: Vec<String> = strings(&manifest["keep_prefixes"]);
    if !keep_prefixes.is_empty() {
        let namco = effect_library::NamcoEffectFile::load(&built).expect("built eff parses");
        let keep_names: Vec<String> = namco
            .entry_names
            .iter()
            .filter(|n| keep_prefixes.iter().any(|p| n.to_ascii_uppercase().starts_with(&p.to_ascii_uppercase())))
            .cloned()
            .collect();
        let refs: Vec<&str> = keep_names.iter().map(String::as_str).collect();
        let before = built.len();
        built = crate::eff::writer::strip_donor_eff_bytes(&built, &refs).expect("stripping to this character's effects");
        println!("kept {} entries under {keep_prefixes:?}: {before} -> {} bytes", keep_names.len(), built.len());
    }

    let namco = effect_library::NamcoEffectFile::load(&built).expect("staged eff parses");
    let ptcl = namco.ptcl_file.as_ref().expect("ptcl");
    let primitives: std::collections::HashSet<u64> = ptcl
        .primitive_info
        .as_ref()
        .map(|info| info.descriptors.iter().map(|d| d.id).collect())
        .unwrap_or_default();
    let texture_ids: std::collections::HashMap<String, u64> = ptcl
        .texture_info
        .as_ref()
        .map(|info| info.descriptors.iter().map(|d| (d.name.clone(), d.id)).collect())
        .unwrap_or_default();
    let mut donors: std::collections::HashMap<String, Vec<u8>> = Default::default();

    for (from, src, new) in &moves {
        let (set_idx, _, names) = locate(&built, new);
        // Same-file clones (assembled effects and aliases) have no donor file to compare to.
        if !from.is_empty() && !roster_names.contains_key(&new.to_ascii_uppercase()) {
            let donor = donors
                .entry(from.clone())
                .or_insert_with(|| std::fs::read(root.join(from)).expect("donor reads"));
            assert_eq!(names, locate(donor, src).2, "{new} did not come across whole");
        }
        let mut missing = Vec::new();
        visit_emitters_ref(&ptcl.emitter_list.emitter_sets[set_idx].emitters, &mut |em| {
            let id = em.data.particle_data.primitive_id;
            if id != 0 && id != u64::MAX && !primitives.contains(&id) {
                missing.push(format!("{}:{id}", em.data.display_name()));
            }
        });
        assert!(missing.is_empty(), "{new}: meshes missing from the carrier: {missing:?}");
    }
    for (name, _) in &swaps {
        // A replaced texture only this character's stripped effects used is pruned with them.
        assert!(
            texture_ids.contains_key(name) || !keep_prefixes.is_empty(),
            "{name} is not in the staged pool"
        );
    }
    let mut read_back = 0usize;
    for r in &retextures {
        let (set_idx, _, names) = locate(&built, &r.entry);
        let mut flat = Vec::new();
        visit_emitters_ref(&ptcl.emitter_list.emitter_sets[set_idx].emitters, &mut |em| {
            flat.push(em.data.clone())
        });
        let data = &flat[names.iter().position(|n| n == &r.emitter).unwrap()];
        for (id, want) in &r.attrs {
            let got = crate::eff::fields::index_of(id)
                .and_then(|i| (crate::eff::fields::table()[i].get)(data))
                .unwrap_or_else(|| panic!("{}/{}: {id} unreadable", r.entry, r.emitter));
            assert!(
                (got.as_f32() - want.as_f32()).abs() < 1e-4,
                "{}/{}: {id} is {got:?}, wanted {want:?}",
                r.entry,
                r.emitter
            );
            read_back += 1;
        }
        assert_eq!(
            data.sampler0.as_ref().map(|s| s.texture_id),
            texture_ids.get(&r.texture).copied(),
            "{}/{} does not sample {}",
            r.entry,
            r.emitter,
            r.texture
        );
    }
    for (entry, before) in &kept {
        assert_eq!(&locate(&built, entry).2, before, "{entry} changed");
    }

    println!(
        "\nbatch staged: {} -> {} bytes | {} effects in ({}) | {} textures replaced | {} emitters retextured, {read_back} values read back | meshes all resolve | {} kept entries unchanged",
        original.len(),
        built.len(),
        moves.len(),
        moves.iter().map(|(_, _, n)| n.as_str()).collect::<Vec<_>>().join(", "),
        swaps.len(),
        retextures.len(),
        kept.len()
    );
    let retextured: std::collections::HashSet<(String, String)> =
        retextures.iter().map(|r| (r.entry.clone(), r.emitter.clone())).collect();
    let mut recolored = 0usize;
    for edit in &recolor_edits {
        if retextured.contains(&(edit.entry_name.clone(), edit.emitter_name.clone())) {
            continue;
        }
        let (set_idx, _, _) = locate(&built, &edit.entry_name);
        let mut flat = Vec::new();
        visit_emitters_ref(&ptcl.emitter_list.emitter_sets[set_idx].emitters, &mut |em| {
            flat.push(em.data.clone())
        });
        let data = &flat[edit.emitter_idx];
        for (id, want) in &edit.fields.attrs {
            let got = crate::eff::fields::index_of(id)
                .and_then(|i| (crate::eff::fields::table()[i].get)(data))
                .unwrap_or_else(|| panic!("{}/{}: {id} unreadable", edit.entry_name, edit.emitter_name));
            assert!(
                (got.as_f32() - want.as_f32()).abs() < 1e-4,
                "{}/{}: {id} is {got:?}, recolour wanted {want:?}",
                edit.entry_name,
                edit.emitter_name
            );
            recolored += 1;
        }
    }
    println!("recolour: {} emitters changed, {recolored} values read back", recolor_edits.len());

    {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).expect("staging dir");
        }
        std::fs::write(out, &built).expect("staged file writes");
        let record = serde_json::json!({ "phase1_effects": &phase1, "phase2_art": &phase2 });
        let record_path = manifest_path.with_file_name("batch.effmod.json");
        std::fs::write(&record_path, serde_json::to_string_pretty(&record).unwrap())
            .expect("record writes");
        println!("wrote {}\nrecorded ops in {}", out.display(), record_path.display());
    }}

fn visit_emitters_ref<F: FnMut(&effect_library::structs::Emitter)>(emitters: &[effect_library::structs::Emitter], f: &mut F) {
    for em in emitters {
        f(em);
        visit_emitters_ref(&em.children, f);
    }
}
