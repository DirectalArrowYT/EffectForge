//! Effect Forge.

mod draw;
mod eff;
mod scratch_dirs;
mod app;
mod engine;
mod log;
mod preview;
mod project;
mod settings;
mod stage;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

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
            println!("{} shader {} textures {names:?} blend {} depth_mask {}", emitter.name, game.shader_index, game.blend_type, game.depth_mask as u8);
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
        std::fs::write(out, &doc.set_on_effect(wanted, id, value).unwrap().bytes).unwrap();
        return;
    }
    // Give every layer of an effect a shader built for a mode, as the panel does: borrow <eff> <out> <effect> <attr> <value>
    if let ["borrow", eff, out, wanted, id, value] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let (doc, report) = doc.borrow_on_effect(wanted, id, value.parse().unwrap()).unwrap();
        for line in report {
            println!("{line}");
        }
        std::fs::write(out, &doc.bytes).unwrap();
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
    // Making an effect from the starter kit, as the window does; each writes <out>:
    //   new-effect <eff> <out> <new name> <piece> [model.glb]    add-layer <eff> <out> <effect> <piece> [model.glb]
    //   remove-layer <eff> <out> <effect> <layer number>    duplicate-layer <eff> <out> <effect> <layer number>    kit (lists the pieces)
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["kit"] => {
            for piece in engine::kit() {
                println!("{:<8} {:<16} {} / {} {:?}", piece.group, piece.name, piece.file, piece.effect, piece.layers);
            }
            return;
        }
        [cmd @ ("new-effect" | "add-layer" | "remove-layer" | "duplicate-layer"), eff, out, name, what, glb @ ..] if glb.len() <= 1 => {
            let doc = engine::Doc::open(Path::new(eff)).unwrap();
            let model = glb.first().map(|p| eff::mesh_io::from_gltf(&std::fs::read(p).expect("the model reads")).expect("a glTF model"));
            let piece = || engine::kit().iter().find(|p| p.name.eq_ignore_ascii_case(what)).expect("a piece of the kit (see: kit)");
            let effect = || doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).expect("that effect");
            let made = match *cmd {
                "new-effect" => doc.new_effect(name, piece(), model.as_ref()),
                "add-layer" => doc.add_layer(&effect().0, piece(), model.as_ref()),
                "duplicate-layer" => {
                    let (entry, set) = effect();
                    doc.duplicate_layer(&entry, set, what.parse().expect("a layer number")).map(|(made, _)| made)
                }
                _ => {
                    let (entry, set) = effect();
                    doc.remove_layer(&entry, set, what.parse().expect("a layer number"))
                }
            };
            match made {
                Ok(made) => std::fs::write(out, &made.bytes).unwrap(),
                Err(e) => {
                    eprintln!("{cmd} failed: {e:#}");
                    std::process::exit(1);
                }
            }
            return;
        }
        ["rename", eff, out, old, new] => {
            match engine::Doc::open(Path::new(eff)).and_then(|doc| doc.rename(old, new)) {
                Ok(doc) => std::fs::write(out, &doc.bytes).unwrap(),
                Err(e) => {
                    eprintln!("rename failed: {e:#}");
                    std::process::exit(1);
                }
            }
            return;
        }
        // A project, as the window keeps it: project <file.forge> <from.eff> [export to]
        ["project", forge, from, rest @ ..] => {
            let bytes = std::fs::read(from).expect("the file to start from");
            let export_to = rest.first().map(PathBuf::from).or_else(|| Some(PathBuf::from(from)));
            let made = project::Project::create(Path::new(forge), &bytes, export_to).unwrap();
            println!("{} holds {}", made.path.display(), made.eff_path().display());
            return;
        }
        // folder <file.forge> <folder> <effect...>: makes the folder if needed and puts the effects in it
        ["folder", forge, folder, effects @ ..] => {
            let mut made = project::Project::load(Path::new(forge)).unwrap();
            let at = match made.folders.iter().position(|f| f.name.eq_ignore_ascii_case(folder)) {
                Some(at) => at,
                None => {
                    made.add_folder(folder).unwrap();
                    made.folders.len() - 1
                }
            };
            for effect in effects.iter() {
                made.move_to(effect, Some(at));
            }
            made.save().unwrap();
            return;
        }
        // export <file.forge> [to]: as Export does, leaving out the UNUSED_ folders
        ["export", forge, rest @ ..] => {
            let made = project::Project::load(Path::new(forge)).unwrap();
            let to = rest.first().map(PathBuf::from).or(made.export_to.clone()).expect("somewhere to export to");
            let bytes = std::fs::read(made.eff_path()).unwrap();
            match project::export(&bytes, &made.left_out(), &to) {
                Ok(skipped) => println!("exported to {} ({skipped} left out)", to.display()),
                Err(e) => {
                    eprintln!("export failed: {e:#}");
                    std::process::exit(1);
                }
            }
            return;
        }
        _ => {}
    }
    // Every effect of a file with its kind and its layers, to pick starter layers from: layers <eff> [effect]
    if let ["layers", eff, rest @ ..] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let bytes = std::fs::read(eff).expect("the file");
        let file = effect_library::NamcoEffectFile::load(&bytes).expect("an effect file");
        let doc = engine::Doc::open(Path::new(eff)).unwrap();
        let tex = |i: Option<u32>| i.and_then(|i| doc.loaded.ptcl.bntx_textures.get(i as usize)).map(|t| t.tex_name.clone());
        for (name, header) in file.entry_names.iter().zip(&file.entries) {
            if rest.first().is_some_and(|w| !name.eq_ignore_ascii_case(w)) {
                continue;
            }
            let Some(set) = (header.emitter_set_id as usize).checked_sub(1) else { continue };
            println!("{name} kind {:#06x} parts {} set {}", header.kind, header.variant_count, doc.loaded.ptcl.emitter_sets[set].name);
            for em in doc.emitters(set) {
                println!("  {}{} tex {:?} mesh {:?}", "  ".repeat(em.depth as usize), em.name, tex(em.texture_index), doc.primitive_of(name, &em.name));
            }
        }
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
                    "sections": emitter.sections,
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
    // trail-shaders <eff> [<out.eff> [<effect>]]: layers whose shader doesn't match how they follow their bone; with
    // <out>, fixed
    if let ["trail-shaders", eff, rest @ ..] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).expect("the file opens");
        let started = std::time::Instant::now();
        let (fixed, found) = match rest {
            [] => (None, doc.trail_mismatches(None)),
            [_, only @ ..] => {
                let (fixed, found) = doc.fix_trail_shaders(only.first().copied()).expect("fixing");
                (Some(fixed), found)
            }
        };
        for m in &found {
            println!("{:<40} {:<28} follow {} shader {:>4}: {}", m.effect, m.name, m.follow_type, m.shader, m.fix.as_deref().unwrap_or("-"));
        }
        let fixable = found.iter().filter(|m| m.fix.is_some()).count();
        println!("{} layers don't match their shader", found.len());
        if let (Some(fixed), [out, ..]) = (fixed, rest) {
            std::fs::write(out, &fixed.bytes).expect("writing");
            let left = fixed.trail_mismatches(None).len();
            println!("{fixable} fixed in {:.1}s, wrote {out}; {left} left in it", started.elapsed().as_secs_f32());
        }
        return;
    }
    // value <eff> <out.eff> <EFFECT> <layer number> <id=value>...: the panel's values on one layer, applied together
    // texture <eff> <out.eff> <EFFECT> <layer number> <png>, model <eff> <out.eff> <EFFECT> <layer number> <glb>: the
    // Swap texture and Swap model buttons
    if let [cmd @ ("value" | "texture" | "model"), eff, out, effect, layer, rest @ ..] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        if !rest.is_empty() {
            let doc = engine::Doc::open(Path::new(eff)).expect("the file opens");
            let (entry, set_idx) = doc.find(effect).expect("the effect is in the file");
            let layer = layer.parse::<usize>().expect("a layer number") - 1;
            let em = doc.emitters(set_idx).get(layer).expect("that layer");
            let target = engine::Target { entry: entry.clone(), set_idx, set_name: doc.loaded.ptcl.emitter_sets[set_idx].name.clone(),
                emitter_idx: layer, emitter_name: em.name.clone() };
            let doc = match *cmd {
                "value" => {
                    let mut values = std::collections::BTreeMap::new();
                    for pair in rest {
                        let (id, v) = pair.split_once('=').expect("id=value");
                        let like = engine::attr(em, id).unwrap_or_else(|| panic!("{} has no field {id}", em.name));
                        values.insert(id.to_string(), engine::typed(like, v.parse().expect("a number")));
                    }
                    doc.apply_values(&engine::Pending::from([((set_idx, layer), values)]))
                }
                "texture" => doc.replace_texture(&target, Path::new(rest[0])),
                _ => std::fs::read(rest[0]).map_err(anyhow::Error::from).and_then(|b| eff::mesh_io::from_gltf(&b))
                    .and_then(|mesh| doc.replace_mesh(&target, &mesh)),
            }.unwrap_or_else(|e| panic!("{cmd}: {e:#}"));
            std::fs::write(out, &doc.bytes).expect("writing");
            return;
        }
    }
    // style <eff> <out.eff> <EFFECT> <layer number> <style label> <on|off>: the Draw style boxes
    if let ["style", eff, out, effect, layer, label, on] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let doc = engine::Doc::open(Path::new(eff)).expect("the file opens");
        let (entry, set_idx) = doc.find(effect).expect("the effect is in the file");
        let style = engine::STYLES.iter().find(|s| s.label.eq_ignore_ascii_case(label)).expect("a style: Soft edges, Rim fade or Heat haze");
        let value = if *on == "on" { style.on } else { 0.0 };
        let (doc, from) = doc.restyle(&entry, set_idx, layer.parse::<usize>().expect("a layer number") - 1, style, value).expect("restyling");
        match from {
            Some(r) => println!("{} {on}: shader from {}; took {:?}", style.label, r.from, r.taken),
            None => println!("no shader in this file or the game's draws it that way; nothing changed"),
        }
        std::fs::write(out, &doc.bytes).expect("writing");
        return;
    }
    // import <into.eff> <from.eff> <out.eff> [replace] <EFFECT or PREFIX*...>
    if let ["import", into, from, out, rest @ ..] = args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        let replace = rest.first() == Some(&"replace");
        let wanted = if replace { &rest[1..] } else { rest };
        let doc = engine::Doc::open(Path::new(into)).expect("the file opens");
        let source = engine::Doc::open(Path::new(from)).expect("the source opens");
        let names: Vec<String> = source.effects().into_iter().map(|(n, _)| n)
            .filter(|n| wanted.iter().any(|w| match w.strip_suffix('*') {
                Some(prefix) => n.to_ascii_uppercase().starts_with(&prefix.to_ascii_uppercase()),
                None => n.eq_ignore_ascii_case(w),
            }))
            .collect();
        let started = std::time::Instant::now();
        match doc.import(Path::new(from), &names, replace) {
            Ok((doc, kept)) => {
                std::fs::write(out, &doc.bytes).expect("writing the file");
                println!("imported {} of {} effects in {:.1}s; kept {} already there", names.len() - kept.len(), names.len(),
                    started.elapsed().as_secs_f32(), kept.len());
            }
            Err(e) => {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    // shot <eff> <effect> <out.png> <frame> [layer number, o<number> to outline it, g<number> to ghost its model, or - for all] [camera turn] [camera tilt] [half view] [swing]
    let shot = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["shot", _, _, out, frame, rest @ ..] => Some(app::Shot {
            out: out.into(),
            frame: frame.parse().expect("a frame number"),
            layer: rest.first().filter(|l| **l != "-" && !l.starts_with(['o', 'g'])).map(|l| l.parse().expect("a layer number")),
            outline: rest.first().and_then(|l| l.strip_prefix('o')).map(|l| l.parse().expect("a layer number")),
            ghost: rest.first().and_then(|l| l.strip_prefix('g')).map(|l| l.parse().expect("a layer number")),
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
    let copy = scratch_dirs::app_storage_root().join("check.eff");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
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
