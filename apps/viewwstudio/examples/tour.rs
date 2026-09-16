//! A user-perspective tour of the shell, rasterised headless.
//!
//! One process, many pictures: the studio is mounted once and driven through
//! the states a person building an app actually passes through, writing a PNG
//! after each. Written for a pre-release review rather than for CI.
//!
//! ```console
//! cargo run -p viewwstudio --example tour -- out/ [--small] [--light] [--only=name]
//! ```

use std::path::PathBuf;

use vieww_foundation::Size;
use vieww_render::FrameDriver;
use viewwstudio::command::{Command, Menu};
use viewwstudio::compile::{Session, Toolchain};
use viewwstudio::state::{PanelTab, Platform, RightTab};
use viewwstudio::{Shell, Studio, View, Workspace, WINDOW};

fn screens() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/screens"))
}

/// One picture in the tour: the file name it is written to, and what to do to
/// the studio before taking it.
///
/// A named pair rather than the tuple spelled out, because the spelled-out form
/// — `(&str, Box<dyn Fn(&Studio)>)` — appears in the vector's type, in the loop
/// that walks it, and in every reader's head at once. `Scenario` says what the
/// two halves are for.
type Scenario = (&'static str, Box<dyn Fn(&Studio)>);

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .map_or_else(|| PathBuf::from("tour"), PathBuf::from);
    let rest: Vec<String> = args.collect();
    let small = rest.iter().any(|a| a == "--small");
    let light = rest.iter().any(|a| a == "--light");
    let render = rest.iter().any(|a| a == "--render");
    let only: Option<&str> = rest.iter().find_map(|a| a.strip_prefix("--only="));
    let prefix = rest
        .iter()
        .find_map(|a| a.strip_prefix("--prefix="))
        .unwrap_or("");

    std::fs::create_dir_all(&out).expect("creating the output directory");

    let window = if small {
        Size::new(1366.0, 679.0)
    } else {
        WINDOW
    };

    let mut driver = FrameDriver::new(window);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();

    // `--workspace=/path/to/project` points the tour at a real application
    // instead of the three bundled screens. This is what turns the tour from a
    // picture of the studio into a picture of somebody *using* it on their own
    // code, which is the only test that finds the conventions the studio takes
    // for granted.
    let root = rest
        .iter()
        .find_map(|a| a.strip_prefix("--workspace="))
        .map_or_else(screens, PathBuf::from);
    let workspace = Workspace::open(&root);
    let target = std::env::var_os(viewwstudio::install::TARGET_DIR_ENV).map_or_else(
        || PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug")),
        PathBuf::from,
    );
    let studio = Studio::with_workspace(&runtime, workspace)
        .with_toolchain(Toolchain::discover(&target), Session::new(0x5C_2EE0).ok());
    studio.dark.set(!light);
    studio.note_window_size(window);

    if render {
        studio.render();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
        while studio.is_compiling() && std::time::Instant::now() < deadline {
            if !studio.poll_compile() {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }

    driver.set_root(Shell {
        studio: studio.clone(),
    });

    let mut cpu = vieww_paint::native::NativeRenderer::new();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "window size"
    )]
    let (width, height) = (window.width as u32, window.height as u32);

    // `--open=src/screens/library.rs`, relative to the workspace root.
    if let Some(name) = rest.iter().find_map(|a| a.strip_prefix("--open=")) {
        studio.open_path(root.join(name));
    }

    let scenarios: Vec<Scenario> = vec![
        (
            "00-render",
            Box::new(|s: &Studio| {
                s.render();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
                while s.is_compiling() && std::time::Instant::now() < deadline {
                    if !s.poll_compile() {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
            }),
        ),
        ("01-welcome", Box::new(|s: &Studio| s.welcome.set(true))),
        (
            "02-open-folder-picker",
            Box::new(|s: &Studio| s.open_picker()),
        ),
        ("03-explorer", Box::new(|_: &Studio| {})),
        (
            "04-command-palette",
            Box::new(|s: &Studio| {
                s.palette_open.set(true);
                s.palette_query.set(String::new());
            }),
        ),
        (
            "05-palette-search",
            Box::new(|s: &Studio| {
                s.palette_open.set(true);
                s.palette_query.set("build".to_owned());
            }),
        ),
        (
            "06-go-to-file",
            Box::new(|s: &Studio| {
                s.palette_open.set(true);
                s.palette_query.set("card".to_owned());
            }),
        ),
        (
            "07-menu-file",
            Box::new(|s: &Studio| s.menu_open.set(Some(Menu::File))),
        ),
        (
            "08-menu-edit",
            Box::new(|s: &Studio| s.menu_open.set(Some(Menu::Edit))),
        ),
        // **Go was missing from this list**, which is how a menu the title bar
        // has shown since it was split out of View went un-photographed
        // through a pre-release review. The tour's own claim is "every state a
        // person passes through"; a menu with no picture is a menu nobody
        // checked. `Menu::BAR` is the list to walk, and every entry in it is
        // now here.
        (
            "08-menu-go",
            Box::new(|s: &Studio| s.menu_open.set(Some(Menu::Go))),
        ),
        (
            "09-menu-view",
            Box::new(|s: &Studio| s.menu_open.set(Some(Menu::View))),
        ),
        (
            "10-menu-render",
            Box::new(|s: &Studio| s.menu_open.set(Some(Menu::Render))),
        ),
        (
            "11-menu-build",
            Box::new(|s: &Studio| s.menu_open.set(Some(Menu::Build))),
        ),
        (
            "12-menu-help",
            Box::new(|s: &Studio| s.menu_open.set(Some(Menu::Help))),
        ),
        (
            "13-find",
            Box::new(|s: &Studio| {
                s.open_find(false);
                s.find_query.set("Container".to_owned());
            }),
        ),
        (
            "14-replace",
            Box::new(|s: &Studio| {
                s.open_find(true);
                s.find_query.set("Container".to_owned());
                s.find_replacement.set("Card".to_owned());
            }),
        ),
        (
            "15-view-search",
            Box::new(|s: &Studio| {
                s.view.set(View::Search);
                s.search_query.set("Text".to_owned());
            }),
        ),
        (
            "16-view-snippets",
            Box::new(|s: &Studio| s.view.set(View::Snippets)),
        ),
        (
            "17-view-problems",
            Box::new(|s: &Studio| s.view.set(View::Problems)),
        ),
        // The twelfth sidebar view, and the only one this tour never opened.
        // It is the one a first-time user is most likely to press, which makes
        // its absence from a pre-release review the worst of the twelve to
        // have.
        (
            "17-view-learn",
            Box::new(|s: &Studio| s.view.set(View::Learn)),
        ),
        (
            "18-view-inspector",
            Box::new(|s: &Studio| s.view.set(View::Inspector)),
        ),
        (
            "18-view-docs",
            Box::new(|s: &Studio| {
                s.view.set(View::Docs);
                s.docs_page.set(2);
            }),
        ),
        (
            "19-view-toolchain",
            Box::new(|s: &Studio| s.view.set(View::Toolchain)),
        ),
        // The half of the Toolchain view that is below the fold on a laptop:
        // the install buttons for what is missing, and the paths and signing
        // fields under them. Scrolled rather than resized, because the point of
        // the picture is that they are reachable at the size people run at.
        (
            "19-toolchain-paths",
            Box::new(|s: &Studio| {
                s.view.set(View::Toolchain);
                // Primed before the jump, because a `jump_to` on a controller that
                // has not been laid out yet is clamped to zero — the extents are
                // whatever the *previous* scenario's view measured. The first real
                // layout replaces these numbers and re-clamps, which is exactly
                // what should happen if the view turns out to be shorter.
                s.sidebar_scroll(View::Toolchain)
                    .resize(vieww_widget::ScrollExtents::new(560.0, 2600.0));
                s.sidebar_scroll(View::Toolchain).jump_to(1450.0);
            }),
        ),
        (
            "20-view-source",
            Box::new(|s: &Studio| s.view.set(View::Source)),
        ),
        (
            "21-view-export",
            Box::new(|s: &Studio| s.view.set(View::Export)),
        ),
        (
            "22-view-tokens",
            Box::new(|s: &Studio| s.view.set(View::Tokens)),
        ),
        (
            "23-view-settings",
            Box::new(|s: &Studio| s.view.set(View::Settings)),
        ),
        (
            "24-panel-output",
            Box::new(|s: &Studio| {
                s.panel_open.set(true);
                s.panel_tab.set(PanelTab::Output);
            }),
        ),
        (
            "25-panel-run",
            Box::new(|s: &Studio| {
                s.panel_open.set(true);
                s.panel_tab.set(PanelTab::Run);
            }),
        ),
        (
            "26-panel-tasks",
            Box::new(|s: &Studio| {
                s.panel_open.set(true);
                s.panel_tab.set(PanelTab::Tasks);
            }),
        ),
        // `PanelTab::ALL` has six entries and this tour photographed four. The
        // two it skipped are the two a user reaches only by clicking a tab
        // they have never clicked, which is exactly the pair most likely to be
        // wrong and least likely to be noticed.
        (
            "26-panel-rustc",
            Box::new(|s: &Studio| {
                s.panel_open.set(true);
                s.panel_tab.set(PanelTab::Rustc);
            }),
        ),
        (
            "26-panel-timings",
            Box::new(|s: &Studio| {
                s.panel_open.set(true);
                s.panel_tab.set(PanelTab::Timings);
            }),
        ),
        (
            "27-preview-android",
            Box::new(|s: &Studio| s.set_platform(Platform::Android)),
        ),
        (
            "28-preview-desktop",
            Box::new(|s: &Studio| s.set_platform(Platform::Desktop)),
        ),
        (
            "29-preview-landscape",
            Box::new(|s: &Studio| {
                s.set_platform(Platform::Ios);
                s.toggle_landscape();
            }),
        ),
        (
            "30-right-inspector",
            Box::new(|s: &Studio| s.right_tab.set(RightTab::Inspector)),
        ),
        ("31-zen", Box::new(|s: &Studio| s.run(Command::ZenMode))),
        ("32-about", Box::new(|s: &Studio| s.run(Command::About))),
        (
            "33-shortcuts",
            Box::new(|s: &Studio| s.run(Command::ShowShortcuts)),
        ),
        (
            "34-many-tabs",
            Box::new(|s: &Studio| {
                for name in ["card_grid.rs", "landing_screen.rs", "settings_form.rs"] {
                    s.open_path(screens().join(name));
                }
                for _ in 0..6 {
                    s.run(Command::NewFile);
                }
            }),
        ),
        (
            "35-unsaved-quit",
            Box::new(|s: &Studio| {
                s.run(Command::NewFile);
                s.request_close();
                s.may_close(None);
            }),
        ),
        (
            "36-new-file-prompt",
            Box::new(|s: &Studio| {
                s.ask_for_name(viewwstudio::state::NameKind::NewFile, &screens());
            }),
        ),
        (
            "37-context-menu",
            Box::new(|s: &Studio| {
                s.open_context_menu(
                    vieww_foundation::Offset::new(180.0, 260.0),
                    screens().join("card_grid.rs"),
                );
            }),
        ),
        ("38-light-theme", Box::new(|s: &Studio| s.dark.set(false))),
        (
            "39-toolchain-light",
            Box::new(|s: &Studio| {
                s.dark.set(false);
                s.view.set(View::Toolchain);
            }),
        ),
        ("40-new-project", Box::new(|s: &Studio| s.new_project())),
        (
            "42-palette-commands",
            Box::new(|s: &Studio| {
                s.palette_open.set(true);
                s.palette_query.set(">".to_owned());
            }),
        ),
        (
            "43-palette-commands-scrolled",
            Box::new(|s: &Studio| {
                s.palette_open.set(true);
                s.palette_query.set(">".to_owned());
                for _ in 0..20 {
                    s.palette_step(1);
                }
            }),
        ),
        (
            "44-palette-symbols",
            Box::new(|s: &Studio| {
                s.palette_open.set(true);
                s.palette_query.set("@".to_owned());
            }),
        ),
        (
            "45-build-error",
            Box::new(|s: &Studio| {
                let mut value = s.active().expect("a buffer").value;
                value
                    .text
                    .push_str("\nfn broken() -> u32 { \"not a number\" }\n");
                s.edit(value);
                s.render();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
                while s.is_compiling() && std::time::Instant::now() < deadline {
                    if !s.poll_compile() {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
                s.panel_tab.set(PanelTab::Problems);
            }),
        ),
        (
            "46-completion",
            Box::new(|s: &Studio| {
                s.request_completion();
            }),
        ),
        // Everything closed. Reachable since the last tab stopped refusing to
        // close, and the one state where the editor has nothing to show — so
        // the picture is the check that it says so rather than going blank.
        // Save on a buffer that has never been on disk: the prompt that used to
        // be a refusal. The reported bug in one picture.
        (
            "55-save-as",
            Box::new(|s: &Studio| {
                s.run(Command::NewFile);
                let mut value = s.active().expect("a buffer").value;
                value.text = "pub fn screen() -> impl Widget { Text::new(\"hello\") }\n".to_owned();
                s.edit(value);
                s.run(Command::Save);
            }),
        ),
        (
            "54-no-file-open",
            Box::new(|s: &Studio| {
                while !s.buffers.get().is_empty() {
                    s.run(Command::CloseTab);
                }
            }),
        ),
        (
            "51-preview-hidden",
            Box::new(|s: &Studio| s.right_open.set(false)),
        ),
        (
            "52-panel-collapsed",
            Box::new(|s: &Studio| s.panel_open.set(false)),
        ),
        (
            "53-both-hidden",
            Box::new(|s: &Studio| {
                s.right_open.set(false);
                s.panel_open.set(false);
            }),
        ),
        (
            "49-dirty-quit",
            Box::new(|s: &Studio| {
                let mut value = s.active().expect("a buffer").value;
                value.text.push_str("\n// unsaved\n");
                s.edit(value);
                s.request_close();
                s.may_close(None);
            }),
        ),
        (
            "50-dirty-tab",
            Box::new(|s: &Studio| {
                let mut value = s.active().expect("a buffer").value;
                value.text.push_str("\n// unsaved\n");
                s.edit(value);
            }),
        ),
        (
            "47-sidebar-narrow",
            Box::new(|s: &Studio| {
                s.sidebar_width.set(viewwstudio::state::MIN_SIDEBAR);
            }),
        ),
        (
            "48-devices",
            Box::new(|s: &Studio| {
                s.right_tab.set(RightTab::Devices);
            }),
        ),
        (
            "41-no-folder",
            Box::new(|s: &Studio| {
                s.root.set(None);
                s.menu_open.set(Some(Menu::Build));
            }),
        ),
        // **The Live Preview, which this tour never photographed at all.**
        //
        // It is the studio's second preview pipeline and the source of its
        // most-reported confusion — a new project's `live.rs` sketches three
        // screens while the application it builds has one. Three pictures,
        // because the confusion lives in the sequence rather than in any one
        // of them: the caution that explains what is about to be drawn, the
        // flow itself with the badge that keeps saying so, and the offer to
        // write the file when a project has none.
        (
            "56-live-caution",
            Box::new(|s: &Studio| {
                s.run(Command::LivePreview);
            }),
        ),
        (
            "57-live-preview",
            Box::new(|s: &Studio| {
                s.accept_live_preview();
            }),
        ),
        (
            "58-live-missing",
            Box::new(|s: &Studio| {
                s.live_missing.set(true);
            }),
        ),
        // The other half of the Safe area toggle. On is the default and the
        // frame draws the island, the home bar and the tinted unsafe regions;
        // off has to be a clean full-bleed rectangle with *no* island, because
        // an obstruction drawn over a screen that was told there is none is
        // the defect this pair was fixed for.
        (
            "59-safe-area-off",
            Box::new(|s: &Studio| {
                s.show_insets.set(false);
            }),
        ),
        // Rotation without a chosen device — which used to be refused outright
        // with "Choose a device first".
        (
            "60-landscape-no-device",
            Box::new(|s: &Studio| {
                s.toggle_landscape();
            }),
        ),
    ];

    // Where the pointer is parked for the `hover-*` pictures below. Hover is a
    // state that only exists while a pointer is over something, so the only way
    // to photograph it is to put one there and let the fade run.
    let hovers: Vec<(&str, f32, f32)> = vec![
        ("hover-tab", 560.0, 70.0),
        ("hover-activity-bar", 30.0, 197.0),
        // The tooltip, which is what the pointer resting on an activity-bar
        // button is *for*. Parked on the Toolchain button rather than the first
        // one, because that is the icon whose name explains least on its own.
        ("hover-activity-tooltip", 30.0, 453.0),
        ("hover-file-row", 180.0, 220.0),
        ("hover-menu-item", 300.0, 105.0),
        ("hover-panel-tab", 470.0, 666.0),
        ("hover-icon-button", 1412.0, 25.0),
    ];

    // **Which pictures had something overflow its box.**
    //
    // The framework prints a line to stderr when a render object runs out of
    // room, and in a run that writes seventy PNGs and prints a line for each
    // one, that scrolls past looking like progress. Collected here and
    // reported at the end as a list, because the whole purpose of this example
    // is a pre-release review and "the sidebar's bulleted lists paint over the
    // editor" is exactly the kind of finding it should hand over rather than
    // leave in a scrollback. Non-zero is a defect: nothing in this shell is
    // supposed to overflow at the sizes it is photographed at.
    let mut overflowed: Vec<(&str, usize)> = Vec::new();
    vieww_render::overflow::forget_reported();

    for (name, setup) in &scenarios {
        if let Some(only) = only {
            if !name.contains(only) {
                continue;
            }
        }
        reset(&studio, !light);
        reset_buffers(&studio, &root);
        setup(&studio);
        driver.draw_frame();
        driver.draw_frame();
        let path = out.join(format!("{prefix}{name}.png"));
        let background = if studio.dark.peek() {
            viewwstudio::StudioTheme::dark().window
        } else {
            viewwstudio::StudioTheme::light().window
        };
        let (png, report) = cpu
            .render_to_png(driver.scene(), width, height, background)
            .expect("rasterising");
        std::fs::write(&path, png).expect("writing the PNG");
        let over = vieww_render::overflow::reported();
        if over > 0 {
            overflowed.push((*name, over));
            vieww_render::overflow::forget_reported();
        }
        println!("{} — {} shapes", path.display(), report.shapes);
    }

    // The hover pictures. One frame to put the pointer down, then frames at
    // advancing timestamps so the highlight's fade actually runs — a
    // `Pressable` moves its wash over `CONTROL_DURATION` and a driver drawing
    // every frame at the same instant would photograph it at zero.
    for (name, x, y) in &hovers {
        if let Some(only) = only {
            if !name.contains(only) {
                continue;
            }
        }
        reset(&studio, !light);
        reset_buffers(&studio, &root);
        if *name == "hover-menu-item" {
            studio.open_menu(Some(Menu::File));
        }
        driver.draw_frame();
        driver.handle_hover(Some(vieww_foundation::Offset::new(*x, *y)));
        let mut now = std::time::Duration::from_millis(0);
        for _ in 0..12 {
            now += std::time::Duration::from_millis(30);
            driver.draw_frame_at(now);
        }
        let path = out.join(format!("{prefix}{name}.png"));
        let background = if studio.dark.peek() {
            viewwstudio::StudioTheme::dark().window
        } else {
            viewwstudio::StudioTheme::light().window
        };
        let (png, report) = cpu
            .render_to_png(driver.scene(), width, height, background)
            .expect("rasterising");
        std::fs::write(&path, png).expect("writing the PNG");
        println!("{} — {} shapes", path.display(), report.shapes);
        let over = vieww_render::overflow::reported();
        if over > 0 {
            overflowed.push((*name, over));
            vieww_render::overflow::forget_reported();
        }
        driver.handle_hover(None);
        driver.draw_frame();
    }

    if overflowed.is_empty() {
        println!("\nno layout overflowed in any picture.");
    } else {
        println!("\nLAYOUT OVERFLOWED in {} picture(s):", overflowed.len());
        for (name, count) in &overflowed {
            println!("  {name}: {count} report(s) — see the stderr lines above");
        }
        std::process::exit(1);
    }
}

/// Put the shell back to a plain open-folder state between pictures.
///
/// **Including its buffers.** Since the last tab can be closed, one scenario
/// can leave the editor genuinely empty — and every scenario after it that
/// reaches for `active()` then panics, which is what happened the first time
/// `54-no-file-open` was added in the middle of the list. Re-opening the
/// workspace here is what keeps the scenarios independent of their order.
fn reset_buffers(studio: &Studio, root: &std::path::Path) {
    // **Including the root.** `41-no-folder` photographs the studio with no
    // folder open, and it does that by clearing `root` — which every scenario
    // after it then inherited, because nothing put it back. The three live
    // pictures added after it were the first scenarios to notice: they came
    // out reading "No live.rs in this workspace" over a `scratch /` breadcrumb,
    // which is a true statement about a workspace that had been closed two
    // pictures earlier and nothing to do with what they are about.
    if studio.buffers.get().is_empty() || studio.root.peek().is_none() {
        studio.open_workspace(root);
    }
}

fn reset(studio: &Studio, dark: bool) {
    studio.menu_open.set(None);
    studio.palette_open.set(false);
    studio.palette_query.set(String::new());
    studio.find_open.set(false);
    studio.find_replacing.set(false);
    studio.close_picker();
    studio.welcome.set(false);
    studio.about.set(false);
    studio.context_menu.set(None);
    studio.close_name_prompt();
    studio.cancel_quit();
    studio.view.set(View::Explorer);
    studio.panel_open.set(true);
    studio.panel_tab.set(PanelTab::Problems);
    studio.right_open.set(true);
    studio.right_tab.set(RightTab::Preview);
    studio
        .sidebar_width
        .set(viewwstudio::state::MIN_SIDEBAR.max(260.0));
    studio.landscape.set(false);
    studio.platform.set(Platform::Ios);
    studio.dark.set(dark);
    studio.search_query.set(String::new());
    // The live preview is sticky — it is a mode, not a dialog — so without
    // these three every scenario after `57-live-preview` would be photographed
    // with a flow in the device frame instead of the screen it is about.
    studio.live_preview.set(false);
    studio.live_caution.set(false);
    studio.live_missing.set(false);
    // The status-bar notice is set by commands and cleared by the next one, so
    // without this a picture inherits the previous scenario's message — the
    // Inspector shot carried "Choose a device first" left over from the
    // landscape one, which is the sort of thing a reviewer then spends ten
    // minutes trying to reproduce.
    studio.notice.set(None);
    studio.show_insets.set(true);
    studio.device.set(None);
    studio.landscape.set(false);
    // **And the render verdict.** `45-build-error` deliberately breaks the
    // buffer, and the `Failed` it leaves behind was inherited by every picture
    // after it — twenty screenshots carrying a red "Render failed" in the
    // corner that had nothing to do with what they were photographing. A
    // review made of pictures cannot afford a permanently lit error light: it
    // is exactly what stops the one real failure from standing out.
    studio.set_preview(if studio.preview_screen.peek().is_some() {
        viewwstudio::state::PreviewState::Rendered
    } else {
        viewwstudio::state::PreviewState::Empty
    });
    studio.diagnostics.set(std::rc::Rc::new(Vec::new()));
}
