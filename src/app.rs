//! The window: effect list on the left, a preview from Smash's camera in the middle, values on the right.

mod controls;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use anyhow::Result;
use eframe::egui::{self, Color32, Mesh, Pos2, Sense, Shape, TextureHandle, TextureOptions, Vec2};

use crate::eff_attrs::AttrValue;
use crate::eff_mesh_io::{self, MeshData};
use crate::effects::EmitterDef;
use crate::engine::{attr, mirror, quarter_turn, Doc, Pending, Target, Tuned};
use crate::preview::Preview as GamePreview;

enum Job {
    Open(PathBuf),
    Values(Pending),
    Texture(Target, PathBuf),
    Mesh(Target, MeshData),
    Duplicate(String, usize, String),
    Save,
    /// Copy an effect into the tuned file.
    Stage(String, usize, PathBuf),
    /// Put the tuned file's effects into the file it was staged from.
    Merge(PathBuf, PathBuf),
}

enum Done {
    Doc(Result<Doc>, String),
    Saved(Result<PathBuf>),
    Tuned(Result<Tuned>, String),
}

/// Files opened before, newest first.
const RECENT_FILES: usize = 12;

fn recent_path() -> PathBuf {
    crate::log::path().with_file_name("recent.txt")
}

fn read_recent() -> Vec<PathBuf> {
    std::fs::read_to_string(recent_path()).unwrap_or_default().lines().map(PathBuf::from).collect()
}

fn remember(recent: &mut Vec<PathBuf>, path: &Path) {
    recent.retain(|p| p != path);
    recent.insert(0, path.to_path_buf());
    recent.truncate(RECENT_FILES);
    let lines: Vec<String> = recent.iter().map(|p| p.display().to_string()).collect();
    let _ = std::fs::write(recent_path(), lines.join("\n"));
}

/// What a panic in a job or a draw says.
fn panic_text(panic: Box<dyn std::any::Any + Send>) -> String {
    panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "unknown panic".to_string())
}

pub struct App {
    /// Shared with a running job, so the window keeps showing it until the new one is ready.
    doc: Option<Arc<Doc>>,
    busy: Option<String>,
    status: String,
    done: Receiver<Done>,
    done_tx: Sender<Done>,
    filter: String,
    effect: Option<(String, usize)>,
    emitter: usize,
    pending: Pending,
    /// Values on their way into the file, shown until they arrive.
    applying: Pending,
    unsaved: bool,
    recent: Vec<PathBuf>,
    /// Walking every effect: the next one to show.
    sweep: Option<usize>,
    edit_started: Option<std::time::Instant>,
    edit_kept: bool,
    shot: Option<Shot>,
    /// The game's shaders have drawn a frame.
    drawn: bool,
    new_name: String,
    /// The model being turned or newly loaded, not yet applied.
    draft_mesh: Option<MeshData>,
    preview: Option<Preview>,
    orbit: Vec2,
    search: String,
    /// An effect to pick once the file has opened.
    select: Option<String>,
    game: Option<GamePreview>,
    last_tick: std::time::Instant,
    /// The open file's tuned file and what it keeps.
    tuned: Option<Tuned>,
    /// In a tuned file, list only the effects it keeps.
    only_tuned: bool,
    /// Layers drawing with another layer's shader, by (set, layer).
    borrowed: std::collections::HashMap<(usize, usize), controls::Borrowed>,
}

/// One picture for the `shot` command.
pub struct Shot {
    pub out: PathBuf,
    pub frame: f32,
    pub layer: Option<usize>,
    pub outline: Option<usize>,
    /// Camera turn and tilt, radians.
    pub orbit: [f32; 2],
    /// Half the view's height, in Smash units.
    pub view: f32,
    /// Move the effect round, for trails.
    pub swing: bool,
    pub settled: u32,
}

struct Preview {
    key: (usize, usize),
    mesh: Option<MeshData>,
    texture: Option<TextureHandle>,
}

impl App {
    pub fn new(open: Option<PathBuf>, select: Option<String>, sweep: bool, shot: Option<Shot>, cc: &eframe::CreationContext) -> Self {
        let (done_tx, done) = channel();
        let mut app = Self {
            doc: None, busy: None, status: "Open an .eff file to begin.".into(), done, done_tx,
            filter: String::new(), effect: None, emitter: 0, pending: Pending::new(), applying: Pending::new(), unsaved: false, recent: read_recent(), sweep: sweep.then_some(0), edit_started: None, edit_kept: true, shot, drawn: false,
            new_name: String::new(), draft_mesh: None, preview: None, orbit: Vec2::ZERO,
            search: String::new(), select: None,
            game: cc.wgpu_render_state.clone().map(GamePreview::new), last_tick: std::time::Instant::now(),
            tuned: None, only_tuned: false, borrowed: Default::default(),
        };
        app.select = select;
        if let Some(path) = open {
            app.run(Job::Open(path), "Opening");
        }
        app
    }

    /// Jobs take a second or more, and own the document while they run.
    fn run(&mut self, job: Job, label: &str) {
        self.busy = Some(label.to_string());
        let doc = self.doc.clone();
        let tx = self.done_tx.clone();
        let label = label.to_string();
        std::thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| match job {
                Job::Open(path) => Done::Doc(Doc::open(&path), "Opened".into()),
                Job::Save => {
                    let doc = doc.expect("a document to save");
                    Done::Saved(doc.save().map(|_| doc.path.clone()))
                }
                Job::Values(pending) => Done::Doc(doc.expect("doc").apply_values(&pending), "Applied".into()),
                Job::Texture(target, png) => Done::Doc(doc.expect("doc").replace_texture(&target, &png), "Texture swapped".into()),
                Job::Mesh(target, mesh) => Done::Doc(doc.expect("doc").replace_mesh(&target, &mesh), "Model swapped".into()),
                Job::Duplicate(entry, set, name) => Done::Doc(doc.expect("doc").duplicate(&entry, set, &name), format!("Duplicated as {name}")),
                Job::Stage(entry, set, tuned) => Done::Tuned(doc.expect("doc").stage(&entry, set, &tuned), format!("{entry} kept in {}", tuned.display())),
                Job::Merge(tuned, into) => Done::Tuned(
                    std::fs::read(&into).map_err(anyhow::Error::from)
                        .and_then(|bytes| crate::engine::merge_tuned(&tuned, &bytes))
                        .and_then(|(bytes, _)| Ok(std::fs::write(&into, bytes)?))
                        .map(|_| Tuned::load(&tuned)),
                    format!("Tuned effects put into {}", into.display())),
            }));
            let _ = tx.send(result.unwrap_or_else(|panic| Done::Doc(Err(anyhow::anyhow!(panic_text(panic))), label)));
        });
    }

    fn poll(&mut self) {
        while let Ok(done) = self.done.try_recv() {
            match done {
                Done::Doc(Ok(doc), note) => {
                    if let Some(name) = self.select.take() {
                        self.effect = doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(&name));
                    }
                    if note == "Opened" {
                        self.borrowed.clear();
                        remember(&mut self.recent, &doc.path);
                        self.tuned = Tuned::path_for(&doc.path).map(|p| Tuned::load(&p));
                        self.only_tuned = Tuned::is_tuned_file(&doc.path);
                    }
                    self.doc = Some(Arc::new(doc));
                    self.busy = None;
                    self.applying.clear();
                    // A change of values leaves every texture and model as it was.
                    if note != "Applied" {
                        self.preview = None;
                        if let Some(game) = &mut self.game {
                            game.invalidate();
                        }
                    }
                    if note != "Opened" {
                        self.unsaved = true;
                    }
                    self.status = note;
                }
                Done::Doc(Err(e), note) => {
                    self.busy = None;
                    self.applying.clear();
                    crate::log::note(&format!("{note} failed: {e:#}"));
                    self.status = format!("{note} failed: {e:#}");
                }
                Done::Saved(Ok(path)) => {
                    self.busy = None;
                    self.unsaved = false;
                    self.status = format!("Saved {}", path.display());
                }
                Done::Saved(Err(e)) => {
                    self.busy = None;
                    self.status = format!("Save failed: {e:#}");
                }
                Done::Tuned(Ok(tuned), note) => {
                    self.busy = None;
                    self.tuned = Some(tuned);
                    self.status = note;
                }
                Done::Tuned(Err(e), note) => {
                    self.busy = None;
                    crate::log::note(&format!("{note} failed: {e:#}"));
                    self.status = format!("Failed: {e:#}");
                }
            }
        }
    }

    fn target(&self) -> Option<Target> {
        let (entry, set_idx) = self.effect.clone()?;
        let doc = self.doc.as_ref()?;
        let set = &doc.loaded.ptcl.emitter_sets[set_idx];
        Some(Target {
            entry, set_idx, set_name: set.name.clone(), emitter_idx: self.emitter,
            emitter_name: set.emitters.get(self.emitter)?.name.clone(),
        })
    }

    fn value(&self, em: &EmitterDef, id: &str) -> Option<f32> {
        let key = (self.effect.as_ref()?.1, self.emitter);
        [&self.pending, &self.applying].iter().find_map(|p| p.get(&key).and_then(|p| p.get(id)).copied()).or_else(|| attr(em, id)).map(|v| v.as_f32())
    }

    fn set(&mut self, em: &EmitterDef, id: &str, value: f32) {
        let Some(old) = attr(em, id) else { return };
        let new = match old {
            AttrValue::Float(_) => AttrValue::Float(value),
            AttrValue::Int(_) => AttrValue::Int(value.round() as i64),
            AttrValue::UInt(_) => AttrValue::UInt(value.round().max(0.0) as u64),
        };
        let key = (self.effect.as_ref().expect("an effect").1, self.emitter);
        self.pending.entry(key).or_default().insert(id.to_string(), new);
    }

    fn load_preview(&mut self, ctx: &egui::Context) {
        let (Some(doc), Some((entry, set_idx))) = (self.doc.as_ref(), self.effect.as_ref()) else { return };
        let key = (*set_idx, self.emitter);
        if self.preview.as_ref().is_some_and(|p| p.key == key) {
            return;
        }
        let Some(em) = doc.emitters(*set_idx).get(self.emitter) else { return };
        let mesh = doc.primitive_of(entry, &em.name).and_then(|p| doc.mesh(p).ok());
        let texture = doc.texture(em).map(|image| {
            let size = [image.width() as usize, image.height() as usize];
            ctx.load_texture("emitter", egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()), TextureOptions::LINEAR)
        });
        self.preview = Some(Preview { key, mesh, texture });
        self.draft_mesh = None;
    }

    fn warnings(&self, em: &EmitterDef, has_mesh: bool) -> Vec<&'static str> {
        let mut out = Vec::new();
        let keys = |id: &str| self.value(em, id).unwrap_or(0.0);
        if has_mesh && keys("particle_color.alpha0_type") == 2.0 && keys("emitter_static.num_scale_keys") > 1.0 {
            out.push("Fade keys plus scale keys: a model emitter set up like this drew nothing in game.");
        }
        let alpha = (0..keys("emitter_static.num_alpha0_keys").max(1.0) as usize)
            .map(|i| keys(&format!("emitter_static.alpha0.keys[{i}].x"))).fold(0.0, f32::max);
        if alpha < 0.1 {
            out.push("Nearly invisible: opacity is under 10%. Converted effects often arrive like this.");
        }
        if has_mesh && keys("emitter_static.rotate_init_z").abs() > 0.01 {
            out.push("The model is rolled by this layer's Z rotation.");
        }
        out
    }

    fn open(&mut self, path: PathBuf) {
        self.pending.clear();
        self.effect = None;
        self.run(Job::Open(path), "Opening");
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let idle = self.busy.is_none();
            if ui.add_enabled(idle, egui::Button::new("Open .eff")).clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("Effect file", &["eff"]).pick_file() {
                    self.open(path);
                }
            }
            let recent = self.recent.clone();
            ui.add_enabled_ui(idle && !recent.is_empty(), |ui| {
                ui.menu_button("Recent", |ui| {
                    for path in recent {
                        if ui.button(path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()).on_hover_text(path.display().to_string()).clicked() {
                            self.open(path);
                        }
                    }
                });
            });
            let waiting = !self.pending.is_empty();
            if ui.add_enabled(idle && self.doc.is_some() && self.unsaved && !waiting, egui::Button::new("Save to mod")).on_hover_text("Writes the .eff you opened. The first save keeps a .forge-backup beside it.").clicked() {
                self.run(Job::Save, "Saving");
            }
            ui.separator();
            if let Some(busy) = &self.busy {
                ui.spinner();
                ui.label(busy);
            } else {
                ui.label(&self.status);
            }
        });
    }

    fn effect_list(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search effects"));
        let Some(doc) = self.doc.clone() else {
            ui.separator();
            ui.label("Recent files");
            for path in self.recent.clone() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ui.add_enabled(self.busy.is_none(), egui::Button::new(name)).on_hover_text(path.display().to_string()).clicked() {
                    self.open(path);
                }
            }
            return;
        };
        let filter = self.filter.to_lowercase();
        let list = doc.effects();
        let tuned = self.tuned.clone().unwrap_or_default();
        if !tuned.effects.is_empty() {
            ui.checkbox(&mut self.only_tuned, format!("Only tuned ({})", tuned.effects.len()))
                .on_hover_text("\u{2605} marks effects kept in the tuned file: rebuilds put these back over the generated ones.");
        }
        let mut picked = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (name, set) in list.iter().filter(|(n, _)| n.to_lowercase().contains(&filter) && (!self.only_tuned || tuned.has(n))) {
                let on = self.effect.as_ref().is_some_and(|e| &e.0 == name);
                let label = if tuned.has(name) { format!("\u{2605} {name}") } else { name.clone() };
                if ui.selectable_label(on, label).clicked() {
                    picked = Some((name.clone(), *set));
                }
            }
        });
        if let Some(effect) = picked {
            self.effect = Some(effect);
            self.emitter = 0;
            self.preview = None;
        }
    }

    /// The game's shaders, once this file's are translated. False while they aren't.
    fn game_view(&mut self, ui: &mut egui::Ui) -> bool {
        let (Some(doc), Some((entry, set_idx)), Some(game)) = (self.doc.as_ref(), self.effect.as_ref(), self.game.as_mut()) else { return false };
        let set_idx = *set_idx;
        crate::log::context(&format!("{} / {entry}", doc.path.display()));
        let emitter = self.emitter;
        let length = game.length(doc, set_idx);
        let now = std::time::Instant::now();
        if game.playing {
            game.age += (now - self.last_tick).as_secs_f32().min(0.1) * 60.0;
            if game.age > length {
                game.age = 0.0;
            }
            ui.ctx().request_repaint();
        }
        self.last_tick = now;
        let available = ui.available_size() - Vec2::new(0.0, 30.0);
        let pixels = ui.ctx().pixels_per_point();
        let size = [(available.x * pixels).max(16.0) as u32, (available.y * pixels).max(16.0) as u32];
        // One bad effect shouldn't take the window with it.
        let rendered = catch_unwind(AssertUnwindSafe(|| game.render(doc, set_idx, size)))
            .unwrap_or_else(|panic| Err(anyhow::anyhow!("{} (see {})", panic_text(panic), crate::log::path().display())));
        match rendered {
            Ok(Some(id)) => {
                self.drawn = true;
                let response = ui.add(egui::Image::new((id, available)).sense(Sense::drag()));
                game.orbit += response.drag_delta() * 0.01;
                ui.horizontal(|ui| {
                    if ui.button(if game.playing { "Pause" } else { "Play" }).clicked() {
                        game.playing = !game.playing;
                    }
                    if ui.button("Restart").clicked() {
                        game.age = 0.0;
                    }
                    if ui.button("Reset view").clicked() {
                        game.orbit = Vec2::ZERO;
                    }
                    ui.add(egui::Slider::new(&mut game.age, 0.0..=length).text("frame"));
                    let mut solo = game.solo.is_some();
                    if ui.checkbox(&mut solo, "Only this layer").changed() {
                        game.solo = solo.then_some(emitter);
                    }
                    if game.solo.is_some() {
                        game.solo = Some(emitter);
                    }
                    ui.checkbox(&mut game.swing, "Swing").on_hover_text("Move the effect round in a circle, as a hand would, to see trails.");
                    let mut outlined = game.outline.is_some();
                    if ui.checkbox(&mut outlined, "Outline").on_hover_text("A yellow line round the selected layer").changed()
                        || (game.outline.is_some() && self.shot.is_none())
                    {
                        game.outline = outlined.then_some(emitter);
                    }
                    ui.label("All layers, the game's shaders.");
                });
                true
            }
            Ok(None) => {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                ui.label("Translating this file's shaders for the preview (first time only). Simple preview for now:");
                false
            }
            Err(e) => {
                crate::log::note(&format!("preview failed: {e:#}"));
                ui.label(format!("Preview failed: {e:#}"));
                false
            }
        }
    }

    fn view(&mut self, ui: &mut egui::Ui) {
        if self.game_view(ui) {
            return;
        }
        let (rect, response) = ui.allocate_exact_size(ui.available_size() - Vec2::new(0.0, 28.0), Sense::drag());
        self.orbit += response.drag_delta() * 0.01;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(52, 48, 72));
        let (Some(doc), Some((_, set_idx))) = (self.doc.as_ref(), self.effect.as_ref()) else { return };
        let Some(em) = doc.emitters(*set_idx).get(self.emitter).cloned() else { return };
        let Some(preview) = self.preview.as_ref() else { return };
        let scale = self.value(&em, "particle_scale.scale_x").unwrap_or(1.0);
        let div = [
            self.value(&em, "emitter_static.tex_scroll_anim0.uv_div_x").unwrap_or(1.0).max(1.0),
            self.value(&em, "emitter_static.tex_scroll_anim0.uv_div_y").unwrap_or(1.0).max(1.0),
        ];
        let roll = self.value(&em, "emitter_static.rotate_init_z").unwrap_or(0.0);
        let colour = [0, 1, 2].map(|i| self.value(&em, ["particle_color.color0_r", "particle_color.color0_g", "particle_color.color0_b"][i]).unwrap_or(1.0));
        let alpha = self.value(&em, "particle_color.alpha0").unwrap_or(1.0).clamp(0.0, 1.0);
        let tint = Color32::from_rgba_unmultiplied(
            (colour[0].clamp(0.0, 1.0) * 255.0) as u8, (colour[1].clamp(0.0, 1.0) * 255.0) as u8,
            (colour[2].clamp(0.0, 1.0) * 255.0) as u8, (alpha * 255.0) as u8);
        // 2 units of Smash = a fighter's height of 16 fills about a third of the view.
        let zoom = rect.height() / 50.0;
        let centre = rect.center();
        // The fighter for scale.
        let foot = centre + Vec2::new(0.0, 8.0 * zoom);
        painter.line_segment([foot, foot - Vec2::new(0.0, 16.0 * zoom)], (2.0, Color32::from_gray(150)));
        let (sy, cy) = self.orbit.x.sin_cos();
        let (sx, cx) = self.orbit.y.sin_cos();
        let (sr, cr) = roll.sin_cos();
        let project = |p: [f32; 3]| {
            let (x, y) = (p[0] * cr - p[1] * sr, p[0] * sr + p[1] * cr);
            let (x, z) = (x * cy + p[2] * sy, -x * sy + p[2] * cy);
            let (y, z) = (y * cx - z * sx, y * sx + z * cx);
            (centre + Vec2::new(x, -y) * scale * zoom, z)
        };
        let texture_id = preview.texture.as_ref().map(|t| t.id());
        let mut mesh = texture_id.map_or_else(Mesh::default, Mesh::with_texture);
        let uv = |u: [f32; 2]| Pos2::new(u[0] / div[0], u[1] / div[1]);
        if let Some(source) = self.draft_mesh.as_ref().or(preview.mesh.as_ref()) {
            let projected: Vec<(Pos2, f32)> = source.positions.iter().map(|p| project(*p)).collect();
            let mut triangles: Vec<[u32; 3]> = source.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
            triangles.sort_by(|a, b| {
                let depth = |t: &[u32; 3]| t.iter().map(|&i| projected[i as usize].1).sum::<f32>();
                depth(a).total_cmp(&depth(b))
            });
            for t in triangles {
                for i in t {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: projected[i as usize].0,
                        uv: source.uv0.get(i as usize).map_or(Pos2::ZERO, |u| uv(*u)),
                        color: tint,
                    });
                    mesh.indices.push(mesh.vertices.len() as u32 - 1);
                }
            }
        } else {
            // A sprite: a square facing the camera.
            let half = scale * 0.5;
            let corners = [[-half, half], [half, half], [half, -half], [-half, -half]];
            for (i, c) in corners.iter().enumerate() {
                let u = [(i == 1 || i == 2) as u8 as f32, (i >= 2) as u8 as f32];
                mesh.vertices.push(egui::epaint::Vertex { pos: project([c[0], c[1], 0.0]).0, uv: uv(u), color: tint });
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
        }
        if texture_id.is_some() {
            painter.add(Shape::mesh(mesh));
        }
        painter.text(rect.left_top() + Vec2::new(8.0, 8.0), egui::Align2::LEFT_TOP,
            "Smash's camera. Drag to look round; the grey line is a fighter's height. An approximation: check in game.",
            egui::FontId::proportional(13.0), Color32::from_gray(200));
        ui.horizontal(|ui| {
            if ui.button("Reset view").clicked() {
                self.orbit = Vec2::ZERO;
            }
            ui.label(format!("{} frames of {}", self.value(&em, "particle_data.life").unwrap_or(0.0), em.name));
        });
    }

    fn panel(&mut self, ui: &mut egui::Ui) {
        let (Some(doc), Some((entry, set_idx))) = (self.doc.clone(), self.effect.clone()) else {
            ui.label("Pick an effect.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| self.panel_body(ui, &doc, &entry, set_idx));
    }

    fn panel_body(&mut self, ui: &mut egui::Ui, doc: &Doc, entry: &str, set_idx: usize) {
        ui.heading(entry);
        let idle = self.busy.is_none();
        self.tuning_row(ui, doc, entry, set_idx, idle);
        let emitters: Vec<EmitterDef> = doc.emitters(set_idx).to_vec();
        ui.label("Layers");
        let mut picked = None;
        for (i, em) in emitters.iter().enumerate() {
            if ui.selectable_label(self.emitter == i, &em.name).clicked() {
                picked = Some(i);
            }
        }
        if let Some(i) = picked {
            self.emitter = i;
            self.preview = None;
        }
        let Some(em) = emitters.get(self.emitter) else { return };
        let has_mesh = doc.primitive_of(entry, &em.name).is_some();
        ui.separator();
        for warning in self.warnings(em, has_mesh) {
            ui.colored_label(Color32::from_rgb(255, 190, 90), warning);
        }
        self.borrowed_note(ui, em);
        egui::CollapsingHeader::new("Look").default_open(true).show(ui, |ui| self.look(ui, em, has_mesh, idle));
        egui::CollapsingHeader::new("Behave like...").show(ui, |ui| self.recipes(ui, em, has_mesh));
        egui::CollapsingHeader::new("When it emits").show(ui, |ui| {
            self.slider(ui, em, "particle_data.life", 1.0..=240.0, "Each lasts (frames)", "How long one particle lives.");
            self.timing(ui, em);
        });
        egui::CollapsingHeader::new("How particles move").show(ui, |ui| self.movement(ui, em));
        egui::CollapsingHeader::new("Over their life").show(ui, |ui| self.over_life(ui, em));
        let trailing = self.value(em, "emission.is_emit_dist_enabled").unwrap_or(0.0) != 0.0;
        egui::CollapsingHeader::new("Trail").id_salt(("trail", set_idx, self.emitter)).default_open(trailing).show(ui, |ui| self.trail(ui, em));
        egui::CollapsingHeader::new("Rotation and following").show(ui, |ui| self.turning(ui, em));
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("new_effect_name").desired_width(150.0));
            if ui.add_enabled(idle && !self.new_name.trim().is_empty(), egui::Button::new("Duplicate effect")).clicked() {
                let name = self.new_name.trim().to_string();
                self.new_name.clear();
                self.run(Job::Duplicate(entry.to_string(), set_idx, name), "Duplicating");
            }
        });
        egui::CollapsingHeader::new("All values").show(ui, |ui| self.all_values(ui, em));
    }

    /// Keeping this effect's tuning over rebuilds of the generated file.
    fn tuning_row(&mut self, ui: &mut egui::Ui, doc: &Doc, entry: &str, set_idx: usize, idle: bool) {
        let Some(tuned) = self.tuned.clone() else {
            ui.label("Not in a git repo: no tuned file to keep effects in.");
            return;
        };
        let in_tuned_file = Tuned::is_tuned_file(&doc.path);
        let kept = tuned.has(entry);
        ui.horizontal_wrapped(|ui| {
            if kept {
                ui.colored_label(Color32::from_rgb(255, 210, 90), "\u{2605} Tuned: rebuilds keep this version");
            }
            let waiting = !self.pending.is_empty() || !self.applying.is_empty();
            let label = if kept { "Update tuned copy" } else { "Keep my tuning" };
            if !in_tuned_file && ui.add_enabled(idle && !waiting, egui::Button::new(label))
                .on_hover_text(format!("Copies this effect, as it is now, into {}. Every rebuild puts it back over the generated version.", tuned.path.display()))
                .clicked()
            {
                self.run(Job::Stage(entry.to_string(), set_idx, tuned.path.clone()), "Keeping");
            }
            if in_tuned_file {
                if kept && ui.button("Stop keeping").on_hover_text("Rebuilds go back to the generated version.").clicked() {
                    let mut tuned = tuned.clone();
                    tuned.effects.retain(|e| !e.eq_ignore_ascii_case(entry));
                    match tuned.save() {
                        Ok(()) => self.tuned = Some(tuned),
                        Err(e) => self.status = format!("Couldn't save the list: {e:#}"),
                    }
                } else if !kept && ui.add_enabled(idle, egui::Button::new("Keep this one")).clicked() {
                    self.run(Job::Stage(entry.to_string(), set_idx, tuned.path.clone()), "Keeping");
                }
                if let Some(into) = tuned.into.clone() {
                    let name = into.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui.add_enabled(idle && !self.unsaved, egui::Button::new(format!("Put into {name}")))
                        .on_hover_text(format!("Writes every tuned effect into {} now, without a rebuild. Save first.", into.display()))
                        .clicked()
                    {
                        self.run(Job::Merge(tuned.path.clone(), into), "Putting tuned effects in");
                    }
                }
            } else if tuned.path.exists() && ui.add_enabled(idle, egui::Button::new("Open tuned file")).clicked() {
                self.select = Some(entry.to_string());
                self.open(tuned.path.clone());
            }
        });
    }

    fn look(&mut self, ui: &mut egui::Ui, em: &EmitterDef, has_mesh: bool, idle: bool) {
        ui.horizontal(|ui| {
            if ui.add_enabled(idle, egui::Button::new("Swap texture…")).on_hover_text("A PNG. Alpha gives the shape. RGB blends between the two colours below: white is Colour, black is Colour 2.").clicked() {
                if let (Some(png), Some(target)) = (rfd::FileDialog::new().add_filter("PNG", &["png"]).pick_file(), self.target()) {
                    self.run(Job::Texture(target, png), "Swapping the texture");
                }
            }
            if ui.add_enabled(idle && has_mesh, egui::Button::new("Swap model…")).on_hover_text("A .glb. Sprite layers have no model to swap.").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("glTF model", &["glb", "gltf"]).pick_file() {
                    match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| eff_mesh_io::from_gltf(&b)) {
                        Ok(mesh) => self.draft_mesh = Some(mesh),
                        Err(e) => self.status = format!("That model didn't load: {e:#}"),
                    }
                }
            }
        });
        if has_mesh {
            ui.horizontal(|ui| {
                ui.label("Turn model");
                for (axis, label) in ["X", "Y", "Z"].iter().enumerate() {
                    if ui.small_button(*label).clicked() {
                        if self.draft_mesh.is_none() {
                            self.draft_mesh = self.preview.as_ref().and_then(|p| p.mesh.clone());
                        }
                        if let Some(mesh) = &mut self.draft_mesh {
                            quarter_turn(mesh, axis);
                        }
                    }
                }
                ui.label("Mirror");
                for (axis, label) in ["X", "Y", "Z"].iter().enumerate() {
                    if ui.small_button(*label).on_hover_text(format!("Flip the model across its {label} axis")).clicked() {
                        if self.draft_mesh.is_none() {
                            self.draft_mesh = self.preview.as_ref().and_then(|p| p.mesh.clone());
                        }
                        if let Some(mesh) = &mut self.draft_mesh {
                            mirror(mesh, axis);
                        }
                    }
                }
                if self.draft_mesh.is_some() && ui.add_enabled(idle, egui::Button::new("Use this model")).clicked() {
                    if let (Some(mesh), Some(target)) = (self.draft_mesh.take(), self.target()) {
                        self.run(Job::Mesh(target, mesh), "Swapping the model");
                    }
                }
            });
        }
        let size = self.value(em, "particle_scale.scale_x").unwrap_or(1.0);
        let mut v = size;
        if ui.add(egui::Slider::new(&mut v, 0.0..=if has_mesh { 2.0 } else { 40.0 }).text("Size")).changed() {
            for id in ["particle_scale.scale_x", "particle_scale.scale_y", "particle_scale.scale_z"] {
                self.set(em, id, v);
            }
        }
        self.slider(ui, em, "particle_scale.scale_random_x", 0.0..=100.0, "Size varies by %", "Each particle is up to this much smaller.")
            .map(|v| ["y", "z"].map(|a| self.set(em, &format!("particle_scale.scale_random_{a}"), v)));
        let ids = ["particle_color.color0_r", "particle_color.color0_g", "particle_color.color0_b"];
        let mut rgb = [0, 1, 2].map(|i| self.value(em, ids[i]).unwrap_or(1.0));
        ui.horizontal(|ui| {
            ui.label("Colour");
            let changed = ui.color_edit_button_rgb(&mut rgb).changed();
            let single = self.value(em, "emitter_static.num_color0_keys").unwrap_or(1.0) <= 1.0;
            if !single {
                ui.label("(changes over its life: edit under All values)");
            }
            if changed {
                for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                    self.set(em, ids[i], rgb[i]);
                    self.set(em, &format!("emitter_static.color0.keys[0].{axis}"), rgb[i]);
                }
            }
        });
        let ids = ["particle_color.color1_r", "particle_color.color1_g", "particle_color.color1_b"];
        let mut rgb = [0, 1, 2].map(|i| self.value(em, ids[i]).unwrap_or(1.0));
        ui.horizontal(|ui| {
            ui.label("Colour 2");
            if ui.color_edit_button_rgb(&mut rgb).on_hover_text("Where the texture's RGB is dark.").changed() {
                for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                    self.set(em, ids[i], rgb[i]);
                    self.set(em, &format!("emitter_static.color1.keys[0].{axis}"), rgb[i]);
                }
                // A shader built for animated colour reads every key, and a fixed colour leaves the rest black.
                self.set(em, "particle_color.color1_type", 2.0);
                self.set(em, "emitter_static.num_color1_keys", 1.0);
            }
        });
        self.slider(ui, em, "emitter_static.color_scale", 0.0..=8.0, "Brightness", "Multiplies the colour. Above 1 glows; fire in Smash runs 1 to 5.");
        let cells = self.value(em, "emitter_static.tex_pattern_anim0.num").unwrap_or(0.0);
        if cells > 1.0 {
            let mut step = self.value(em, "emitter_static.tex_pattern_anim0.frequency").unwrap_or(1.0);
            if ui.add(egui::Slider::new(&mut step, 0.25..=8.0).logarithmic(true).text("Frames per texture step"))
                .on_hover_text("How long each step of the texture's animation shows. Higher is slower: 2 is half speed.").changed()
            {
                self.set(em, "emitter_static.tex_pattern_anim0.frequency", step);
            }
        }
        let mut alpha = self.value(em, "particle_color.alpha0").unwrap_or(1.0);
        if ui.add(egui::Slider::new(&mut alpha, 0.0..=1.0).text("Opacity")).changed() {
            self.set(em, "particle_color.alpha0", alpha);
            for axis in ["x", "y", "z"] {
                self.set(em, &format!("emitter_static.alpha0.keys[0].{axis}"), alpha);
            }
        }
        let mut over = self.value(em, "render_state.is_depth_test").unwrap_or(1.0) == 0.0;
        if ui.checkbox(&mut over, "Draw over the stage").changed() {
            self.set(em, "render_state.is_depth_test", if over { 0.0 } else { 1.0 });
        }
        let mut glow = self.value(em, "render_state.blend_type").unwrap_or(0.0) == 1.0;
        if ui.checkbox(&mut glow, "Glow (additive)").changed() {
            self.set(em, "render_state.blend_type", if glow { 1.0 } else { 0.0 });
        }
    }

    fn turning(&mut self, ui: &mut egui::Ui, em: &EmitterDef) {
        ui.label("Rotation").on_hover_text("Turns the model or sprite within the effect. Spin is degrees a frame.");
        for axis in ["x", "y", "z"] {
            let (start, spin_id) = (format!("emitter_static.rotate_init_{axis}"), format!("emitter_static.rotate_add_{axis}"));
            let mut angle = self.value(em, &start).unwrap_or(0.0).to_degrees();
            let mut spin = self.value(em, &spin_id).unwrap_or(0.0).to_degrees();
            let (mut turned, mut spun) = (false, false);
            ui.horizontal(|ui| {
                ui.label(axis.to_uppercase());
                turned |= ui.add(egui::Slider::new(&mut angle, -180.0..=180.0).suffix("\u{b0}")).changed();
                for (label, step) in [("-90", -90.0), ("+90", 90.0)] {
                    if ui.small_button(label).clicked() {
                        angle += step;
                        turned = true;
                    }
                }
                spun |= ui.add(egui::DragValue::new(&mut spin).speed(0.5).suffix("\u{b0}/f")).on_hover_text("Spin").changed();
            });
            if turned {
                // Keep it in a half turn either way, so the buttons never run off the slider.
                self.set(em, &start, ((angle + 180.0).rem_euclid(360.0) - 180.0).to_radians());
            }
            if spun {
                self.set(em, &spin_id, spin.to_radians());
            }
            if turned || spun {
                // The shader ignores an axis this flag leaves off.
                self.set(em, &format!("particle_data.is_rotate_{axis}"), 1.0);
            }
        }
        self.slider(ui, em, "emitter_static.rotate_init_rand_z", 0.0..=std::f32::consts::PI, "Random roll (radians)", "Each particle starts turned up to this much either way: makes puffs look different.");
        ui.separator();
        let mut follow = self.value(em, "emitter_info.follow_type").unwrap_or(0.0) as i32;
        let before = follow;
        ui.label("Once a particle appears, it…").on_hover_text("How it keeps to the bone the move's script attaches the effect to.");
        for (value, label, hover) in controls::FOLLOW_TYPES {
            ui.radio_value(&mut follow, value, label).on_hover_text(hover);
        }
        if follow != before {
            self.set(em, "emitter_info.follow_type", follow as f32);
        }
    }
}

impl App {
    /// `sweep`: show every effect in turn, early, midway and late, so anything that can't draw lands in the log. Last,
    /// one edit, to see the window keep its file while it goes in.
    fn walk(&mut self, ctx: &egui::Context) {
        const AGES: [f32; 3] = [5.0, 20.0, 60.0];
        let Some(next) = self.sweep else { return };
        self.edit_kept &= self.edit_started.is_none() || self.doc.is_some();
        let Some(doc) = self.doc.clone() else { return };
        if crate::eff_shader_cache::ready(&doc.working()).is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        ctx.request_repaint();
        let list = doc.effects();
        if next < list.len() * AGES.len() {
            self.effect = list.get(next % list.len()).cloned();
            self.sweep = Some(next + 1);
            if let Some(game) = &mut self.game {
                game.age = AGES[next / list.len()];
                game.playing = false;
            }
            return;
        }
        match self.edit_started {
            None if self.busy.is_none() => {
                let Some(em) = self.effect.as_ref().and_then(|e| doc.emitters(e.1).first()).cloned() else { return };
                crate::log::note(&format!("sweep finished: {} effects", list.len()));
                self.set(&em, "particle_data.life", 20.0);
                self.edit_started = Some(std::time::Instant::now());
            }
            Some(started) if self.busy.is_none() && self.pending.is_empty() && self.applying.is_empty() => {
                crate::log::note(&format!("edit took {:.2}s: {}, file on screen throughout: {}", started.elapsed().as_secs_f32(), self.status, self.edit_kept));
                self.sweep = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
    }

    /// `shot`: save the window's own frame to a file, then close. Needs neither focus nor the desktop.
    fn take_shot(&mut self, ctx: &egui::Context) {
        let Some(shot) = self.shot.as_mut() else { return };
        ctx.request_repaint();
        let image = ctx.input(|i| i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        }));
        if let Some(image) = image {
            let [width, height] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let saved = image::RgbaImage::from_raw(width as u32, height as u32, rgba).map(|i| i.save(&shot.out));
            crate::log::note(&format!("shot {}: {saved:?}", shot.out.display()));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let Some(game) = self.game.as_mut() else { return };
        if self.drawn {
            game.age = shot.frame;
            game.playing = false;
            game.solo = shot.layer;
            game.outline = shot.outline;
            game.orbit = Vec2::new(shot.orbit[0], shot.orbit[1]);
            game.half_view = shot.view;
            game.swing = shot.swing;
            shot.settled += 1;
            // A few frames for the egui texture to catch up with the preview.
            if shot.settled == 5 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let ctx = ui.ctx().clone();
        // Changes go in as soon as a slider is let go.
        if self.busy.is_none() && !self.pending.is_empty() && !ctx.input(|i| i.pointer.any_down()) {
            let pending = std::mem::take(&mut self.pending);
            self.applying = pending.clone();
            self.run(Job::Values(pending), "Applying");
        }
        if self.busy.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.walk(&ctx);
        self.take_shot(&ctx);
        self.load_preview(&ctx);
        egui::Panel::top("top").show_inside(ui, |ui| self.top_bar(ui));
        egui::Panel::left("effects").default_size(260.0).show_inside(ui, |ui| self.effect_list(ui));
        egui::Panel::right("values").default_size(360.0).show_inside(ui, |ui| self.panel(ui));
        egui::CentralPanel::default().show_inside(ui, |ui| self.view(ui));
    }
}
