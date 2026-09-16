//! A first-run walkthrough, in order, with the pictures a reviewer needs.
//!
//! ```console
//! cargo run -p viewwstudio --example walkthrough -- out/ [--small] [--light]
//! ```
//!
//! # How this differs from `tour`
//!
//! `tour` photographs *states*: it resets the shell between pictures and pokes
//! one signal each time, which is the right shape for "does every view still
//! draw". This is the other question — **does the studio hold together when one
//! person uses it for twenty minutes** — so nothing is reset. The session runs
//! forwards: scaffold a project, open a screen, render it, break it, read the
//! error, fix it, run the live demo, take a lesson, resize the panes, open the
//! settings. Each picture is the studio as the previous step left it.
//!
//! State that leaks between steps is the point rather than a nuisance: a stale
//! preview badge that never clears, a panel that does not come back, a dialog
//! that leaves the tree changed — those only appear in a session, and a review
//! made of independent screenshots cannot see them.

use std::path::PathBuf;
use std::time::Duration;

use vieww_foundation::{Offset, PointerEvent, PointerId, Size};
use vieww_render::FrameDriver;
use viewwstudio::compile::{Session, Toolchain};
use viewwstudio::state::{PanelTab, Platform, RightTab};
use viewwstudio::{Shell, Studio, View, Workspace, WINDOW};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .map_or_else(|| PathBuf::from("walkthrough"), PathBuf::from);
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

    let workspace = Workspace::open(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/screens"
    )));
    let target =
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug"));
    let studio = Studio::with_workspace(&runtime, workspace)
        .with_toolchain(Toolchain::discover(&target), Session::new(0x5A_1AB0).ok());
    studio.dark.set(!light);
    studio.note_window_size(window);

    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();

    let mut shots = Shots::new(out, window, small, light);

    // 1. The window as it opens.
    shots.take(&mut driver, &studio, "01-opened");

    // 2. What a new user is shown first.
    studio.welcome.set(true);
    shots.take(&mut driver, &studio, "02-welcome");
    studio.welcome.set(false);

    // 3. A screen, rendered — the studio's whole promise in one click.
    studio.render();
    settle(&studio);
    shots.take(&mut driver, &studio, "03-rendered");

    // 4. Break it. This is the state everybody reaches by accident.
    {
        let mut value = studio.active().expect("a buffer").value;
        value
            .text
            .push_str("\nfn broken() -> u32 { \"not a number\" }\n");
        studio.edit(value);
        studio.render();
        settle(&studio);
        studio.panel_tab.set(PanelTab::Problems);
    }
    shots.take(&mut driver, &studio, "04-error-and-stale-frame");

    // 5. Undo the damage and render again: does the badge clear?
    studio.run(viewwstudio::command::Command::Undo);
    studio.render();
    settle(&studio);
    shots.take(&mut driver, &studio, "05-fixed-again");

    // 6. The bottom panel's new seam, dragged taller.
    if let Some(seam) = row_seam(&driver, window) {
        drag(&mut driver, seam, Offset::new(0.0, -70.0), 2000);
    }
    shots.take(&mut driver, &studio, "06-panel-taller");

    // 7. And the panel collapsed from its tab strip — the "hider".
    studio.panel_open.set(false);
    shots.take(&mut driver, &studio, "07-panel-collapsed");
    studio.panel_open.set(true);

    // 8. The preview note dismissed, which is a new control.
    studio.preview_note.set(false);
    shots.take(&mut driver, &studio, "08-note-dismissed");

    // 8b. The preview seam dragged to its floor: the editor takes the window
    // and the pane's toolbar reflows rather than letting the Render button
    // slide under the clip. This is the state a screenshot of the beta was
    // filed against — "the render pane is not minimizing, the render button
    // is going behind the window" — so the walkthrough keeps a picture of the
    // fixed version of exactly that.
    if let Some(seam) = column_seam(&driver, window) {
        drag(&mut driver, seam, Offset::new(260.0, 0.0), 2600);
    }
    shots.take(&mut driver, &studio, "08b-preview-narrow");
    // And back, so the rest of the session has a pane worth looking at.
    // -160 rather than -260: the pane started at 460 and the floor is 300, so
    // this returns it to where the session had it.
    if let Some(seam) = column_seam(&driver, window) {
        drag(&mut driver, seam, Offset::new(-160.0, 0.0), 2800);
    }

    // 9. The live preview, from the top: this workspace has no live.rs, so the
    // first press says so and offers to write one.
    studio.run(viewwstudio::command::Command::LivePreview);
    shots.take(&mut driver, &studio, "09a-live-missing");
    studio.live_missing.set(false);
    studio.create_live_file();
    shots.take(&mut driver, &studio, "09b-live-file-created");

    studio.run(viewwstudio::command::Command::LivePreview);
    shots.take(&mut driver, &studio, "09c-live-caution");
    studio.accept_live_preview();
    shots.take(&mut driver, &studio, "10-live-home");

    // 9d. Typing in live.rs changes the frame with no save and no build.
    if let Some(buffer) = studio.active() {
        let mut value = buffer.value;
        value.text = value
            .text
            .replace("title \"Inbox\"", "title \"My own inbox\"");
        studio.edit(value);
    }
    shots.take(&mut driver, &studio, "09e-live-edited");

    // 10. A tap on the demo's second row, which is what makes it a demo.
    if let Some(point) = first_tappable(&mut driver, &studio) {
        tap(&mut driver, point, 5000);
    }
    shots.take(&mut driver, &studio, "11-live-detail");
    studio.exit_live_preview();

    // 11. A lesson, loaded and rendered — the Learn view end to end.
    studio.view.set(View::Learn);
    studio.load_lesson(viewwstudio::lessons::lesson(6));
    studio.render();
    settle(&studio);
    shots.take(&mut driver, &studio, "12-lesson-rendered");

    // 12. The settings a person has to fill in before Android will build.
    studio.view.set(View::Toolchain);
    shots.take(&mut driver, &studio, "13-toolchain-top");
    for step in 1..=40u64 {
        driver.handle_scroll(&vieww_foundation::ScrollEvent::new(
            Offset::new(180.0, 400.0),
            Offset::new(0.0, -60.0),
            Duration::from_millis(6000 + step * 16),
        ));
        driver.draw_frame_at(Duration::from_millis(6000 + step * 16));
    }
    shots.take(&mut driver, &studio, "14-toolchain-paths");

    // 13. The narrow sidebar, which is where the cramping was reported.
    studio.sidebar_width.set(viewwstudio::state::MIN_SIDEBAR);
    shots.take(&mut driver, &studio, "15-toolchain-narrow");
    studio.sidebar_width.set(248.0);

    // 14. Export, and the devices tab that a phone build needs.
    studio.view.set(View::Export);
    shots.take(&mut driver, &studio, "16-export");
    studio.right_tab.set(RightTab::Devices);
    shots.take(&mut driver, &studio, "17-devices");
    studio.right_tab.set(RightTab::Preview);

    // 15. The other platforms the preview claims to simulate.
    studio.set_platform(Platform::Android);
    shots.take(&mut driver, &studio, "18-android");
    studio.set_platform(Platform::Ios);

    // 16. Settings, where a beta user goes when something looks wrong.
    studio.view.set(View::Settings);
    shots.take(&mut driver, &studio, "19-settings");

    // 17. The two destructive dialogs, in the order they bite.
    studio.view.set(View::Explorer);
    if let Some(root) = studio.root.peek().clone() {
        studio.ask_to_delete(root.join("card_grid.rs"));
        shots.take(&mut driver, &studio, "20-delete-a-file");
        studio.cancel_delete();

        studio.ask_to_delete(root);
        shots.take(&mut driver, &studio, "21-delete-the-workspace");
        studio.cancel_delete();
    }

    // 18. The command palette, which is how everything else is reached.
    studio.run(viewwstudio::command::Command::CommandPalette);
    shots.take(&mut driver, &studio, "22-palette");
    studio.palette_open.set(false);

    // 19. Light, at the end, so the whole session is seen in both.
    studio.dark.set(light);
    shots.take(&mut driver, &studio, "23-other-theme");
}

/// Where the panel's seam is, if it is showing.
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

/// Where the preview pane's seam is, if it is showing.
///
/// The same find-by-cursor trick as [`row_seam`]: the seam is the only place
/// down this column that promises a column resize, so the promise locates it
/// without either pane's width being known here.
fn column_seam(driver: &FrameDriver, window: Size) -> Option<Offset> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a window width"
    )]
    let width = window.width as u32;
    // The *rightmost* promise: the sidebar's seam is also a ResizeColumn, and
    // scanning from the left would grab that one. The preview's is the last
    // seam before the window's edge.
    (0..width)
        .rev()
        .find(|x| {
            driver.cursor_at(Offset::new(*x as f32, window.height / 2.0))
                == vieww_foundation::Cursor::ResizeColumn
        })
        .map(|x| Offset::new(x as f32 - 3.0, window.height / 2.0))
}

/// A point inside the live demo that changes the route, found rather than
/// guessed — the device frame moves with the window and the panes.
fn first_tappable(driver: &mut FrameDriver, studio: &Studio) -> Option<Offset> {
    let before = studio.live_state.route.get();
    let mut clock = 4000;
    for y in (140..600).step_by(6) {
        for x in (1000..1400).step_by(10) {
            clock += 300;
            tap(driver, Offset::new(x as f32, y as f32), clock);
            if studio.live_state.route.get() != before {
                studio.live_state.route.set(before);
                driver.draw_frame_at(Duration::from_millis(clock + 200));
                return Some(Offset::new(x as f32, y as f32));
            }
        }
    }
    None
}

fn tap(driver: &mut FrameDriver, at: Offset, clock: u64) {
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        at,
        Duration::from_millis(clock),
    ));
    driver.draw_frame_at(Duration::from_millis(clock));
    driver.draw_frame_at(Duration::from_millis(clock + 110));
    driver.handle_pointer(&PointerEvent::up(
        PointerId(1),
        at,
        Duration::from_millis(clock + 120),
    ));
    driver.draw_frame_at(Duration::from_millis(clock + 130));
}

fn drag(driver: &mut FrameDriver, from: Offset, by: Offset, clock: u64) {
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        from,
        Duration::from_millis(clock),
    ));
    driver.draw_frame_at(Duration::from_millis(clock));
    driver.draw_frame_at(Duration::from_millis(clock + 110));
    let mut previous = from;
    for step in 1u8..=8 {
        let to = Offset::new(
            from.dx + by.dx * f32::from(step) / 8.0,
            from.dy + by.dy * f32::from(step) / 8.0,
        );
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            to,
            Duration::from_millis(clock + 110 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(clock + 110 + u64::from(step)));
        previous = to;
    }
    driver.handle_pointer(&PointerEvent::up(
        PointerId(1),
        previous,
        Duration::from_millis(clock + 140),
    ));
    driver.draw_frame_at(Duration::from_millis(clock + 140));
}

fn settle(studio: &Studio) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while studio.is_compiling() && std::time::Instant::now() < deadline {
        if !studio.poll_compile() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

/// Writes the PNGs, and keeps the frame clock moving so animations settle.
struct Shots {
    out: PathBuf,
    window: Size,
    prefix: String,
    clock: u64,
    cpu: vieww_paint::native::NativeRenderer,
}

impl Shots {
    fn new(out: PathBuf, window: Size, small: bool, light: bool) -> Self {
        let mut prefix = String::new();
        if small {
            prefix.push_str("small-");
        }
        if light {
            prefix.push_str("light-");
        }
        Self {
            out,
            window,
            prefix,
            clock: 10_000,
            cpu: vieww_paint::native::NativeRenderer::new(),
        }
    }

    fn take(&mut self, driver: &mut FrameDriver, studio: &Studio, name: &str) {
        // Several frames at advancing times: a wash, a caret and a spinner all
        // move over their own durations, and a picture taken at one instant
        // catches them mid-flight.
        for _ in 0..4 {
            self.clock += 40;
            driver.draw_frame_at(Duration::from_millis(self.clock));
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a window size in logical pixels"
        )]
        let (width, height) = (self.window.width as u32, self.window.height as u32);
        let background = if studio.dark.peek() {
            viewwstudio::StudioTheme::dark().window
        } else {
            viewwstudio::StudioTheme::light().window
        };
        let path = self.out.join(format!("{}{name}.png", self.prefix));
        let (png, report) = self
            .cpu
            .render_to_png(driver.scene(), width, height, background)
            .expect("rasterising");
        std::fs::write(&path, png).expect("writing the PNG");
        println!("{} — {} shapes", path.display(), report.shapes);
    }
}
