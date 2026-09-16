//! The release-candidate walkthrough: drives the studio the way a person
//! actually would, and photographs everything.
//!
//! This is the third leg of the studio's headless-review scripts, and it is
//! deliberately not a rewrite of the other two:
//!
//! - `tour` renders every *state* the shell can be in, by setting the signal
//!   that state is built from. Fast, and exhaustive over "does every screen
//!   still draw" — but it never asks whether a person can actually *reach*
//!   that state by hand.
//! - `walkthrough` drives one held-together *session* forward with real
//!   pointer drags, so state that leaks between steps has somewhere to show
//!   up.
//! - This script drives every *command* the palette exposes
//!   (`Command::ALL`), for real, plus the interactions neither of the above
//!   scripts touches at all: right-click for a context menu, hover-to-tooltip
//!   the way a mouse actually produces it, drag-to-select text, and typed
//!   input delivered as an IME commit rather than a signal assignment.
//!
//! Every click, drag and hover is aimed at a point found the same way a
//! screen reader would find it — [`vieww_render::SemanticsTree`] by label —
//! never a hardcoded pixel. That is not a style preference: it is what makes
//! this survive the studio being resized, retitled or relaid-out, and what
//! `ci/mobile/device-suite.sh` already leans on for the same reason (see its
//! `tap_targets` for the on-device version of this idea).
//!
//! ```console
//! cargo run -p viewwstudio --release --example release_check -- out/ [--small] [--light]
//! ```
//!
//! Two things come out of `out/`:
//!
//! - `NNN-name.png` — one frame per step, for a person to skim.
//! - `manifest.json` — one record per step: what was done, whether a target
//!   label was found, and whether the renderer itself reported a frame error
//!   ([`vieww_render::FrameDriver::take_frame_errors`]). That last one is the
//!   actual pass/fail signal; the screenshots are for a human. The process
//!   exits non-zero if any step produced a frame error, so `ci/certify/release-check.sh`
//!   can use this as a gate as well as a report.
//!
//! `ci/certify/release-check.sh` is what actually gets run before a beta: it runs this
//! example in every theme/size combination on the host platform, then builds
//! one HTML gallery out of however many of Linux, Windows and macOS ran it.

use std::path::PathBuf;
use std::time::Duration;

use vieww_foundation::{
    ImeEvent, KeyEvent, Modifiers, NamedKey, Offset, PointerButton, PointerDeviceKind,
    PointerEvent, PointerId, ScrollEvent, Size,
};
use vieww_render::FrameDriver;
use viewwstudio::command::{Command, Menu};
use viewwstudio::compile::{Session as CompileSession, Toolchain};
use viewwstudio::state::{PanelTab, Platform, RightTab, View};
use viewwstudio::{Shell, Studio, Workspace, WINDOW};

fn screens() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/screens"))
}

/// Commands that are unsafe or pointless to fire headless in a sweep: they
/// touch the network (`Push`/`Pull`/`Fetch`), start a real multi-minute
/// `rustc`/`cargo` invocation this script is not waiting on
/// (`Build`/`BuildRelease`/`BuildAndRun`/`ExportSelected`), or shell out to a
/// toolchain that plainly is not on this machine (`ScanDevices`). Every other
/// command in `Command::ALL` runs for real. Skipped rather than silently
/// omitted: each one still gets a manifest line saying so, which is what
/// keeps this list itself honest — a command added to `Command::ALL` and
/// never added here still shows up as "ran", not as a silent gap.
const SKIP: &[Command] = &[
    Command::Push,
    Command::Pull,
    Command::Fetch,
    Command::Build,
    Command::BuildRelease,
    Command::BuildAndRun,
    Command::ExportSelected,
    Command::ScanDevices,
    Command::RestartAnalyzer,
    Command::CancelBuild,
    Command::CancelExport,
    Command::CancelAllTasks,
];

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .map_or_else(|| PathBuf::from("release-check"), PathBuf::from);
    let rest: Vec<String> = args.collect();
    let small = rest.iter().any(|a| a == "--small");
    let light = rest.iter().any(|a| a == "--light");
    std::fs::create_dir_all(&out).expect("an output directory");

    let window = if small {
        Size::new(1366.0, 679.0)
    } else {
        WINDOW
    };

    let mut driver = FrameDriver::new(window);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();

    let root = screens();
    let workspace = Workspace::open(&root);
    let target = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/release"));
    let studio = Studio::with_workspace(&runtime, workspace).with_toolchain(
        Toolchain::discover(&target),
        CompileSession::new(0x9E_1EC4).ok(),
    );
    studio.dark.set(!light);
    studio.note_window_size(window);

    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();

    let mut rec = Session::new(driver, studio, out, window, small, light);

    rec.section("boot");
    rec.shot("opened");

    // ---------------------------------------------------------------
    // Phase 1 — every command, fired for real, in palette order.
    // ---------------------------------------------------------------
    rec.section("commands");
    for command in Command::ALL {
        if SKIP.contains(&command) {
            rec.skip(
                command,
                "network, long-running, or needs hardware/toolchain",
            );
            continue;
        }
        rec.run_command(command);
        // Whatever the command opened (a menu, the palette, a dialog), leave
        // it closed for the next one — see `close_overlays`'s doc for why
        // this is a direct reset rather than an Escape keypress.
        rec.close_overlays();
    }

    // ---------------------------------------------------------------
    // Phase 2 — every sidebar view, every panel tab, every preview platform,
    // every right-pane tab: the four enums the activity bar, the bottom
    // panel, the device frame and the inspector column are each built from.
    // ---------------------------------------------------------------
    rec.section("views");
    for view in View::ALL {
        rec.studio.view.set(view);
        rec.shot(&format!("view-{view:?}"));
    }

    // A real scroll, wheel-shaped, over the one sidebar view long enough to
    // need it — the Toolchain paths are what the beta screenshot in
    // `walkthrough.rs` was filed against.
    rec.studio.view.set(View::Toolchain);
    // `_`, not `step`: the counter is never read — the clock is advanced by
    // `rec.clock += 16` inside the body, and the range only says how many
    // frames of scroll to feed. A named binding here is an unused variable,
    // which `-D warnings` in the clippy stage makes an error.
    for _ in 1..=40u64 {
        rec.clock += 16;
        let c = rec.clock;
        rec.driver.handle_scroll(&ScrollEvent::new(
            Offset::new(180.0, 400.0),
            Offset::new(0.0, -60.0),
            Duration::from_millis(c),
        ));
        rec.driver.draw_frame_at(Duration::from_millis(c));
    }
    rec.shot("toolchain-scrolled");

    rec.section("panel-tabs");
    rec.studio.panel_open.set(true);
    for tab in PanelTab::ALL {
        rec.studio.panel_tab.set(tab);
        rec.shot(&format!("panel-{tab:?}"));
    }

    rec.section("preview-platforms");
    for platform in Platform::ALL {
        rec.studio.set_platform(platform);
        rec.shot(&format!("platform-{platform:?}"));
    }
    rec.studio.set_platform(Platform::Desktop);

    rec.section("right-tabs");
    for tab in RightTab::ALL {
        rec.studio.right_tab.set(tab);
        rec.shot(&format!("right-{tab:?}"));
    }
    rec.studio.right_tab.set(RightTab::Preview);

    // ---------------------------------------------------------------
    // Phase 3 — every menu, opened by clicking the title bar, not by setting
    // the signal, so this is also the check that the title bar's hit regions
    // still line up with what it draws.
    // ---------------------------------------------------------------
    rec.section("menus");
    for menu in Menu::BAR {
        let label = format!("{menu:?}");
        if !rec.click(&label) {
            // Fall back to the direct signal, so one relabelled button does
            // not take the rest of the menu sweep down with it — but the
            // fallback itself is recorded as a warning, because a menu button
            // a click cannot find is exactly the kind of regression this
            // script exists to catch.
            rec.studio.menu_open.set(Some(menu));
        }
        rec.shot(&format!("menu-{menu:?}"));
        rec.close_overlays();
    }

    // ---------------------------------------------------------------
    // Phase 4 — a real session: open a project, touch files, type, select,
    // cut/copy/paste, find and replace by typing (not by setting the query
    // signal), drag every seam, right-click for a context menu, hover for
    // tooltips, and the two destructive dialogs.
    // ---------------------------------------------------------------
    rec.section("session");

    rec.click("Open Folder…");
    rec.shot("open-folder-picker");
    rec.studio.close_picker();
    rec.shot("explorer");

    for name in ["card_grid.rs", "landing_screen.rs", "settings_form.rs"] {
        rec.studio.open_path(screens().join(name));
    }
    rec.shot("many-tabs");

    // Hover each open tab — a mouse move with no button down, which is the
    // only thing that can trigger a tooltip. `find_all` rather than `find`,
    // because every open tab shares the same accessible role and the point is
    // to look at more than one.
    for tab in rec.find_all_with_role("Tab") {
        rec.hover_at(tab, "hover-tab");
    }

    // Right-click a file row in the Explorer for its context menu.
    rec.studio.view.set(View::Explorer);
    if let Some(row) = rec.find("card_grid.rs") {
        rec.right_click_at(row, "context-menu");
        rec.key(NamedKey::Escape, Modifiers::NONE);
    }

    // Type real text into the active buffer through the IME commit path,
    // then select a run of it by dragging, then cut/copy/paste it.
    rec.studio.open_path(screens().join("card_grid.rs"));
    rec.set_focus_active_editor();
    rec.type_text("\n// release_check was here\n");
    rec.shot("typed");
    if let Some((start, end)) = rec.editor_text_span() {
        rec.drag_raw(start, Offset::new(end.dx - start.dx, 0.0), "select-drag");
    }
    rec.run_command(Command::Copy);
    rec.run_command(Command::Cut);
    rec.run_command(Command::Paste);
    rec.shot("cut-copy-paste");
    rec.run_command(Command::Undo);
    rec.run_command(Command::Undo);

    // Find, typed rather than set, then Enter to run the search for real.
    rec.run_command(Command::Find);
    rec.type_text("Container");
    rec.key(NamedKey::Enter, Modifiers::NONE);
    rec.shot("find-typed");
    rec.run_command(Command::Replace);
    rec.key(NamedKey::Tab, Modifiers::NONE);
    rec.type_text("Card");
    rec.shot("replace-typed");
    rec.run_command(Command::CloseFind);

    // Drag every seam the shell has: sidebar, bottom panel, preview pane.
    if let Some(seam) = row_seam(&rec.driver, window) {
        rec.drag_raw(seam, Offset::new(0.0, -80.0), "drag-panel-taller");
    }
    if let Some(seam) = column_seam(&rec.driver, window) {
        rec.drag_raw(seam, Offset::new(220.0, 0.0), "drag-preview-narrower");
        rec.drag_raw(
            Offset::new(seam.dx + 220.0, seam.dy),
            Offset::new(-220.0, 0.0),
            "drag-preview-restored",
        );
    }
    if let Some(handle) = rec.find("Resize the sidebar") {
        rec.drag_raw(handle, Offset::new(120.0, 0.0), "drag-sidebar-wider");
    }

    // The command palette, then a typed query and a real arrow-key selection
    // — the palette itself is reached by every other route already (the
    // command sweep above fires `Command::CommandPalette` directly, which is
    // exactly what its shortcut and its title-bar button both do), so the new
    // ground this covers is typing into it and moving the selection.
    rec.studio.run(Command::CommandPalette);
    rec.type_text("save");
    rec.shot("palette-typed");
    rec.key(NamedKey::ArrowDown, Modifiers::NONE);
    rec.shot("palette-selected");
    rec.key(NamedKey::Escape, Modifiers::NONE);

    // The two destructive dialogs, cancelled — this is a review of what they
    // say, not a request to actually delete anything.
    if let Some(root_path) = rec.studio.root.peek().clone() {
        rec.studio.ask_to_delete(root_path.join("card_grid.rs"));
        rec.shot("delete-a-file");
        rec.studio.cancel_delete();

        rec.studio.ask_to_delete(root_path);
        rec.shot("delete-the-workspace");
        rec.studio.cancel_delete();
    }

    // Unsaved changes at quit.
    rec.run_command(Command::NewFile);
    rec.studio.request_close();
    rec.studio.may_close(None);
    rec.shot("unsaved-quit");
    rec.close_overlays();

    // ---------------------------------------------------------------
    // Phase 5 — the same handful of load-bearing screens, in the theme the
    // run did not start in, so both themes are represented even though the
    // run itself only started one.
    // ---------------------------------------------------------------
    rec.section("other-theme");
    rec.studio.dark.set(light);
    rec.studio.view.set(View::Explorer);
    rec.shot("explorer");
    rec.studio.view.set(View::Settings);
    rec.shot("settings");
    rec.studio.view.set(View::Toolchain);
    rec.shot("toolchain");
    rec.run_command(Command::About);
    rec.shot("about");
    rec.close_overlays();

    rec.finish();
}

// =====================================================================
// The recorder
// =====================================================================

/// Everything one run of the script accumulates: the driver, the studio, a
/// manifest of what happened, and the renderer used to turn a scene into a
/// PNG.
struct Session {
    driver: FrameDriver,
    studio: Studio,
    out: PathBuf,
    window: Size,
    prefix: String,
    clock: u64,
    shot_index: u32,
    cpu: vieww_paint::native::NativeRenderer,
    section: String,
    entries: Vec<String>,
    warnings: Vec<String>,
    frame_errors: u32,
}

impl Session {
    fn new(
        driver: FrameDriver,
        studio: Studio,
        out: PathBuf,
        window: Size,
        small: bool,
        light: bool,
    ) -> Self {
        let mut prefix = String::new();
        if small {
            prefix.push_str("small-");
        }
        if light {
            prefix.push_str("light-");
        }
        Self {
            driver,
            studio,
            out,
            window,
            prefix,
            clock: 10_000,
            shot_index: 0,
            cpu: vieww_paint::native::NativeRenderer::new(),
            section: "boot".to_owned(),
            entries: Vec::new(),
            warnings: Vec::new(),
            frame_errors: 0,
        }
    }

    fn section(&mut self, name: &str) {
        self.section = name.to_owned();
    }

    /// Settle a few frames, rasterise, write the PNG, and record the manifest
    /// line. Called after (almost) every interaction below.
    fn shot(&mut self, name: &str) {
        for _ in 0..4 {
            self.clock += 40;
            self.driver.draw_frame_at(Duration::from_millis(self.clock));
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a window size in logical pixels"
        )]
        let (width, height) = (self.window.width as u32, self.window.height as u32);
        let background = if self.studio.dark.peek() {
            viewwstudio::StudioTheme::dark().window
        } else {
            viewwstudio::StudioTheme::light().window
        };
        self.shot_index += 1;
        let file = format!(
            "{}{:03}-{}-{name}.png",
            self.prefix, self.shot_index, self.section
        );
        let path = self.out.join(&file);
        let (png, report) = self
            .cpu
            .render_to_png(self.driver.scene(), width, height, background)
            .expect("rasterising");
        std::fs::write(&path, png).expect("writing the PNG");

        let errors = self.driver.take_frame_errors();
        let blank = report.shapes == 0;
        if blank {
            self.warn(&format!("{file}: zero shapes — likely a blank frame"));
        }
        if !errors.is_empty() {
            self.frame_errors += errors.len() as u32;
            for error in &errors {
                self.warn(&format!("{file}: frame error: {error:?}"));
            }
        }
        self.entries.push(json_line(
            &self.section,
            name,
            &file,
            report.shapes,
            errors.len(),
        ));
    }

    fn warn(&mut self, message: &str) {
        eprintln!("release_check: WARNING: {message}");
        self.warnings.push(message.to_owned());
    }

    fn skip(&mut self, command: Command, why: &str) {
        eprintln!("release_check: skipping {command:?} ({why})");
        self.entries.push(format!(
            "    {{\"section\": \"commands\", \"skipped\": \"{command:?}\", \"why\": {why:?}}}"
        ));
    }

    fn run_command(&mut self, command: Command) {
        self.studio.run(command);
        self.shot(&format!("{command:?}"));
    }

    /// Set every overlay signal this script opens back to closed, so the next
    /// step starts clean. A direct reset rather than an Escape keypress,
    /// because Escape is only wired to `CloseFind` in the global chord table
    /// (see `command.rs`'s `no_two_commands_share_a_chord` test) — every
    /// other overlay closes itself locally, and this sweep needs one
    /// mechanism that reliably closes all of them between eighty-odd steps.
    fn close_overlays(&mut self) {
        self.studio.palette_open.set(false);
        self.studio.menu_open.set(None);
        self.studio.welcome.set(false);
        self.studio.cancel_delete();
        self.studio.close_context_menu();
        self.studio.close_picker();
    }

    // -- finding things by their accessible label, the way a screen reader
    //    (and `ci/mobile/device-suite.sh`) would ------------------------------------

    fn find(&self, label_contains: &str) -> Option<Offset> {
        let needle = label_contains.to_lowercase();
        self.driver
            .semantics()
            .nodes()
            .iter()
            .find(|n| {
                n.label
                    .as_deref()
                    .is_some_and(|l| l.to_lowercase().contains(&needle))
            })
            .map(|n| {
                let r = n.bounds;
                Offset::new(
                    r.origin().dx + r.size().width / 2.0,
                    r.origin().dy + r.size().height / 2.0,
                )
            })
    }

    fn find_all_with_role(&self, role_debug_contains: &str) -> Vec<Offset> {
        self.driver
            .semantics()
            .nodes()
            .iter()
            .filter(|n| format!("{:?}", n.role).contains(role_debug_contains))
            .map(|n| {
                let r = n.bounds;
                Offset::new(
                    r.origin().dx + r.size().width / 2.0,
                    r.origin().dy + r.size().height / 2.0,
                )
            })
            .collect()
    }

    // -- real interactions ----------------------------------------------------

    fn tap_at(&mut self, at: Offset) {
        self.clock += 200;
        let c = self.clock;
        self.driver.handle_pointer(
            &PointerEvent::down(PointerId(1), at, Duration::from_millis(c))
                .with_kind(PointerDeviceKind::Mouse),
        );
        self.driver.draw_frame_at(Duration::from_millis(c));
        self.driver.draw_frame_at(Duration::from_millis(c + 90));
        self.driver.handle_pointer(
            &PointerEvent::up(PointerId(1), at, Duration::from_millis(c + 100))
                .with_kind(PointerDeviceKind::Mouse),
        );
        self.driver.draw_frame_at(Duration::from_millis(c + 110));
    }

    /// Click something found by label. `false`, and a warning, if nothing
    /// with that label exists right now — never a panic, because a missing
    /// label partway through an 80-step sweep should not take the rest of the
    /// sweep down with it; it should be the thing this script exists to flag.
    fn click(&mut self, label: &str) -> bool {
        match self.find(label) {
            Some(at) => {
                self.tap_at(at);
                true
            }
            None => {
                self.warn(&format!(
                    "click: no element labelled like {label:?} was found"
                ));
                false
            }
        }
    }

    fn right_click_at(&mut self, at: Offset, name: &str) {
        self.clock += 200;
        let c = self.clock;
        self.driver.handle_pointer(
            &PointerEvent::down(PointerId(1), at, Duration::from_millis(c))
                .with_kind(PointerDeviceKind::Mouse)
                .with_button(PointerButton::Secondary),
        );
        self.driver.draw_frame_at(Duration::from_millis(c + 40));
        self.driver.handle_pointer(
            &PointerEvent::up(PointerId(1), at, Duration::from_millis(c + 60))
                .with_kind(PointerDeviceKind::Mouse)
                .with_button(PointerButton::Secondary),
        );
        self.shot(name);
    }

    /// A hover the way a mouse produces one: move to the point with no
    /// button down, let the tooltip's own delay elapse, photograph it, then
    /// move the pointer away so the next hover starts clean.
    fn hover_at(&mut self, at: Offset, name: &str) {
        self.driver.handle_hover(Some(at));
        for step in 1..=10 {
            self.clock += 60;
            self.driver
                .draw_frame_at(Duration::from_millis(self.clock + step));
        }
        self.shot(name);
        self.driver.handle_hover(None);
    }

    fn drag_raw(&mut self, from: Offset, by: Offset, name: &str) {
        self.clock += 300;
        let c = self.clock;
        self.driver.handle_pointer(
            &PointerEvent::down(PointerId(1), from, Duration::from_millis(c))
                .with_kind(PointerDeviceKind::Mouse),
        );
        self.driver.draw_frame_at(Duration::from_millis(c + 20));
        let mut previous = from;
        for step in 1u8..=8 {
            let to = Offset::new(
                from.dx + by.dx * f32::from(step) / 8.0,
                from.dy + by.dy * f32::from(step) / 8.0,
            );
            self.driver.handle_pointer(
                &PointerEvent::moved(
                    PointerId(1),
                    previous,
                    to,
                    Duration::from_millis(c + 20 + u64::from(step)),
                )
                .with_kind(PointerDeviceKind::Mouse),
            );
            self.driver
                .draw_frame_at(Duration::from_millis(c + 20 + u64::from(step)));
            previous = to;
        }
        self.driver.handle_pointer(
            &PointerEvent::up(PointerId(1), previous, Duration::from_millis(c + 40))
                .with_kind(PointerDeviceKind::Mouse),
        );
        self.shot(name);
    }

    fn key(&mut self, key: NamedKey, modifiers: Modifiers) {
        self.clock += 40;
        self.driver.handle_key(
            &KeyEvent::named(key, Duration::from_millis(self.clock)).with_modifiers(modifiers),
        );
    }

    /// Real typed text, through the same IME-commit path a platform bridge
    /// uses for an ordinary keystroke — not `Studio::edit`, which is what
    /// `tour` and `walkthrough` use to seed content instantly.
    fn type_text(&mut self, text: &str) {
        self.driver.handle_ime(&ImeEvent::commit(text));
        self.clock += 40;
        self.driver.draw_frame_at(Duration::from_millis(self.clock));
    }

    fn set_focus_active_editor(&mut self) {
        if let Some(at) = self.find_all_with_role("TextField").first().copied() {
            self.tap_at(at);
        }
    }

    /// A rough span across a line of the active buffer's first visible row,
    /// good enough to demonstrate a drag-selection without depending on exact
    /// glyph metrics — the editor's own hit-testing resolves it to real
    /// caret positions either way.
    fn editor_text_span(&self) -> Option<(Offset, Offset)> {
        let region = self.find_all_with_role("TextField").first().copied()?;
        Some((
            Offset::new(region.dx - 80.0, region.dy),
            Offset::new(region.dx + 80.0, region.dy),
        ))
    }

    fn finish(self) {
        let mut body = String::from("{\n  \"steps\": [\n");
        body.push_str(&self.entries.join(",\n"));
        body.push_str(&format!(
            "\n  ],\n  \"warnings\": {},\n  \"frame_errors\": {}\n}}\n",
            json_array(&self.warnings),
            self.frame_errors
        ));
        let manifest_name = format!("{}manifest.json", self.prefix);
        std::fs::write(self.out.join(&manifest_name), body).expect("writing the manifest");

        println!(
            "release_check: {} screenshots, {} warnings, {} frame errors — {}",
            self.shot_index,
            self.warnings.len(),
            self.frame_errors,
            self.out.join(&manifest_name).display()
        );
        if self.frame_errors > 0 {
            std::process::exit(1);
        }
    }
}

fn json_line(section: &str, name: &str, file: &str, shapes: usize, errors: usize) -> String {
    format!(
        "    {{\"section\": {section:?}, \"name\": {name:?}, \"file\": {file:?}, \"shapes\": {shapes}, \"frame_errors\": {errors}}}"
    )
}

fn json_array(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".to_owned();
    }
    let mut out = String::from("[\n");
    for (i, item) in items.iter().enumerate() {
        out.push_str("    ");
        out.push_str(&format!("{item:?}"));
        if i + 1 < items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]");
    out
}

// =====================================================================
// Seam-finding, lifted from `walkthrough.rs` — the seam is the only place
// down a row or column that promises a resize cursor, so this locates it
// without either pane's width or height being known here.
// =====================================================================

fn row_seam(driver: &FrameDriver, window: Size) -> Option<Offset> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a window height"
    )]
    let height = window.height as u32;
    (0..height)
        .find(|y| {
            driver.cursor_at(Offset::new(window.width / 2.0, *y as f32))
                == vieww_foundation::Cursor::ResizeRow
        })
        .map(|y| Offset::new(window.width / 2.0, y as f32 + 3.0))
}

fn column_seam(driver: &FrameDriver, window: Size) -> Option<Offset> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a window width"
    )]
    let width = window.width as u32;
    (0..width)
        .rev()
        .find(|x| {
            driver.cursor_at(Offset::new(*x as f32, window.height / 2.0))
                == vieww_foundation::Cursor::ResizeColumn
        })
        .map(|x| Offset::new(x as f32 - 3.0, window.height / 2.0))
}
