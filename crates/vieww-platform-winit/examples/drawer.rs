//! A navigation drawer, over the screen it belongs to.
//!
//! ```console
//! cargo run -p vieww-platform-winit --example drawer --release
//!
//! # the same tree read right-to-left, where the panel changes edge
//! cargo run -p vieww-platform-winit --example drawer --release -- --rtl
//! ```
//!
//! # What this is for, and why a test could not do it
//!
//! `Drawer`'s arithmetic is asserted in `vieww-widget/src/controls/drawer.rs` —
//! the panel is 304 wide or the room available minus a touch target, whichever
//! is less, and the scrim never falls below a finger at any device width. Those
//! are numbers, and numbers are exactly what a test is good at.
//!
//! **What a test cannot check is whether the thing reads as a drawer**, and that
//! is a whole-screen judgement made in about a second: a panel hard against one
//! edge, running the full height, with the screen behind it dimmed but plainly
//! still there. Every failure mode here is a correct number that looks wrong —
//! a panel that floats a pixel off the edge, a scrim so faint the screen behind
//! does not read as disabled, a panel that stops short of the bottom.
//!
//! # Three things to check
//!
//! 1. **Press *Open the menu*.** The panel arrives against the leading edge —
//!    left here, right under `--rtl` — full height, corner to corner.
//!
//! 2. **The strip of scrim down the far side is deliberate and is the point.**
//!    It is one touch target wide at minimum, because *the scrim is how you
//!    dismiss the drawer*. A drawer that reached the far edge would be a panel
//!    with no way out. On a window this wide the panel is at its 304 maximum and
//!    the strip is much wider than the minimum; drag the window narrow and watch
//!    the panel give up width rather than the strip.
//!
//!    **The panel and the page are the same colour**, and the scrim is the only
//!    thing dividing them — see [`BACKGROUND`]. If the drawer's edge is hard to
//!    find, that is the scrim being too weak rather than the panel being
//!    wrongly coloured, and `ModalBarrier::color` is where it lives.
//!
//! 3. **Press the scrim.** The drawer goes. That is `on_dismiss` popping the
//!    modal route, which is the whole reason a drawer is pushed as a route
//!    rather than dropped into a screen.
//!
//! The end drawer on the second button is the same widget with
//! `DrawerSide::End`, and it should land against the opposite edge in each
//! direction — four combinations from two buttons and one flag.
//!
//! # Run it in release
//!
//! Debug builds of the layout pass are ten to thirty times slower.

use std::rc::Rc;

use vieww_element::NavigatorController;
use vieww_foundation::{Color, Size, TextDirection};
use vieww_platform_winit::App;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
// `Drawer`, `DrawerSide` and `Route` come through the prelude; `Directionality`
// deliberately does not, so that a physical layout stays the default.
use vieww_widget::{widget_node_from, Directionality};

const SURFACE: Size = Size {
    width: 720.0,
    height: 560.0,
};

/// The window behind everything — **the theme's own surface, deliberately**.
///
/// The panel is `colors.surface` too, so the drawer and the page it covers are
/// exactly the same colour and **the scrim is the only thing separating them**.
/// That is the design being demonstrated rather than a shortcut: `Dialog` and
/// `BottomSheet` work the same way, and a drawer is the hardest case for it
/// because it sits flush against an edge with scrim on one side only.
///
/// An earlier version of this file picked its own near-black, nine points off
/// `surface`. That reads as a panel with a faint edge and flatters the widget —
/// it looked like separation the drawer had earned, when the drawer had earned
/// none of it. Taking it from the theme means this screen cannot drift from
/// what an application using `Theme` actually gets.
const BACKGROUND: Color = ThemeData::dark().colors.surface;

fn main() {
    let rtl = std::env::args().skip(1).any(|arg| arg == "--rtl");

    let app = App::new()
        .title("vieww — drawer")
        .size(SURFACE)
        .background(BACKGROUND);

    let result = app.run(move |driver: &mut FrameDriver| {
        let runtime = driver.elements().runtime().clone();
        let nav = NavigatorController::new(
            &runtime,
            Route::new("bootstrap", Rc::new(|_| SizedBox::shrink().into())),
        );
        nav.replace(Route::new("home", {
            let nav = nav.clone();
            Rc::new(move |ctx| page(&nav, ctx))
        }));
        // Without this a popped route is never retired, so the drawer would
        // linger after its own dismiss handler ran.
        nav.attach(driver.tickers());

        driver.set_root(Shell { nav, rtl });
    });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("drawer failed: {error}"),
    }
}

/// The screen the drawer opens over.
fn page(nav: &NavigatorController, ctx: &BuildContext) -> WidgetNode {
    let theme = ThemeData::of(ctx);

    let open = |nav: &NavigatorController, side: DrawerSide, name: &'static str| {
        let nav = nav.clone();
        move || {
            let popper = nav.clone();
            nav.push(Route::modal(
                name,
                Rc::new(move |ctx| {
                    let popper = popper.clone();
                    Drawer::new()
                        .side(side)
                        .on_dismiss(move || {
                            popper.pop();
                        })
                        .child(menu(ctx))
                        .into()
                }),
            ));
        }
    };

    Center::new()
        .child(
            Flex::column()
                .main_axis_size(MainAxisSize::Min)
                .spacing(16.0)
                .children(children![
                    Text::new("Press a button, then press the scrim to dismiss.")
                        .style(theme.text.body),
                    Button::new("Open the menu").on_pressed(open(
                        nav,
                        DrawerSide::Start,
                        "drawer-start"
                    )),
                    Button::new("Open the end drawer")
                        .style(ButtonStyle::Text)
                        .on_pressed(open(nav, DrawerSide::End, "drawer-end")),
                ]),
        )
        .into()
}

/// What is inside the panel. Ordinary widgets — the drawer is a container.
///
/// **Every string here takes its style from the theme's scale**, and that is not
/// tidiness. `Text` is a *render* widget: the factory reads its style straight
/// into a render object, so it never sees a `BuildContext` and never learns a
/// theme exists. Its colour therefore defaults to [`Color::BLACK`] — which on
/// this panel's near-black surface is an invisible menu, and is exactly what the
/// first screenshot of this example showed.
///
/// Taking `theme.text` also means the sizes come from the type scale rather than
/// from numbers chosen here, so the panel matches the rest of an application.
fn menu(ctx: &BuildContext) -> WidgetNode {
    let theme = ThemeData::of(ctx);
    let item = |label: &str| Text::new(label).style(theme.text.body);

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .main_axis_size(MainAxisSize::Min)
        .spacing(4.0)
        .children(children![
            Text::new("Mail").style(theme.text.title),
            SizedBox::height(8.0),
            item("Inbox"),
            item("Starred"),
            item("Snoozed"),
            item("Sent"),
            item("Drafts"),
        ])
        .into()
}

/// The root: a theme, a reading direction, and the navigator's stack.
#[derive(Debug)]
struct Shell {
    nav: NavigatorController,
    rtl: bool,
}

impl Widget for Shell {
    fn debug_name(&self) -> &'static str {
        "Shell"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let direction = if self.rtl {
            TextDirection::Rtl
        } else {
            TextDirection::Ltr
        };
        Theme::new(ThemeData::dark())
            .child(Directionality::new(direction).child(Navigator::new(self.nav.routes())))
            .into()
    }
}

widget_node_from!(Shell);
