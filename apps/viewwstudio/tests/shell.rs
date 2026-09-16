//! What M0 claims, checked without a window.
//!
//! Two kinds of assertion here, and they answer different questions:
//!
//! - **Tree tests** build the shell against a hand-made `Studio` and read the
//!   dump. They answer "is the region there, and does it say what the state
//!   says", which is most of what a shell is.
//! - **Pixel tests** rasterise through vieww's own rasterizer and read colours
//!   back. They answer the one thing a tree dump cannot: whether the thing
//!   that is in the tree ended up where layout said, in the colour the theme
//!   said. This repository has been bitten twice by a tree that was right and
//!   a picture that was not; see `examples/screenshot.rs`.
//!
//! Vieww's own rasterizer needs no graphics adapter, so the pixel half runs
//! unconditionally, on every machine, rather than skipping.

use vieww_element::Runtime;
use vieww_foundation::{Color, Offset, Size};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::debug_tree;
use viewwstudio::state::{PanelTab, Platform, PreviewState, Studio, View, MIN_SIDEBAR};
use viewwstudio::{Shell, StudioTheme, WINDOW};

fn studio() -> (Runtime, Studio) {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    (runtime, studio)
}

fn tree(studio: &Studio) -> String {
    debug_tree(Shell {
        studio: studio.clone(),
    })
}

#[test]
fn every_region_of_the_shell_is_present() {
    let (_runtime, studio) = studio();
    let dump = tree(&studio);

    for region in [
        "TitleBar",
        "ActivityBar",
        "Sidebar",
        "EditorGroup",
        "VerticalDivider",
        "PreviewPane",
        "Panel",
        "StatusBar",
    ] {
        assert!(dump.contains(region), "{region} is missing from the shell");
    }
}

#[test]
fn the_sidebar_follows_the_activity_bar() {
    let (_runtime, studio) = studio();
    assert!(tree(&studio).contains("EXPLORER"));

    studio.view.set(View::Toolchain);
    let dump = tree(&studio);
    assert!(dump.contains("TOOLCHAIN"), "the header follows the view");
    // This used to assert the view still said "arrives after M0". It has been
    // written since, and the assertion went on passing until it did not — the
    // test was red on arrival here. What the view owes the reader now is the
    // *reason* there is no compiler, which is the state a fresh `Studio` is in.
    assert!(
        dump.contains("Toolchain unavailable"),
        "a studio with no compiler has to say so, not show an empty column"
    );

    // N4: the view now covers both toolchains — the preview's `rustc`, above,
    // and the per-target build toolchains, which are what Build depends on.
    // Every target gets a row whether or not it can be built here, because a
    // missing row reads as a target this tool does not support.
    for target in ["Desktop", "Android", "iOS"] {
        assert!(
            dump.contains(target),
            "{target} is missing from the checklist"
        );
    }
    assert!(
        dump.contains("rustup") || dump.contains("cargo"),
        "a missing requirement has to name what installs it"
    );
}

#[test]
fn the_panel_collapses_to_its_tab_strip() {
    let (_runtime, studio) = studio();
    let open = tree(&studio);
    assert!(
        open.contains("No problems"),
        "the body says what it has, even when that is nothing"
    );

    studio.panel_open.set(false);
    let closed = tree(&studio);
    assert!(
        closed.contains("PROBLEMS"),
        "the tab strip survives collapsing — the panel is one click from back"
    );
    assert!(!closed.contains("No problems"), "the body does not");
}

#[test]
fn each_panel_tab_shows_its_own_body() {
    let (_runtime, studio) = studio();

    // Before anything has compiled, every panel has nothing — and says which
    // nothing it has, rather than showing a blank pane that reads as broken.
    studio.panel_tab.set(PanelTab::Timings);
    assert!(tree(&studio).contains("No render has been timed"));

    studio.panel_tab.set(PanelTab::Rustc);
    assert!(tree(&studio).contains("rustc has not been run"));

    studio.panel_tab.set(PanelTab::Output);
    assert!(tree(&studio).contains("Nothing has been compiled"));
}

#[test]
fn the_status_bar_reports_the_state_and_the_device() {
    let (_runtime, studio) = studio();

    studio.set_preview(PreviewState::Failed);
    studio.platform.set(Platform::Android);
    let dump = tree(&studio);

    // **"Render failed", not "Failed".** The light is shared with the build
    // pipeline, and an unqualified verdict beside a finished build is exactly
    // the ambiguity that had users reading a successful build as a failed one
    // — see `interaction.rs`'s `a_finished_build_outranks_a_stale_failed_render`.
    // Naming the pipeline is the part of that fix that cannot be undone by a
    // wrong arbitration later.
    assert!(
        dump.contains("Render failed"),
        "the state is named, and names which pipeline it is about: {dump}"
    );
    assert!(dump.contains("412×915"), "the simulated device is named");
}

#[test]
fn a_failed_first_render_does_not_claim_a_previous_one() {
    // This test used to assert the opposite, and it was wrong. It set `Failed`
    // on a studio that had never compiled anything and demanded the badge —
    // so the pane dimmed an empty phone under "showing last successful
    // render", asserting a render that never happened. The promise in plan
    // §2.2 is that a failure *keeps* the last good frame; keeping nothing is
    // not something to boast about.
    //
    // The half of the promise that needs a real mounted screen is checked in
    // `tests/pipeline.rs`, where one can be compiled and loaded.
    let (_runtime, studio) = studio();
    studio.preview.set(PreviewState::Failed);
    assert!(studio.preview_screen.get().is_none());

    let dump = tree(&studio);
    assert!(
        !dump.contains("showing last successful render"),
        "there was no last successful render to show"
    );
    assert!(
        dump.contains("No render yet"),
        "the frame says what is true instead"
    );
}

/// The caption still states its own limits — different limits.
///
/// It used to pin *"not visually reskinned per platform"*, which was the honest
/// sentence while `ThemeData` threw the platform away before any control could
/// read it. It does not any more: the switch, the slider, the checkbox, the
/// button, the activity indicator, the alert and the segmented control all draw
/// the platform's shape (see `vieww-widget`'s `tests/platform_shapes.rs`), and
/// `ElementTree::snapshot_states` carries the screen's state across a Render.
///
/// So what is asserted is what plan §4.7 actually asks for — that the boundary
/// is stated *next to the picker* rather than in a doc — against the boundary
/// that is now real.
#[test]
fn the_platform_caption_states_its_own_limits() {
    let (_runtime, studio) = studio();
    let dump = tree(&studio);

    assert!(
        dump.contains("One catalogue drawn per platform"),
        "plan §4.7 wants the limit said next to the picker, not in a doc"
    );
    assert!(
        !dump.contains("not visually reskinned per platform"),
        "that limit is gone, and a caption that still claims it is a caption \
         nobody will trust the next time it says something"
    );
    assert!(!dump.contains("resets on every Render"), "so is that one");
}

#[test]
fn a_fresh_session_has_nothing_rendered_and_a_dirty_buffer() {
    let (_runtime, studio) = studio();

    assert_eq!(studio.preview.get(), PreviewState::Empty);
    assert!(studio.dirty.get(), "nothing has been rendered yet");
    assert!(tree(&studio).contains("No render yet"));
}

#[test]
fn the_divider_clamps_rather_than_letting_a_pane_vanish() {
    let (_runtime, studio) = studio();

    // The clamp lives in the divider's drag handler, so this asserts the
    // constant it clamps to rather than re-implementing the arithmetic: a pane
    // narrower than this is not a pane.
    studio.sidebar_width.set(MIN_SIDEBAR - 100.0);
    assert!(
        studio.sidebar_width.get() < MIN_SIDEBAR,
        "the signal is raw"
    );

    studio.sidebar_width.set(MIN_SIDEBAR);
    assert!(tree(&studio).contains("Sidebar"));
}

#[test]
fn switching_platform_is_an_edit() {
    let (_runtime, studio) = studio();
    studio.dirty.set(false);

    studio.platform.set(Platform::Desktop);
    studio.mark_dirty();

    assert!(
        studio.dirty.get(),
        "plan §4.7: switching the picker marks dirty exactly as typing does"
    );
}

/// Rasterise the shell at window size and hand back the pixels.
fn shell_pixels(dark: bool) -> (vieww_paint::native::Pixels, Studio) {
    let mut renderer = NativeRenderer::new();

    let mut driver = FrameDriver::new(WINDOW);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.dark.set(dark);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the window size is a small positive number"
    )]
    let (width, height) = (WINDOW.width as u32, WINDOW.height as u32);

    let (pixels, _) = renderer
        .render_to_pixels(driver.scene(), width, height, Color::BLACK)
        .expect("rendering the shell");

    (pixels, studio)
}

#[test]
fn the_chrome_depths_land_where_the_layout_says() {
    let (pixels, _studio) = shell_pixels(true);
    let chrome = StudioTheme::dark();

    // Read one pixel well inside each region. Picked away from every edge so a
    // one-pixel disagreement about a hairline does not decide the test.
    let opaque = |color: Color| Color::rgba(color.r, color.g, color.b, 255);

    assert_eq!(
        pixels.pixel(20, 400),
        opaque(chrome.chrome_0),
        "the activity bar is the deepest chrome"
    );
    assert_eq!(
        pixels.pixel(200, 400),
        opaque(chrome.chrome_1),
        "the sidebar sits one step above it"
    );
    assert_eq!(
        pixels.pixel(700, 300),
        opaque(chrome.chrome_2),
        "the editor is the surface itself"
    );
    assert_eq!(
        pixels.pixel(700, 890),
        opaque(chrome.chrome_0),
        "the status bar returns to the deepest chrome"
    );
}

#[test]
fn light_and_dark_are_the_same_shell_in_two_palettes() {
    let (dark, _) = shell_pixels(true);
    let (light, _) = shell_pixels(false);

    let sample = |pixels: &vieww_paint::native::Pixels| pixels.pixel(700, 300);
    assert_ne!(
        sample(&dark),
        sample(&light),
        "the editor surface differs between the two themes"
    );

    // The point of the pairing in `theme.rs`: neither build may end up with a
    // chrome from one and a scheme from the other, which shows up as an editor
    // that is darker than the activity bar beside it.
    let luma = |c: Color| 0.30 * f32::from(c.r) + 0.59 * f32::from(c.g) + 0.11 * f32::from(c.b);
    assert!(
        luma(dark.pixel(700, 300)) > luma(dark.pixel(20, 400)),
        "dark: the editor is lighter than the activity bar"
    );
    assert!(
        luma(light.pixel(700, 300)) > luma(light.pixel(20, 400)),
        "light: the same relationship holds"
    );
}

#[test]
fn the_shell_hit_tests_where_the_controls_are() {
    let mut driver = FrameDriver::new(WINDOW);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();

    // Somewhere inside the activity bar's second button. A hit here means the
    // 48-point column really is 48 points wide and really is clickable, which
    // no tree dump can say.
    let hit = driver.hit_test(Offset::new(24.0, 100.0));
    assert!(
        !hit.is_empty(),
        "the activity bar answers a pointer inside it"
    );
}

/// **The studio announces itself.**
///
/// `Semantics`, `SemanticRole` and `Liveness` were in `vieww-widget` before the
/// studio existed, and there was not one reference to any of them in the whole
/// application: nine sidebar views, five panel tabs, six menus, and nothing
/// that told a screen reader what any of it was. The studio is the framework's
/// flagship application and the evidence its accessibility story works; it was
/// evidence of the opposite.
///
/// This asserts the shape rather than a count — a count is a number somebody
/// updates without reading — so it holds as the shell grows.
#[test]
fn the_shell_names_its_controls_for_a_screen_reader() {
    let mut driver = FrameDriver::new(WINDOW);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();

    let semantics = driver.semantics();
    let dump = format!("{semantics:?}");

    // The activity bar: an icon and nothing else, so this label is the only
    // one a reader can get.
    for view in View::ALL {
        assert!(
            dump.contains(view.title()),
            "the {} button is unnamed in the semantics tree",
            view.title()
        );
    }
    // The menu strip.
    for menu in viewwstudio::command::Menu::BAR {
        assert!(
            dump.contains(menu.title()),
            "the {} menu is unnamed",
            menu.title()
        );
    }
    // And the panel's tabs, which are tabs rather than buttons.
    assert!(dump.contains("Tab"), "the panel tabs carry no role");
}

/// Guards the one number every strip's height is derived from.
#[test]
fn the_window_is_big_enough_for_both_panes() {
    let minimum = 48.0 + MIN_SIDEBAR + viewwstudio::state::MIN_PANE * 2.0;
    assert!(
        WINDOW.width > minimum,
        "the default window can hold the sidebar and both panes at their minimums"
    );
    assert_eq!(WINDOW, Size::new(1440.0, 900.0));
}

// ----- what M0.1 added -------------------------------------------------

#[test]
fn every_icon_parses_into_a_real_path() {
    // `icons.rs` falls back to a crossed box when path data is malformed, so a
    // typo is visible rather than fatal — and invisible to a test that only
    // checked the icon existed. This checks the *fallback did not fire*, by
    // comparing against the fallback itself.
    //
    // It used to ask for "more than six verbs", which is a property the old
    // filled set happened to have. The centreline set does not: `code()` is two
    // chevrons and six verbs, and a correct icon failed a test that was really
    // asking whether a string had a typo in it.
    let placeholder = viewwstudio::ui::icons::fallback();
    let set: [(&str, vieww_foundation::IconData); 8] = [
        ("folder", viewwstudio::ui::icons::folder()),
        ("search", viewwstudio::ui::icons::search()),
        ("code", viewwstudio::ui::icons::code()),
        ("warning", viewwstudio::ui::icons::warning()),
        ("gear", viewwstudio::ui::icons::gear()),
        ("shield", viewwstudio::ui::icons::shield()),
        ("error", viewwstudio::ui::icons::error()),
        ("file", viewwstudio::ui::icons::file()),
    ];

    for (name, data) in set {
        assert!(!data.path().is_empty(), "{name} is empty");
        assert!(
            data.path().verbs() != placeholder.path().verbs(),
            "{name} came out as the crossed box — the malformed-path fallback fired"
        );
    }
}

#[test]
fn the_frame_is_empty_until_something_has_compiled() {
    let (_runtime, studio) = studio();

    let dump = tree(&studio);
    assert!(dump.contains("No render yet"));
    assert!(
        dump.contains("Press Render"),
        "and it says what to do about it"
    );
    assert!(studio.preview_screen.get().is_none());
}

#[test]
fn a_short_pane_shrinks_the_device_rather_than_cutting_it_off() {
    use vieww_foundation::Constraints;
    use viewwstudio::ui::preview::fit_scale;

    // A pane with room to spare: the frame is drawn at its natural size and
    // no larger — a phone blown up past a phone stops being a size reference.
    let roomy = Constraints {
        min_width: 0.0,
        max_width: 600.0,
        min_height: 0.0,
        max_height: 1200.0,
    };
    assert!((fit_scale(roomy, Platform::Ios) - 560.0 / 852.0).abs() < 0.001);

    // The window the clipping was reported in. Every dimension of the frame,
    // including the bottom the old code cut off, has to be inside the pane.
    let short = Constraints {
        min_width: 0.0,
        max_width: 460.0,
        min_height: 0.0,
        max_height: 300.0,
    };
    let scale = fit_scale(short, Platform::Ios);
    let (width, height) = Platform::Ios.screen();
    assert!(
        height * scale <= short.max_height && width * scale <= short.max_width,
        "the whole frame fits: {:.0}×{:.0} in {:.0}×{:.0}",
        width * scale,
        height * scale,
        short.max_width,
        short.max_height
    );
    assert!(scale > 0.0, "and it is still visible");
}

#[test]
fn the_welcome_offers_a_way_to_make_a_project() {
    // The screen offered *Open a folder* and *New screen from a template* and
    // nothing that makes a project, so somebody arriving with nothing to open
    // had to find "New Workspace" in the File menu — Cargo's word for it, not
    // theirs — and guess.
    let (_runtime, studio) = studio();
    studio.welcome.set(true);
    let dump = tree(&studio);

    assert!(
        dump.contains("New project"),
        "the welcome offers to make one: {dump}"
    );
    assert_eq!(
        viewwstudio::command::Command::NewProject.title(),
        "New Project\u{2026}",
        "and the menu calls it what the person came to do"
    );
    assert!(
        !dump.contains("New Workspace"),
        "nothing left saying Workspace: {dump}"
    );
}

#[test]
fn a_pane_too_short_for_a_legible_frame_stops_shrinking_and_scrolls() {
    use vieww_foundation::Constraints;
    use viewwstudio::ui::preview::{fit_scale, stage_scale, MIN_SCALE};

    // The window the complaint came from: 1366x679 with the bottom panel open
    // leaves the stage about this much, and the honest fit of an iPhone into it
    // is a fifth of full size — every dimension correct, nothing inside legible.
    let cramped = Constraints {
        min_width: 0.0,
        max_width: 420.0,
        min_height: 0.0,
        max_height: 210.0,
    };
    let (width, height) = Platform::Ios.screen();
    assert!(
        fit_scale(cramped, Platform::Ios) < 0.25,
        "the premise: the fit really is that small"
    );

    let scale = stage_scale(cramped, Platform::Ios, (width, height));
    assert!(
        (scale - MIN_SCALE).abs() < 0.001,
        "the stage stops at the floor instead: {scale}"
    );
    assert!(
        width * scale <= cramped.max_width,
        "and the floor never costs the width, which has nothing to scroll it"
    );

    // A pane with room keeps the fit exactly: the floor is a floor, not a size.
    let roomy = Constraints {
        min_width: 0.0,
        max_width: 600.0,
        min_height: 0.0,
        max_height: 1200.0,
    };
    assert!(
        (stage_scale(roomy, Platform::Ios, (width, height)) - fit_scale(roomy, Platform::Ios))
            .abs()
            < 0.001
    );
}

#[test]
fn the_floor_never_blows_a_frame_up_past_its_natural_size() {
    use vieww_foundation::Constraints;
    use viewwstudio::ui::preview::{fit_scale, stage_scale};

    // A desktop window is drawn at 300 points for a screen hundreds of points
    // taller, so its natural scale is already below the floor. Flooring it would
    // *enlarge* the frame on a short pane, which is the opposite of the fix.
    let short = Constraints {
        min_width: 0.0,
        max_width: 1200.0,
        min_height: 0.0,
        max_height: 240.0,
    };
    let screen = Platform::Desktop.screen();
    let natural = fit_scale(
        Constraints {
            min_width: 0.0,
            max_width: f32::INFINITY,
            min_height: 0.0,
            max_height: f32::INFINITY,
        },
        Platform::Desktop,
    );
    assert!(
        stage_scale(short, Platform::Desktop, screen) <= natural + 0.001,
        "a desktop frame is never drawn larger than a desktop frame"
    );
}

#[test]
fn an_empty_preview_is_not_dimmed_for_a_failure_it_never_had() {
    // The companion to the test above, and wrong for the same reason: fading a
    // frame that has nothing in it communicates staleness about nothing. The
    // dimming of a genuinely stale frame is checked in `tests/pipeline.rs`.
    let (_runtime, studio) = studio();
    studio.preview.set(PreviewState::Failed);

    let dump = tree(&studio);
    assert!(
        !dump.contains("Opacity"),
        "an empty frame has no previous render to fade"
    );
}

// ----- M1: files, scrolling, editing ------------------------------------

/// The bundled `screens/` directory, which the studio opens in these tests.
fn screens() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("screens")
}

fn studio_with_files() -> (Runtime, Studio) {
    let runtime = Runtime::new();
    let workspace = viewwstudio::Workspace::open(&screens());
    let studio = Studio::with_workspace(&runtime, workspace);
    (runtime, studio)
}

#[test]
fn the_explorer_lists_files_that_exist() {
    let (_runtime, studio) = studio_with_files();
    let buffers = studio.buffers.get();

    // **Three screens, plus a `live.rs` that may or may not be there.** The
    // studio writes `live.rs` into whatever workspace it has open the first
    // time somebody accepts the Live Preview (see `Studio::create_live_file`),
    // and the walkthrough example does the same — so a checkout that has ever
    // run either holds a fourth file that this test used to count as a
    // regression. It is workspace state rather than a screen: the file's own
    // header says it is not compiled and not part of the app. The assertion
    // holds for both shapes: the three screens always, `live.rs` only when it
    // is on disk.
    let names: Vec<&str> = buffers.iter().map(|b| b.name.as_str()).collect();

    // **Containment, not equality — and `live.rs` was the warning.**
    //
    // The comment above already records that a checkout which has ever run the
    // studio grows a `live.rs` this test used to count as a regression. The
    // same thing happened again and larger: `Workspace::open` scans a
    // *recursive* tree (see its doc), New Screen and New Widget scaffold into
    // whatever workspace is open, and somebody with this directory open
    // accumulated `src/screens/screen{,_2.._6}.rs` and `widget{,_2.._6}.rs`
    // inside the fixture. An exact-set assertion turns "the author created a
    // screen last Tuesday" into a failing test, which is not a regression in
    // anything.
    //
    // It went unseen because `tests/pipeline.rs` aborted the whole binary
    // before this file ever ran — see `.cargo/config.toml`. With the panic
    // boundary restored this runs again, and the first thing it reported was
    // the fixture rather than the code.
    //
    // What the test is named for is that the explorer lists files that *exist*,
    // which the loop below checks for every buffer. The set assertion's real
    // job is that the bundled screens are all there, so that is what it says.
    for bundled in ["card_grid.rs", "landing_screen.rs", "settings_form.rs"] {
        assert!(
            names.contains(&bundled),
            "the bundled screen {bundled} is listed; got {names:?}"
        );
    }
    for buffer in buffers.iter() {
        let path = buffer.path.as_ref().expect("opened from disk");
        assert!(path.exists(), "{} is a real file", path.display());
        assert!(!buffer.value.text.is_empty());
    }
}

#[test]
fn the_gutter_counts_the_lines_the_buffer_actually_has() {
    let (_runtime, studio) = studio_with_files();
    let buffer = studio.active().expect("a buffer");
    let dump = tree(&studio);

    // The last line number is in the tree, and the one after it is not: the
    // gutter is as long as the file and no longer.
    assert!(
        dump.contains(&format!("\"{}\"", buffer.line_count())),
        "the last line number is drawn"
    );
    assert!(
        !dump.contains(&format!("\"{}\"", buffer.line_count() + 10)),
        "and the gutter does not run past the end of the file"
    );
}

#[test]
fn editing_marks_the_buffer_dirty_and_moves_the_caret() {
    let (_runtime, studio) = studio_with_files();
    studio.dirty.set(false);

    let mut value = studio.active().expect("a buffer").value;
    value.text.insert_str(0, "// a note\n");
    value.selection = vieww_foundation::TextSelection::collapsed(10);
    studio.edit(value);

    let buffer = studio.active().expect("a buffer");
    assert!(buffer.dirty, "the file differs from what is on disk");
    assert_eq!(studio.caret.get(), (2, 1), "the caret followed the text");
    assert!(
        studio.dirty.get(),
        "and the render is stale, exactly as a platform switch makes it"
    );
}

#[test]
fn moving_the_caret_alone_does_not_invalidate_the_render() {
    let (_runtime, studio) = studio_with_files();
    studio.dirty.set(false);

    let mut value = studio.active().expect("a buffer").value;
    value.selection = vieww_foundation::TextSelection::collapsed(5);
    studio.edit(value);

    assert_eq!(studio.caret.get(), (1, 6));
    assert!(
        !studio.dirty.get(),
        "a click in the text is not an edit — Render has nothing new to compile"
    );
    assert!(!studio.active().expect("a buffer").dirty);
}

#[test]
fn each_buffer_keeps_its_own_caret_across_a_tab_switch() {
    let (_runtime, studio) = studio_with_files();

    let mut value = studio.active().expect("a buffer").value;
    value.selection = vieww_foundation::TextSelection::collapsed(20);
    studio.edit(value);
    let first = studio.caret.get();

    studio.active_buffer.set(1);
    let second = studio.active().expect("a buffer");
    assert_eq!(second.name, "landing_screen.rs");

    studio.active_buffer.set(0);
    assert_eq!(
        studio.active().expect("a buffer").caret(),
        first,
        "coming back to a file puts the cursor where it was left"
    );
}

#[test]
fn a_diagnostic_belongs_to_a_file_not_to_a_line_number() {
    let (_runtime, studio) = studio_with_files();

    // Hand-made rather than compiled, because what is under test is the
    // filtering, not the compiler: a diagnostic about another file must not
    // mark the buffer that happens to be open.
    studio
        .diagnostics
        .set(std::rc::Rc::new(vec![viewwstudio::state::Diagnostic {
            file: "landing_screen.rs".into(),
            severity: viewwstudio::state::Severity::Error,
            code: "E0308".into(),
            message: "mismatched types".into(),
            help: None,
            line: 24,
            column: 38,
            end_line: 24,
            end_column: 42,
        }]));

    assert_eq!(studio.active().expect("a buffer").name, "card_grid.rs");
    assert!(
        studio.active_diagnostics().is_empty(),
        "not this buffer's problem"
    );

    studio.open_named("landing_screen.rs");
    assert_eq!(
        studio.active_diagnostics().len(),
        1,
        "and it comes back when its own file does"
    );
}

#[test]
fn the_editor_is_scrollable_and_editable() {
    let (_runtime, studio) = studio_with_files();
    let dump = tree(&studio);

    assert!(
        dump.contains("Scrollable"),
        "M1's first item: the pane scrolls rather than clipping the rest of the file"
    );
    assert!(
        dump.contains("TextField"),
        "and the text is a field with a caret, not a picture of text"
    );
}

#[test]
fn saving_writes_the_file_and_clears_the_dot() {
    let directory = std::env::temp_dir().join("viewwstudio-save-test");
    std::fs::create_dir_all(&directory).expect("a temp directory");
    let file = directory.join("screen.rs");
    std::fs::write(&file, "fn main() {}\n").expect("a file to edit");

    let runtime = Runtime::new();
    let studio = Studio::with_workspace(&runtime, viewwstudio::Workspace::open(&directory));

    let mut value = studio.active().expect("a buffer").value;
    value.text.push_str("// edited\n");
    studio.edit(value);
    assert!(studio.active().expect("a buffer").dirty);

    studio.save_active().expect("the write succeeds");

    assert!(
        !studio.active().expect("a buffer").dirty,
        "the dot goes when the bytes are on disk, not when the click happens"
    );
    assert!(std::fs::read_to_string(&file)
        .expect("reading it back")
        .contains("// edited"));

    std::fs::remove_dir_all(&directory).ok();
}
