// SPDX-License-Identifier: MIT
//! The chooser window, in egui (eframe): pure Rust, the same code on Linux, Windows and macOS.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, RichText};

use crate::library::{Section, SetKind};
use crate::usage::CHARS_PER_K;
use crate::util;

/// Seconds for the whole load; the skill runs the program with a 600 s Bash timeout.
pub const POPUP_TIMEOUT: u64 = 540;

/// What the user did.
pub enum Outcome {
    /// Set name → (ticked documents, remember as preset), in section order.
    Picked(Vec<(String, BTreeSet<String>, bool)>),
    Cancelled,
    Timeout,
}

/// Size (characters of the squeezed text) of each readable document; absent = unreadable.
pub type Sizes = BTreeMap<String, usize>;

struct Row {
    doc: String,
    shown: String,
    heading: Option<String>,
    on: bool,
    readable: bool,
    new: bool,
}

struct SectionState {
    name: String,
    sub: String,
    main: Vec<Row>,
    other: Vec<Row>,
    other_label: String,
    other_open: bool,
    saved: BTreeSet<String>,
    first: bool,
    remember: bool,
}

impl SectionState {
    fn rows_mut(&mut self) -> impl Iterator<Item = &mut Row> {
        self.main.iter_mut().chain(self.other.iter_mut())
    }
}

fn folder(d: &str) -> &str {
    d.rsplit_once('/').map_or("", |(f, _)| f)
}

/// Rows for `docs`, with a folder heading wherever the folder changes (only when more than one
/// folder shows).
fn rows(docs: &[String], sec: &Section, sizes: &Sizes) -> Vec<Row> {
    let folders: HashSet<&str> = docs.iter().map(|d| folder(d)).collect();
    let grouped = folders.len() > 1;
    let mut last: Option<&str> = None;
    docs.iter()
        .map(|d| {
            let f = folder(d);
            let heading = (grouped && last != Some(f)).then(|| {
                last = Some(f);
                if f.is_empty() { "(top level)".to_string() } else { format!("{f}/") }
            });
            let readable = sizes.contains_key(d);
            Row {
                doc: d.clone(),
                shown: if grouped { d.rsplit('/').next().unwrap_or(d).to_string() } else { d.clone() },
                heading,
                on: readable && sec.ticked.contains(d),
                readable,
                new: sec.new.contains(d),
            }
        })
        .collect()
}

fn subtitle(sec: &Section) -> String {
    if sec.first {
        if sec.kind == SetKind::Custom {
            "new set: nothing saved yet. Tick the research this one needs; it is saved as its preset.".into()
        } else {
            format!("{} documents (no preset saved yet, all ticked)", sec.docs.len())
        }
    } else {
        let kept = sec.ticked.difference(&sec.new).count();
        let mut s = format!("saved preset: {kept} documents");
        if !sec.new.is_empty() {
            s.push_str(&format!(", {} new in the library", sec.new.len()));
        }
        s
    }
}

struct App {
    sections: Vec<SectionState>,
    sizes: Sizes,
    setup: bool,
    names: String,
    command: String,
    until: Instant,
    shot: Option<Shot>,
    result: Arc<Mutex<Option<Outcome>>>,
    closing: bool,
}

impl App {
    fn k(&self, d: &str) -> f64 {
        self.sizes.get(d).copied().unwrap_or(0) as f64 / CHARS_PER_K as f64
    }

    fn chosen(&self) -> BTreeSet<String> {
        self.sections
            .iter()
            .flat_map(|s| s.main.iter().chain(&s.other))
            .filter(|r| r.on)
            .map(|r| r.doc.clone())
            .collect()
    }

    fn finish(&mut self, ctx: &egui::Context, value: Outcome) {
        let mut slot = self.result.lock().expect("not poisoned");
        if slot.is_none() {
            *slot = Some(value);
        }
        drop(slot);
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn done(&mut self, ctx: &egui::Context) {
        let picks = self
            .sections
            .iter()
            .map(|s| {
                let ticked = s.main.iter().chain(&s.other).filter(|r| r.on).map(|r| r.doc.clone()).collect();
                (s.name.clone(), ticked, s.remember)
            })
            .collect();
        self.finish(ctx, Outcome::Picked(picks));
    }

    fn doc_rows(ui: &mut egui::Ui, rows: &mut [Row], sizes: &Sizes, indent: f32, grouped_indent: f32) {
        for r in rows {
            if let Some(h) = &r.heading {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    ui.label(RichText::new(h).small().strong());
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(if r.heading.is_some() || grouped_indent > indent { grouped_indent } else { indent });
                if r.readable {
                    let k = sizes.get(&r.doc).copied().unwrap_or(0) as f64 / CHARS_PER_K as f64;
                    ui.checkbox(&mut r.on, &r.shown);
                    ui.label(RichText::new(format!("~{k:.0}k")).small());
                    if r.new {
                        ui.label(RichText::new("NEW").strong().color(Color32::from_rgb(0xff, 0xcc, 0x33)));
                    }
                } else {
                    ui.add_enabled(false, egui::Checkbox::new(&mut r.on, RichText::new(&r.shown).strikethrough()));
                    ui.label(RichText::new("MISSING or unreadable").color(Color32::from_rgb(0xff, 0x66, 0x66)));
                }
            });
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // runs even while the window is hidden, so the timeout always fires
        if !self.closing && Instant::now() >= self.until {
            self.finish(ctx, Outcome::Timeout);
        }
        if !self.closing && (ctx.input(|i| i.viewport().close_requested()) || shot_taken(&mut self.shot, ctx)) {
            self.finish(ctx, Outcome::Cancelled);
        }
        ctx.request_repaint_after(Duration::from_millis(if self.shot.is_some() { 50 } else { 1000 }));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.closing {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.finish(&ctx, Outcome::Cancelled);
            return;
        }
        let chosen = self.chosen();
        let can_ok = self.setup || !chosen.is_empty();
        if can_ok && ctx.input(|i| i.key_pressed(egui::Key::Enter)) && ctx.memory(|m| m.focused().is_none()) {
            self.done(&ctx);
            return;
        }
        let margin = egui::Frame::central_panel(ui.style()).inner_margin(14.0);

        egui::Panel::top("head").frame(margin).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let title = if self.setup { "Change the saved presets" } else { "Claude Code wants to load research" };
                ui.label(RichText::new(title).heading().strong());
                ui.label(RichText::new(" for ").heading());
                ui.label(RichText::new(&self.names).heading().strong());
            });
            ui.label(if self.setup {
                "Tick what each should contain, then press Save presets. Nothing is loaded."
            } else {
                "Tick what this session needs, then press Load. Nothing is sent to Claude until then; \
                 Cancel loads nothing."
            });
            if !self.setup && !self.command.is_empty() {
                ui.label(RichText::new(format!("Asked by: {}", self.command)).small().monospace());
            }
        });

        let mut ok_clicked = false;
        let mut cancel_clicked = false;
        egui::Panel::bottom("bar").frame(margin).show(ui, |ui| {
            let k = chosen.iter().map(|d| self.k(d)).fold(0.0, |a, b| a + b); // an empty f64 sum is -0.0
            let tail = if self.setup { "" } else { " will be loaded" };
            ui.label(RichText::new(format!("{} documents, ~{k:.0}k tokens{tail}", chosen.len())).strong());
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label =
                    if self.setup { "Save presets".to_string() } else { format!("Load {} documents", chosen.len()) };
                let ok = egui::Button::new(RichText::new(label).strong()).fill(if can_ok {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().widgets.inactive.weak_bg_fill
                });
                ok_clicked = ui.add_enabled(can_ok, ok).clicked();
                cancel_clicked =
                    ui.button(if self.setup { "Close without saving" } else { "Cancel (load nothing)" }).clicked();
            });
        });

        egui::CentralPanel::default().frame(margin).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let sizes = self.sizes.clone();
                let setup = self.setup;
                for sec in &mut self.sections {
                    ui.add_space(12.0);
                    ui.label(RichText::new(&sec.name).heading().strong());
                    ui.label(RichText::new(&sec.sub).small());
                    ui.horizontal(|ui| {
                        if ui.button("Select all").clicked() {
                            sec.rows_mut().for_each(|r| r.on = r.readable);
                        }
                        if ui.button("Select none").clicked() {
                            sec.rows_mut().for_each(|r| r.on = false);
                        }
                        if ui.add_enabled(!sec.first, egui::Button::new("Back to saved preset")).clicked() {
                            let saved = sec.saved.clone();
                            sec.rows_mut().for_each(|r| r.on = saved.contains(&r.doc));
                        }
                    });
                    let grouped = sec.main.iter().any(|r| r.heading.is_some());
                    App::doc_rows(ui, &mut sec.main, &sizes, 12.0, if grouped { 24.0 } else { 12.0 });
                    if !sec.other.is_empty() {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            let id = ui.make_persistent_id(("other", &sec.name));
                            egui::CollapsingHeader::new(&sec.other_label)
                                .id_salt(id)
                                .default_open(sec.other_open)
                                .show(ui, |ui| {
                                    let grouped = sec.other.iter().any(|r| r.heading.is_some());
                                    App::doc_rows(ui, &mut sec.other, &sizes, 0.0, if grouped { 12.0 } else { 0.0 });
                                });
                        });
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.add_enabled(
                            !setup,
                            egui::Checkbox::new(
                                &mut sec.remember,
                                format!("Remember this choice as the preset for {}", sec.name),
                            ),
                        );
                    });
                }
            });
        });

        if cancel_clicked {
            self.finish(&ctx, Outcome::Cancelled);
        } else if ok_clicked {
            self.done(&ctx);
        }
    }
}

/// True when a window can be shown at all.
fn have_display() -> Result<(), String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        if !set("DISPLAY") && !set("WAYLAND_DISPLAY") {
            return Err("no display".into());
        }
    }
    Ok(())
}

/// Show the chooser and wait for the user. Err when no window can be shown.
pub fn popup(sections: &[Section], sizes: Sizes, setup: bool, command: &str) -> Result<Outcome, String> {
    have_display()?;
    let states = sections
        .iter()
        .map(|sec| {
            let other_open = sec.main.is_empty();
            SectionState {
                name: sec.name.clone(),
                sub: subtitle(sec),
                main: rows(&sec.main, sec, &sizes),
                other: rows(&sec.other, sec, &sizes),
                other_label: if other_open {
                    "Your library: tick the research to use".into()
                } else {
                    format!("Add other research from your library ({} more)", sec.other.len())
                },
                other_open,
                saved: sec.ticked.clone(),
                first: sec.first,
                remember: sec.remember,
            }
        })
        .collect();
    let names = sections.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ");
    let result = Arc::new(Mutex::new(None));
    let app = App {
        sections: states,
        sizes,
        setup,
        names,
        command: command.to_string(),
        until: Instant::now() + remaining(),
        shot: Shot::from_env(),
        result: Arc::clone(&result),
        closing: false,
    };
    let title = format!("The Researcher - {}", if setup { "change saved presets" } else { "choose research to load" });
    let size = if app.shot.is_some() { [720.0, 560.0] } else { [720.0, 780.0] };
    run(&title, size, Box::new(app))?;
    let out = result.lock().expect("not poisoned").take();
    Ok(out.unwrap_or(Outcome::Cancelled))
}

// ---------------------------------------------------------------- shared window plumbing

/// Seconds since the epoch at which every popup of this load gives up. Kept in
/// RESEARCHER_DEADLINE, so a restarted run (after the first-use window) shares it.
static DEADLINE: LazyLock<u64> = LazyLock::new(|| {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let d = std::env::var("RESEARCHER_DEADLINE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|d| *d > now && *d <= now + POPUP_TIMEOUT)
        .unwrap_or(now + POPUP_TIMEOUT);
    // SAFETY: called before any window or thread of ours exists; nothing else reads the
    // environment concurrently at this point.
    unsafe { std::env::set_var("RESEARCHER_DEADLINE", d.to_string()) };
    d
});

/// Time left until the deadline (fixes the deadline on first use).
pub fn remaining() -> Duration {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Duration::from_secs(DEADLINE.saturating_sub(now))
}

fn run(title: &str, size: [f32; 2], app: Box<dyn eframe::App>) -> Result<(), String> {
    have_display()?;
    let left = remaining();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(title).with_app_id("researcher").with_inner_size(size),
        ..Default::default()
    };
    // A last guard: if the window never answers (a stuck compositor), stop well inside the
    // skill's 600 s limit instead of being killed without a word.
    std::thread::spawn(move || {
        std::thread::sleep(left + Duration::from_secs(20));
        println!("TIMEOUT: popup closed after {POPUP_TIMEOUT} s without a choice, nothing loaded");
        std::process::exit(4);
    });
    eframe::run_native(
        "researcher",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            Ok(app)
        }),
    )
    .map_err(|e| e.to_string())
}

/// RESEARCHER_SCREENSHOT=FILE.png: save the window as a PNG after it has settled, then close
/// it as Cancel (nothing loaded, nothing saved). Used by tools/screenshots.sh for the README.
struct Shot {
    path: String,
    frames: u32,
    requested: bool,
    done: Arc<AtomicBool>,
}

impl Shot {
    fn from_env() -> Option<Shot> {
        let path = std::env::var("RESEARCHER_SCREENSHOT").ok().filter(|p| !p.is_empty())?;
        Some(Shot { path, frames: 0, requested: false, done: Arc::new(AtomicBool::new(false)) })
    }
}

/// Drives a screenshot; true once the picture is written.
fn shot_taken(shot: &mut Option<Shot>, ctx: &egui::Context) -> bool {
    let Some(s) = shot else { return false };
    s.frames += 1;
    if !s.requested && s.frames >= 20 {
        s.requested = true;
        let (path, done) = (s.path.clone(), Arc::clone(&s.done));
        ctx.request_screenshot(move |img| {
            if let Err(e) = save_png(&path, &img) {
                eprintln!("screenshot {path}: {e}");
            }
            done.store(true, Ordering::SeqCst);
        });
        ctx.request_repaint();
    }
    s.done.load(Ordering::SeqCst)
}

fn save_png(path: &str, img: &egui::ColorImage) -> Result<(), String> {
    let [w, h] = img.size;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    enc.write_header().and_then(|mut wr| wr.write_image_data(&data)).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- first use: the research folder

/// What the first-use window gives back.
pub enum FirstUse {
    Folder(String),
    Cancelled,
    Timeout,
}

struct FolderApp {
    path: String,
    config: String,
    error: Option<String>,
    until: Instant,
    shot: Option<Shot>,
    result: Arc<Mutex<Option<FirstUse>>>,
    closing: bool,
}

impl FolderApp {
    fn finish(&mut self, ctx: &egui::Context, value: FirstUse) {
        let mut slot = self.result.lock().expect("not poisoned");
        if slot.is_none() {
            *slot = Some(value);
        }
        drop(slot);
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn confirm(&mut self, ctx: &egui::Context) {
        let full = util::user_path(util::py_strip(&self.path));
        if util::py_strip(&self.path).is_empty() {
            self.error = Some("Type a folder or choose one.".into());
        } else if std::path::Path::new(&full).exists() && !std::path::Path::new(&full).is_dir() {
            self.error = Some(format!("{full} is a file, not a folder."));
        } else {
            self.finish(ctx, FirstUse::Folder(full));
        }
    }
}

impl eframe::App for FolderApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.closing && Instant::now() >= self.until {
            self.finish(ctx, FirstUse::Timeout);
        }
        if !self.closing && (ctx.input(|i| i.viewport().close_requested()) || shot_taken(&mut self.shot, ctx)) {
            self.finish(ctx, FirstUse::Cancelled);
        }
        ctx.request_repaint_after(Duration::from_millis(if self.shot.is_some() { 50 } else { 1000 }));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.closing {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.finish(&ctx, FirstUse::Cancelled);
            return;
        }
        let margin = egui::Frame::central_panel(ui.style()).inner_margin(14.0);
        egui::CentralPanel::default().frame(margin).show(ui, |ui| {
            ui.label(RichText::new("Where is your research?").heading().strong());
            ui.add_space(6.0);
            ui.label(
                "Choose the folder that holds your research documents (.html, .md, .txt, .pdf), \
                 or the folder where you will make or copy them from now on. Subfolders become \
                 groups. The Researcher only reads this folder; it never changes or moves your files.",
            );
            ui.add_space(10.0);
            let mut enter = false;
            ui.horizontal(|ui| {
                let edit = egui::TextEdit::singleline(&mut self.path).desired_width(ui.available_width() - 140.0);
                let resp = ui.add(edit);
                if resp.changed() {
                    self.error = None;
                }
                enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Choose folder…").clicked() {
                    let start = util::user_path(util::py_strip(&self.path));
                    let mut dialog = rfd::FileDialog::new().set_title("Your research folder");
                    let existing = std::iter::successors(Some(std::path::PathBuf::from(&start)), |p| {
                        p.parent().map(std::path::Path::to_path_buf)
                    })
                    .find(|p| p.is_dir());
                    if let Some(dir) = existing {
                        dialog = dialog.set_directory(dir);
                    }
                    if let Some(dir) = dialog.pick_folder() {
                        self.path = util::tilde(&dir.to_string_lossy());
                        self.error = None;
                    }
                }
            });
            let full = util::user_path(util::py_strip(&self.path));
            let note = if util::py_strip(&self.path).is_empty() {
                String::new()
            } else if std::path::Path::new(&full).is_dir() {
                "This folder exists; its documents show up in the next window.".into()
            } else {
                "This folder does not exist yet; it will be made, empty. Save or copy research into it.".into()
            };
            ui.label(RichText::new(note).small());
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).color(Color32::from_rgb(0xff, 0x66, 0x66)));
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!(
                    "This is saved as your library in {}; you can change it there any time.",
                    self.config
                ))
                .small(),
            );
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let ok =
                    egui::Button::new(RichText::new("Use this folder").strong()).fill(ui.visuals().selection.bg_fill);
                if ui.add(ok).clicked() || enter {
                    self.confirm(&ctx);
                }
                if ui.button("Cancel (load nothing)").clicked() {
                    self.finish(&ctx, FirstUse::Cancelled);
                }
            });
        });
    }
}

/// Ask for the research folder on first use. Err when no window can be shown.
pub fn first_use(default: &str, config: &str) -> Result<FirstUse, String> {
    let result = Arc::new(Mutex::new(None));
    let app = FolderApp {
        path: util::tilde(default),
        config: util::tilde(config),
        error: None,
        until: Instant::now() + remaining(),
        shot: Shot::from_env(),
        result: Arc::clone(&result),
        closing: false,
    };
    run("The Researcher - choose your research folder", [640.0, 270.0], Box::new(app))?;
    let out = result.lock().expect("not poisoned").take();
    Ok(out.unwrap_or(FirstUse::Cancelled))
}
