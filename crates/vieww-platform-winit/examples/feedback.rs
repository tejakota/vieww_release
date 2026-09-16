//! Loading, empty, error and transient states — the screens an application is
//! in most of the time and that nobody ever looks at.
//!
//! ```console
//! cargo run -p vieww-platform-winit --example feedback --release
//! ```
//!
//! # What this is for
//!
//! `docs/AIMS.md` §J is about the accessibility of transient UI, and the
//! mechanism it describes is asserted in `crates/vieww/tests/announcements.rs`
//! by mounting a widget and asking what would be said. That is the right test
//! and it leaves the other half open: **nobody has watched a snackbar arrive.**
//!
//! The failures here are all of the same kind — a state that is *correct* and
//! reads as a bug. A skeleton whose blocks do not match the shape of the content
//! that replaces them makes the page jump. An empty state that looks like a
//! failed load. A snackbar that covers the button you just pressed.
//!
//! # Things to check
//!
//! 1. **Press *Load*.** The skeleton runs for a second and is replaced by the
//!    list. Watch the **left edge and the row heights**: if anything shifts when
//!    the real rows arrive, the skeleton is the wrong shape, and that shift is
//!    the entire reason skeletons exist rather than a spinner.
//!
//! 2. **Press *Fail* then *Empty*.** Three states from one switch, and the point
//!    is that they are visibly *different kinds of nothing* — an error offers a
//!    retry, an empty state offers an action, and a user must not have to read
//!    the text to tell which they are looking at.
//!
//! 3. **Press *Notify*.** The snackbar comes up at the bottom, over the content,
//!    and leaves. It is also `Custom("alert")` and therefore live, which is what
//!    `announcements.rs` asserts and what a screen reader would speak here.
//!
//! 4. **Hover or focus the tooltip target.** The tooltip is `Custom("tooltip")`
//!    and polite — the role fix recorded in `TRACKER.md` for 2026-08-16.
//!
//! 5. **The animated block never stops moving**, and the container morphs
//!    between two sizes and colours when toggled. Both are here so that a curve
//!    that looks wrong — a linear ease pretending to be an ease-out — is
//!    visible, which no test asserts.
//!
//! # Run it in release
//!
//! Debug builds of the layout pass are ten to thirty times slower.

use std::sync::Arc;
use std::time::Duration;

use vieww_element::{Runtime, Signal};
use vieww_foundation::task::{AsyncValue, FrameWaker, Spawn, Threads};
use vieww_foundation::Size;
use vieww_platform_winit::App;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

const SURFACE: Size = Size {
    width: 860.0,
    height: 700.0,
};

const BACKGROUND: Color = ThemeData::dark().colors.surface;

/// How long the fake load takes. Long enough to look at the skeleton, short
/// enough that nobody waits.
const LOAD: Duration = Duration::from_millis(1200);

fn main() {
    let app = App::new()
        .title("vieww — feedback")
        .size(SURFACE)
        .background(BACKGROUND);

    // Taken before `run` consumes the app, for `stream.rs`'s reason: the waker
    // has to be in hand to be passed into the tree that is about to be built.
    let waker = app.waker();

    let result = app.run(move |driver: &mut FrameDriver| {
        let runtime = driver.elements().runtime().clone();
        driver.set_root(Shell {
            state: State::new(&runtime),
            waker: Wake(Arc::new(waker)),
        });
    });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("feedback failed: {error}"),
    }
}

/// Which of the four content states the page is showing.
///
/// One enum rather than three booleans, for `lifecycle.rs`'s reason: the
/// question the page asks is *what am I showing*, and as separate flags that
/// becomes a conjunction each call site gets subtly wrong — the state where a
/// page is both loading and empty being the one nobody tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Loading,
    Loaded,
    Empty,
    Failed,
}

#[derive(Debug, Clone)]
struct State {
    phase: Signal<Phase>,
    notice: Signal<Option<String>>,
    expanded: Signal<bool>,
    ticks: Signal<u32>,
    /// Bumped on every *Load*, and used as the `AsyncBuilder`'s key.
    ///
    /// Without it a second press reconciles onto the same element, which keeps
    /// the finished task and shows the rows instantly — correct reconciliation
    /// behaviour, and not what the button says it does.
    generation: Signal<u32>,
}

impl State {
    fn new(runtime: &Runtime) -> Self {
        Self {
            phase: runtime.signal(Phase::Loaded),
            notice: runtime.signal(None),
            expanded: runtime.signal(false),
            ticks: runtime.signal(0_u32),
            generation: runtime.signal(0_u32),
        }
    }
}

/// The waker, wrapped so the widgets holding it can still derive `Debug`.
///
/// `FrameWaker` is deliberately not `Debug` — it is a handle into the event
/// loop, and there is nothing useful to print. The workspace lints require
/// `Debug` on public types, and every widget here is one, so the wrapper says
/// that once rather than each widget hand-writing an impl.
#[derive(Clone)]
struct Wake(Arc<dyn FrameWaker>);

impl std::fmt::Debug for Wake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Wake")
    }
}

#[derive(Debug)]
struct Shell {
    state: State,
    waker: Wake,
}

impl Widget for Shell {
    fn debug_name(&self) -> &'static str {
        "Shell"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // `Overlay` for `examples/controls`' reason: a tooltip and a menu are
        // decided deep in the tree and drawn at the top of it, and this is the
        // place that lets them. Inside `Theme`, so an entry is built with the
        // application's colours.
        Theme::new(ThemeData::dark())
            .child(Overlay::new().child(Body {
                state: self.state.clone(),
                waker: self.waker.clone(),
            }))
            .into()
    }
}

widget_node_from!(Shell);

#[derive(Debug)]
struct Body {
    state: State,
    waker: Wake,
}

impl Widget for Body {
    fn debug_name(&self) -> &'static str {
        "Body"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let content = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(20.0)
            .children(children![
                Controls {
                    state: self.state.clone()
                },
                Constrained::new(Constraints::tight(Size::new(420.0, 260.0))).child(Content {
                    state: self.state.clone(),
                    waker: self.waker.clone(),
                }),
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(24.0)
                    .children(children![
                        Confirmation::new("Changes saved"),
                        InlineError::new(icons::close(), "Could not save"),
                    ]),
                // Drag a file from the desktop over the window: the border
                // thickens and takes the accent while it is over, and lets go
                // again when it leaves. It reacts to the *window* because that
                // is all the platform reports — see `FileDropZone`'s docs.
                Constrained::new(Constraints::tight(Size::new(320.0, 96.0))).child(
                    FileDropZone::new()
                        .label("Attach a document")
                        .child(Text::new("Drop a file here").style(theme.text.label))
                ),
                Text::new("Motion").style(theme.text.title),
                Motion {
                    state: self.state.clone()
                },
            ]);

        // The snackbar is a sibling over the page rather than a child of it, so
        // that it covers content instead of pushing it — a snackbar that reflows
        // the page under the user's finger is the failure this arrangement
        // exists to avoid.
        let mut stack =
            Stack::new().children(children![Padding::new(EdgeInsets::all(28.0)).child(content)]);
        if let Some(message) = self.state.notice.get() {
            let clear = self.state.notice.clone();
            // **`push`, not `children`.** `Stack::children` *replaces* the
            // list, so the second call dropped the page and left a snackbar
            // alone on an empty screen — which is what this did until somebody
            // pressed the button and looked.
            stack = stack.push(Snackbar::new(message).action("Undo", move || clear.set(None)));
        }
        SafeArea::new().child(stack).into()
    }
}

widget_node_from!(Body);

/// The four buttons that drive the page, plus the tooltip target.
#[derive(Debug)]
struct Controls {
    state: State,
}

impl Widget for Controls {
    fn debug_name(&self) -> &'static str {
        "Controls"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let to = |state: &State, phase: Phase| {
            let signal = state.phase.clone();
            move || signal.set(phase)
        };
        let load = self.state.phase.clone();
        let generation = self.state.generation.clone();
        let notify = self.state.notice.clone();

        Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .spacing(12.0)
            .children(children![
                Button::new("Load").on_pressed(move || {
                    generation.set(generation.peek() + 1);
                    load.set(Phase::Loading);
                }),
                Button::new("Empty")
                    .style(ButtonStyle::Text)
                    .on_pressed(to(&self.state, Phase::Empty)),
                Button::new("Fail")
                    .style(ButtonStyle::Text)
                    .on_pressed(to(&self.state, Phase::Failed)),
                Button::new("Notify")
                    .style(ButtonStyle::Text)
                    .on_pressed(move || notify.set(Some("Message sent".to_owned()))),
                // A tooltip is a sibling of what it describes, anchored to it.
                // Left unanchored here so it takes its default placement, which
                // is the placement most applications get.
                Tooltip::new("This is a tooltip"),
                Text::new("← polite, and announced")
                    .style(theme.text.label)
                    .color(theme.colors.on_surface_variant),
            ])
            .into()
    }
}

widget_node_from!(Controls);

/// The four states of a list: loading, loaded, empty, failed.
#[derive(Debug)]
struct Content {
    state: State,
    waker: Wake,
}

impl Widget for Content {
    fn debug_name(&self) -> &'static str {
        "Content"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let phase = self.state.phase.get();

        match phase {
            // `AsyncBuilder` is what an application would really use here: it
            // owns the pending-to-ready transition rather than the page tracking
            // it by hand, and the skeleton is simply its pending arm. The work
            // runs on a worker thread and the waker turns its completion into a
            // frame — no polling anywhere.
            Phase::Loading => {
                let spawner: Arc<dyn Spawn> = Arc::new(Threads);
                let rendered = theme.clone();
                AsyncBuilder::new(
                    spawner,
                    Arc::clone(&self.waker.0),
                    move || {
                        std::thread::sleep(LOAD);
                        Ok::<(), ()>(())
                    },
                    move |value: &AsyncValue<(), ()>| match value {
                        AsyncValue::Pending => skeleton(&rendered),
                        _ => rows(&rendered),
                    },
                )
                .key(i64::from(self.state.generation.get()))
                .into()
            }
            Phase::Loaded => rows(&theme),
            Phase::Empty => EmptyState::new(icons::add(), "No messages yet")
                .description("Anything you send will show up here.")
                .action("Compose", || {})
                .into(),
            Phase::Failed => InlineError::new(icons::close(), "Could not reach the server")
                .retry({
                    let retry = self.state.phase.clone();
                    move || retry.set(Phase::Loading)
                })
                .into(),
        }
    }
}

widget_node_from!(Content);

/// The loading placeholder — **the same shape as [`rows`]**, deliberately.
///
/// That correspondence is the whole contract of a skeleton and is the thing to
/// check by eye: three rows, a circle and two bars each, at the row height the
/// real content uses. Everything that makes skeletons worth having depends on
/// the page not moving when the real rows arrive, and nothing enforces it — the
/// two functions are next to each other so that a change to one is obvious in
/// the other.
fn skeleton(theme: &ThemeData) -> WidgetNode {
    let line = || {
        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .spacing(12.0)
            .children(children![
                Skeleton::circle(36.0),
                Flexible::new(1).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(6.0)
                        .children(children![
                            Skeleton::text().width(180.0),
                            Skeleton::text().width(120.0).height(10.0),
                        ])
                ),
            ])
    };
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .spacing(theme.metrics.gap)
        .children(children![line(), line(), line()])
        .into()
}

/// What the skeleton is standing in for. See [`skeleton`].
fn rows(theme: &ThemeData) -> WidgetNode {
    let line = |name: &str, body: &str| {
        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .spacing(12.0)
            .children(children![
                Avatar::initials(name),
                Flexible::new(1).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(6.0)
                        .children(children![
                            Text::new(name).style(theme.text.body),
                            Text::new(body)
                                .style(theme.text.label)
                                .color(theme.colors.on_surface_variant),
                        ])
                ),
            ])
    };
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .spacing(theme.metrics.gap)
        .children(children![
            line("Ada Lovelace", "Re: the analytical engine"),
            line("Grace Hopper", "Found the bug"),
            line("Alan Turing", "On computable numbers"),
        ])
        .into()
}

/// `Animated`, `AnimatedContainer` and `Offstage`, which are only judgeable in
/// motion.
#[derive(Debug)]
struct Motion {
    state: State,
}

impl Widget for Motion {
    fn debug_name(&self) -> &'static str {
        "Motion"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let expanded = self.state.expanded.get();
        let toggle = self.state.expanded.clone();
        let ticks = self.state.ticks.get();
        let bump = self.state.ticks.clone();

        Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .spacing(20.0)
            .children(children![
                // Morphs between two sizes and two colours on the same curve.
                AnimatedContainer::new()
                    .duration(Duration::from_millis(320))
                    .curve(Curve::EASE_OUT)
                    .color(if expanded {
                        theme.colors.primary
                    } else {
                        theme.colors.surface_variant
                    })
                    .size(if expanded { 160.0 } else { 80.0 }, 80.0),
                // A raw `Animated`: the builder is handed the value, so the
                // interpolation is visible rather than implied.
                Animated::new(if expanded { 1.0 } else { 0.0 })
                    .duration(Duration::from_millis(320))
                    .curve(Curve::EASE_IN_OUT)
                    .build(move |t| {
                        Opacity::new(0.25 + 0.75 * t)
                            .child(
                                SizedBox::from_size(Size::new(80.0, 80.0))
                                    .child(ColoredBox::new(ThemeData::dark().colors.error)),
                            )
                            .into()
                    }),
                Button::new(if expanded { "Collapse" } else { "Expand" })
                    .on_pressed(move || toggle.set(!expanded)),
                // `Offstage` keeps its child in the tree and off the screen —
                // not laid out, not painted, not hit tested. The counter proves
                // the subtree is still *there*: it keeps its state across being
                // hidden, which a subtree that had been unmounted would not.
                Offstage::new(!expanded).child(
                    Button::new(format!("Counted {ticks}"))
                        .style(ButtonStyle::Text)
                        .on_pressed(move || bump.set(bump.peek() + 1))
                ),
            ])
            .into()
    }
}

widget_node_from!(Motion);
