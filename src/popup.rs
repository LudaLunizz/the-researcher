// SPDX-License-Identifier: MIT
//! The chooser window, in egui (eframe): pure Rust, the same code on Linux, Windows and macOS.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::library::{Section, SetKind};
use crate::usage::CHARS_PER_K;

/// Seconds; the skill runs the program with a 600 s Bash timeout.
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
    started: Instant,
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
        if !self.closing && self.started.elapsed() >= Duration::from_secs(POPUP_TIMEOUT) {
            self.finish(ctx, Outcome::Timeout);
        }
        if !self.closing && ctx.input(|i| i.viewport().close_requested()) {
            self.finish(ctx, Outcome::Cancelled);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
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
            let k: f64 = chosen.iter().map(|d| self.k(d)).sum();
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
        started: Instant::now(),
        result: Arc::clone(&result),
        closing: false,
    };
    let title = format!("The Researcher - {}", if setup { "change saved presets" } else { "choose research to load" });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_app_id("researcher")
            .with_inner_size([720.0, 780.0]),
        ..Default::default()
    };

    // A last guard: if the window never answers (a stuck compositor), stop well inside the
    // skill's 600 s limit instead of being killed without a word.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(POPUP_TIMEOUT + 20));
        println!("TIMEOUT: popup closed after {POPUP_TIMEOUT} s without a choice, nothing loaded");
        std::process::exit(4);
    });

    eframe::run_native(
        "researcher",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())?;
    let out = result.lock().expect("not poisoned").take();
    Ok(out.unwrap_or(Outcome::Cancelled))
}
