//! The window.

use std::path::PathBuf;
use std::rc::Rc;

use vieww_platform_winit::App;
use viewwstudio::compile::{Session, Toolchain};
use viewwstudio::{Shell, Studio, Workspace, WINDOW};

fn main() {
    // One optional argument: the directory of `.rs` screens to open. Not a
    // project — a flat list of files, per the plan's non-goals — and an
    // unreadable one gives the scratch buffer rather than an error, because a
    // studio that will not open is worse than one with nothing in it.
    let root = std::env::args().nth(1).map(PathBuf::from);

    // Where `libvieww.rlib` and its dependencies live. `CARGO_MANIFEST_DIR` is
    // the app's own directory in a development checkout, so the profile
    // directory is three levels up — and if it is not there, `discover` says
    // so and the studio opens with the Render button explaining itself rather
    // than failing to start.
    // **This used to be one directory: the framework checkout's own target.**
    // A studio copied anywhere else could not find `libvieww.rlib` and so
    // could not render — which meant every user needed a clone of the vieww
    // repository and a completed build of it. `install::target_dir` is the
    // search chain that makes an *installed* studio work; see that module for
    // the order and why the checkout comes last.
    let target = viewwstudio::install::target_dir();
    let toolchain = Toolchain::discover(&target);
    if let Err(error) = &toolchain {
        eprintln!("vieww Studio: no preview compiler — {error}");
    }
    let mut session = Session::new(std::process::id().into()).ok();

    // The platform's capabilities, built here rather than inside `run` because
    // two things need them *before* there is a window: the stored session
    // (which decides how big the window is) and the studio itself.
    let mut services = vieww_platform_winit::services::platform();
    // Wrap the studio's `AssetBundle` in a `WorkspaceBundle` so a previewed
    // screen's `assets/` resolves against the open workspace first, then the
    // studio's own bundle. The wrapper is held on the studio so the workspace
    // can change without re-registering the service.
    let workspace_bundle = viewwstudio::assets::WorkspaceBundle::install(&mut services);
    let services = vieww_foundation::SharedServices::new(services);
    let store = services.get::<dyn vieww_foundation::Storage>();
    let stored = Studio::stored_session(store.as_ref());

    // Where to open. An explicit argument wins; otherwise the workspace from
    // last time, if it is still there. A root that has been deleted or
    // unmounted falls through to the scratch workspace rather than to an
    // error — the studio has to open, and the recent list keeps the entry so
    // the user can try again once the drive is back.
    let root = root.or_else(|| stored.root.clone().filter(|path| path.is_dir()));
    let workspace = root
        .as_deref()
        .map_or_else(Workspace::scratch, Workspace::open);

    // Restored geometry, or the default. `WINDOW` is still what a first launch
    // gets, and `Session::parse` refuses a stored size too small to find the
    // close button on, so a corrupted file cannot open an unusable window.
    let window_size = stored.window.map_or(WINDOW, |(width, height)| {
        vieww_foundation::Size::new(width, height)
    });

    // Set by the close hook, read by nothing else: it is how the veto tells
    // the *next* close request that the user has already answered.
    let quit_gate: std::rc::Rc<std::cell::RefCell<Option<Studio>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let for_close = std::rc::Rc::clone(&quit_gate);

    let app = App::new()
        .title("vieww Studio")
        .size(window_size)
        .background(viewwstudio::StudioTheme::dark().chrome_0)
        // **The guard that did not exist.** Before this, clicking the window's
        // close button on a modified buffer lost it: no dialog, no autosave, no
        // recovery file. `Studio::may_close` writes the session and the
        // settings out on a clean exit and vetoes a dirty one, and the shell
        // draws the dialog the veto is waiting on.
        .on_close_request(move |request| {
            let studio = for_close.borrow();
            let Some(studio) = studio.as_ref() else {
                return vieww_platform_winit::Closing::Allowed;
            };
            if studio.may_close(Some((request.width, request.height))) {
                vieww_platform_winit::Closing::Allowed
            } else {
                vieww_platform_winit::Closing::Vetoed
            }
        });

    // The window handle, taken for the same reason the waker is: `run`
    // consumes the app. It is how the quit dialog's answer reaches the
    // platform — closing a window is not something application code can do
    // directly, and the studio only sets a flag.
    let windows = app.windows();

    // Taken before `run`, which consumes the app. It is what lets a compile
    // finishing on a worker thread cause a frame — without it the result sits
    // in its channel until the user happens to move the mouse.
    let waker = app.waker();

    // Held so the frame hook can reach the studio. `RefCell<Option<_>>` rather
    // than a plain `Option` because `before_frame` takes `FnMut` and the build
    // closure runs once, inside `run`, after the hook is already installed.
    let shared: std::rc::Rc<std::cell::RefCell<Option<Studio>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let for_hook = std::rc::Rc::clone(&shared);

    // The last settings written, so the frame hook can tell a change from a
    // frame. `None` until the first poll, which is the baseline rather than a
    // reason to write the file.
    let last_settings: std::cell::RefCell<Option<viewwstudio::settings::Settings>> =
        std::cell::RefCell::new(None);

    // When the recovery files were last written. `None` until the first frame,
    // which starts the clock rather than writing.
    let last_recovery: std::cell::RefCell<Option<std::time::Instant>> =
        std::cell::RefCell::new(None);

    let result = app
        .before_frame(move |driver| {
            // Two things every frame, both no-ops on almost all of them.
            //
            // 1. Give focus somewhere to be, so a keystroke has an object to
            //    bubble out from. See `viewwstudio::seed_focus`.
            // 2. Pick up a finished compile. This is the whole of what moving
            //    `rustc` off this thread costs: one non-blocking channel read.
            viewwstudio::seed_focus(driver);
            if let Some(studio) = for_hook.borrow().as_ref() {
                // 0. Anything that panicked while building last frame. First,
                //    because it is the only one that changes what the user is
                //    looking at rather than adding to it — see
                //    `Studio::drain_build_errors`.
                studio.drain_build_errors(driver);
                // Before the poll, because the poll is what swaps the screen:
                // this takes the state off the tree the user has been touching,
                // while it is still the tree. A `Cell` read on every frame that
                // is not the first of a render.
                studio.capture_preview_state(driver);
                // And after it, for the frame *following* the swap — the new
                // guest's elements do not exist until this frame's build has
                // run, so the restore lands one frame later. See
                // `Studio::restore_preview_state`.
                studio.restore_preview_state(driver);
                studio.poll_compile();
                // Pick up external file changes from the watcher. A no-op on
                // the common frame where nothing moved; a `git checkout`
                // outside the window reaches the Explorer this way, and an
                // open, clean buffer is reloaded before it can be overwritten.
                studio.drain_watcher();
                // 3. Pick up whatever the running build has printed. Same
                //    shape as the compile poll above and the same cost when
                //    nothing is running: one `try_recv` that comes back empty.
                //    This is what makes `cargo build` output appear as it
                //    happens rather than in one lump at the end.
                studio.poll_builds();
                // 4. And every other child process — a `rustfmt`, an export
                //    step, a device scan. Same shape, same cost.
                studio.poll_jobs();
                // 5. And read the render tree, for the Inspector. Costs an
                //    early return on every frame the pane is not open, which is
                //    nearly all of them — see `Studio::capture_tree`.
                studio.capture_tree(driver);
                // 6. And the damage, the semantics tree and any pending pick.
                //    Three boolean reads on a frame with every overlay off.
                studio.capture_overlays(driver);
                // 7. Anything `rust-analyzer` said, and the auto-render
                //    debounce. Both a `try_recv` and a clock comparison when
                //    nothing is running, which is the usual case.
                studio.poll_analyzer();
                studio.poll_auto_render();
                // 8. Write the preferences out if any of them moved. One
                //    struct comparison on a frame where nothing did.
                studio.poll_settings(&last_settings);
                // 9. And the unsaved buffers, to the recovery directory. Same
                //    shape again: a clock comparison on almost every frame.
                //    This is what makes a crash — a layout panic in a previewed
                //    screen, an OOM kill, a power cut — cost at most
                //    `RECOVERY_INTERVAL` of typing rather than everything since
                //    the last save.
                studio.poll_recovery(&last_recovery);
                // No splash hook here on purpose. The overlay is one
                // `Animated`, which lives in its element's state — the frame
                // loop already ticks it, `FrameDriver::is_animating` already
                // keeps frames coming while it moves, and it stops asking for
                // them the moment it lands. What stood here was a commented-out
                // block whose body called `request_close()`, which would have
                // quit the application on every frame of the splash.
                // The window's own size, which nothing inside a build can ask
                // for. The context menu needs it to decide whether to flip.
                studio.note_window_size(driver.size());
                // 10. Files dragged onto the window. The platform has gathered
                //     these since before the studio existed and nothing here
                //     had ever read them — so a drag-and-drop, which is how
                //     most people open a file in a desktop editor the first
                //     time, did nothing at all.
                let dropped = driver.take_dropped_files();
                if !dropped.is_empty() {
                    studio.accept_dropped(dropped.paths());
                }
                // 9. And carry out a close the quit dialog asked for. The
                //    close hook lets this one through, because answering the
                //    dialog is what set `quit_confirmed`.
                if studio.take_close_request() {
                    let _ = vieww_render::Windows::close(
                        windows.as_ref(),
                        vieww_render::WindowKey::PRIMARY,
                    );
                }
            }
        })
        .run(|driver| {
            let runtime = driver.elements().runtime().clone();
            // Teaches the driver about `Shortcuts`. A shell mounted without
            // this panics at the first frame rather than quietly losing every
            // keyboard shortcut, which is the failure worth having.
            viewwstudio::install(driver);

            let mut studio = Studio::with_workspace(&runtime, workspace.clone())
                .with_toolchain(toolchain.clone(), session.take())
                .with_waker(waker.clone())
                .with_services(services.clone())
                .with_workspace_bundle(workspace_bundle.clone());
            // **Surface unregistered-render-object warnings in the Problems
            // panel rather than on stderr.** The render layer used to print
            // these to a console a GUI application does not have, so a guest
            // widget whose render object was never registered drew nothing
            // *and* said nothing about why. The sink hands the warning to the
            // studio's diagnostics signal, where it lands beside compile
            // errors and is reachable from the same place.
            //
            // The closure captures a clone of the studio's diagnostics signal,
            // which is `Rc`-backed and so cheaply cloneable. The render layer
            // holds it as `Box<dyn Fn>` — not `Send + Sync`, because the
            // render tree is single-threaded and the sink runs on the same
            // thread that owns the signal.
            let diagnostics = studio.diagnostics.clone();
            vieww_render::set_unregistered_render_object_sink(Some(Box::new(
                move |name, message| {
                    let mut next = (*diagnostics.peek()).clone();
                    next.push(viewwstudio::state::Diagnostic {
                        file: "(preview)".to_owned(),
                        severity: viewwstudio::state::Severity::Warning,
                        code: "unregistered-render-object".to_owned(),
                        message: format!(
                            "{message} — open the screen's source and call \
                            `RenderOwner::register::<{name}, _>(..)` before its first frame, \
                            or remove the widget if it was a typo."
                        ),
                        help: None,
                        line: 1,
                        column: 1,
                        end_line: 1,
                        end_column: 1,
                    });
                    diagnostics.set(Rc::new(next));
                },
            )));
            // **Render-object panics report to the Problems panel too.** A
            // `CustomPainter::paint` or any render object whose `layout`
            // panics used to abort the process; with both sides built
            // `-C prefer-dynamic`, the render tree's per-node `catch_unwind`
            // catches them, and this sink routes the report to the same
            // diagnostics list the unregistered-render-object warning uses.
            let diagnostics = studio.diagnostics.clone();
            vieww_render::set_render_panic_sink(Some(Box::new(move |name, message| {
                let mut next = (*diagnostics.peek()).clone();
                next.push(viewwstudio::state::Diagnostic {
                    file: "(preview)".to_owned(),
                    severity: viewwstudio::state::Severity::Error,
                    code: "render-panic".to_owned(),
                    message: format!(
                        "{message} — the widget `{name}` panicked inside the render \
                             tree. The studio is still running, but that part of the \
                             screen may be blank or unresponsive until you fix it."
                    ),
                    help: None,
                    line: 1,
                    column: 1,
                    end_line: 1,
                    end_column: 1,
                });
                diagnostics.set(Rc::new(next));
            })));
            // Without this a fling scrolls under the finger and stops dead the
            // moment it lifts: `Tickers` is what advances the simulation.
            studio.editor_scroll.attach(driver.tickers());
            // The horizontal one, for the same reason: without a `Tickers` a
            // sideways fling stops dead the moment the finger lifts.
            studio.editor_scroll_x.attach(driver.tickers());
            // And the tab strip, so a flick along it coasts like every other
            // list in the shell rather than stopping under the finger.
            studio.tabs_scroll.attach(driver.tickers());
            // One per sidebar view (`Studio::sidebar_scroll`) — same reason,
            // same call, just seven of them.
            for controller in &studio.sidebar_scroll {
                controller.attach(driver.tickers());
            }
            // One per bottom-panel tab, and the Devices list. Same reason again:
            // a flick that stops dead under the finger reads as a frozen list.
            for controller in &studio.panel_scroll {
                controller.attach(driver.tickers());
            }
            studio.devices_scroll.attach(driver.tickers());
            studio.menu_scroll.attach(driver.tickers());
            // And the caret. A blink driven by the frame scheduler rather than
            // a clock of its own stays in step with everything else on screen.
            studio.attach_blink(driver.tickers());
            // Settings, then the session, then the workspace's own recent
            // entry — in that order, because applying the session can open
            // buffers and applying the settings can change how they are drawn.
            studio.load_persisted();
            // After the settings, because a theme file overrides what they
            // chose, and before anything is drawn. `&mut` because the keymap is
            // resolved once and then read from every frame.
            studio.load_customisations();
            if let Some(root) = studio.root.peek() {
                studio.remember_workspace(&root);
            }
            // After the session, so a file the session already opened is
            // restored into the buffer it belongs to rather than a second copy
            // of it. Anything a previous run did not get to save comes back
            // here — dirty, and announced.
            studio.restore_recovered();
            // First launch, or a launch with nothing to come back to. See
            // `Studio::should_welcome` — a studio that restored a session
            // opens on the work.
            if studio.should_welcome() {
                studio.welcome.set(true);
            }
            *shared.borrow_mut() = Some(studio.clone());
            *quit_gate.borrow_mut() = Some(studio.clone());
            // **Render once, now.**
            //
            // The preview pane opened on "No render yet — press Render to
            // compile the buffer and mount it here" and stayed there until
            // somebody did. In the screencast this was written from, nobody
            // ever did: the sample buffer was emptied first, and from that
            // point on there was nothing that *could* render. A first-run user
            // never saw the one thing the application is for.
            //
            // The buffer a fresh studio opens with compiles and renders
            // (`buffer::SCRATCH`), and a folder opened from `argv[1]` puts a
            // real screen in the editor — so in both cases there is something
            // worth mounting before the first frame. `render` is asynchronous
            // and `poll_compile` above picks the result up, so this costs the
            // startup path nothing.
            //
            // Guarded on the toolchain because `render` with no `rustc` is a
            // refusal, and a refusal on launch would put an error in the
            // Problems panel of a studio nobody has asked anything of yet.
            if studio.toolchain.is_ok() {
                studio.render();
            }
            // Raise the splash. Nothing but "this is a launch": how far the
            // animation has got is the overlay's own business, and it takes
            // itself off the screen when it is done. Set after
            // `load_persisted`/`restore_recovered` so a restored session does
            // not race the splash's first frame.
            studio.splash.set(true);
            driver.set_root(Shell { studio });
        });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("vieww Studio failed to start: {error}"),
    }
}
