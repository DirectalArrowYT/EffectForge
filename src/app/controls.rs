//! The panel's plain-language sections: when a layer emits, how its particles move, how they change over their life,
//! and trails. Each control writes the same fields "All values" lists, named for what they do on screen.

use std::ops::RangeInclusive;

use eframe::egui::{self, Color32, Sense, Vec2};

use super::App;
use crate::eff::fields::{self, AttrKind};
use crate::eff::model::EmitterDef;

/// `emitter_info.follow_type`, as the game's own effects use it: every trail in Smash is 1.
pub const FOLLOW_TYPES: [(i32, &str, &str); 3] = [
    (0, "Moves with the bone", "Every particle stays attached to the bone, as if painted on it."),
    (1, "Stays where born", "A particle is left where it appeared, so a moving bone draws a trail."),
    (2, "Follows position only", "Particles move with the bone but don't turn with it."),
];

/// `shape_info.volume_type`: where in the layer particles appear.
const SHAPES: [&str; 16] = [
    "Point", "Circle", "Circle (even spacing)", "Filled circle", "Sphere", "Sphere (even)", "Sphere (even, 64)",
    "Filled sphere", "Cylinder", "Filled cylinder", "Box", "Filled box", "Line", "Line (even spacing)", "Rectangle",
    "Model surface",
];

/// Opacity over a particle's life: (fraction of life, opacity as a share of the layer's).
const FADES: [(&str, &[(f32, f32)]); 4] = [
    ("Steady", &[(0.0, 1.0)]),
    ("Fade out", &[(0.0, 1.0), (1.0, 0.0)]),
    ("Fade in and out", &[(0.0, 0.0), (0.2, 1.0), (0.7, 1.0), (1.0, 0.0)]),
    ("Flash", &[(0.0, 1.0), (0.3, 0.35), (1.0, 0.0)]),
];

const ALPHA_TYPE: &str = "particle_color.alpha0_type";
const BILLBOARD: &str = "particle_data.billboard_type";

/// A layer drawing with another layer's shader: where from, the values to put back, and of those the ones its own
/// shader is built for (looked for again on the way back).
pub struct Borrowed {
    pub(super) from: String,
    pub(super) undo: Vec<(String, f32)>,
    pub(super) modes: Vec<(String, f32)>,
}

/// A shader to look for once values just set are in: for layer `layer` of set `set` (played by `entry`), one built
/// for `want`. `back` when this goes back to the layer's own look, otherwise the values `want` replaced.
pub struct Reshade {
    pub(super) entry: String,
    pub(super) set: usize,
    pub(super) layer: usize,
    pub(super) want: Vec<(String, f32)>,
    pub(super) what: String,
    pub(super) back: Option<Vec<(String, f32)>>,
}

/// How one of Smash's own layers behaves, from `recipes.json` (made by gen_recipes.py from the game's files).
struct Recipe {
    group: String,
    name: String,
    blurb: String,
    source: String,
    billboard: f32,
    keyed_alpha: bool,
    values: Vec<(String, f32)>,
}

fn recipes() -> &'static [Recipe] {
    static RECIPES: std::sync::OnceLock<Vec<Recipe>> = std::sync::OnceLock::new();
    RECIPES.get_or_init(|| {
        let list: Vec<serde_json::Value> = serde_json::from_str(include_str!("recipes.json")).expect("recipes.json");
        list.iter().map(|r| {
            let text = |k: &str| r[k].as_str().unwrap_or_default().to_string();
            let mode = |k: &str| r["modes"][k].as_f64().unwrap_or(0.0) as f32;
            let mut values: Vec<_> = r["values"].as_object().into_iter().flatten()
                .filter_map(|(k, v)| Some((k.clone(), v.as_f64()? as f32))).collect();
            values.sort_by(|a, b| a.0.cmp(&b.0));
            Recipe {
                group: text("group"), name: text("name"), blurb: text("blurb"), source: text("source"),
                billboard: mode(BILLBOARD), keyed_alpha: mode(ALPHA_TYPE) == 2.0, values,
            }
        }).collect()
    })
}

/// Directions for the push, in the layer's own axes.
const DIRECTIONS: [(&str, [f32; 3]); 6] =
    [("Up", [0.0, 1.0, 0.0]), ("Down", [0.0, -1.0, 0.0]), ("Left", [-1.0, 0.0, 0.0]), ("Right", [1.0, 0.0, 0.0]),
     ("Toward camera", [0.0, 0.0, 1.0]), ("Away", [0.0, 0.0, -1.0])];

impl App {
    pub(super) fn slider(&mut self, ui: &mut egui::Ui, em: &EmitterDef, id: &str, range: RangeInclusive<f32>, label: &str, hover: &str) -> Option<f32> {
        let mut value = self.value(em, id)?;
        let response = ui.add(egui::Slider::new(&mut value, range).text(label)).on_hover_text(hover);
        response.changed().then(|| {
            self.set(em, id, value);
            value
        })
    }

    /// Replace a key track: `prefix` is e.g. `emitter_static.alpha0`, each key (fraction of life, x y z).
    fn write_keys(&mut self, em: &EmitterDef, prefix: &str, count_id: &str, keys: &[(f32, [f32; 3])]) {
        for (i, (time, value)) in keys.iter().enumerate() {
            for (axis, v) in ["x", "y", "z"].iter().zip(value) {
                self.set(em, &format!("{prefix}.keys[{i}].{axis}"), *v);
            }
            self.set(em, &format!("{prefix}.keys[{i}].time"), *time);
        }
        self.set(em, count_id, keys.len() as f32);
    }

    fn keys(&self, em: &EmitterDef, prefix: &str, count_id: &str) -> Vec<(f32, f32)> {
        let count = self.value(em, count_id).unwrap_or(0.0).clamp(0.0, 8.0) as usize;
        (0..count).map(|i| {
            (self.value(em, &format!("{prefix}.keys[{i}].time")).unwrap_or(0.0), self.value(em, &format!("{prefix}.keys[{i}].x")).unwrap_or(0.0))
        }).collect()
    }

    /// A small line of a key track over the particle's life.
    fn curve(ui: &mut egui::Ui, keys: &[(f32, f32)], top: f32) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(150.0, 28.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, Color32::from_gray(35));
        let at = |(t, v): (f32, f32)| rect.left_bottom() + Vec2::new(t.clamp(0.0, 1.0) * rect.width(), -(v / top.max(1e-3)).clamp(0.0, 1.0) * rect.height());
        let mut points: Vec<_> = keys.iter().map(|k| at(*k)).collect();
        if let (Some(first), Some(last)) = (keys.first(), keys.last()) {
            points.insert(0, at((0.0, first.1)));
            points.push(at((1.0, last.1)));
        }
        painter.add(egui::Shape::line(points, (1.5, Color32::from_rgb(255, 200, 90))));
    }

    /// When a layer emits and how many.
    pub(super) fn timing(&mut self, ui: &mut egui::Ui, em: &EmitterDef) {
        self.slider(ui, em, "emission.start", 0.0..=120.0, "Starts at frame", "Frames after the effect starts before this layer emits.");
        let by_distance = self.value(em, "emission.is_emit_dist_enabled").unwrap_or(0.0) != 0.0;
        let once = self.value(em, "emission.is_one_time").unwrap_or(0.0) != 0.0;
        let mut keeps = !once;
        if ui.checkbox(&mut keeps, "Keeps emitting").on_hover_text("On: emits until the effect is removed. Off: emits for a set time.").changed() {
            self.set(em, "emission.is_one_time", if keeps { 0.0 } else { 1.0 });
        }
        if once {
            self.slider(ui, em, "emission.duration", 1.0..=120.0, "Emits for (frames)", "How long it keeps emitting. 1 is a single burst.");
        }
        if by_distance {
            ui.label("Emits as it moves: see Trail.");
        } else {
            self.slider(ui, em, "emission.rate", 0.0..=60.0, "Particles each time", "How many appear at once.");
            let mut every = self.value(em, "emission.interval").unwrap_or(0.0) + 1.0;
            if ui.add(egui::Slider::new(&mut every, 1.0..=60.0).text("Every (frames)")).on_hover_text("1 is every frame.").changed() {
                self.set(em, "emission.interval", (every - 1.0).max(0.0));
            }
        }
        self.slider(ui, em, "particle_data.life_random", 0.0..=100.0, "Life varies by %", "Each particle lasts up to this much less than 'Lasts'.");
    }

    /// How particles move once born.
    pub(super) fn movement(&mut self, ui: &mut egui::Ui, em: &EmitterDef) {
        self.slider(ui, em, "particle_velocity.all_direction", 0.0..=5.0, "Burst outward", "Speed away from the centre, every way at once.");
        if let Some(speed) = self.slider(ui, em, "particle_velocity.designated_dir_scale", 0.0..=5.0, "Push", "Speed in the direction below.") {
            // A push needs a direction; an emitter that never had one has zeros.
            let dir = ["x", "y", "z"].map(|a| self.value(em, &format!("particle_velocity.designated_dir_{a}")).unwrap_or(0.0));
            if speed > 0.0 && dir.iter().all(|v| *v == 0.0) {
                self.set(em, "particle_velocity.designated_dir_y", 1.0);
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Push toward");
            for (label, dir) in DIRECTIONS {
                if ui.small_button(label).clicked() {
                    for (a, v) in ["x", "y", "z"].iter().zip(dir) {
                        self.set(em, &format!("particle_velocity.designated_dir_{a}"), v);
                    }
                }
            }
        });
        self.slider(ui, em, "particle_velocity.diffusion_dir_angle", 0.0..=180.0, "Spread (degrees)", "How far either side of the push direction particles may go.");
        self.slider(ui, em, "particle_velocity.vel_random", 0.0..=100.0, "Speed varies by %", "Each particle moves up to this much slower.");
        let mut gravity = self.value(em, "emitter_static.gravity_scale").unwrap_or(0.0);
        if ui.add(egui::Slider::new(&mut gravity, -0.2..=0.2).text("Gravity")).on_hover_text("Positive falls, negative rises (heat, smoke).").changed() {
            for prefix in ["emitter_static", "emission"] {
                self.set(em, &format!("{prefix}.gravity_scale"), gravity.abs());
                // Down for a fall, up for a rise.
                for (a, v) in ["x", "y", "z"].iter().zip([0.0, -gravity.signum(), 0.0]) {
                    self.set(em, &format!("{prefix}.gravity_dir_{a}"), v);
                }
            }
        }
        let mut drag = 1.0 - self.value(em, "emitter_static.air_res").unwrap_or(1.0);
        if ui.add(egui::Slider::new(&mut drag, 0.0..=0.3).text("Slow down")).on_hover_text("How quickly particles lose speed. 0 keeps it.").changed() {
            self.set(em, "emitter_static.air_res", 1.0 - drag);
        }
        if !self.has_mesh(em) {
            let on = self.value(em, BILLBOARD) == Some(5.0);
            let mut want = on;
            ui.checkbox(&mut want, "Stretch along motion").on_hover_text(
                "Each particle faces the camera but stretches the way it moves, like hit sparks. Borrows the nearest \
                 shader that draws this way, from this file or the game's.");
            if want != on {
                self.need_modes(em, &[(BILLBOARD, if want { 5.0 } else { 0.0 })], "Stretch along motion");
                if want && self.value(em, "emitter_static.add_vel_to_scale").unwrap_or(0.0) <= 0.0 {
                    self.set(em, "emitter_static.add_vel_to_scale", 1.0);
                }
            }
            if on {
                self.slider(ui, em, "emitter_static.add_vel_to_scale", 0.0..=4.0, "Stretch amount", "How much longer a particle gets for its speed.");
            }
        }
        ui.separator();
        let mut shape = self.value(em, "shape_info.volume_type").unwrap_or(0.0) as usize;
        let before = shape;
        egui::ComboBox::from_label("Appear in").selected_text(SHAPES.get(shape).copied().unwrap_or("?")).show_ui(ui, |ui| {
            for (i, name) in SHAPES.iter().enumerate() {
                ui.selectable_value(&mut shape, i, *name);
            }
        });
        if shape != before {
            self.set(em, "shape_info.volume_type", shape as f32);
        }
        if shape > 0 {
            let mut radius = self.value(em, "shape_info.volume_radius_x").unwrap_or(1.0);
            if ui.add(egui::Slider::new(&mut radius, 0.0..=20.0).text("Shape size")).changed() {
                for a in ["x", "y", "z"] {
                    self.set(em, &format!("shape_info.volume_radius_{a}"), radius);
                }
            }
        }
    }

    /// How a particle changes over its life.
    pub(super) fn over_life(&mut self, ui: &mut egui::Ui, em: &EmitterDef) {
        let alpha = self.keys(em, "emitter_static.alpha0", "emitter_static.num_alpha0_keys");
        let top = alpha.iter().map(|k| k.1).fold(0.0, f32::max).max(self.value(em, "particle_color.alpha0").unwrap_or(1.0)).max(0.01);
        ui.horizontal(|ui| {
            ui.label("Opacity");
            Self::curve(ui, &alpha, top);
        });
        ui.horizontal_wrapped(|ui| {
            for (label, shape) in FADES {
                if ui.small_button(label).clicked() {
                    let keys: Vec<_> = shape.iter().map(|(t, v)| (*t, [v * top; 3])).collect();
                    self.write_keys(em, "emitter_static.alpha0", "emitter_static.num_alpha0_keys", &keys);
                    if shape.len() > 1 {
                        // A shader built for fixed opacity ignores the keys.
                        self.need_modes(em, &[(ALPHA_TYPE, 2.0)], label);
                    } else {
                        self.set(em, "particle_color.alpha0", top);
                    }
                }
            }
        });
        let scale = self.keys(em, "emitter_static.scale_anim", "emitter_static.num_scale_keys");
        let (mut start, mut end) = match scale.as_slice() {
            [] => (1.0, 1.0),
            [only] => (only.1, only.1),
            [first, .., last] => (first.1, last.1),
        };
        ui.horizontal(|ui| {
            ui.label("Size");
            Self::curve(ui, &scale, scale.iter().map(|k| k.1).fold(1.0, f32::max));
        });
        let changed = ui.add(egui::Slider::new(&mut start, 0.0..=3.0).text("Size at birth \u{d7}")).changed()
            | ui.add(egui::Slider::new(&mut end, 0.0..=3.0).text("Size at end \u{d7}")).changed();
        if changed {
            self.write_keys(em, "emitter_static.scale_anim", "emitter_static.num_scale_keys", &[(0.0, [start; 3]), (1.0, [end; 3])]);
        }
    }

    /// A layer that leaves particles behind as its bone moves.
    pub(super) fn trail(&mut self, ui: &mut egui::Ui, em: &EmitterDef) {
        let on = self.value(em, "emission.is_emit_dist_enabled").unwrap_or(0.0) != 0.0;
        let mut want = on;
        ui.checkbox(&mut want, "Leave a trail as it moves").on_hover_text(
            "Emits by distance instead of time, and leaves each particle where it appeared: how Smash's own fire trails \
             (Roy, Ike) are built. A trail only shows when the move's script attaches this effect to a bone that moves.");
        if want != on {
            self.set(em, "emission.is_emit_dist_enabled", if want { 1.0 } else { 0.0 });
            self.set(em, "emitter_info.follow_type", if want { 1.0 } else { 0.0 });
            // the shader has to match, which can only be seen to once the change is in
            self.trail_fix = self.effect.as_ref().map(|e| e.0.clone());
            if want {
                self.set(em, "emission.is_one_time", 0.0);
                self.set(em, "emission.interval", 0.0);
                if self.value(em, "emission.emitter_dist_unit").unwrap_or(0.0) <= 0.0 {
                    self.set(em, "emission.emitter_dist_unit", 2.0);
                }
                if self.value(em, "emission.emitter_dist_max").unwrap_or(0.0) <= 0.0 {
                    self.set(em, "emission.emitter_dist_max", 10.0);
                }
                if self.value(em, "emission.rate").unwrap_or(0.0) < 1.0 {
                    self.set(em, "emission.rate", 1.0);
                }
                if let Some(game) = &mut self.game {
                    game.swing = true;
                }
            }
        }
        if !want {
            return;
        }
        let mut gap = self.value(em, "emission.emitter_dist_unit").unwrap_or(2.0);
        if ui.add(egui::Slider::new(&mut gap, 0.2..=20.0).logarithmic(true).text("Gap between particles"))
            .on_hover_text("Distance the bone moves between particles. Smaller is a denser trail.").changed()
        {
            self.set(em, "emission.emitter_dist_unit", gap);
        }
        self.slider(ui, em, "emission.rate", 1.0..=10.0, "Particles per gap", "How many appear each time.");
        self.slider(ui, em, "particle_data.life", 1.0..=120.0, "Trail length (frames)", "How long each particle stays.");
        if ui.button("Taper it").on_hover_text("Each particle fades out and shrinks to a third over its life.").clicked() {
            let top = self.value(em, "particle_color.alpha0").unwrap_or(1.0).max(0.01);
            self.write_keys(em, "emitter_static.alpha0", "emitter_static.num_alpha0_keys", &[(0.0, [top; 3]), (1.0, [0.0; 3])]);
            self.need_modes(em, &[(ALPHA_TYPE, 2.0)], "Taper");
            self.write_keys(em, "emitter_static.scale_anim", "emitter_static.num_scale_keys", &[(0.0, [1.0; 3]), (1.0, [0.33; 3])]);
        }
        ui.label("Turn on Swing under the preview to see it move.");
    }

    fn has_mesh(&self, em: &EmitterDef) -> bool {
        let (Some(doc), Some((entry, _))) = (&self.doc, &self.effect) else { return false };
        doc.primitive_of(entry, &em.name).is_some()
    }

    /// Make sure the layer's shader is built for `want`. Some fields (billboard, alpha type) are compiled into each
    /// layer's shader, so changing the value alone does nothing: the values are set, and once they are in, the layer
    /// moves to the nearest shader in the file or the game that is built for them (see [`crate::engine::Doc::reshade`]).
    fn need_modes(&mut self, em: &EmitterDef, want: &[(&str, f32)], what: &str) {
        if want.iter().all(|(id, v)| self.value(em, id).map_or(true, |c| c == *v)) {
            return;
        }
        let Some((entry, set)) = self.effect.clone() else { return };
        let back = want.iter().filter_map(|(id, _)| Some((id.to_string(), self.value(em, id)?))).collect();
        for (id, v) in want {
            self.set(em, id, *v);
        }
        self.reshade = Some(Reshade {
            entry, set, layer: self.emitter, what: what.to_string(), back: Some(back),
            want: want.iter().map(|(id, v)| (id.to_string(), *v)).collect(),
        });
    }

    /// Says which layer's shader this one borrowed, with a way back.
    pub(super) fn borrowed_note(&mut self, ui: &mut egui::Ui, em: &EmitterDef) {
        let Some(key) = self.effect.as_ref().map(|e| (e.1, self.emitter)) else { return };
        let Some(from) = self.borrowed.get(&key).map(|b| b.from.clone()) else { return };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(Color32::from_rgb(140, 200, 255), format!("Draws with the shader of {from}."))
                .on_hover_text("Some looks (stretching, fading) are compiled into a layer's shader, so this layer uses the \
                                nearest one that draws that way. Use its own puts back what changed for it.");
            if ui.small_button("Use its own").clicked() {
                if let Some(b) = self.borrowed.remove(&key) {
                    for (id, v) in &b.undo {
                        self.set(em, id, *v);
                    }
                    // its own shader may have left the file, so the nearest to its old look is found again
                    let (entry, set) = self.effect.clone().expect("an effect");
                    self.reshade = Some(Reshade { entry, set, layer: self.emitter, want: b.modes, what: "Its own look".into(), back: None });
                }
            }
        });
    }

    /// Presets from Smash's own effects, and a few sliders that each move several values at once.
    pub(super) fn recipes(&mut self, ui: &mut egui::Ui, em: &EmitterDef, has_mesh: bool) {
        ui.label("Copy how one of Smash's own layers moves, lives and fades. This layer keeps its texture, colour and size.");
        let mut group = "";
        for recipe in recipes() {
            if recipe.group != group {
                group = &recipe.group;
                ui.label(egui::RichText::new(group).strong());
            }
            if ui.small_button(&recipe.name).on_hover_text(format!("{}\nFrom {}", recipe.blurb, recipe.source)).clicked() {
                self.apply_recipe(em, recipe, has_mesh);
            }
        }
        ui.separator();
        ui.label(egui::RichText::new("Feel").strong());
        let burst = self.value(em, "particle_velocity.all_direction").unwrap_or(0.0);
        let push = self.value(em, "particle_velocity.designated_dir_scale").unwrap_or(0.0);
        let mut force = burst.max(push);
        if ui.add(egui::Slider::new(&mut force, 0.0..=5.0).text("Force"))
            .on_hover_text("How hard particles are thrown: burst and push together, keeping their mix.").changed()
        {
            if burst.max(push) <= 0.0 {
                self.set(em, "particle_velocity.all_direction", force);
            } else {
                let k = force / burst.max(push);
                self.set(em, "particle_velocity.all_direction", burst * k);
                self.set(em, "particle_velocity.designated_dir_scale", push * k);
            }
        }
        self.slider(ui, em, "emission.rate", 0.0..=60.0, "Amount", "How many appear each time it emits.");
        let mut random = self.value(em, "particle_velocity.vel_random").unwrap_or(0.0);
        if ui.add(egui::Slider::new(&mut random, 0.0..=100.0).text("Randomness"))
            .on_hover_text("How much particles differ in speed, size and life.").changed()
        {
            for id in ["particle_velocity.vel_random", "particle_data.life_random", "particle_scale.scale_random_x",
                       "particle_scale.scale_random_y", "particle_scale.scale_random_z"] {
                self.set(em, id, random);
            }
        }
    }

    fn apply_recipe(&mut self, em: &EmitterDef, recipe: &Recipe, has_mesh: bool) {
        let alpha = self.keys(em, "emitter_static.alpha0", "emitter_static.num_alpha0_keys");
        let top = alpha.iter().map(|k| k.1).fold(self.value(em, "particle_color.alpha0").unwrap_or(1.0), f32::max).max(0.01);
        let mut want = Vec::new();
        if recipe.keyed_alpha {
            want.push((ALPHA_TYPE, 2.0));
        }
        // Stretching only means something on a sprite.
        let billboard = self.value(em, BILLBOARD).unwrap_or(0.0);
        if !has_mesh && (recipe.billboard == 5.0) != (billboard == 5.0) {
            want.push((BILLBOARD, if recipe.billboard == 5.0 { 5.0 } else { 0.0 }));
        }
        self.need_modes(em, &want, &recipe.name);
        let recipe_top = recipe.values.iter()
            .filter(|(id, _)| id.starts_with("emitter_static.alpha0.keys") && !id.ends_with(".time"))
            .map(|(_, v)| *v).fold(0.0, f32::max).max(0.01);
        for (id, v) in &recipe.values {
            if id.starts_with("emitter_static.alpha0.keys") {
                // The game layer's opacity shape, at this layer's own strength.
                if recipe.keyed_alpha {
                    self.set(em, id, if id.ends_with(".time") { *v } else { v * top / recipe_top });
                }
            } else if id != "emitter_static.num_alpha0_keys" || recipe.keyed_alpha {
                self.set(em, id, *v);
            }
        }
        if !recipe.keyed_alpha {
            // The game layer's opacity never changes.
            self.write_keys(em, "emitter_static.alpha0", "emitter_static.num_alpha0_keys", &[(0.0, [top; 3])]);
            self.set(em, "particle_color.alpha0", top);
        }
        self.status = format!("Behaves like {} ({}). Texture, colour and size kept.", recipe.name, recipe.source);
    }

    /// Every field, in the format's own groups.
    pub(super) fn all_values(&mut self, ui: &mut egui::Ui, em: &EmitterDef) {
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search values"));
        let search = self.search.to_lowercase();
        let table = fields::table();
        for group in fields::GROUPS {
            let rows: Vec<_> = table.iter()
                .filter(|a| a.group == *group && (a.id.contains(&search) || a.label.to_lowercase().contains(&search)))
                .filter(|a| self.value(em, a.id).is_some())
                .collect();
            if rows.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(format!("{group} ({})", rows.len()))
                .id_salt(group)
                .open((!search.is_empty()).then_some(true))
                .show(ui, |ui| {
                    for a in rows {
                        let Some(mut value) = self.value(em, a.id) else { continue };
                        let changed = ui.horizontal(|ui| {
                            let changed = match a.kind {
                                AttrKind::Flag => {
                                    let mut on = value != 0.0;
                                    let changed = ui.checkbox(&mut on, "").changed();
                                    value = if on { 1.0 } else { 0.0 };
                                    changed
                                }
                                AttrKind::Enum(names) => {
                                    let mut chosen = value as usize;
                                    let before = chosen;
                                    egui::ComboBox::from_id_salt(a.id).selected_text(names.get(chosen).copied().unwrap_or("?")).show_ui(ui, |ui| {
                                        for (i, name) in names.iter().enumerate() {
                                            ui.selectable_value(&mut chosen, i, *name);
                                        }
                                    });
                                    value = chosen as f32;
                                    chosen != before
                                }
                                AttrKind::Float { speed } => ui.add(egui::DragValue::new(&mut value).speed(speed)).changed(),
                                AttrKind::Int | AttrKind::UInt => ui.add(egui::DragValue::new(&mut value).speed(1.0)).changed(),
                            };
                            ui.label(a.label).on_hover_text(format!("{}\n{}", a.id, a.doc));
                            changed
                        }).inner;
                        if changed {
                            self.set(em, a.id, value);
                        }
                    }
                });
        }
    }
}
