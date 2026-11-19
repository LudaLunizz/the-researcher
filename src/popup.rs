// SPDX-License-Identifier: MIT
//! The chooser window, in egui (eframe): pure Rust, the same code on Linux, Windows and macOS.
//! One window, up to two tabs (Chats | Research), each with its own ticks, filter, time chips and
//! names; one Load button loads what is ticked in both.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local};
use eframe::egui::{self, Color32, RichText};

use crate::bands::{self, Chip};
use crate::library::{Section, SetKind};
use crate::usage::CHARS_PER_K;
use crate::util;

/// Seconds for the whole load; the skill runs the program with a 600 s Bash timeout.
pub const POPUP_TIMEOUT: u64 = 540;

/// What the user picked in one section.
pub struct Pick {
    pub name: String,
    pub ticked: BTreeSet<String>,
    pub remember: bool,
    /// The preset name to save under (chats; differs from `name` when renamed).
    pub save_as: String,
    /// A new display name for the section (research sets); Some("") = back to its own name.
    pub title: Option<String>,
}

/// What the user did in one tab.
pub struct TabPick {
    pub picks: Vec<Pick>,
    /// Row id → new name ("" = back to the automatic name).
    pub renames: BTreeMap<String, String>,
    pub chip: Chip,
    pub by_time: bool,
}

/// What the user did: one TabPick per tab, in the order the tabs were given (None: a tab
/// that was never opened, so never loaded).
pub enum Outcome {
    Picked(Vec<Option<TabPick>>),
    Cancelled,
    Timeout,
}

/// Size (characters of the squeezed text) of each readable document; absent = unreadable.
pub type Sizes = BTreeMap<String, usize>;

/// How the rows of a tab read. The default shows research documents by their id; the chats tab
/// shows earlier chats by their first prompt, under their project folder.
pub struct Look {
    /// Shown instead of the id.
    pub labels: BTreeMap<String, String>,
    /// Shown again when a row's name is cleared (default: from its id).
    pub defaults: BTreeMap<String, String>,
    /// Small text after the size (for chats: the last-active day).
    pub notes: BTreeMap<String, String>,
    /// Heading each row sits under, instead of the folder part of its id.
    pub groups: BTreeMap<String, String>,
    /// Seconds since the epoch: last active (chats) or last changed (documents).
    pub times: BTreeMap<String, i64>,
    /// Shown instead of a section's name (research sets).
    pub titles: BTreeMap<String, String>,
    /// Preset names in use (a preset cannot be renamed onto one).
    pub taken: BTreeSet<String>,
    /// Chats instead of research documents.
    pub chats: bool,
    pub chip: Chip,
    pub by_time: bool,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            labels: BTreeMap::new(),
            defaults: BTreeMap::new(),
            notes: BTreeMap::new(),
            groups: BTreeMap::new(),
            times: BTreeMap::new(),
            titles: BTreeMap::new(),
            taken: BTreeSet::new(),
            chats: false,
            chip: Chip::All,
            by_time: false,
        }
    }
}

/// One tab of the window.
pub struct Tab {
    pub sections: Vec<Section>,
    pub sizes: Sizes,
    pub look: Look,
}

/// Makes a tab when it is first opened (it may take a while: chats are scanned then). None =
/// nothing to show.
pub type Loader = Box<dyn FnOnce() -> Option<Tab> + Send>;

/// A tab ready to show, or one that loads only when the user clicks it.
pub enum TabSlot {
    Ready(Box<Tab>),
    Later { chats: bool, load: Loader },
}

/// One tab of the window as it runs.
struct Slot {
    chats: bool,
    state: Option<TabState>,
    load: Option<Loader>,
    loading: Option<std::sync::mpsc::Receiver<Option<Tab>>>,
}

impl Slot {
    fn new(t: TabSlot) -> Slot {
        match t {
            TabSlot::Ready(tab) => {
                Slot { chats: tab.look.chats, state: Some(tab_state(&tab)), load: None, loading: None }
            }
            TabSlot::Later { chats, load } => Slot { chats, state: None, load: Some(load), loading: None },
        }
    }

    /// Start loading a tab that waits for its first click (in a thread: the window keeps
    /// drawing).
    fn open(&mut self) {
        if let Some(load) = self.load.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(load());
            });
            self.loading = Some(rx);
        }
    }

    /// Pick up a finished load; true when one came in.
    fn poll(&mut self) -> bool {
        let Some(rx) = &self.loading else { return false };
        let got = match rx.try_recv() {
            Ok(t) => t,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None, // the loader panicked
        };
        self.state = got.as_ref().map(tab_state);
        self.loading = None;
        true
    }

    /// What the tab button says after the name.
    fn status(&self) -> String {
        match (&self.state, &self.load, &self.loading) {
            (Some(t), ..) => format!("{} ticked", t.chosen().len()),
            (None, Some(_), _) => "click to load".into(),
            (None, None, Some(_)) => "loading…".into(),
            _ => "nothing to show".into(),
        }
    }
}

/// A preset name: lowercase letters, digits, `.`, `_`, `-`.
pub fn valid_preset(n: &str) -> bool {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[a-z0-9][a-z0-9._-]*$").expect("valid"));
    RE.is_match(n)
}

struct Row {
    doc: String,
    label: Option<String>,
    default: Option<String>,
    note: String,
    folder: String,
    when: Option<i64>,
    on: bool,
    readable: bool,
    new: bool,
}

impl Row {
    /// The name shown: a rename made in this window, else the saved label, else from the id
    /// (`short`: only its last part, when it sits under its folder heading).
    fn shown(&self, renames: &BTreeMap<String, String>, short: bool) -> String {
        let from_id = || if short { self.doc.rsplit('/').next().unwrap_or(&self.doc) } else { &self.doc }.to_string();
        match renames.get(&self.doc).map(String::as_str) {
            Some("") => self.default.clone().unwrap_or_else(from_id),
            Some(n) => n.to_string(),
            None => self.label.clone().unwrap_or_else(from_id),
        }
    }
}

struct SectionState {
    name: String,
    /// Display name (research sets); empty = the name.
    title: String,
    /// The preset name to save under (chats).
    save_as: String,
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
    fn rows(&self) -> impl Iterator<Item = &Row> {
        self.main.iter().chain(&self.other)
    }

    fn rows_mut(&mut self) -> impl Iterator<Item = &mut Row> {
        self.main.iter_mut().chain(self.other.iter_mut())
    }
}

struct TabState {
    chats: bool,
    sections: Vec<SectionState>,
    sizes: Sizes,
    filter: String,
    chip: Chip,
    by_time: bool,
    /// Some row has a time: the chips show.
    timed: bool,
    renames: BTreeMap<String, String>,
    titles0: BTreeMap<String, String>,
    taken: BTreeSet<String>,
}

impl TabState {
    fn chosen(&self) -> BTreeSet<String> {
        self.sections.iter().flat_map(SectionState::rows).filter(|r| r.on).map(|r| r.doc.clone()).collect()
    }

    fn in_chip(&self, r: &Row, now: &DateTime<Local>) -> bool {
        self.chip == Chip::All || r.when.is_some_and(|w| self.chip.holds(now, &bands::local(w)))
    }

    /// Ticked rows the chip hides (they are still loaded).
    fn hidden(&self, now: &DateTime<Local>) -> usize {
        let rows: BTreeMap<&String, &Row> =
            self.sections.iter().flat_map(SectionState::rows).filter(|r| r.on).map(|r| (&r.doc, r)).collect();
        rows.values().filter(|r| !self.in_chip(r, now)).count()
    }

    /// Rows inside each chip (each id once).
    fn chip_count(&self, chip: Chip, now: &DateTime<Local>) -> usize {
        let docs: BTreeMap<&String, Option<i64>> =
            self.sections.iter().flat_map(SectionState::rows).map(|r| (&r.doc, r.when)).collect();
        docs.values().filter(|w| chip == Chip::All || w.is_some_and(|w| chip.holds(now, &bands::local(w)))).count()
    }

    /// A chat preset name that cannot be saved, with the reason.
    fn bad_name(&self) -> Option<String> {
        if !self.chats {
            return None;
        }
        let mut seen = HashSet::new();
        for s in self.sections.iter().filter(|s| s.remember) {
            let n = s.save_as.trim();
            if !valid_preset(n) {
                return Some(format!("\"{n}\" cannot be a preset name: use lowercase letters, digits, . _ -"));
            }
            if (n != s.name && self.taken.contains(n)) || !seen.insert(n.to_string()) {
                return Some(format!("a preset named \"{n}\" exists already: pick another name"));
            }
        }
        None
    }

    fn noun(&self) -> &'static str {
        if self.chats { "chats" } else { "documents" }
    }
}

fn subtitle(sec: &Section, chats: bool) -> String {
    if chats {
        let kept = sec.ticked.len();
        return if sec.first {
            "new preset: nothing saved yet. Tick the chats this one needs; tick Remember to keep them as its preset."
                .into()
        } else if sec.new.is_empty() {
            format!("saved preset: {kept} chats")
        } else {
            format!("saved preset: {kept} chats, {} newer chats since (marked NEW)", sec.new.len())
        };
    }
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

fn folder(d: &str) -> &str {
    d.rsplit_once('/').map_or("", |(f, _)| f)
}

fn rows(docs: &[String], sec: &Section, sizes: &Sizes, look: &Look) -> Vec<Row> {
    docs.iter()
        .map(|d| {
            let readable = sizes.contains_key(d);
            Row {
                doc: d.clone(),
                label: look.labels.get(d).cloned(),
                default: look.defaults.get(d).cloned(),
                note: look.notes.get(d).cloned().unwrap_or_default(),
                folder: look.groups.get(d).cloned().unwrap_or_else(|| {
                    let f = folder(d);
                    if f.is_empty() { "(top level)".to_string() } else { format!("{f}/") }
                }),
                when: look.times.get(d).copied(),
                on: readable && sec.ticked.contains(d),
                readable,
                new: sec.new.contains(d),
            }
        })
        .collect()
}

/// A rename in progress: the tab, the section, the row id ("" = the section's own name).
struct Edit {
    tab: usize,
    sec: usize,
    doc: String,
    buf: String,
    focus: bool,
}

/// What a frame's drawing of the rows asks for.
#[derive(Default)]
struct Asked {
    start: Option<(String, String)>,
    commit: Option<(String, String)>,
}

/// Everything a list of rows is drawn with.
struct View<'a> {
    filter: &'a str,
    chip: Chip,
    by_time: bool,
    chats: bool,
    now: &'a DateTime<Local>,
    renames: &'a BTreeMap<String, String>,
    sizes: &'a Sizes,
    editing: Option<&'a mut Edit>,
}

impl View<'_> {
    fn visible(&self, r: &Row, short: bool) -> bool {
        let chip_ok = self.chip == Chip::All || r.when.is_some_and(|w| self.chip.holds(self.now, &bands::local(w)));
        if !chip_ok {
            return false;
        }
        if self.filter.is_empty() {
            return true;
        }
        format!("{} {} {}", r.shown(self.renames, short), r.folder, r.doc).to_lowercase().contains(self.filter)
    }
}

fn draw_rows(ui: &mut egui::Ui, rows: &mut [Row], v: &mut View, indent: f32) -> Asked {
    let mut asked = Asked::default();
    let folders: HashSet<&str> = rows.iter().map(|r| r.folder.as_str()).collect();
    let folder_heads = !v.by_time && (v.chats || folders.len() > 1);
    let short = folder_heads && !v.chats;
    let mut order: Vec<usize> = (0..rows.len()).collect();
    if v.by_time {
        order.sort_by_key(|&i| std::cmp::Reverse(rows[i].when.unwrap_or(i64::MIN))); // stable
    }
    let mut last: Option<String> = None;
    let pad = if folder_heads || v.by_time { indent + 12.0 } else { indent };
    for i in order {
        let r = &mut rows[i];
        if !v.visible(r, short) {
            continue;
        }
        let head = if v.by_time {
            Some(r.when.map_or("Unknown time", |w| bands::band(v.now, &bands::local(w)).label()).to_string())
        } else if folder_heads {
            Some(r.folder.clone())
        } else {
            None
        };
        if head.is_some() && head != last {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.label(RichText::new(head.as_deref().unwrap_or("")).small().strong());
            });
            last = head;
        }
        let shown = r.shown(v.renames, short);
        ui.horizontal(|ui| {
            ui.add_space(pad);
            if let Some(e) = v.editing.as_deref_mut().filter(|e| e.doc == r.doc) {
                let hint = if v.chats { "empty = the first prompt again" } else { "empty = the file name again" };
                let resp = ui.add(egui::TextEdit::singleline(&mut e.buf).hint_text(hint).desired_width(360.0));
                if e.focus {
                    resp.request_focus();
                    e.focus = false;
                }
                if resp.lost_focus() {
                    asked.commit = Some((r.doc.clone(), util::py_strip(&e.buf).to_string()));
                }
                ui.label(RichText::new("Enter keeps it, Esc undoes").small().weak());
            } else if r.readable {
                let cb = ui.checkbox(&mut r.on, &shown);
                let pen = ui.small_button("✏").on_hover_text("Rename");
                if cb.double_clicked() || pen.clicked() {
                    asked.start = Some((r.doc.clone(), shown.clone()));
                }
                let k = v.sizes.get(&r.doc).copied().unwrap_or(0) as f64 / CHARS_PER_K as f64;
                ui.label(RichText::new(format!("~{k:.0}k")).small());
                if !r.note.is_empty() {
                    ui.label(RichText::new(&r.note).small().weak());
                }
                if v.by_time && v.chats {
                    ui.label(RichText::new(&r.folder).small().weak()); // the folder heading moves onto the row
                }
                if !v.chats && shown != r.doc && !shown.is_empty() && r.doc.rsplit('/').next() != Some(shown.as_str()) {
                    ui.label(RichText::new(&r.doc).small().weak().monospace()); // the real id under a display name
                }
                if r.new {
                    ui.label(RichText::new("NEW").strong().color(Color32::from_rgb(0xff, 0xcc, 0x33)));
                }
            } else {
                ui.add_enabled(false, egui::Checkbox::new(&mut r.on, RichText::new(&shown).strikethrough()));
                ui.label(RichText::new("MISSING or unreadable").color(Color32::from_rgb(0xff, 0x66, 0x66)));
            }
        });
    }
    asked
}

struct App {
    tabs: Vec<Slot>,
    active: usize,
    setup: bool,
    command: String,
    edit: Option<Edit>,
    until: Instant,
    shot: Option<Shot>,
    result: Arc<Mutex<Option<Outcome>>>,
    closing: bool,
}

impl App {
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
        let tabs = self
            .tabs
            .iter()
            .map(|slot| slot.state.as_ref())
            .map(|t| {
                t.map(|t| TabPick {
                    picks: t
                        .sections
                        .iter()
                        .map(|s| {
                            let title0 = t.titles0.get(&s.name).cloned().unwrap_or_default();
                            Pick {
                                name: s.name.clone(),
                                ticked: s.rows().filter(|r| r.on).map(|r| r.doc.clone()).collect(),
                                remember: s.remember,
                                save_as: s.save_as.trim().to_string(),
                                title: (s.title.trim() != title0).then(|| s.title.trim().to_string()),
                            }
                        })
                        .collect(),
                    renames: t.renames.clone(),
                    chip: t.chip,
                    by_time: t.by_time,
                })
            })
            .collect();
        self.finish(ctx, Outcome::Picked(tabs));
    }

    /// The Load button's text and the summary line.
    fn totals(&self, now: &DateTime<Local>) -> (String, String, usize) {
        let mut parts = Vec::new();
        let mut short = Vec::new();
        let (mut k, mut n, mut hidden) = (0.0, 0, 0);
        for t in self.tabs.iter().filter_map(|s| s.state.as_ref()) {
            let c = t.chosen();
            k += c.iter().map(|d| t.sizes.get(d).copied().unwrap_or(0) as f64 / CHARS_PER_K as f64).sum::<f64>();
            n += c.len();
            hidden += t.hidden(now);
            if !c.is_empty() || self.tabs.len() == 1 {
                parts.push(format!("{} {}", c.len(), t.noun()));
                short.push(format!("{} {}", c.len(), if t.chats { "chats" } else { "docs" }));
            }
        }
        let k = k + 0.0; // an empty f64 sum is -0.0
        let what = if parts.is_empty() { "nothing ticked".to_string() } else { parts.join(" + ") };
        let tail = if self.setup { "" } else { " will be loaded" };
        let mut line = format!("{what}, ~{k:.0}k tokens{tail}");
        if hidden > 0 {
            line.push_str(&format!(" ({hidden} ticked but hidden by the time chip)"));
        }
        let button = if self.setup {
            "Save presets".to_string()
        } else if self.tabs.len() == 1 {
            format!("Load {}", parts.join(""))
        } else if short.is_empty() {
            "Load".to_string()
        } else {
            format!("Load {}", short.join(" + "))
        };
        (button, line, n)
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
        if self.tabs.iter_mut().map(Slot::poll).filter(|got| *got).count() > 0 {
            ctx.request_repaint();
        }
        let busy = self.tabs.iter().any(|s| s.loading.is_some());
        ctx.request_repaint_after(Duration::from_millis(if self.shot.is_some() || busy { 50 } else { 1000 }));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.closing {
            return;
        }
        // Esc undoes a rename in progress; otherwise it is Cancel
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.edit.take().is_none() {
            self.finish(&ctx, Outcome::Cancelled);
            return;
        }
        let now = Local::now();
        let (button, line, n) = self.totals(&now);
        let bad = self.tabs.iter().filter_map(|s| s.state.as_ref()).find_map(TabState::bad_name);
        let can_ok = (self.setup || n > 0) && bad.is_none() && self.edit.is_none();
        if can_ok && ctx.input(|i| i.key_pressed(egui::Key::Enter)) && ctx.memory(|m| m.focused().is_none()) {
            self.done(&ctx);
            return;
        }
        let margin = egui::Frame::central_panel(ui.style()).inner_margin(14.0);
        let host = host_name();

        egui::Panel::top("head").frame(margin).show(ui, |ui| {
            if self.tabs.len() > 1 {
                ui.horizontal(|ui| {
                    for (i, t) in self.tabs.iter_mut().enumerate() {
                        let name = if t.chats { "Chats" } else { "Research" };
                        let text = RichText::new(format!("{name} ({})", t.status())).heading();
                        if ui.selectable_label(self.active == i, text).clicked() {
                            self.active = i;
                            self.edit = None;
                            t.open(); // a tab that waited for this click loads now
                        }
                    }
                });
                ui.separator();
            }
            let Some(tab) = self.tabs[self.active].state.as_mut() else { return };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let title = match (self.setup, tab.chats) {
                    (true, _) => "Change the saved presets".to_string(),
                    (false, true) => format!("{host} wants to load earlier chats"),
                    (false, false) => format!("{host} wants to load research"),
                };
                let names = tab.sections.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ");
                ui.label(RichText::new(&title).heading().strong());
                ui.label(RichText::new(" for ").heading());
                ui.label(RichText::new(names).heading().strong());
            });
            ui.label(if self.setup {
                "Tick what each should contain, then press Save presets. Nothing is loaded.".to_string()
            } else {
                format!(
                    "Tick what this session needs, then press Load. Nothing is sent to {} until then; \
                     Cancel loads nothing.",
                    host_short()
                )
            });
            if !self.setup && !self.command.is_empty() {
                ui.label(RichText::new(format!("Asked by: {}", self.command)).small().monospace());
            }
            ui.add_space(4.0);
            if tab.timed {
                ui.horizontal(|ui| {
                    for chip in Chip::EVERY {
                        let text = format!("{} ({})", chip.label(), tab.chip_count(chip, &now));
                        if ui.selectable_label(tab.chip == chip, text).clicked() {
                            tab.chip = chip;
                        }
                    }
                    ui.add_space(16.0);
                    ui.label("Group by:");
                    if ui.selectable_label(tab.by_time, "time").clicked() {
                        tab.by_time = true;
                    }
                    if ui.selectable_label(!tab.by_time, "folder").clicked() {
                        tab.by_time = false;
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.add(
                    egui::TextEdit::singleline(&mut tab.filter)
                        .hint_text(if tab.chats {
                            "words from the name, the first prompt or the folder"
                        } else {
                            "words from the name or the id"
                        })
                        .desired_width(ui.available_width() - 70.0),
                );
                if !tab.filter.is_empty() && ui.button("Clear").clicked() {
                    tab.filter.clear();
                }
            });
        });

        let mut ok_clicked = false;
        let mut cancel_clicked = false;
        egui::Panel::bottom("bar").frame(margin).show(ui, |ui| {
            ui.label(RichText::new(&line).strong());
            if let Some(b) = &bad {
                ui.label(RichText::new(b).color(Color32::from_rgb(0xff, 0x66, 0x66)));
            }
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = egui::Button::new(RichText::new(&button).strong()).fill(if can_ok {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().widgets.inactive.weak_bg_fill
                });
                ok_clicked = ui.add_enabled(can_ok, ok).clicked();
                cancel_clicked =
                    ui.button(if self.setup { "Close without saving" } else { "Cancel (load nothing)" }).clicked();
            });
        });

        let active = self.active;
        let setup = self.setup;
        let mut start: Option<(usize, String, String)> = None;
        let mut commit: Option<(usize, String, String)> = None;
        egui::CentralPanel::default().frame(margin).show(ui, |ui| {
            let slot = &mut self.tabs[active];
            let Some(tab) = slot.state.as_mut() else {
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    if slot.loading.is_some() {
                        ui.spinner();
                        let what =
                            if slot.chats { "earlier chats (the first time can take a while)" } else { "research" };
                        ui.label(format!("Loading {what}…"));
                    } else {
                        ui.label(if slot.chats {
                            "No earlier chats to show."
                        } else {
                            "No research to show: set `default_set` (and a library) in the config."
                        });
                    }
                });
                return;
            };
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let filter = tab.filter.trim().to_lowercase();
                let (chip, by_time, chats) = (tab.chip, tab.by_time, tab.chats);
                let narrowed = !filter.is_empty() || chip != Chip::All;
                for (si, sec) in tab.sections.iter_mut().enumerate() {
                    let mut editing = self.edit.as_mut().filter(|e| e.tab == active && e.sec == si);
                    ui.add_space(12.0);
                    ui.horizontal(|ui| match editing.as_deref_mut().filter(|e| e.doc.is_empty()) {
                        Some(e) => {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut e.buf)
                                    .hint_text("empty = its own name again")
                                    .desired_width(300.0),
                            );
                            if e.focus {
                                resp.request_focus();
                                e.focus = false;
                            }
                            if resp.lost_focus() {
                                sec.title = util::py_strip(&e.buf).to_string();
                                commit = Some((si, String::new(), String::new()));
                            }
                        }
                        None => {
                            let shown = if sec.title.is_empty() { &sec.name } else { &sec.title };
                            ui.label(RichText::new(shown).heading().strong());
                            if !sec.title.is_empty() {
                                ui.label(RichText::new(&sec.name).small().weak().monospace());
                            }
                            if !chats && ui.small_button("✏").on_hover_text("Give this set a display name").clicked()
                            {
                                start = Some((si, String::new(), sec.title.clone()));
                            }
                        }
                    });
                    ui.label(RichText::new(&sec.sub).small());
                    let in_view = |r: &Row| {
                        let chip_ok = chip == Chip::All || r.when.is_some_and(|w| chip.holds(&now, &bands::local(w)));
                        chip_ok
                            && (filter.is_empty()
                                || format!("{} {} {}", r.shown(&tab.renames, false), r.folder, r.doc)
                                    .to_lowercase()
                                    .contains(&filter))
                    };
                    ui.horizontal(|ui| {
                        // with a filter or a chip on, the buttons act on the rows that show
                        if ui.button(if narrowed { "Select shown" } else { "Select all" }).clicked() {
                            sec.rows_mut().filter(|r| in_view(r)).for_each(|r| r.on = r.readable);
                        }
                        if ui.button(if narrowed { "Unselect shown" } else { "Select none" }).clicked() {
                            sec.rows_mut().filter(|r| in_view(r)).for_each(|r| r.on = false);
                        }
                        if ui.add_enabled(!sec.first, egui::Button::new("Back to saved preset")).clicked() {
                            let saved = sec.saved.clone();
                            sec.rows_mut().for_each(|r| r.on = saved.contains(&r.doc));
                        }
                    });
                    let mut view = View {
                        filter: &filter,
                        chip,
                        by_time,
                        chats,
                        now: &now,
                        renames: &tab.renames,
                        sizes: &tab.sizes,
                        editing: editing.filter(|e| !e.doc.is_empty()),
                    };
                    let mut asks = vec![draw_rows(ui, &mut sec.main, &mut view, 12.0)];
                    if !sec.other.is_empty() {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            let id = ui.make_persistent_id(("other", chats, &sec.name));
                            egui::CollapsingHeader::new(&sec.other_label)
                                .id_salt(id)
                                .default_open(sec.other_open)
                                .open((!filter.is_empty()).then_some(true))
                                .show(ui, |ui| asks.push(draw_rows(ui, &mut sec.other, &mut view, 0.0)));
                        });
                    }
                    for a in asks {
                        if let Some((doc, shown)) = a.start {
                            start = Some((si, doc, shown));
                        }
                        if let Some((doc, name)) = a.commit {
                            commit = Some((si, doc, name));
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        let what = if chats {
                            "Remember this choice as the preset"
                        } else {
                            "Remember this choice as the preset for"
                        };
                        let label = if chats { what.to_string() } else { format!("{what} {}", sec.name) };
                        ui.add_enabled(!setup, egui::Checkbox::new(&mut sec.remember, label));
                        if chats && (sec.remember || setup) {
                            ui.label("named");
                            ui.add(egui::TextEdit::singleline(&mut sec.save_as).desired_width(160.0))
                                .on_hover_text("Renaming moves the saved ticks to the new name");
                        }
                    });
                }
            });
        });

        if let Some((si, doc, buf)) = start {
            self.edit = Some(Edit { tab: active, sec: si, doc, buf, focus: true });
        }
        if let Some((_, doc, name)) = commit {
            if !doc.is_empty()
                && let Some(t) = self.tabs[active].state.as_mut()
            {
                t.renames.insert(doc, name);
            }
            self.edit = None;
        }
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

fn tab_state(t: &Tab) -> TabState {
    let look = &t.look;
    let sections = t
        .sections
        .iter()
        .map(|sec| {
            let other_open = sec.main.is_empty();
            SectionState {
                name: sec.name.clone(),
                title: look.titles.get(&sec.name).cloned().unwrap_or_default(),
                save_as: sec.name.clone(),
                sub: subtitle(sec, look.chats),
                main: rows(&sec.main, sec, &t.sizes, look),
                other: rows(&sec.other, sec, &t.sizes, look),
                other_label: match (other_open, look.chats) {
                    (true, true) => "Your chats: tick the ones to load".into(),
                    (true, false) => "Your library: tick the research to use".into(),
                    (false, true) => format!("Add other chats ({} more)", sec.other.len()),
                    (false, false) => format!("Add other research from your library ({} more)", sec.other.len()),
                },
                other_open,
                saved: sec.ticked.clone(),
                first: sec.first,
                remember: sec.remember,
            }
        })
        .collect();
    TabState {
        chats: look.chats,
        sections,
        sizes: t.sizes.clone(),
        filter: String::new(),
        chip: look.chip,
        by_time: look.by_time,
        timed: !look.times.is_empty(),
        renames: BTreeMap::new(),
        titles0: look.titles.clone(),
        taken: look.taken.clone(),
    }
}

/// Show the chooser on tab `start` and wait for the user. Err when no window can be shown.
pub fn popup(tabs: Vec<TabSlot>, start: usize, setup: bool, command: &str) -> Result<Outcome, String> {
    have_display()?;
    let result = Arc::new(Mutex::new(None));
    let what = match (tabs.len(), &tabs.first()) {
        (1, Some(TabSlot::Ready(t))) if t.look.chats => "choose chats to load",
        (1, _) => "choose research to load",
        _ => "choose chats and research to load",
    };
    let active = start.min(tabs.len().saturating_sub(1));
    let mut app = App {
        tabs: tabs.into_iter().map(Slot::new).collect(),
        active,
        setup,
        command: command.to_string(),
        edit: None,
        until: Instant::now() + remaining(),
        shot: Shot::from_env(),
        result: Arc::clone(&result),
        closing: false,
    };
    if let Some(slot) = app.tabs.get_mut(active) {
        slot.open(); // --start on the other side: that tab is wanted, load it now
    }
    let title = format!("The Researcher - {}", if setup { "change saved presets" } else { what });
    let size = if app.shot.is_some() { [720.0, 560.0] } else { [760.0, 800.0] };
    run(&title, size, Box::new(app))?;
    let out = result.lock().expect("not poisoned").take();
    Ok(out.unwrap_or(Outcome::Cancelled))
}

#[cfg(test)]
mod tab_tests {
    use super::*;

    fn tab(docs: &[(&str, Option<i64>, bool)], chip: Chip) -> TabState {
        let row = |(d, w, on): &(&str, Option<i64>, bool)| Row {
            doc: d.to_string(),
            label: None,
            default: None,
            note: String::new(),
            folder: String::new(),
            when: *w,
            on: *on,
            readable: true,
            new: false,
        };
        TabState {
            chats: true,
            sections: vec![SectionState {
                name: "a".into(),
                title: String::new(),
                save_as: "a".into(),
                sub: String::new(),
                main: docs.iter().map(row).collect(),
                other: Vec::new(),
                other_label: String::new(),
                other_open: false,
                saved: BTreeSet::new(),
                first: false,
                remember: false,
            }],
            sizes: Sizes::new(),
            filter: String::new(),
            chip,
            by_time: true,
            timed: true,
            renames: BTreeMap::new(),
            titles0: BTreeMap::new(),
            taken: ["b".to_string()].into(),
        }
    }

    #[test]
    fn ticks_survive_a_chip_switch_and_hidden_ones_are_counted() {
        let now = Local::now();
        let old = now.timestamp() - 40 * 86400;
        let mut t =
            tab(&[("new", Some(now.timestamp()), true), ("old", Some(old), true), ("none", None, false)], Chip::Week);
        assert_eq!(t.chosen().len(), 2);
        assert_eq!(t.hidden(&now), 1);
        assert_eq!(t.chip_count(Chip::Week, &now), 1);
        assert_eq!(t.chip_count(Chip::All, &now), 3);
        t.chip = Chip::All;
        assert_eq!((t.chosen().len(), t.hidden(&now)), (2, 0));
    }

    #[test]
    fn preset_names_are_checked_before_saving() {
        let mut t = tab(&[], Chip::All);
        assert_eq!(t.bad_name(), None); // Remember off: nothing is saved
        t.sections[0].remember = true;
        assert_eq!(t.bad_name(), None);
        t.sections[0].save_as = "Bad Name".into();
        assert!(t.bad_name().unwrap().contains("cannot be a preset name"));
        t.sections[0].save_as = "b".into();
        assert!(t.bad_name().unwrap().contains("exists already"));
    }

    #[test]
    fn a_later_tab_loads_only_when_opened() {
        let called = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&called);
        let load: Loader = Box::new(move || {
            flag.store(true, Ordering::SeqCst);
            Some(Tab { sections: Vec::new(), sizes: Sizes::new(), look: Look { chats: true, ..Look::default() } })
        });
        let mut slot = Slot::new(TabSlot::Later { chats: true, load });
        assert_eq!(slot.status(), "click to load");
        assert!(!slot.poll());
        std::thread::sleep(Duration::from_millis(50));
        assert!(!called.load(Ordering::SeqCst)); // nothing ran before the click
        slot.open();
        let t0 = Instant::now();
        while !slot.poll() {
            assert!(t0.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(called.load(Ordering::SeqCst));
        assert_eq!(slot.status(), "0 ticked");
        slot.open(); // a second click loads nothing again
        assert!(slot.loading.is_none());
    }

    #[test]
    fn a_cleared_name_falls_back() {
        let mut renames = BTreeMap::new();
        let r = Row {
            doc: "rust/linux".into(),
            label: Some("Saved".into()),
            default: None,
            note: String::new(),
            folder: "rust/".into(),
            when: None,
            on: false,
            readable: true,
            new: false,
        };
        assert_eq!(r.shown(&renames, true), "Saved");
        renames.insert("rust/linux".to_string(), "New".to_string());
        assert_eq!(r.shown(&renames, true), "New");
        renames.insert("rust/linux".to_string(), String::new());
        assert_eq!(r.shown(&renames, true), "linux");
        assert_eq!(r.shown(&renames, false), "rust/linux");
    }
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

/// The program that asked for the load, for the popup text: `RESEARCHER_HOST` if set, `opencode` when
/// opencode started us (it sets `OPENCODE=1` for its tools), otherwise Claude Code.
pub fn host_name() -> String {
    host_from(std::env::var("RESEARCHER_HOST").ok(), std::env::var_os("OPENCODE").is_some())
}

/// Same, for "Nothing is sent to ..." ("Claude" for Claude Code).
fn host_short() -> String {
    match host_name().as_str() {
        "Claude Code" => "Claude".to_string(),
        other => other.to_string(),
    }
}

fn host_from(over: Option<String>, opencode: bool) -> String {
    match over.map(|h| h.trim().to_string()) {
        Some(h) if !h.is_empty() => h,
        _ if opencode => "opencode".to_string(),
        _ => "Claude Code".to_string(),
    }
}

#[cfg(test)]
mod host_tests {
    use super::host_from;

    #[test]
    fn host_follows_override_then_opencode() {
        assert_eq!(host_from(None, false), "Claude Code");
        assert_eq!(host_from(None, true), "opencode");
        assert_eq!(host_from(Some("Zed".into()), true), "Zed");
        assert_eq!(host_from(Some("  ".into()), true), "opencode");
    }
}
