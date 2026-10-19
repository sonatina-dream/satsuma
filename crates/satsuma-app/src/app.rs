//! The menu-bar app: status item, drag monitoring, wheel, jobs and settings.

use crate::draw::rect;
use crate::editor::EditorWindow;
use crate::events::{self, UiEvent};
use crate::float::FloatWindow;
use crate::hud::{Hud, HudModel};
use crate::system::{self, ns};
use crate::wheel::{WheelModel, WheelWindow};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadOnly, Message};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBezierPath, NSColor,
    NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength, NSWindow, NSWindowDelegate,
};
use objc2_foundation::{
    MainThreadMarker, NSBundle, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRunLoop,
    NSRunLoopCommonModes, NSSize, NSTimer,
};
use satsuma_core::gesture::{DragTracker, Effect, Sample};
use satsuma_core::settings::{FloatBehavior, Settings, Strength, TerminalApp};
use satsuma_core::wheel::{initial_keyboard_mode, wheel_items};
use satsuma_core::{Action, Mods, WheelItem, WheelMode};
use satsuma_engine::{capability, detect_format, plan, run_plan, BatchProgress, BatchReport, Cancel, Context, Env, Operation};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const COMBO_CHOICES: [Mods; 8] = [
    Mods::SHIFT,
    Mods::COMMAND,
    Mods::CONTROL,
    Mods::OPTION,
    Mods::OPTION.union(Mods::SHIFT),
    Mods::CONTROL.union(Mods::SHIFT),
    Mods::COMMAND.union(Mods::SHIFT),
    Mods::CONTROL.union(Mods::OPTION),
];

enum JobMsg {
    Progress(BatchProgress),
    Done { report: BatchReport, skipped: Vec<(PathBuf, String)> },
    Failed(String),
}

struct RunningJob {
    cancel: Cancel,
    rx: Receiver<JobMsg>,
    title: String,
}

struct PendingJob {
    op: Operation,
    files: Vec<PathBuf>,
    at: NSPoint,
}

/// Work that may show modal UI, so it runs after the controller borrow is released.
enum Deferred {
    Alert(String, String),
    Move(Vec<PathBuf>),
    AirDrop(Vec<PathBuf>),
    Welcome,
}

struct Controller {
    mtm: MainThreadMarker,
    settings: Settings,
    settings_path: PathBuf,
    env: Arc<Env>,
    tracker: DragTracker,
    type_cache: Option<(isize, bool)>,
    wheel: WheelWindow,
    keyboard_files: Option<Vec<PathBuf>>,
    hud: Hud,
    hud_hide_at: Option<Instant>,
    /// When the mouse comes up over a petal, the drop message arrives just after; keep the
    /// panel on screen briefly so the drop is not lost.
    wheel_hide_at: Option<Instant>,
    floats: Vec<FloatWindow>,
    editors: Vec<EditorWindow>,
    status_item: Retained<NSStatusItem>,
    header: Option<Retained<NSMenuItem>>,
    delegate: Retained<AppDelegate>,
    job: Option<RunningJob>,
    queue: Vec<PendingJob>,
    deferred: Vec<Deferred>,
    /// Debug builds: quit once the queue drains (used by SATSUMA_SELFTEST_DROP).
    exit_when_idle: bool,
}

/// `SATSUMA_DEBUG=1` logs gestures and jobs to stderr.
fn debug_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SATSUMA_DEBUG").is_some_and(|v| v != "0"))
}

macro_rules! debug {
    ($($arg:tt)*) => {
        if debug_enabled() {
            eprintln!("satsuma: {}", format!($($arg)*));
        }
    };
}

thread_local! {
    static CONTROLLER: RefCell<Option<Controller>> = const { RefCell::new(None) };
}

/// Runs `f` on the controller unless it is already in use (re-entrant call).
fn with_controller<R>(f: impl FnOnce(&mut Controller) -> R) -> Option<R> {
    CONTROLLER.with(|c| c.try_borrow_mut().ok().and_then(|mut c| c.as_mut().map(f)))
}

fn run_deferred() {
    let work = with_controller(|c| std::mem::take(&mut c.deferred)).unwrap_or_default();
    for item in work {
        let mtm = system::main_thread();
        match item {
            Deferred::Alert(title, message) => system::alert(mtm, &title, &message),
            Deferred::Welcome => system::alert(
                mtm,
                "Satsuma is ready",
                "Satsuma lives in your menu bar.\n\n1. Drag files from Finder or the desktop.\n2. While dragging, tap Shift to convert, Command for tools, or Control for file actions.\n3. Drop onto a petal. Results are saved next to the originals.\n\nYou can change the keys in the menu bar under Wheel Keys.",
            ),
            Deferred::Move(files) => {
                if let Some(dest) = system::choose_folder(mtm, "Move Here", "Choose where to move the items.") {
                    let errors = system::move_into(&files, &dest);
                    if !errors.is_empty() {
                        system::alert(mtm, "Some items couldn't be moved", &errors.join("\n"));
                    }
                }
            }
            Deferred::AirDrop(files) => {
                if let Err(e) = system::airdrop(mtm, &files) {
                    system::alert(mtm, "AirDrop", &e);
                }
            }
        }
    }
}

impl Controller {
    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&self.settings_path) {
            self.deferred.push(Deferred::Alert("Couldn't save settings".into(), e.to_string()));
        }
        self.rebuild_menu();
    }

    fn items_for(&self, mode: WheelMode, files: &[PathBuf]) -> Vec<WheelItem> {
        let formats: Vec<_> = files.iter().map(|p| detect_format(p)).collect();
        let env = &self.env;
        wheel_items(mode, &formats, &|item| capability::item_available(env, item, &formats))
    }

    fn model(&self, mode: WheelMode, items: Vec<WheelItem>, keyboard: bool, files: &[PathBuf]) -> WheelModel {
        let selected = if keyboard && !items.is_empty() { Some(0) } else { None };
        let bytes: Option<u64> = files
            .iter()
            .map(|f| std::fs::metadata(f).ok().filter(|m| m.is_file()).map(|m| m.len()))
            .sum();
        WheelModel {
            mode,
            items,
            selected,
            keyboard,
            feedback: self.settings.feedback,
            combos: self.settings.wheels,
            hub: satsuma_core::wheel::hub_caption(files.len(), bytes),
        }
    }

    // ------------------------------------------------------------ the 60 Hz tick

    fn tick(&mut self) {
        self.sample_drag();
        for event in events::drain() {
            self.handle(event);
        }
        self.poll_job();
        if self.wheel_hide_at.is_some_and(|t| Instant::now() >= t) {
            self.wheel_hide_at = None;
            self.wheel.hide();
        }
        if let Some(at) = self.hud_hide_at {
            if Instant::now() >= at {
                self.hud.hide();
                self.hud_hide_at = None;
            }
        }
    }

    fn sample_drag(&mut self) {
        let mouse_down = system::left_mouse_down();
        let pb = system::drag_pasteboard();
        let count = pb.changeCount();
        let has_files = if self.tracker.needs_type_check(mouse_down, count as i64) {
            match self.type_cache {
                Some((c, v)) if c == count => v,
                _ => {
                    let v = system::has_file_urls(&pb);
                    self.type_cache = Some((count, v));
                    v
                }
            }
        } else {
            false
        };
        let loc = system::mouse_location();
        let sample = Sample {
            mouse_down,
            drag_change_count: count as i64,
            drag_has_files: has_files,
            mods: system::current_mods(),
            location: (loc.x, loc.y),
        };
        let was_active = self.tracker.is_file_drag_active();
        let effect = self.tracker.update(&sample, &self.settings.wheels);
        if self.tracker.is_file_drag_active() != was_active {
            debug!("file drag {}", if was_active { "ended" } else { "started" });
        }
        if let Some(e) = &effect {
            debug!("gesture {e:?} (mods {:?})", sample.mods);
        }
        if self.keyboard_files.is_some() {
            // The menu-opened wheel owns the panel; ignore drag gestures meanwhile.
            self.tracker.wheel_closed();
            return;
        }
        match effect {
            Some(Effect::Open { mode, at }) => {
                self.wheel_hide_at = None;
                let files = system::file_paths(&pb);
                let items = self.items_for(mode, &files);
                let model = self.model(mode, items, false, &files);
                self.wheel.show(self.mtm, model, NSPoint::new(at.0, at.1));
            }
            Some(Effect::Switch { mode }) => {
                let files = system::file_paths(&pb);
                let items = self.items_for(mode, &files);
                self.wheel.switch(mode, items);
            }
            Some(Effect::Close) => {
                if self.wheel.view.model().selected.is_some() {
                    self.wheel_hide_at = Some(Instant::now() + Duration::from_millis(400));
                } else {
                    self.wheel.hide();
                }
            }
            None => {}
        }
    }

    fn handle(&mut self, event: UiEvent) {
        debug!("event {event:?}");
        match event {
            UiEvent::WheelDrop { item, files, at } => {
                self.wheel_hide_at = None;
                self.wheel.hide();
                self.tracker.wheel_closed();
                self.dispatch(item, files, at);
            }
            UiEvent::WheelPick { item } => {
                let at = self.wheel.anchor();
                self.wheel.hide();
                if let Some(files) = self.keyboard_files.take() {
                    self.dispatch(item, files, at);
                }
            }
            UiEvent::WheelCancel => {
                if self.keyboard_files.take().is_some() {
                    self.wheel.hide();
                }
            }
            UiEvent::WheelSwitch(mode) => {
                if let Some(files) = self.keyboard_files.clone() {
                    let items = self.items_for(mode, &files);
                    self.wheel.switch(mode, items);
                }
            }
            UiEvent::HudClose => match &self.job {
                Some(job) => job.cancel.cancel(),
                None => {
                    self.hud.hide();
                    self.hud_hide_at = None;
                }
            },
            UiEvent::FloatAdd { window, files } => {
                if let Some(f) = self.floats.iter().find(|f| f.window_number() == window) {
                    f.add(&files);
                }
            }
            UiEvent::FloatRemoveSelected { window } => {
                if let Some(i) = self.floats.iter().position(|f| f.window_number() == window) {
                    self.floats[i].view.remove_selected();
                    if self.floats[i].view.len() == 0 {
                        self.floats[i].panel.close();
                    } else {
                        self.floats[i].fit();
                    }
                }
            }
            UiEvent::WindowClosed { window } => {
                self.floats.retain(|f| f.window_number() != window);
                if let Some(i) = self.editors.iter().position(|e| e.window_number() == window) {
                    let editor = self.editors.remove(i);
                    editor.teardown();
                }
            }
            UiEvent::EditorApply { op, files, at } => self.enqueue(PendingJob { op, files, at }),
            UiEvent::WindowResignedKey { window } => {
                if self.keyboard_files.is_some() && self.wheel.panel.windowNumber() == window {
                    self.keyboard_files = None;
                    self.wheel.hide();
                }
            }
        }
    }

    fn flash(&mut self, at: NSPoint, title: &str, subtitle: &str) {
        self.hud.show(
            self.mtm,
            at,
            HudModel { title: title.into(), subtitle: subtitle.into(), fraction: None, error: false },
        );
        self.hud_hide_at = Some(Instant::now() + Duration::from_millis(1400));
    }

    fn dispatch(&mut self, item: WheelItem, files: Vec<PathBuf>, at: NSPoint) {
        match item {
            WheelItem::Convert(f) => self.enqueue(PendingJob { op: Operation::Convert(f), files, at }),
            WheelItem::Tool(t) if t.has_editor() => self.open_editor(t, files),
            WheelItem::Tool(t) => self.enqueue(PendingJob { op: Operation::Tool(t), files, at }),
            WheelItem::Action(action) => self.perform_action(action, files, at),
        }
    }

    fn open_editor(&mut self, tool: satsuma_core::Tool, files: Vec<PathBuf>) {
        let delegate: &ProtocolObject<dyn NSWindowDelegate> = ProtocolObject::from_ref(&*self.delegate);
        let editor = match tool {
            satsuma_core::Tool::CropImage => EditorWindow::crop(self.mtm, files, delegate, true),
            satsuma_core::Tool::EditImage => EditorWindow::adjust(self.mtm, files, delegate, true),
            _ => EditorWindow::frame(self.mtm, files, delegate, true),
        };
        match editor {
            Ok(e) => self.editors.push(e),
            Err(e) => self.deferred.push(Deferred::Alert(format!("Couldn't open {}", tool.info().name), e)),
        }
    }

    fn perform_action(&mut self, action: Action, files: Vec<PathBuf>, at: NSPoint) {
        let n = files.len();
        let noun = if n == 1 { "item".to_string() } else { format!("{n} items") };
        match action {
            Action::Copy => {
                system::copy_files(&files);
                self.flash(at, "Copied", &format!("{noun} ready to paste in Finder"));
            }
            Action::CopyPath => {
                system::copy_paths(&files);
                self.flash(at, "Copied Path", &if n == 1 { files[0].display().to_string() } else { format!("{n} paths, one per line") });
            }
            Action::Terminal => {
                let app = self.settings.terminal;
                if let Err(e) = system::open_terminal(&files[0], app.bundle_id(), app.name()) {
                    self.deferred.push(Deferred::Alert("Terminal".into(), e));
                }
            }
            Action::Move => self.deferred.push(Deferred::Move(files)),
            Action::AirDrop => self.deferred.push(Deferred::AirDrop(files)),
            Action::Float => {
                let existing = self.settings.float_behavior == FloatBehavior::AddToExisting && !self.floats.is_empty();
                if existing {
                    let f = self.floats.last().unwrap();
                    f.add(&files);
                    f.panel.orderFrontRegardless();
                } else {
                    let delegate: &ProtocolObject<dyn NSWindowDelegate> = ProtocolObject::from_ref(&*self.delegate);
                    let f = FloatWindow::new(self.mtm, delegate);
                    f.view.add(&files);
                    f.show_near(at);
                    self.floats.push(f);
                }
            }
        }
    }

    // ------------------------------------------------------------ jobs

    fn enqueue(&mut self, job: PendingJob) {
        self.queue.push(job);
        if self.job.is_none() {
            self.start_next();
        }
    }

    fn start_next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let PendingJob { op, files, at } = self.queue.remove(0);
        let (tx, rx) = channel();
        let cancel = Cancel::new();
        let env = self.env.clone();
        let settings = self.settings.clone();
        let job_cancel = cancel.clone();
        let title = op.title();
        std::thread::spawn(move || {
            let p = match plan(&env, &files, op) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(JobMsg::Failed(e.to_string()));
                    return;
                }
            };
            let ctx = Context { env: &env, settings: &settings, cancel: &job_cancel };
            let progress_tx = tx.clone();
            let report = run_plan(&ctx, &p, &mut |bp| {
                let _ = progress_tx.send(JobMsg::Progress(bp));
            });
            let _ = tx.send(JobMsg::Done { report, skipped: p.skipped });
        });
        self.hud.show(
            self.mtm,
            at,
            HudModel { title: title.clone(), subtitle: "Preparing…".into(), fraction: Some(0.0), error: false },
        );
        self.hud_hide_at = None;
        self.job = Some(RunningJob { cancel, rx, title });
        self.set_working(true);
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        let mut finished: Option<(HudModel, Duration)> = None;
        let mut latest: Option<BatchProgress> = None;
        loop {
            match job.rx.try_recv() {
                Ok(JobMsg::Progress(p)) => latest = Some(p),
                Ok(JobMsg::Done { report, skipped }) => {
                    finished = Some(summarize(&job.title, &report, &skipped));
                    break;
                }
                Ok(JobMsg::Failed(msg)) => {
                    finished = Some((
                        HudModel { title: "Couldn't start".into(), subtitle: msg, fraction: None, error: true },
                        Duration::from_secs(6),
                    ));
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some((
                        HudModel { title: "Operation failed".into(), subtitle: "The worker stopped unexpectedly.".into(), fraction: None, error: true },
                        Duration::from_secs(6),
                    ));
                    break;
                }
            }
        }
        if let Some(p) = latest {
            let title = job.title.clone();
            self.hud.update(HudModel {
                title,
                subtitle: format!("{} of {} saved", p.done, p.total),
                fraction: Some(p.fraction),
                error: false,
            });
        }
        if let Some((model, linger)) = finished {
            debug!("job finished: {} — {}", model.title, model.subtitle);
            if self.exit_when_idle && self.queue.is_empty() {
                NSApplication::sharedApplication(self.mtm).terminate(None);
            }
            self.job = None;
            self.hud.update(model);
            self.hud_hide_at = Some(Instant::now() + linger);
            self.set_working(false);
            self.start_next();
        }
    }

    fn set_working(&self, working: bool) {
        if let Some(h) = &self.header {
            h.setTitle(&ns(if working { "Satsuma is working…" } else { "Satsuma is ready" }));
        }
    }

    // ------------------------------------------------------------ menu

    fn rebuild_menu(&mut self) {
        let menu = build_menu(self.mtm, &self.delegate, &self.settings, self.job.is_some());
        self.header = menu.itemAtIndex(0);
        self.status_item.setMenu(Some(&menu));
    }

    fn open_keyboard_wheel(&mut self, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }
        let has_conversions = !self.items_for(WheelMode::Conversions, &files).is_empty();
        let mode = initial_keyboard_mode(has_conversions);
        let items = self.items_for(mode, &files);
        let model = self.model(mode, items, true, &files);
        let at = system::mouse_location();
        self.keyboard_files = Some(files);
        let screen = system::screen_frame_for(at, self.mtm);
        let center = NSPoint::new(screen.origin.x + screen.size.width / 2.0, screen.origin.y + screen.size.height / 2.0);
        self.wheel.show(self.mtm, model, center);
    }
}

fn summarize(title: &str, report: &BatchReport, skipped: &[(PathBuf, String)]) -> (HudModel, Duration) {
    let saved = report.outputs.len();
    if report.cancelled {
        return (HudModel { title: "Cancelled".into(), subtitle: format!("{saved} saved before cancelling"), fraction: None, error: false }, Duration::from_millis(1200));
    }
    if let Some((path, err)) = report.errors.first() {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let more = if report.errors.len() > 1 { format!(" (+{} more)", report.errors.len() - 1) } else { String::new() };
        return (
            HudModel { title: "Finished with errors".into(), subtitle: format!("{name}: {err}{more}"), fraction: None, error: true },
            Duration::from_secs(8),
        );
    }
    if saved == 0 {
        let reason = skipped.first().map(|(_, r)| r.clone()).unwrap_or_else(|| "Nothing to do.".into());
        return (HudModel { title: "Nothing to do".into(), subtitle: reason, fraction: None, error: true }, Duration::from_secs(5));
    }
    let subtitle = if saved == 1 {
        format!("Saved {}", report.outputs[0].file_name().unwrap_or_default().to_string_lossy())
    } else {
        format!("{saved} of {saved} saved")
    };
    (HudModel { title: title.into(), subtitle, fraction: Some(1.0), error: false }, Duration::from_millis(1600))
}

// ---------------------------------------------------------------- menu building

fn menu_item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str, target: &AnyObject, tag: isize, checked: bool) -> Retained<NSMenuItem> {
    let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &ns(title), action, &ns(key)) };
    if action.is_some() {
        unsafe { item.setTarget(Some(target)) };
    }
    item.setTag(tag);
    if checked {
        item.setState(NSControlStateValueOn);
    }
    item
}

fn submenu(mtm: MainThreadMarker, title: &str, items: Vec<Retained<NSMenuItem>>) -> Retained<NSMenuItem> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns(title));
    menu.setAutoenablesItems(false);
    for i in items {
        menu.addItem(&i);
    }
    let parent = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &ns(title), None, &ns("")) };
    parent.setSubmenu(Some(&menu));
    parent
}

fn login_item_enabled() -> bool {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};
    is_bundled() && unsafe { SMAppService::mainAppService().status() } == SMAppServiceStatus::Enabled
}

fn is_bundled() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

fn build_menu(mtm: MainThreadMarker, delegate: &AppDelegate, s: &Settings, working: bool) -> Retained<NSMenu> {
    let t: &AnyObject = delegate;
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns("Satsuma"));
    menu.setAutoenablesItems(false);
    let header = menu_item(mtm, if working { "Satsuma is working…" } else { "Satsuma is ready" }, None, "", t, 0, false);
    header.setEnabled(false);
    menu.addItem(&header);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item(mtm, "Convert Files…", Some(sel!(convertFiles:)), "o", t, 0, false));
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let mut wheel_menus = Vec::new();
    for (mi, mode) in WheelMode::ALL.into_iter().enumerate() {
        let current = s.wheels.combo(mode);
        let choices: Vec<_> = COMBO_CHOICES
            .iter()
            .enumerate()
            .map(|(ci, m)| {
                menu_item(mtm, &format!("{}  {}", m.symbols(), m.describe()), Some(sel!(setWheelKey:)), "", t, (mi * 16 + ci) as isize, *m == current)
            })
            .collect();
        wheel_menus.push(submenu(mtm, &format!("{}: {}", mode.name(), current.describe()), choices));
    }
    menu.addItem(&submenu(mtm, "Wheel Keys", wheel_menus));

    let strength = |base: isize, current: Strength| {
        vec![
            menu_item(mtm, "Balanced", Some(sel!(setStrength:)), "", t, base, current == Strength::Balanced),
            menu_item(mtm, "Strong", Some(sel!(setStrength:)), "", t, base + 1, current == Strength::Strong),
        ]
    };
    let sizes: Vec<_> = [0u32, 4096, 2048, 1024]
        .into_iter()
        .map(|px| {
            let title = if px == 0 { "Keep Original Dimensions".to_string() } else { format!("Longest Edge {px} px") };
            let checked = s.image_max_edge.unwrap_or(0) == px;
            menu_item(mtm, &title, Some(sel!(setImageSize:)), "", t, px as isize, checked)
        })
        .collect();
    menu.addItem(&submenu(
        mtm,
        "Compression",
        vec![
            submenu(mtm, "Images", strength(0, s.image_compression)),
            submenu(mtm, "Image Size", sizes),
            submenu(mtm, "Video", strength(10, s.video_compression)),
            submenu(mtm, "Audio", strength(20, s.audio_compression)),
        ],
    ));
    let parts: Vec<_> = (2..=6)
        .map(|n| menu_item(mtm, &format!("{n} Equal Parts"), Some(sel!(setSplitParts:)), "", t, n, s.split_video_parts == n as u32))
        .collect();
    menu.addItem(&submenu(mtm, "Split Video Into", parts));
    menu.addItem(&submenu(
        mtm,
        "Terminal App",
        vec![
            menu_item(mtm, "Terminal", Some(sel!(setTerminal:)), "", t, 0, s.terminal == TerminalApp::Terminal),
            menu_item(mtm, "iTerm2", Some(sel!(setTerminal:)), "", t, 1, s.terminal == TerminalApp::ITerm2),
        ],
    ));
    menu.addItem(&submenu(
        mtm,
        "Float",
        vec![
            menu_item(mtm, "Add Files to an Existing Float", Some(sel!(setFloatBehavior:)), "", t, 0, s.float_behavior == FloatBehavior::AddToExisting),
            menu_item(mtm, "Create a New Float", Some(sel!(setFloatBehavior:)), "", t, 1, s.float_behavior == FloatBehavior::CreateNew),
        ],
    ));
    menu.addItem(&menu_item(mtm, "Sound and Haptic Feedback", Some(sel!(toggleFeedback:)), "", t, 0, s.feedback));
    menu.addItem(&menu_item(mtm, "Launch at Login", Some(sel!(toggleLogin:)), "", t, 0, login_item_enabled()));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item(mtm, "How to Use Satsuma", Some(sel!(showHelp:)), "", t, 0, false));
    menu.addItem(&menu_item(mtm, "Quit Satsuma", Some(sel!(quit:)), "q", t, 0, false));
    menu
}

/// A citrus-slice template icon for the menu bar.
fn status_icon() -> Retained<NSImage> {
    let size = NSSize::new(18.0, 18.0);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    #[allow(deprecated)]
    image.lockFocus();
    NSColor::blackColor().set();
    let ring = NSBezierPath::bezierPathWithOvalInRect(rect(2.0, 2.0, 14.0, 14.0));
    ring.setLineWidth(1.6);
    ring.stroke();
    for k in 0..6 {
        let a = std::f64::consts::PI / 2.0 - k as f64 * std::f64::consts::PI / 3.0;
        let seg = NSBezierPath::bezierPath();
        seg.moveToPoint(NSPoint::new(9.0 + 1.6 * a.cos(), 9.0 + 1.6 * a.sin()));
        seg.lineToPoint(NSPoint::new(9.0 + 5.6 * a.cos(), 9.0 + 5.6 * a.sin()));
        seg.setLineWidth(1.3);
        seg.stroke();
    }
    #[allow(deprecated)]
    image.unlockFocus();
    image.setTemplate(true);
    image
}

// ---------------------------------------------------------------- delegate

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaAppDelegate"]
    pub struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            self.start();
        }
    }

    unsafe impl NSWindowDelegate for AppDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, notification: &NSNotification) {
            if let Some(w) = notification.object().and_then(|o| o.downcast::<NSWindow>().ok()) {
                events::post(UiEvent::WindowClosed { window: w.windowNumber() });
            }
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, notification: &NSNotification) {
            if let Some(w) = notification.object().and_then(|o| o.downcast::<NSWindow>().ok()) {
                events::post(UiEvent::WindowResignedKey { window: w.windowNumber() });
            }
        }
    }

    impl AppDelegate {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            with_controller(|c| c.tick());
            run_deferred();
        }

        #[unsafe(method(convertFiles:))]
        fn convert_files(&self, _sender: Option<&AnyObject>) {
            let files = system::choose_files(self.mtm());
            with_controller(|c| c.open_keyboard_wheel(files));
        }

        #[unsafe(method(setWheelKey:))]
        fn set_wheel_key(&self, sender: &NSMenuItem) {
            let tag = sender.tag() as usize;
            let (mode, choice) = (WheelMode::ALL[(tag / 16).min(2)], COMBO_CHOICES[(tag % 16).min(COMBO_CHOICES.len() - 1)]);
            with_controller(|c| {
                // Swap with whichever wheel already uses the combination, so each stays unique.
                let old = c.settings.wheels.combo(mode);
                for other in WheelMode::ALL {
                    if other != mode && c.settings.wheels.combo(other) == choice {
                        c.settings.wheels.set(other, old);
                    }
                }
                c.settings.wheels.set(mode, choice);
                c.save_settings();
            });
        }

        #[unsafe(method(setStrength:))]
        fn set_strength(&self, sender: &NSMenuItem) {
            let tag = sender.tag();
            let strength = if tag % 10 == 1 { Strength::Strong } else { Strength::Balanced };
            with_controller(|c| {
                match tag / 10 {
                    0 => c.settings.image_compression = strength,
                    1 => c.settings.video_compression = strength,
                    _ => c.settings.audio_compression = strength,
                }
                c.save_settings();
            });
        }

        #[unsafe(method(setImageSize:))]
        fn set_image_size(&self, sender: &NSMenuItem) {
            let px = sender.tag() as u32;
            with_controller(|c| {
                c.settings.image_max_edge = if px == 0 { None } else { Some(px) };
                c.save_settings();
            });
        }

        #[unsafe(method(setSplitParts:))]
        fn set_split_parts(&self, sender: &NSMenuItem) {
            let n = sender.tag() as u32;
            with_controller(|c| {
                c.settings.split_video_parts = n;
                c.save_settings();
            });
        }

        #[unsafe(method(setTerminal:))]
        fn set_terminal(&self, sender: &NSMenuItem) {
            let app = if sender.tag() == 1 { TerminalApp::ITerm2 } else { TerminalApp::Terminal };
            with_controller(|c| {
                c.settings.terminal = app;
                c.save_settings();
            });
        }

        #[unsafe(method(setFloatBehavior:))]
        fn set_float_behavior(&self, sender: &NSMenuItem) {
            let b = if sender.tag() == 1 { FloatBehavior::CreateNew } else { FloatBehavior::AddToExisting };
            with_controller(|c| {
                c.settings.float_behavior = b;
                c.save_settings();
            });
        }

        #[unsafe(method(toggleFeedback:))]
        fn toggle_feedback(&self, _sender: Option<&AnyObject>) {
            with_controller(|c| {
                c.settings.feedback = !c.settings.feedback;
                c.save_settings();
            });
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: Option<&AnyObject>) {
            use objc2_service_management::SMAppService;
            let mtm = self.mtm();
            if !is_bundled() {
                system::alert(mtm, "Launch at Login", "Launch at Login works when Satsuma runs from Satsuma.app. Build it with scripts/bundle.sh.");
                return;
            }
            let service = unsafe { SMAppService::mainAppService() };
            let result = if login_item_enabled() {
                unsafe { service.unregisterAndReturnError() }
            } else {
                unsafe { service.registerAndReturnError() }
            };
            if let Err(e) = result {
                system::alert(mtm, "Couldn't update Launch at Login", &e.localizedDescription().to_string());
            }
            with_controller(|c| c.rebuild_menu());
        }

        #[unsafe(method(showHelp:))]
        fn show_help(&self, _sender: Option<&AnyObject>) {
            with_controller(|c| c.deferred.push(Deferred::Welcome));
            run_deferred();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl AppDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }

    fn start(&self) {
        let mtm = self.mtm();
        let settings_path = Settings::default_path();
        let settings = Settings::load(&settings_path);
        let status_item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = status_item.button(mtm) {
            button.setImage(Some(&status_icon()));
            button.setToolTip(Some(&ns("Satsuma — drag files and tap Shift, Command or Control")));
        }
        let wheel = WheelWindow::new(mtm);
        wheel.panel.setDelegate(Some(ProtocolObject::from_ref(self)));
        let welcome = !settings.has_completed_welcome;
        let delegate: Retained<AppDelegate> = self.retain();
        let mut controller = Controller {
            mtm,
            settings,
            settings_path,
            env: Arc::new(Env::discover()),
            tracker: DragTracker::new(),
            type_cache: None,
            wheel,
            keyboard_files: None,
            hud: Hud::new(mtm),
            hud_hide_at: None,
            wheel_hide_at: None,
            floats: Vec::new(),
            editors: Vec::new(),
            status_item,
            header: None,
            delegate,
            job: None,
            queue: Vec::new(),
            deferred: Vec::new(),
            exit_when_idle: false,
        };
        #[cfg(debug_assertions)]
        if let Some(event) = selftest_drop() {
            controller.exit_when_idle = true;
            events::post(event);
        }
        controller.rebuild_menu();
        if welcome {
            controller.settings.has_completed_welcome = true;
            controller.save_settings();
            controller.deferred.push(Deferred::Welcome);
        }
        if !controller.env.has_ffmpeg() {
            controller.deferred.push(Deferred::Alert(
                "Audio and video need ffmpeg".into(),
                "Install it with `brew install ffmpeg`, then restart Satsuma. Images, documents, PDFs, spreadsheets, subtitles and archives work without it.".into(),
            ));
        }
        debug!(
            "menu-bar app started; settings {}; ffmpeg {:?}; status item window {:?}",
            controller.settings_path.display(),
            controller.env.ffmpeg,
            controller.status_item.button(mtm).and_then(|b| b.window()).map(|w| (w.windowNumber(), w.isVisible()))
        );
        CONTROLLER.with(|c| *c.borrow_mut() = Some(controller));

        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(1.0 / 60.0, self, sel!(tick:), None, true)
        };
        unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    }
}

/// Debug hook: `SATSUMA_SELFTEST_DROP="convert:mp3|/path/a.wav|/path/b.wav"` (or `tool:<id>|…`)
/// posts the same event a real drop on a petal posts, then quits when the job is done.
#[cfg(debug_assertions)]
fn selftest_drop() -> Option<UiEvent> {
    let spec = std::env::var("SATSUMA_SELFTEST_DROP").ok()?;
    let mut parts = spec.split('|');
    let (kind, id) = parts.next()?.split_once(':')?;
    let item = match kind {
        "convert" => WheelItem::Convert(satsuma_core::Format::from_id(id)?),
        "tool" => WheelItem::Tool(satsuma_core::Tool::from_id(id)?),
        _ => return None,
    };
    let files = parts.map(PathBuf::from).collect();
    Some(UiEvent::WheelDrop { item, files, at: system::mouse_location() })
}

pub fn run() {
    let mtm = system::main_thread();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate = AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}
