//! The window: effect list on the left, a preview from Smash's camera in the middle, values on the right.

mod controls;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use anyhow::Result;
use eframe::egui::{self, Color32, Mesh, Pos2, Sense, Shape, TextureHandle, TextureOptions, Vec2};

use crate::eff::fields::AttrValue;
use crate::eff::mesh_io::{self, MeshData};
use crate::eff::model::EmitterDef;
use crate::engine::{attr, game_root, kit, AlphaFrom, TextureFrom, mirror, quarter_turn, set_game_root, Doc, Pending, Piece, Target, Tuned, STYLES};
use crate::preview::Preview as GamePreview;
use crate::project::{self, Project};
use crate::settings::{Settings, Theme};

enum Job {
    Open(PathBuf),
    Values(Pending),
    Texture(Target, PathBuf, TextureFrom),
    Mesh(Target, MeshData),
    Duplicate(String, usize, String),
    /// A new effect of this name from a piece of the kit.
    NewEffect(String, &'static Piece, Option<MeshData>),
    /// A piece of the kit added to an effect as more layers.
    AddLayer(String, &'static Piece, Option<MeshData>),
    /// Layer n of an effect (and the layers under it) copied, the copy placed after them.
    DuplicateLayer(String, usize, usize),
    /// Layer n of an effect (and the layers under it) taken out.
    RemoveLayer(String, usize, usize),
    /// Layer n of an effect switched to a value of `STYLES[.3]`, on a shader drawn that way.
    Restyle(String, usize, usize, usize, f32),
    /// Effect `.0` called `.1`.
    Rename(String, String),
    Save,
    /// The file without the skipped effects, written to the path.
    Export(Vec<String>, PathBuf),
    /// Copy an effect into the tuned file.
    Stage(String, usize, PathBuf),
    /// Put the tuned file's effects into the file it was staged from.
    Merge(PathBuf, PathBuf),
    /// Effects copied in from another .eff; replacing ones of the same name when true.
    Import(PathBuf, Vec<String>, bool),
    /// Layers of this effect (or every effect) put on shaders that match how they follow their bone.
    FixTrails(Option<String>),
    /// A layer moved to a shader built for values just set.
    Reshade(controls::Reshade),
}

/// Effects to copy in from another file, being picked.
struct ImportPick {
    from: PathBuf,
    /// Every effect in it, with whether it is picked.
    names: Vec<(String, bool)>,
    filter: String,
    replace: bool,
}

enum Done {
    Doc(Result<Doc>, String),
    /// A layer copied: the file, and the copy's layer number to show.
    Layer(Result<Doc>, usize),
    Saved(Result<PathBuf>),
    Exported(Result<usize>, PathBuf),
    Tuned(Result<Tuned>, String),
    /// A layer on a new shader: what to show, and the layer (set, layer) with where its shader came from, or `None`
    /// to forget that it borrowed one.
    /// `bool`: this went back to the layer's own look.
    Reshaded(Result<Doc>, String, (usize, usize), Option<controls::Borrowed>, bool),
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
    /// The name for an effect made from the kit.
    create_name: String,
    /// The model being turned or newly loaded, not yet applied.
    draft_mesh: Option<MeshData>,
    /// How the next swapped-in picture becomes a texture.
    texture_from: TextureFrom,
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
    /// The open file's project: its folders and where it exports.
    project: Option<Project>,
    /// A rename on its way, for the project's folders to follow once it lands.
    renaming: Option<(String, String)>,
    rename_to: String,
    /// The import window, while open.
    importing: Option<ImportPick>,
    /// An effect whose trail was just switched: its shaders are checked once the change is in.
    trail_fix: Option<String>,
    /// A shader to find once the values being applied are in.
    reshade: Option<controls::Reshade>,
    folder_name: String,
    /// A folder being renamed and the name typed so far.
    folder_edit: Option<(usize, String)>,
    /// Effects picked together in a project's list (ctrl- and shift-click), to drag or move at once.
    chosen: Vec<String>,
    /// Where a shift-click range starts.
    anchor: Option<String>,
    /// Arrow keys not yet acted on: effects to move down the list (up is negative), with shift held, and frames to
    /// step the preview.
    nav: (i32, bool),
    frame_step: i32,
    /// Scroll the list to the effect shown, once: it was picked with the keys.
    reveal: bool,
    settings: Settings,
    /// Since when the file has had changes not saved, counted from the last edit.
    unsaved_since: Option<std::time::Instant>,
    /// The save running was started by auto save.
    auto_saving: bool,
}

/// Auto save waits this long after the last change.
const AUTO_SAVE_AFTER: std::time::Duration = std::time::Duration::from_secs(3);

/// What every effect in a project's list draws with.
struct ListItems<'a> {
    chosen: &'a [String],
    current: Option<String>,
    folders: Vec<String>,
    actions: Vec<ListAction>,
    /// Names in the order drawn, for shift-click ranges.
    drawn: Vec<String>,
    /// Scroll to the current effect.
    reveal: bool,
}

/// What the list was asked to do, done once it is drawn.
enum ListAction {
    /// Clicked, with the keys held.
    Pick(String, usize, egui::Modifiers),
    Move(Vec<String>, Option<usize>),
    Skip(usize, bool),
    RenameFolder(usize, String),
    RemoveFolder(usize),
}

/// One picture for the `shot` command.
pub struct Shot {
    pub out: PathBuf,
    pub frame: f32,
    pub layer: Option<usize>,
    pub outline: Option<usize>,
    pub ghost: Option<usize>,
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
            new_name: String::new(), create_name: String::new(), draft_mesh: None, texture_from: TextureFrom::default(), preview: None, orbit: Vec2::ZERO,
            search: String::new(), select: None,
            game: cc.wgpu_render_state.clone().map(GamePreview::new), last_tick: std::time::Instant::now(),
            tuned: None, only_tuned: false, borrowed: Default::default(),
            project: None, renaming: None, rename_to: String::new(), importing: None, trail_fix: None, reshade: None, folder_name: String::new(), folder_edit: None,
            chosen: Vec::new(), anchor: None, nav: (0, false), frame_step: 0, reveal: false,
            settings: Settings::load(), unsaved_since: None, auto_saving: false,
        };
        app.settings.theme.apply(&cc.egui_ctx);
        if let Some(game) = &mut app.game {
            game.backdrop = app.settings.theme.backdrop();
        }
        app.select = select;
        if app.shot.is_none() {
            crate::engine::prewarm_game_shaders();
        }
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
                Job::Rename(old, new) => Done::Doc(doc.expect("doc").rename(&old, &new), format!("Renamed {old} to {new}")),
                Job::Export(left_out, to) => {
                    let doc = doc.expect("a document to export");
                    Done::Exported(project::export(&doc.bytes, &left_out, &to), to)
                }
                Job::Save => {
                    let doc = doc.expect("a document to save");
                    Done::Saved(doc.save().map(|_| doc.path.clone()))
                }
                Job::Values(pending) => Done::Doc(doc.expect("doc").apply_values(&pending), "Applied".into()),
                Job::Texture(target, png, how) => match doc.expect("doc").replace_texture_from(&target, &png, how) {
                    Ok((doc, notes)) if notes.is_empty() => Done::Doc(Ok(doc), "Texture swapped".into()),
                    Ok((doc, notes)) => Done::Doc(Ok(doc), format!("Texture swapped, but {}", notes.join("; "))),
                    Err(e) => Done::Doc(Err(e), "Swapping the texture".into()),
                },
                Job::Mesh(target, mesh) => Done::Doc(doc.expect("doc").replace_mesh(&target, &mesh), "Model swapped".into()),
                Job::Duplicate(entry, set, name) => Done::Doc(doc.expect("doc").duplicate(&entry, set, &name), format!("Duplicated as {name}")),
                Job::NewEffect(name, piece, model) => Done::Doc(doc.expect("doc").new_effect(&name, piece, model.as_ref()),
                    format!("Made {name} from {}. Keep my tuning to have rebuilds keep it", piece.name)),
                Job::AddLayer(entry, piece, model) => Done::Doc(doc.expect("doc").add_layer(&entry, piece, model.as_ref()), format!("Added {}", piece.name)),
                Job::DuplicateLayer(entry, set, layer) => match doc.expect("doc").duplicate_layer(&entry, set, layer) {
                    Ok((doc, at)) => Done::Layer(Ok(doc), at),
                    Err(e) => Done::Layer(Err(e), layer),
                },
                Job::RemoveLayer(entry, set, layer) => Done::Doc(doc.expect("doc").remove_layer(&entry, set, layer), "Layer removed".into()),
                Job::Restyle(entry, set, layer, style, value) => {
                    let style = &STYLES[style];
                    let what = format!("{} {}", style.label, if value != 0.0 { "on" } else { "off" });
                    match doc.expect("doc").restyle(&entry, set, layer, style, value) {
                        Ok((doc, Some(r))) => {
                            let mut note = format!("{what}: shader from {}", r.from);
                            if !r.taken.is_empty() {
                                let names: Vec<&str> = r.taken.iter().map(|id| {
                                    crate::eff::fields::table().iter().find(|a| a.id == id).map_or(id.as_str(), |a| a.label)
                                }).collect();
                                note += &format!("; it also needs {} as that layer has them", names.join(", "));
                            }
                            Done::Doc(Ok(doc), note)
                        }
                        Ok((doc, None)) => Done::Doc(Ok(doc), format!("{what}: no shader in this file{} draws this layer that way, so nothing changed",
                            if game_root().is_some() { " or the game's" } else { " (choose the game files folder to look in the game's too)" })),
                        Err(e) => Done::Doc(Err(e), what),
                    }
                }
                Job::Stage(entry, set, tuned) => Done::Tuned(doc.expect("doc").stage(&entry, set, &tuned), format!("{entry} kept in {}", tuned.display())),
                Job::Merge(tuned, into) => Done::Tuned(
                    std::fs::read(&into).map_err(anyhow::Error::from)
                        .and_then(|bytes| crate::engine::merge_tuned(&tuned, &bytes))
                        .and_then(|(bytes, _)| Ok(std::fs::write(&into, bytes)?))
                        .map(|_| Tuned::load(&tuned)),
                    format!("Tuned effects put into {}", into.display())),
                Job::Reshade(r) => {
                    let doc = doc.expect("doc");
                    let (key, back) = ((r.set, r.layer), r.back.is_none());
                    let want: Vec<(&str, f32)> = r.want.iter().map(|(id, v)| (id.as_str(), *v)).collect();
                    let before = doc.emitters(r.set).get(r.layer).cloned();
                    let was = |id: &str| before.as_ref().and_then(|em| attr(em, id)).map(|v| v.as_f32());
                    match doc.reshade(&r.entry, r.set, r.layer, &want, &[]) {
                        Ok((doc, Some(found))) => {
                            let names: Vec<&str> = found.taken.iter().map(|id| {
                                crate::eff::fields::table().iter().find(|a| a.id == id).map_or(id.as_str(), |a| a.label)
                            }).collect();
                            let mut note = format!("{}: shader from {}", r.what, found.from);
                            if !names.is_empty() {
                                note += &format!("; it also needs {} as that layer has them", names.join(", "));
                            }
                            // what goes back: the values asked for, and what the shader also needed, as they were
                            let borrowed = r.back.map(|mut undo| {
                                let modes = undo.clone();
                                for id in &found.taken {
                                    if let Some(v) = was(id).filter(|_| !undo.iter().any(|(i, _)| i == id)) {
                                        undo.push((id.clone(), v));
                                    }
                                }
                                controls::Borrowed { from: found.from, undo, modes }
                            });
                            Done::Reshaded(Ok(doc), note, key, borrowed, back)
                        }
                        Ok((doc, None)) => Done::Reshaded(Ok(doc), format!("{}: no shader in this file{} draws this layer that way, so                             the value is set but the preview and the game won't show it", r.what,
                            if game_root().is_some() { " or the game's" } else { " (choose the game files folder to look in the game's too)" }), key, None, back),
                        Err(e) => Done::Reshaded(Err(e), r.what, key, None, back),
                    }
                }
                Job::FixTrails(only) => match doc.expect("doc").fix_trail_shaders(only.as_deref()) {
                    Ok((doc, found)) => {
                        let fixed = found.iter().filter(|m| m.fix.is_some()).count();
                        let mut note = match found.len() {
                            0 => "Every layer's shader already matches how it follows its bone".to_string(),
                            n if fixed == n => format!("Put {n} layers on shaders that match how they follow their bone"),
                            n => format!("Put {fixed} of {n} layers on shaders that match how they follow their bone; the rest have none"),
                        };
                        if fixed < found.len() && game_root().is_none() {
                            note += " (choose the game files folder: the shaders come from the game's own effects)";
                        }
                        Done::Doc(Ok(doc), note)
                    }
                    Err(e) => Done::Doc(Err(e), "Fixing trail shaders".into()),
                },
                Job::Import(from, names, replace) => {
                    let file = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    match doc.expect("doc").import(&from, &names, replace) {
                        Ok((doc, kept)) if kept.is_empty() => Done::Doc(Ok(doc), format!("Imported {} effects from {file}", names.len())),
                        Ok((doc, kept)) => Done::Doc(Ok(doc), format!("Imported {} effects from {file}; kept {} already here as they were",
                            names.len() - kept.len(), kept.len())),
                        Err(e) => Done::Doc(Err(e), "Import".into()),
                    }
                }
            }));
            let _ = tx.send(result.unwrap_or_else(|panic| Done::Doc(Err(anyhow::anyhow!(panic_text(panic))), label)));
        });
    }

    fn poll(&mut self) {
        while let Ok(done) = self.done.try_recv() {
            // a copied layer lands like any other change, then shows the copy
            let (done, show_layer) = match done {
                Done::Layer(Ok(doc), at) => (Done::Doc(Ok(doc), "Layer duplicated".into()), Some(at)),
                Done::Layer(Err(e), _) => (Done::Doc(Err(e), "Duplicating the layer".into()), None),
                // a borrow keeps what to put back, merged with any earlier one so the way back is to the layer's own
                Done::Reshaded(doc, note, key, borrowed, back) => {
                    let earlier = self.borrowed.remove(&key);
                    match (borrowed, earlier) {
                        (Some(mut b), Some(earlier)) if doc.is_ok() => {
                            let first = |now: &mut Vec<(String, f32)>, was: Vec<(String, f32)>| {
                                for (id, v) in was.into_iter().rev() {
                                    now.retain(|(i, _)| *i != id);
                                    now.insert(0, (id, v));
                                }
                            };
                            first(&mut b.undo, earlier.undo);
                            first(&mut b.modes, earlier.modes);
                            self.borrowed.insert(key, b);
                        }
                        (Some(b), None) if doc.is_ok() => {
                            self.borrowed.insert(key, b);
                        }
                        // nothing new borrowed: the layer is still on the earlier shader, unless it went back
                        (_, Some(earlier)) if !back || doc.is_err() => {
                            self.borrowed.insert(key, earlier);
                        }
                        _ => {}
                    }
                    (Done::Doc(doc, note), None)
                }
                done => (done, None),
            };
            if let Some(at) = show_layer {
                self.emitter = at;
            }
            match done {
                Done::Doc(Ok(doc), note) => {
                    if let Some(name) = self.select.take() {
                        self.effect = doc.effects().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(&name));
                        self.emitter = 0;
                    }
                    if let Some((old, new)) = self.renaming.take() {
                        self.chosen.clear();
                        self.anchor = None;
                        if let Some(project) = &mut self.project {
                            project.rename_effect(&old, &new);
                        }
                    }
                    if note == "Opened" {
                        self.chosen.clear();
                        self.anchor = None;
                        self.project = project::project_of_eff(&doc.path).filter(|p| p.exists()).and_then(|p| Project::load(&p).ok());
                        if let Some(project) = &mut self.project {
                            let names: Vec<String> = doc.effects().into_iter().map(|(n, _)| n).collect();
                            project.tidy(&names);
                        }
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
                    // a trail just switched on or off: its layers want shaders to match
                    // a value compiled into the shader: the layer wants one built for it
                    if note == "Applied" {
                        if let Some(r) = self.reshade.take() {
                            let what = r.what.clone();
                            self.run(Job::Reshade(r), &format!("Finding a shader for {what}"));
                        } else if let Some(effect) = self.trail_fix.take() {
                            if self.doc.as_ref().is_some_and(|d| !d.trail_mismatches(Some(&effect)).is_empty()) {
                                self.run(Job::FixTrails(Some(effect)), "Matching the shader to the trail");
                            }
                        }
                    }
                    self.status = note;
                }
                Done::Layer(..) | Done::Reshaded(..) => unreachable!("turned into Done::Doc above"),
                Done::Doc(Err(e), note) => {
                    self.busy = None;
                    self.renaming = None;
                    self.applying.clear();
                    crate::log::note(&format!("{note} failed: {e:#}"));
                    self.status = format!("{note} failed: {e:#}");
                }
                Done::Saved(Ok(path)) => {
                    self.busy = None;
                    self.unsaved = false;
                    let how = if std::mem::take(&mut self.auto_saving) { "Auto saved" } else { "Saved" };
                    self.status = format!("{how} {}", path.display());
                    if let Some(project) = &self.project {
                        match project.save() {
                            Ok(()) => self.status = format!("{how} {}", project.path.display()),
                            Err(e) => self.status = format!("Saved the effects, but not the folders: {e:#}"),
                        }
                    }
                }
                Done::Exported(Ok(skipped), to) => {
                    self.busy = None;
                    let left = if skipped == 0 { String::new() } else { format!(", {skipped} skipped") };
                    self.status = format!("Exported to {}{left}", to.display());
                }
                Done::Exported(Err(e), _) => {
                    self.busy = None;
                    self.status = format!("Export failed: {e:#}");
                }
                Done::Saved(Err(e)) => {
                    self.busy = None;
                    // a failing save would otherwise be retried every few seconds
                    if std::mem::take(&mut self.auto_saving) {
                        self.settings.auto_save = false;
                    }
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
        // a project opens its own effect file, and the project comes with it
        let path = if project::is_project(&path) {
            match Project::load(&path) {
                Ok(project) => project.eff_path(),
                Err(e) => {
                    self.status = format!("{e:#}");
                    return;
                }
            }
        } else {
            path
        };
        self.run(Job::Open(path), "Opening");
    }

    /// The project items of the File menu: start one from the open file, or export the open one.
    fn project_items(&mut self, ui: &mut egui::Ui, ready: bool) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(project) = self.project.clone() else {
            if ui.add_enabled(ready, egui::Button::new("New project…"))
                .on_hover_text("Work in a copy of this file, with its effects sorted into folders. Export puts it in the mod, or anywhere; folders marked to skip stay out.")
                .clicked()
            {
                let dir = crate::scratch_dirs::app_storage_root().join("projects");
                let _ = std::fs::create_dir_all(&dir);
                let stem = doc.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if let Some(path) = rfd::FileDialog::new().add_filter("Effect Forge project", &[project::EXTENSION])
                    .set_directory(&dir).set_file_name(format!("{stem}.{}", project::EXTENSION)).save_file()
                {
                    // a tuned file exports back into the file it was staged from
                    let export_to = Tuned::is_tuned_file(&doc.path).then(|| Tuned::load(&doc.path).into).flatten()
                        .unwrap_or_else(|| doc.path.clone());
                    match Project::create(&path, &doc.bytes, Some(export_to)) {
                        Ok(project) => {
                            self.select = self.effect.as_ref().map(|e| e.0.clone());
                            self.open(project.eff_path());
                            self.unsaved = false;
                        }
                        Err(e) => self.status = format!("{e:#}"),
                    }
                }
            }
            return;
        };
        if let Some(to) = project.export_to.clone() {
            let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if ui.add_enabled(ready, egui::Button::new(format!("Export to {name}"))).on_hover_text(self.export_hint(&to)).clicked() {
                self.export(to);
            }
        }
        let left_out = project.left_out();
        let skipping = if left_out.is_empty() { String::new() } else { format!(" Skips {} effects in folders marked to skip.", left_out.len()) };
        if ui.add_enabled(ready, egui::Button::new("Export to…")).on_hover_text(format!("Pick where to export; the project remembers it.{skipping}")).clicked() {
            let mut dialog = rfd::FileDialog::new().add_filter("Effect file", &["eff"]);
            if let Some(to) = &project.export_to {
                if let Some(dir) = to.parent() {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(name) = to.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
            }
            if let Some(to) = dialog.save_file() {
                if let Some(project) = &mut self.project {
                    project.export_to = Some(to.clone());
                    if let Err(e) = project.save() {
                        self.status = format!("{e:#}");
                    }
                }
                self.run(Job::Export(left_out, to), "Exporting");
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            let idle = self.busy.is_none();
            let waiting = !self.pending.is_empty() || !self.applying.is_empty();
            ui.menu_button("File", |ui| self.file_menu(ui, idle, waiting));
            ui.menu_button("Options", |ui| self.options_menu(ui));
            ui.separator();
            // the two things done most, one click away
            let can_save = idle && self.doc.is_some() && self.unsaved && !waiting;
            let save = if self.project.is_some() { "Save project" } else { "Save to mod" };
            if ui.add_enabled(can_save, egui::Button::new(save)).on_hover_text(self.save_hint()).clicked() {
                self.run(Job::Save, "Saving");
            }
            if let Some(to) = self.project.as_ref().and_then(|p| p.export_to.clone()) {
                let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ui.add_enabled(idle && !waiting, egui::Button::new(format!("Export to {name}"))).on_hover_text(self.export_hint(&to)).clicked() {
                    self.export(to);
                }
            }
            ui.separator();
            if let Some(busy) = &self.busy {
                ui.spinner();
                ui.label(busy);
            } else {
                if self.unsaved {
                    let note = if self.settings.auto_save { "\u{25CF} saving soon" } else { "\u{25CF} unsaved" };
                    ui.colored_label(ui.visuals().warn_fg_color, note);
                }
                ui.label(&self.status);
            }
        });
    }

    fn save_hint(&self) -> &'static str {
        if self.project.is_some() {
            "Saves the project: its effects and folders. Export puts them in the mod."
        } else {
            "Writes the .eff you opened. The first save keeps a .forge-backup beside it."
        }
    }

    fn export_hint(&self, to: &Path) -> String {
        let left_out = self.project.as_ref().map_or(0, |p| p.left_out().len());
        let skipping = if left_out == 0 { String::new() } else { format!(" Skips {left_out} effects in folders marked to skip.") };
        format!("Writes {}. The first export keeps a .forge-backup of what was there.{skipping}", to.display())
    }

    fn export(&mut self, to: PathBuf) {
        let left_out = self.project.as_ref().map(|p| p.left_out()).unwrap_or_default();
        self.run(Job::Export(left_out, to), "Exporting");
    }

    fn file_menu(&mut self, ui: &mut egui::Ui, idle: bool, waiting: bool) {
        if ui.add_enabled(idle, egui::Button::new("Open…")).on_hover_text("An .eff, or a project (.forge).").clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("Effect file or project", &["eff", project::EXTENSION]).pick_file() {
                self.open(path);
            }
        }
        let recent = self.recent.clone();
        ui.add_enabled_ui(idle && !recent.is_empty(), |ui| {
            ui.menu_button("Open recent", |ui| {
                for path in recent {
                    if ui.button(path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()).on_hover_text(path.display().to_string()).clicked() {
                        self.open(path);
                    }
                }
            });
        });
        ui.separator();
        let can_save = idle && self.doc.is_some() && self.unsaved && !waiting;
        let save = if self.project.is_some() { "Save project" } else { "Save to mod" };
        if ui.add_enabled(can_save, egui::Button::new(save)).on_hover_text(self.save_hint()).clicked() {
            self.run(Job::Save, "Saving");
        }
        let hint = "Copy effects from another .eff into this file, under the same names.";
        if ui.add_enabled(idle && self.doc.is_some() && !waiting, egui::Button::new("Import effects…")).on_hover_text(hint).clicked() {
            if let Some(from) = rfd::FileDialog::new().add_filter("Effect file", &["eff"]).pick_file() {
                match crate::engine::effect_names(&from) {
                    Ok(names) => {
                        self.importing = Some(ImportPick { from, names: names.into_iter().map(|n| (n, false)).collect(), filter: String::new(), replace: false });
                    }
                    Err(e) => self.status = format!("{e:#}"),
                }
            }
        }
        let hint = "Layers that stay where their particles are born (trails) need a shader that keeps them there, \
                    and the rest one that doesn't. Puts every layer that has the wrong one on one that matches, \
                    copied from the game's own effects when this file has none.";
        if ui.add_enabled(idle && self.doc.is_some() && !waiting, egui::Button::new("Fix trail shaders")).on_hover_text(hint).clicked() {
            self.run(Job::FixTrails(None), "Fixing trail shaders");
        }
        ui.separator();
        self.project_items(ui, idle && !waiting);
        ui.separator();
        let root = crate::engine::game_root();
        let hint = match &root {
            Some(root) => format!("The starter kit reads from {}.", root.display()),
            None => "The dumped game folder that holds the effect folder; the starter kit reads from it.".to_string(),
        };
        if ui.button("Game files folder…").on_hover_text(hint).clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                match set_game_root(&dir) {
                    Ok(root) => self.status = format!("Kit reads from {}", root.display()),
                    Err(e) => self.status = format!("{e:#}"),
                }
            }
        }
    }

    /// A warning, with its fix, when the layer's shader doesn't match how it follows its bone.
    fn trail_shader_note(&mut self, ui: &mut egui::Ui, doc: &Doc, entry: &str, em: &EmitterDef, idle: bool) {
        let stored = |id: &str| crate::engine::attr(em, id).map(|v| v.as_f32());
        let (Some(follow), Some(shader)) = (stored("emitter_info.follow_type"), stored("shader_references.shader_index")) else { return };
        let Some(reads) = doc.shader_reads_birth(shader as u32) else { return };
        if reads == (follow != 0.0) {
            return;
        }
        let text = if follow != 0.0 {
            "In game this layer rides its bone and leaves no trail: its shader doesn't keep particles where they were born."
        } else {
            "In game this layer leaves its particles behind: its shader keeps them where they were born."
        };
        ui.colored_label(Color32::from_rgb(255, 190, 90), text);
        if ui.add_enabled(idle && self.pending.is_empty(), egui::Button::new("Fix the shader"))
            .on_hover_text("Puts this effect's layers on shaders that match, copied from the game's own effects if need be. Nothing else changes.")
            .clicked()
        {
            self.run(Job::FixTrails(Some(entry.to_string())), "Fixing trail shaders");
        }
    }

    /// The window that picks effects to copy in from another file.
    fn import_window(&mut self, ctx: &egui::Context) {
        let Some(pick) = &mut self.importing else { return };
        let here: std::collections::HashSet<String> = self.doc.as_ref()
            .map(|d| d.effects().into_iter().map(|(n, _)| n.to_ascii_lowercase()).collect())
            .unwrap_or_default();
        let idle = self.busy.is_none();
        let mut open = true;
        let mut go = false;
        let title = format!("Import from {}", pick.from.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
        egui::Window::new(title).id(egui::Id::new("import effects")).open(&mut open).default_size([380.0, 480.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.text_edit_singleline(&mut pick.filter);
            });
            let filter = pick.filter.to_lowercase();
            let shown = |n: &str| n.to_lowercase().contains(&filter);
            ui.horizontal(|ui| {
                if ui.button("Pick shown").clicked() {
                    pick.names.iter_mut().filter(|(n, _)| shown(n)).for_each(|(_, on)| *on = true);
                }
                if ui.button("Pick none").clicked() {
                    pick.names.iter_mut().for_each(|(_, on)| *on = false);
                }
            });
            ui.checkbox(&mut pick.replace, "Replace effects with the same name")
                .on_hover_text("Off: an effect this file already has is left as it is.");
            ui.separator();
            let count = pick.names.iter().filter(|(_, on)| *on).count();
            ui.add_enabled_ui(idle && count > 0, |ui| {
                go = ui.button(format!("Import {count}")).clicked();
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (name, on) in pick.names.iter_mut().filter(|(n, _)| shown(n)) {
                    if here.contains(&name.to_ascii_lowercase()) {
                        ui.checkbox(on, format!("{name}  (already here)"))
                            .on_hover_text("This file has an effect by this name: it is replaced only with the box above ticked.");
                    } else {
                        ui.checkbox(on, name.as_str());
                    }
                }
            });
        });
        if go {
            let pick = self.importing.take().expect("picking");
            let names = pick.names.into_iter().filter(|(_, on)| *on).map(|(n, _)| n).collect();
            self.run(Job::Import(pick.from, names, pick.replace), "Importing");
        } else if !open {
            self.importing = None;
        }
    }

    fn options_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Theme", |ui| {
            for theme in Theme::ALL {
                if ui.radio(self.settings.theme == theme, theme.name()).clicked() {
                    self.settings.theme = theme;
                    theme.apply(ui.ctx());
                    if let Some(game) = &mut self.game {
                        game.backdrop = theme.backdrop();
                    }
                    self.settings.save();
                }
            }
        });
        if ui.checkbox(&mut self.settings.auto_save, "Auto save")
            .on_hover_text("Saves a few seconds after your last change. Outside a project that writes the .eff you opened.")
            .changed()
        {
            self.settings.save();
        }
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
        self.new_effect_row(ui, &doc);
        let filter = self.filter.to_lowercase();
        let list = doc.effects();
        let tuned = self.tuned.clone().unwrap_or_default();
        if !tuned.effects.is_empty() {
            ui.checkbox(&mut self.only_tuned, format!("Only tuned ({})", tuned.effects.len()))
                .on_hover_text("\u{2605} marks effects kept in the tuned file: rebuilds put these back over the generated ones.");
        }
        if self.project.is_some() {
            self.project_list(ui, &list, &filter);
            return;
        }
        let mut picked = None;
        let reveal = std::mem::take(&mut self.reveal);
        let shown: Vec<&(String, usize)> = list.iter().filter(|(n, _)| n.to_lowercase().contains(&filter) && (!self.only_tuned || tuned.has(n))).collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (name, set) in &shown {
                let on = self.effect.as_ref().is_some_and(|e| &e.0 == name);
                let label = if tuned.has(name) { format!("\u{2605} {name}") } else { name.clone() };
                let response = ui.selectable_label(on, label);
                if on && reveal {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    picked = Some((name.clone(), *set));
                }
            }
        });
        let names: Vec<String> = shown.iter().map(|(n, _)| n.clone()).collect();
        if let Some(name) = step(&names, self.effect.as_ref().map(|e| e.0.as_str()), std::mem::take(&mut self.nav.0)) {
            picked = shown.iter().find(|(n, _)| *n == name).map(|(n, s)| (n.clone(), *s));
            self.reveal = true;
        }
        if let Some(effect) = picked {
            self.effect = Some(effect);
            self.emitter = 0;
            self.preview = None;
        }
    }

    /// A project's effects by folder. Drag an effect onto a folder to move it; a ticked folder is skipped on export.
    fn project_list(&mut self, ui: &mut egui::Ui, list: &[(String, usize)], filter: &str) {
        let Some(project) = self.project.clone() else { return };
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.folder_name).hint_text("folder name").desired_width(120.0));
            if ui.add_enabled(!self.folder_name.trim().is_empty(), egui::Button::new("New folder")).clicked() {
                if let Some(project) = &mut self.project {
                    match project.add_folder(&self.folder_name) {
                        Ok(()) => {
                            self.folder_name.clear();
                            self.unsaved = true;
                        }
                        Err(e) => self.status = format!("{e:#}"),
                    }
                }
            }
        });
        let shown = |name: &String| name.to_lowercase().contains(filter);
        let dragging = egui::DragAndDrop::has_payload_of_type::<Vec<String>>(ui.ctx());
        let chosen = self.chosen.clone();
        let mut items = ListItems {
            chosen: &chosen,
            current: self.effect.as_ref().map(|e| e.0.clone()),
            folders: project.folders.iter().map(|f| f.name.clone()).collect(),
            actions: Vec::new(),
            drawn: Vec::new(),
            reveal: std::mem::take(&mut self.reveal),
        };
        let folder_edit = &mut self.folder_edit;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, folder) in project.folders.iter().enumerate() {
                let effects: Vec<&(String, usize)> = list.iter().filter(|(n, _)| folder.effects.iter().any(|e| e.eq_ignore_ascii_case(n))).collect();
                let dropped = drop_zone(ui, dragging, |ui| {
                    let id = ui.make_persistent_id(("folder", i));
                    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                        .show_header(ui, |ui| {
                            let mut skip = folder.is_unused();
                            if ui.checkbox(&mut skip, "").on_hover_text("Skip on export: leaves this folder's effects out of the exported file. Adds UNUSED_ to the name.").changed() {
                                items.actions.push(ListAction::Skip(i, skip));
                            }
                            match folder_edit {
                                Some((at, text)) if *at == i => {
                                    let edit = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                                    if edit.lost_focus() {
                                        if ui.input(|inp| inp.key_pressed(egui::Key::Enter)) {
                                            items.actions.push(ListAction::RenameFolder(i, text.clone()));
                                        }
                                        *folder_edit = None;
                                    } else {
                                        edit.request_focus();
                                    }
                                }
                                _ => {
                                    let mut text = egui::RichText::new(format!("{} ({})", folder.name, effects.len())).strong();
                                    if folder.is_unused() {
                                        text = text.weak().strikethrough();
                                    }
                                    let label = ui.label(text);
                                    let label = if folder.is_unused() { label.on_hover_text("Skipped on export") } else { label };
                                    label.context_menu(|ui| {
                                        if ui.button("Rename folder").clicked() {
                                            *folder_edit = Some((i, folder.name.clone()));
                                            ui.close();
                                        }
                                        if ui.button("Remove folder").on_hover_text("Its effects go back to no folder; none are deleted.").clicked() {
                                            items.actions.push(ListAction::RemoveFolder(i));
                                            ui.close();
                                        }
                                    });
                                }
                            }
                        })
                        .body(|ui| {
                            for (name, set) in effects.iter().filter(|(n, _)| shown(n)) {
                                effect_item(ui, name, *set, Some(i), &mut items);
                            }
                        });
                });
                if let Some(names) = dropped {
                    items.actions.push(ListAction::Move((*names).clone(), Some(i)));
                }
            }
            let loose: Vec<&(String, usize)> = list.iter().filter(|(n, _)| project.folder_of(n).is_none()).collect();
            let dropped = drop_zone(ui, dragging, |ui| {
                if !project.folders.is_empty() || dragging {
                    ui.label(egui::RichText::new(format!("Not in a folder ({})", loose.len())).strong());
                }
                for (name, set) in loose.iter().filter(|(n, _)| shown(n)) {
                    effect_item(ui, name, *set, None, &mut items);
                }
                if dragging {
                    ui.weak("Drop here to take it out of its folder");
                }
            });
            if let Some(names) = dropped {
                items.actions.push(ListAction::Move((*names).clone(), None));
            }
        });
        let ListItems { mut actions, drawn, .. } = items;
        if let Some(name) = step(&drawn, self.effect.as_ref().map(|e| e.0.as_str()), std::mem::take(&mut self.nav.0)) {
            if let Some((_, set)) = list.iter().find(|(n, _)| *n == name) {
                let keys = egui::Modifiers { shift: self.nav.1, ..Default::default() };
                actions.push(ListAction::Pick(name, *set, keys));
                self.reveal = true;
            }
        }
        for action in actions {
            match action {
                ListAction::Pick(name, set, keys) => {
                    let range = |a: &str, b: &str| {
                        let at = |n: &str| drawn.iter().position(|d| d == n);
                        let (Some(a), Some(b)) = (at(a), at(b)) else { return vec![b.to_string()] };
                        drawn[a.min(b)..=a.max(b)].to_vec()
                    };
                    if keys.shift {
                        let anchor = self.anchor.clone().unwrap_or_else(|| name.clone());
                        self.chosen = range(&anchor, &name);
                        self.anchor = Some(anchor);
                    } else if keys.command {
                        if let Some(at) = self.chosen.iter().position(|c| *c == name) {
                            self.chosen.remove(at);
                        } else {
                            self.chosen.push(name.clone());
                        }
                        self.anchor = Some(name.clone());
                    } else {
                        self.chosen = vec![name.clone()];
                        self.anchor = Some(name.clone());
                    }
                    self.effect = Some((name, set));
                    self.emitter = 0;
                    self.preview = None;
                }
                other => {
                    let Some(project) = &mut self.project else { continue };
                    let result = match other {
                        ListAction::Move(names, to) => {
                            for name in &names {
                                project.move_to(name, to);
                            }
                            Ok(())
                        }
                        ListAction::Skip(i, skip) => project.set_skipped(i, skip),
                        ListAction::RenameFolder(i, name) => project.rename_folder(i, &name),
                        ListAction::RemoveFolder(i) => {
                            project.remove_folder(i);
                            Ok(())
                        }
                        ListAction::Pick(..) => Ok(()),
                    };
                    match result {
                        Ok(()) => self.unsaved = true,
                        Err(e) => self.status = format!("{e:#}"),
                    }
                }
            }
        }
    }

    /// Making an effect: a name, then a piece of the kit to start it from.
    fn new_effect_row(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.create_name).hint_text("new effect name").desired_width(120.0));
            let name = self.create_name.trim().to_string();
            let taken = doc.effects().iter().any(|(n, _)| n.eq_ignore_ascii_case(&name));
            let ready = self.busy.is_none() && self.pending.is_empty() && !name.is_empty() && !taken;
            let hint = if taken { "This file already has an effect of that name." } else { "Name it, then pick the first layer from the game's own." };
            ui.add_enabled_ui(ready, |ui| {
                ui.menu_button("New effect", |ui| {
                    if let Some((piece, model)) = self.kit_menu(ui).and_then(|p| Some((p, self.model_for(p)?))) {
                        self.select = Some(name.clone());
                        self.create_name.clear();
                        self.run(Job::NewEffect(name.clone(), piece, model), "Making the effect");
                    }
                })
                .response
                .on_hover_text(hint)
                .on_disabled_hover_text(hint);
            });
        });
    }

    /// For a blank model piece, the model to start from, asked for now: `None` when none was picked (or it didn't
    /// load), `Some(None)` for any other piece.
    fn model_for(&mut self, piece: &Piece) -> Option<Option<MeshData>> {
        if !piece.is_model() {
            return Some(None);
        }
        let path = rfd::FileDialog::new().set_title("The model to start from").add_filter("glTF model", &["glb", "gltf"]).pick_file()?;
        match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| mesh_io::from_gltf(&b)) {
            Ok(mesh) => Some(Some(mesh)),
            Err(e) => {
                self.status = format!("That model didn't load: {e:#}");
                None
            }
        }
    }

    /// The kit by group, or the folder it is read from when there is none yet. The piece picked, if one was.
    fn kit_menu(&mut self, ui: &mut egui::Ui) -> Option<&'static Piece> {
        if game_root().is_none() {
            ui.label("The kit is read from the game's own effect files.");
            if ui.button("Choose game files folder…").on_hover_text("The dumped game folder that holds the effect folder (effect/system/common/ef_common.eff).").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    match set_game_root(&dir) {
                        Ok(root) => self.status = format!("Kit reads from {}", root.display()),
                        Err(e) => self.status = format!("{e:#}"),
                    }
                }
                ui.close();
            }
            return None;
        }
        let mut picked = None;
        let mut groups: Vec<&str> = Vec::new();
        for piece in kit() {
            if !groups.contains(&piece.group.as_str()) {
                groups.push(&piece.group);
            }
        }
        for group in groups {
            ui.menu_button(group, |ui| {
                for piece in kit().iter().filter(|p| p.group == group) {
                    if ui.button(&piece.name).on_hover_text(&piece.blurb).clicked() {
                        picked = Some(piece);
                        ui.close();
                    }
                }
            });
        }
        picked
    }

    /// The game's shaders, once this file's are translated. False while they aren't.
    fn game_view(&mut self, ui: &mut egui::Ui) -> bool {
        let (Some(doc), Some((entry, set_idx)), Some(game)) = (self.doc.as_ref(), self.effect.as_ref(), self.game.as_mut()) else { return false };
        let set_idx = *set_idx;
        crate::log::context(&format!("{} / {entry}", doc.path.display()));
        let emitter = self.emitter;
        let length = game.length(doc, set_idx);
        let now = std::time::Instant::now();
        let frames = std::mem::take(&mut self.frame_step);
        if frames != 0 {
            game.playing = false;
            game.age = (game.age.round() + frames as f32).clamp(0.0, length);
        }
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
                    let mut ghosted = game.ghost.is_some();
                    if ui.checkbox(&mut ghosted, "Ghost").on_hover_text("The selected layer's model as a flat see-through shape, to see where it sits").changed()
                        || (game.ghost.is_some() && self.shot.is_none())
                    {
                        game.ghost = ghosted.then_some(emitter);
                    }
                    if game.ghost.is_some() {
                        ui.spacing_mut().slider_width = 60.0;
                        ui.add(egui::Slider::new(&mut game.ghost_opacity, 0.05..=1.0).show_value(false))
                            .on_hover_text(format!("Ghost opacity {:.0}%", game.ghost_opacity * 100.0));
                    }
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
        let idle = self.busy.is_none();
        ui.horizontal(|ui| {
            ui.heading(entry);
            // the tuned list keeps effects by name
            let kept = self.tuned.as_ref().is_some_and(|t| t.has(entry));
            let ready = idle && self.pending.is_empty() && self.applying.is_empty() && !kept;
            ui.add_enabled_ui(ready, |ui| {
                ui.menu_button("Rename", |ui| {
                    if self.rename_to.is_empty() {
                        self.rename_to = entry.to_string();
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.rename_to).desired_width(180.0));
                    let check = crate::engine::check_effect_name(&self.rename_to).map(str::to_string);
                    let taken = check.as_ref().is_ok_and(|n| n != entry && doc.effects().iter().any(|(e, _)| e.eq_ignore_ascii_case(n)));
                    match (&check, taken) {
                        (Err(e), _) => {
                            ui.colored_label(Color32::from_rgb(255, 190, 90), e.to_string());
                        }
                        (Ok(_), true) => {
                            ui.colored_label(Color32::from_rgb(255, 190, 90), "This file already has an effect of that name.");
                        }
                        _ => {}
                    }
                    let ok = check.as_ref().is_ok_and(|n| n != entry) && !taken;
                    if ui.add_enabled(ok, egui::Button::new("Rename")).on_hover_text("Scripts that play the effect by its old name need the new one.").clicked() {
                        let new = check.unwrap();
                        self.renaming = Some((entry.to_string(), new.clone()));
                        self.select = Some(new.clone());
                        self.rename_to.clear();
                        self.run(Job::Rename(entry.to_string(), new), "Renaming");
                        ui.close();
                    }
                })
                .response
                .on_disabled_hover_text(if kept { "Kept in the tuned file, which lists it by name." } else { "Wait for the values to go in." });
            });
        });
        if self.project.is_none() {
            self.tuning_row(ui, doc, entry, set_idx, idle);
        }
        let emitters: Vec<EmitterDef> = doc.emitters(set_idx).to_vec();
        ui.horizontal(|ui| {
            ui.label("Layers");
            // values not yet in the file are kept by layer number, which adding or removing would shift
            let ready = idle && self.pending.is_empty() && self.applying.is_empty();
            ui.add_enabled_ui(ready, |ui| {
                ui.menu_button("Add layer", |ui| {
                    if let Some((piece, model)) = self.kit_menu(ui).and_then(|p| Some((p, self.model_for(p)?))) {
                        self.emitter = emitters.len();
                        self.preview = None;
                        self.run(Job::AddLayer(entry.to_string(), piece, model), "Adding the layer");
                    }
                })
                .response
                .on_hover_text("One of the game's own layers, copied in with its texture, model and shader.");
                if ui.button("Duplicate layer").on_hover_text("Copies the selected layer and any under it.").clicked() {
                    self.duplicate_layer(entry, set_idx, self.emitter);
                }
                let last = emitters.len() <= 1;
                if ui.add_enabled(!last, egui::Button::new("Remove layer"))
                    .on_hover_text("Takes out the selected layer and any under it.")
                    .on_disabled_hover_text("An effect needs at least one layer.")
                    .clicked()
                {
                    self.remove_layer(entry, set_idx, self.emitter);
                }
            });
        });
        let ready = idle && self.pending.is_empty() && self.applying.is_empty();
        let mut picked = None;
        for (i, em) in emitters.iter().enumerate() {
            let response = ui.selectable_label(self.emitter == i, &em.name);
            if response.clicked() {
                picked = Some(i);
            }
            response.context_menu(|ui| {
                ui.add_enabled_ui(ready, |ui| {
                    if ui.button("Duplicate layer").on_hover_text("Copies it and any layers under it.").clicked() {
                        self.duplicate_layer(entry, set_idx, i);
                        ui.close();
                    }
                    if ui.add_enabled(emitters.len() > 1, egui::Button::new("Remove layer"))
                        .on_hover_text("Takes it out with any layers under it.")
                        .on_disabled_hover_text("An effect needs at least one layer.")
                        .clicked()
                    {
                        self.remove_layer(entry, set_idx, i);
                        ui.close();
                    }
                });
            });
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
        self.trail_shader_note(ui, doc, entry, em, idle);
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

    fn duplicate_layer(&mut self, entry: &str, set_idx: usize, layer: usize) {
        self.preview = None;
        self.run(Job::DuplicateLayer(entry.to_string(), set_idx, layer), "Duplicating the layer");
    }

    fn remove_layer(&mut self, entry: &str, set_idx: usize, layer: usize) {
        self.emitter = 0;
        self.preview = None;
        self.run(Job::RemoveLayer(entry.to_string(), set_idx, layer), "Removing the layer");
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
            if ui.add_enabled(idle, egui::Button::new("Swap texture…")).on_hover_text("A PNG. Its alpha gives the shape, or what shape says beside it. RGB blends between the two colours below: white is Colour, black is Colour 2.").clicked() {
                if let (Some(png), Some(target)) = (rfd::FileDialog::new().add_filter("PNG", &["png"]).pick_file(), self.target()) {
                    self.run(Job::Texture(target, png, self.texture_from), "Swapping the texture");
                }
            }
            egui::ComboBox::from_id_salt("shape_from")
                .selected_text(format!("shape: {}", AlphaFrom::ALL.iter().find(|(a, _)| *a == self.texture_from.alpha).map_or("", |(_, l)| *l)))
                .show_ui(ui, |ui| {
                    for (alpha, label) in AlphaFrom::ALL {
                        ui.selectable_value(&mut self.texture_from.alpha, alpha, label);
                    }
                })
                .response.on_hover_text("Where the next swapped-in picture's shape comes from: its alpha, one colour \
                    channel (textures often keep a mask in each), or its brightness (a glow drawn on black).");
            ui.checkbox(&mut self.texture_from.white, "white").on_hover_text("Drop the picture's colour for white, so the \
                layer's colours alone tint it. For masks and greyscale shapes.");
            if ui.add_enabled(idle && has_mesh, egui::Button::new("Swap model…")).on_hover_text("A .glb. Sprite layers have no model to swap.").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("glTF model", &["glb", "gltf"]).pick_file() {
                    match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| mesh_io::from_gltf(&b)) {
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
            // the file keeps frames per step; speed is its inverse
            let step = self.value(em, "emitter_static.tex_pattern_anim0.frequency").unwrap_or(1.0).max(0.01);
            let mut speed = 1.0 / step;
            if ui.add(egui::Slider::new(&mut speed, 0.125..=4.0).logarithmic(true).text("Animated texture speed"))
                .on_hover_text("How fast the texture's animation plays. 1 is the game's normal speed, 2 twice as fast, 0.5 half.").changed()
            {
                self.set(em, "emitter_static.tex_pattern_anim0.frequency", 1.0 / speed);
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
        ui.label("Draw style").on_hover_text("Built into the layer's shader: switching one puts the layer on a shader drawn that way,             found in this file or the game's own effects.");
        let ready = idle && self.pending.is_empty() && self.applying.is_empty();
        ui.horizontal_wrapped(|ui| {
            for (i, style) in STYLES.iter().enumerate() {
                if style.models_only && !has_mesh {
                    continue;
                }
                let mut on = self.value(em, style.field).unwrap_or(0.0) != 0.0;
                if ui.add_enabled(ready, egui::Checkbox::new(&mut on, style.label)).on_hover_text(style.hover).changed() {
                    if let Some(t) = self.target() {
                        let value = if on { style.on } else { 0.0 };
                        self.run(Job::Restyle(t.entry, t.set_idx, t.emitter_idx, i, value), &format!("Finding a shader for {}", style.label));
                    }
                }
            }
        });
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
        if crate::draw::shaders::ready(&doc.working()).is_none() {
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

    /// Up and down walk the effect list (shift picks a range in a project), left and right step the preview a frame.
    /// Left to the focused widget while text is being typed.
    fn arrow_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.doc.is_none() {
            return;
        }
        use egui::Key::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp};
        let (moves, frames, shift) = ctx.input_mut(|i| {
            let count = |i: &egui::InputState, key| i.events.iter().filter(|e| matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key)).count() as i32;
            let moves = count(i, ArrowDown) - count(i, ArrowUp);
            let frames = count(i, ArrowRight) - count(i, ArrowLeft);
            // taken, so a focused button doesn't also move focus with them
            i.events.retain(|e| !matches!(e, egui::Event::Key { key: ArrowDown | ArrowUp | ArrowLeft | ArrowRight, .. }));
            (moves, frames, i.modifiers.shift)
        });
        if moves != 0 {
            self.nav = (self.nav.0 + moves, shift);
            ctx.request_repaint();
        }
        if frames != 0 {
            self.frame_step += frames;
            ctx.request_repaint();
        }
    }

    /// Save once the file has been left alone for a moment.
    fn auto_save(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        let editing = self.busy.is_some() || !self.pending.is_empty() || !self.applying.is_empty() || ctx.input(|i| i.pointer.any_down());
        if !self.unsaved || self.doc.is_none() {
            self.unsaved_since = None;
            return;
        }
        if editing || self.unsaved_since.is_none() {
            self.unsaved_since = Some(now);
        }
        if !self.settings.auto_save || self.shot.is_some() {
            return;
        }
        let since = self.unsaved_since.unwrap_or(now);
        if !editing && now.duration_since(since) >= AUTO_SAVE_AFTER {
            self.auto_saving = true;
            self.run(Job::Save, "Auto saving");
        } else {
            ctx.request_repaint_after(AUTO_SAVE_AFTER);
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
            game.ghost = shot.ghost;
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
        self.arrow_keys(&ctx);
        self.auto_save(&ctx);
        self.walk(&ctx);
        self.take_shot(&ctx);
        self.load_preview(&ctx);
        egui::Panel::top("top").show_inside(ui, |ui| self.top_bar(ui));
        egui::Panel::left("effects").default_size(260.0).show_inside(ui, |ui| self.effect_list(ui));
        egui::Panel::right("values").default_size(360.0).show_inside(ui, |ui| self.panel(ui));
        egui::CentralPanel::default().show_inside(ui, |ui| self.view(ui));
        self.import_window(&ctx);
    }
}

/// The effect `by` places from `current` in `order`, kept within it: the first or last when none is current.
fn step(order: &[String], current: Option<&str>, by: i32) -> Option<String> {
    if by == 0 || order.is_empty() {
        return None;
    }
    let at = match current.and_then(|c| order.iter().position(|n| n == c)) {
        Some(at) => (at as i32 + by).clamp(0, order.len() as i32 - 1) as usize,
        None if by > 0 => 0,
        None => order.len() - 1,
    };
    (Some(order[at].as_str()) != current).then(|| order[at].clone())
}

/// One effect in a project's list: click to show it (ctrl-click adds it to the picked ones, shift-click picks a
/// range), drag it, with the others picked, onto a folder, or right-click to move them.
fn effect_item(ui: &mut egui::Ui, name: &str, set: usize, in_folder: Option<usize>, items: &mut ListItems) {
    items.drawn.push(name.to_string());
    let picked = items.chosen.iter().any(|c| c == name);
    let on = picked || items.current.as_deref() == Some(name);
    let response = ui.selectable_label(on, name).interact(Sense::drag());
    if items.reveal && items.current.as_deref() == Some(name) {
        response.scroll_to_me(None);
    }
    // the effects that go with it: every picked one when it is picked, else just it
    let moving = if picked { items.chosen.to_vec() } else { vec![name.to_string()] };
    response.dnd_set_drag_payload(moving.clone());
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if moving.len() > 1 {
            response.show_tooltip_text(format!("{} effects", moving.len()));
        }
    }
    if response.clicked() {
        let keys = ui.input(|i| i.modifiers);
        items.actions.push(ListAction::Pick(name.to_string(), set, keys));
    }
    let actions = &mut items.actions;
    let folders = &items.folders;
    response.context_menu(|ui| {
        ui.label(if moving.len() > 1 { format!("Move {} effects to", moving.len()) } else { "Move to".to_string() });
        for (i, folder) in folders.iter().enumerate() {
            if (moving.len() > 1 || Some(i) != in_folder) && ui.button(folder).clicked() {
                actions.push(ListAction::Move(moving.clone(), Some(i)));
                ui.close();
            }
        }
        if (moving.len() > 1 || in_folder.is_some()) && ui.button("No folder").clicked() {
            actions.push(ListAction::Move(moving.clone(), None));
            ui.close();
        }
    });
}

/// `add` drawn as a place to drop a dragged effect: outlined while one is over it. The effect dropped, if one was.
fn drop_zone(ui: &mut egui::Ui, dragging: bool, add: impl FnOnce(&mut egui::Ui)) -> Option<Arc<Vec<String>>> {
    let rect = ui.scope(add).response.rect;
    if !dragging || !ui.rect_contains_pointer(rect) {
        return None;
    }
    ui.painter().rect_stroke(rect.expand(1.0), 3.0, ui.visuals().selection.stroke, egui::StrokeKind::Outside);
    if ui.input(|i| i.pointer.any_released()) {
        return egui::DragAndDrop::take_payload::<Vec<String>>(ui.ctx());
    }
    None
}

#[cfg(test)]
mod arrow_tests {
    use super::step;

    #[test]
    fn arrows_walk_the_list() {
        let order: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(step(&order, Some("a"), 1).as_deref(), Some("b"));
        assert_eq!(step(&order, Some("b"), -1).as_deref(), Some("a"));
        assert_eq!(step(&order, Some("c"), 1), None, "stays at the end");
        assert_eq!(step(&order, Some("a"), -1), None, "stays at the start");
        assert_eq!(step(&order, Some("a"), 5).as_deref(), Some("c"));
        assert_eq!(step(&order, None, 1).as_deref(), Some("a"));
        assert_eq!(step(&order, Some("gone"), -1).as_deref(), Some("c"));
        assert_eq!(step(&order, Some("b"), 0), None);
        assert_eq!(step(&[], None, 1), None);
    }
}
