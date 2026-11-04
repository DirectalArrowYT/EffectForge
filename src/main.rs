//! Effect Forge.

mod draw;
mod eff;
mod scratch_dirs;
mod app;
mod engine;
mod log;
mod preview;
mod stage;

use std::path::Path;

fn main() {
    log::install();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [cmd, carrier, root, builds_dir, manifest, out] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        if *cmd == "stage" {
            stage::stage_manifest(carrier, Path::new(root), Path::new(builds_dir), manifest, Path::new(out));
            return;
        }
    }
    if let ["info", eff, wanted] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let (_, set) = doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(wanted)).expect("that effect");
        for (i, game) in doc.loaded.game[set].iter().enumerate() {
            let Some(game) = game else { continue };
            let emitter = &doc.emitters(set)[i];
            let names = game.samplers.map(|s| s.and_then(|(t, _)| doc.loaded.ptcl.bntx_textures.get(t)).map(|t| t.tex_name.clone()));
            println!("{} shader {} textures {names:?} blend {} depth_mask {}", emitter.name, game.shader_index, game.stored[0x89e], game.stored[0x89b]);
            let word = |n: usize| f32::from_le_bytes(game.block[n * 4..n * 4 + 4].try_into().unwrap());
            println!("    uv {:?}", (184..204).map(word).collect::<Vec<_>>());
            println!("    colour0 keys {:?}", (240..256).map(word).collect::<Vec<_>>());
            println!("    colour1 keys {:?}", (304..320).map(word).collect::<Vec<_>>());
            println!("    colour scale {} loops {:?} alpha keys {:?}", word(236), (32..38).map(word).collect::<Vec<_>>(), (272..304).map(word).collect::<Vec<_>>());
            if let Some((t, _)) = game.samplers[0] {
                let name = &doc.loaded.ptcl.bntx_textures[t].tex_name;
                let sampled = eff::textures::decode_for_sampling(doc.loaded.texture_pool.as_deref().unwrap_or_default(), t, name).unwrap();
                let top = &sampled.levels[0];
                let mut sum = [0f64; 4];
                let mut max = [0u8; 4];
                for pixel in top.pixels() {
                    for c in 0..4 {
                        sum[c] += pixel.0[c] as f64 / 255.0;
                        max[c] = max[c].max(pixel.0[c]);
                    }
                }
                println!("    texture mean {:?} max {max:?} srgb {}", sum.map(|v| v / top.pixels().len() as f64), sampled.srgb);
            }
        }
        return;
    }
    if let ["colour2", eff, out, wanted, layer, r, g, b] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let (_, set) = doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(wanted)).expect("that effect");
        let index = doc.emitters(set).iter().position(|e| e.name == *layer).expect("that layer");
        let mut attrs = std::collections::BTreeMap::new();
        for (i, value) in [r, g, b].into_iter().enumerate() {
            let value = eff::fields::AttrValue::Float(value.parse().unwrap());
            attrs.insert(format!("particle_color.color1_{}", ["r", "g", "b"][i]), value);
            attrs.insert(format!("emitter_static.color1.keys[0].{}", ["x", "y", "z"][i]), value);
        }
        attrs.insert("particle_color.color1_type".to_string(), eff::fields::AttrValue::Int(2));
        attrs.insert("emitter_static.num_color1_keys".to_string(), eff::fields::AttrValue::Int(1));
        let doc = doc.apply_values(&engine::Pending::from([((set, index), attrs)])).unwrap();
        std::fs::write(out, &doc.bytes).unwrap();
        return;
    }
    // A value on every layer of an effect, to try something in game: set <eff> <out> <effect> <attribute> <value>
    if let ["set", eff, out, wanted, id, value] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let (_, set) = doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(wanted)).expect("that effect");
        let mut pending = engine::Pending::new();
        for (index, emitter) in doc.emitters(set).iter().enumerate() {
            let new = match engine::attr(emitter, id).expect("that attribute") {
                eff::fields::AttrValue::Float(_) => eff::fields::AttrValue::Float(value.parse().unwrap()),
                eff::fields::AttrValue::Int(_) => eff::fields::AttrValue::Int(value.parse().unwrap()),
                eff::fields::AttrValue::UInt(_) => eff::fields::AttrValue::UInt(value.parse().unwrap()),
            };
            pending.entry((set, index)).or_default().insert(id.to_string(), new);
        }
        std::fs::write(out, &doc.apply_values(&pending).unwrap().bytes).unwrap();
        return;
    }
    // Give every layer of an effect a shader built for a mode, as the panel does: borrow <eff> <out> <effect> <attr> <value>
    if let ["borrow", eff, out, wanted, id, value] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let (entry, set) = doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(wanted)).expect("that effect");
        let value: f32 = value.parse().unwrap();
        let mut pending = engine::Pending::new();
        for (index, emitter) in doc.emitters(set).iter().enumerate() {
            let current = |id: &str| engine::attr(emitter, id).map(|v| v.as_f32());
            if current(id) == Some(value) {
                println!("{}: already", emitter.name);
                continue;
            }
            match doc.shader_donor(&entry, emitter, &[(id, value)], current) {
                Some(d) => {
                    println!("{}: shader {} from {} / {}", emitter.name, d.shader_index, d.effect, d.layer);
                    let row = pending.entry((set, index)).or_default();
                    for (key, v) in [("shader_references.shader_index", d.shader_index), (id, value)] {
                        let new = match engine::attr(emitter, key).unwrap() {
                            eff::fields::AttrValue::Float(_) => eff::fields::AttrValue::Float(v),
                            eff::fields::AttrValue::Int(_) => eff::fields::AttrValue::Int(v as i64),
                            eff::fields::AttrValue::UInt(_) => eff::fields::AttrValue::UInt(v as u64),
                        };
                        row.insert(key.to_string(), new);
                    }
                }
                None => println!("{}: no donor", emitter.name),
            }
        }
        std::fs::write(out, &doc.apply_values(&pending).unwrap().bytes).unwrap();
        return;
    }
    // After a rebuild: put the hand-tuned effects back. merge-tuned <tuned.eff> <built.eff>, in place.
    if let ["merge-tuned", tuned, into] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let bytes = std::fs::read(into).expect("the built file");
        match engine::merge_tuned(Path::new(tuned), &bytes) {
            Ok((merged, count)) => {
                std::fs::write(into, merged).expect("writing the built file");
                println!("{count} tuned effects put into {into}");
            }
            Err(e) => {
                eprintln!("merging tuned effects failed: {e:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    // stage-effect <built.eff> <effect>: copy one effect into its tuned file, as the window's button does.
    if let ["stage-effect", eff, wanted] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let (entry, set) = doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(wanted)).expect("that effect");
        let path = engine::Tuned::path_for(Path::new(eff)).expect("a git repo above the file");
        let tuned = doc.stage(&entry, set, &path).unwrap();
        println!("{} now keeps {:?}", path.display(), tuned.effects);
        return;
    }
    // Values across a whole file, to see how the game's own effects use a field: attrs <eff> <id,id,...> [id=value filter]
    if let ["attrs", eff, ids, rest @ ..] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let filter = rest.first().and_then(|f| f.split_once('='));
        for (name, set) in doc.effects() {
            for emitter in doc.emitters(set) {
                let value = |id: &str| engine::attr(emitter, id).map(|v| v.as_f32());
                if filter.is_some_and(|(id, want)| value(id) != want.parse().ok()) {
                    continue;
                }
                let values: Vec<String> = ids.split(',').map(|id| format!("{id}={:?}", value(id))).collect();
                println!("{name} / {} : {}", emitter.name, values.join(" "));
            }
        }
        return;
    }
    // Every emitter's values as JSON lines, for studying the game's own files: dump <eff> <out.jsonl>
    if let ["dump", eff, out] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let table = eff::fields::table();
        let tex = |i: Option<u32>| i.and_then(|i| doc.loaded.ptcl.bntx_textures.get(i as usize)).map(|t| t.tex_name.clone());
        let mut lines = String::new();
        for (name, set) in doc.effects() {
            for emitter in doc.emitters(set) {
                let mut values = serde_json::Map::new();
                for (attr, value) in table.iter().zip(&emitter.attrs) {
                    if let Some(v) = value {
                        let v = match *v {
                            eff::fields::AttrValue::Float(f) => serde_json::json!(f),
                            eff::fields::AttrValue::Int(i) => serde_json::json!(i),
                            eff::fields::AttrValue::UInt(u) => serde_json::json!(u),
                        };
                        values.insert(attr.id.to_string(), v);
                    }
                }
                let row = serde_json::json!({
                    "effect": name, "emitter": emitter.name, "depth": emitter.depth,
                    "tex0": tex(emitter.texture_index), "tex1": tex(emitter.texture_index1),
                    "primitive": doc.primitive_of(&name, &emitter.name),
                    "sections": emitter.subsections.iter().map(|s| s.magic.clone()).collect::<Vec<_>>(),
                    "v": values,
                });
                lines.push_str(&row.to_string());
                lines.push('\n');
            }
        }
        std::fs::write(out, lines).unwrap();
        return;
    }
    if let ["check", eff, png, effect] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        check(eff, png, effect);
        return;
    }
    let sweep = args.first().is_some_and(|a| a == "sweep");
    // shot <eff> <effect> <out.png> <frame> [layer number, o<number> to outline it, or - for all] [camera turn] [camera tilt] [half view] [swing]
    let shot = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["shot", _, _, out, frame, rest @ ..] => Some(app::Shot {
            out: out.into(),
            frame: frame.parse().expect("a frame number"),
            layer: rest.first().filter(|l| **l != "-" && !l.starts_with('o')).map(|l| l.parse().expect("a layer number")),
            outline: rest.first().and_then(|l| l.strip_prefix('o')).map(|l| l.parse().expect("a layer number")),
            orbit: [rest.get(1), rest.get(2)].map(|v| v.map_or(0.0, |v| v.parse().expect("an angle"))),
            view: rest.get(3).map_or(preview::HALF_VIEW, |v| v.parse().expect("a view size")),
            swing: rest.get(4).is_some_and(|v| *v == "swing"),
            settled: 0,
        }),
        _ => None,
    };
    let args = if sweep || shot.is_some() { &args[1..] } else { &args[..] };
    let open = args.first().map(std::path::PathBuf::from);
    let mut gpu = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut gpu.wgpu_setup {
        // The game's shaders take up to ten vertex streams, and wgpu allows eight unless asked.
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut descriptor = base(adapter);
            descriptor.required_limits.max_vertex_buffers = adapter.limits().max_vertex_buffers;
            descriptor
        });
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title("Effect Forge").with_inner_size([1400.0, 860.0]),
        wgpu_options: gpu,
        ..Default::default()
    };
    eframe::run_native("Effect Forge", options, Box::new(|cc| Ok(Box::new(app::App::new(open, args.get(1).cloned(), sweep, shot, cc))))).expect("the window opens");
}

/// Run one of every edit on a copy of a file, and read each back.
fn check(eff: &str, png: &str, effect: &str) {
    use engine::{Doc, Pending, Target};
    let timed = |label: &str, started: std::time::Instant| println!("{label:<18} {:.1}s", started.elapsed().as_secs_f32());
    let copy = std::env::temp_dir().join("forge_check.eff");
    std::fs::copy(eff, &copy).unwrap();
    let t = std::time::Instant::now();
    let doc = Doc::open(&copy).unwrap();
    timed("open", t);
    let (entry, set_idx) = doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(effect)).expect("the effect is in the file");
    let target = Target { entry: entry.clone(), set_idx, set_name: doc.loaded.ptcl.emitter_sets[set_idx].name.clone(), emitter_idx: 0,
        emitter_name: doc.emitters(set_idx)[0].name.clone() };
    let mut pending = Pending::new();
    pending.entry((set_idx, 0)).or_default().insert("particle_scale.scale_x".into(), eff::fields::AttrValue::Float(0.5));
    let t = std::time::Instant::now();
    let doc = doc.apply_values(&pending).unwrap();
    timed("value", t);
    println!("  scale reads {:?}", engine::attr(&doc.emitters(set_idx)[0], "particle_scale.scale_x"));
    let t = std::time::Instant::now();
    let doc = doc.replace_texture(&target, Path::new(png)).unwrap();
    timed("texture", t);
    println!("  texture index {:?}", doc.emitters(set_idx)[0].texture_index.map(|i| doc.loaded.ptcl.bntx_textures[i as usize].tex_name.clone()));
    let mut mesh = doc.mesh(doc.primitive_of(&entry, &target.emitter_name).unwrap()).unwrap();
    engine::quarter_turn(&mut mesh, 1);
    let t = std::time::Instant::now();
    let doc = doc.replace_mesh(&target, &mesh).unwrap();
    timed("model", t);
    println!("  primitive {:?}", doc.primitive_of(&entry, &target.emitter_name));
    let t = std::time::Instant::now();
    let doc = doc.duplicate(&entry, set_idx, "forge_check_copy").unwrap();
    timed("duplicate", t);
    println!("  found {}", doc.effects().iter().any(|(n, _)| n.eq_ignore_ascii_case("forge_check_copy")));
    let t = std::time::Instant::now();
    doc.save().unwrap();
    timed("save", t);
}
