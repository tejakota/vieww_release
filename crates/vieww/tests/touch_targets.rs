//! Touch-target minimum, as a **test failure** rather than a design-review note.
//!
//! ```console
//! cargo test -p vieww --test touch_targets
//! ```
//!
//! # What `docs/AIMS.md` asks for
//!
//! Every platform's own guidance sets a floor on how small a tappable thing may
//! be — 48dp on Android, 44pt on iOS, both already encoded as
//! [`Metrics::touch_target`](vieww::widget::Metrics::touch_target) and used by
//! four controls (`Button`, `Checkbox`, `Radio`, `Switch`) through the private
//! `touch_target()` helper in `vieww-widget`. Nothing enforced that every other
//! interactive control also cleared it — a regression there was a visual
//! nitpick nobody would catch without a ruler held up to a screenshot.
//!
//! # Why this is one end-to-end test and not a helper called from unit tests
//!
//! `inflate()` (widget-tree-shape only) cannot answer this at all — the
//! question is about laid-out pixels, and pixels only exist after a real
//! `FrameDriver` has run layout. So this builds a gallery of every interactive
//! built-in control side by side, draws one real frame, and reads their laid-out
//! [`SemanticsNode::bounds`] back — the same route [`keys_to_semantics`] uses to
//! avoid encoding layout by hand. A control that shrinks below the platform
//! minimum fails this test the moment it does, with no screenshot required.

use std::rc::Rc;

use vieww::foundation::{IconData, Size};
use vieww::prelude::*;
use vieww::{FrameDriver, Role, SemanticsNode};

const SURFACE: Size = Size {
    width: 400.0,
    height: 3000.0,
};

/// An icon good for nothing but occupying an `IconData` slot — its shape is
/// never asserted on, only the box the framework puts around it.
fn glyph() -> IconData {
    icons::add()
}

/// Every built-in interactive control, together, each with a handler set so it
/// reads as enabled — a disabled control is exempt from the minimum, since
/// nothing can mis-tap something that does not respond.
#[derive(Debug)]
struct Gallery;

impl Widget for Gallery {
    fn debug_name(&self) -> &'static str {
        "Gallery"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Theme::new(ThemeData::light())
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .children(children![
                        Button::new("Button").on_pressed(|| {}),
                        FloatingActionButton::new(glyph()).on_pressed(|| {}),
                        Checkbox::new(false).on_changed(Rc::new(|_| {})),
                        Radio::new(false).on_selected(Rc::new(|()| {})),
                        Switch::new(false).on_changed(Rc::new(|_| {})),
                        // `Chip` is deliberately excluded: its own doc comment
                        // (`controls/chip.rs`, on `body()`) documents trading
                        // reach for density as the entire point of the control —
                        // "a row of chips at finger height would be a row of
                        // buttons" — so it is a considered exception to the
                        // platform minimum, not a control this test should
                        // hold to it.
                        Pagination::new(0, 5).on_page_selected(|_| {}),
                        Breadcrumbs::new(vec!["One".into(), "Two".into()]).on_selected(|_| {}),
                        Accordion::new(Text::new("Header"), Text::new("Body")).on_toggled(|_| {}),
                        TabBar::new(vec!["A".to_string(), "B".to_string()], 0)
                            .on_selected(Rc::new(|_| {})),
                        SegmentedControl::new(vec!["A".to_string(), "B".to_string()], 0)
                            .on_selected(Rc::new(|_| {})),
                        BottomNavigation::new(
                            vec![
                                BottomNavItem::new(glyph(), "One"),
                                BottomNavItem::new(glyph(), "Two"),
                            ],
                            0
                        )
                        .on_selected(Rc::new(|_| {})),
                    ]),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(Gallery);

/// Roles a screen reader would let a user tap, drag, or otherwise activate —
/// the set the platform minimum actually protects. [`Role::Label`] and
/// [`Role::ScrollView`] are deliberately excluded: neither is something a
/// fingertip is asked to land on precisely.
fn is_interactive(role: Role) -> bool {
    matches!(
        role,
        Role::Button | Role::CheckBox | Role::Radio | Role::Switch | Role::Tab | Role::Slider
    )
}

fn undersized(nodes: &[SemanticsNode], minimum: f32) -> Vec<&SemanticsNode> {
    nodes
        .iter()
        .filter(|node| node.enabled && is_interactive(node.role))
        .filter(|node| node.bounds.width() < minimum - 0.5 || node.bounds.height() < minimum - 0.5)
        .collect()
}

#[test]
fn every_enabled_interactive_control_clears_its_platform_minimum() {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Gallery);
    driver.draw_frame();

    let minimum = ThemeData::light().metrics.touch_target;
    let semantics = driver.semantics();
    let nodes = semantics.nodes();

    let interactive = nodes
        .iter()
        .filter(|node| node.enabled && is_interactive(node.role))
        .count();
    assert!(
        interactive >= 8,
        "the gallery is supposed to put at least eight interactive controls on \
         screen at once; got {interactive} — a control that stopped declaring \
         semantics would silently drop out of this test rather than fail it, so \
         this is the tripwire for that: {}",
        semantics.describe()
    );

    let failures = undersized(nodes, minimum);
    assert!(
        failures.is_empty(),
        "{} control(s) are smaller than the platform's {minimum}px minimum on at \
         least one axis:\n{}",
        failures.len(),
        failures
            .iter()
            .map(|node| format!(
                "  {:?} {:?}: {:.1}x{:.1}",
                node.role,
                node.label,
                node.bounds.width(),
                node.bounds.height()
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// A control nobody in this repository wrote, deliberately built too small.
///
/// Twenty points square, with a tap handler and nothing else — exactly what a
/// third party gets when they reach for `GestureDetector`, and exactly the case
/// the old opt-in helper could not reach.
#[derive(Debug)]
struct ThirdPartyControl;

impl Widget for ThirdPartyControl {
    fn debug_name(&self) -> &'static str {
        "ThirdPartyControl"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Declares itself, the way any control meant to be usable would — which
        // is also what puts its laid-out box in reach of the assertion below.
        Semantics::container("Tiny")
            .role(SemanticRole::Button)
            .child(
                GestureDetector::new()
                    .on_tap(|_| {})
                    .child(SizedBox::from_size(Size::new(20.0, 20.0))),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(ThirdPartyControl);

/// The small control, centred in a surface much larger than it.
#[derive(Debug)]
struct Centred;

impl Widget for Centred {
    fn debug_name(&self) -> &'static str {
        "Centred"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Theme::new(ThemeData::light())
            .child(Center::new().child(ThirdPartyControl))
            .into()
    }
}

vieww::widget::widget_node_from!(Centred);

#[test]
fn a_control_this_repository_did_not_write_is_reachable_without_asking() {
    // **The other half of `docs/AIMS.md` §I, and the half that was still open.**
    //
    // The test above proves the *built-in* controls clear the minimum, and they
    // did that by each calling a private helper by hand. A third party building
    // on `GestureDetector` got nothing from that arrangement — which made the
    // green suite above a statement about this repository's diligence rather
    // than about the framework.
    //
    // Here the control is 20x20, asks for nothing, and is tapped 8pt outside
    // itself. It has to receive that tap.
    let surface = Size::new(200.0, 200.0);
    let mut driver = FrameDriver::new(surface);
    driver.set_root(Centred);
    driver.draw_frame();

    let centre = Offset::new(surface.width / 2.0, surface.height / 2.0);
    let inside = driver.hit_test(centre);
    assert!(
        !inside.is_empty(),
        "the control does not take a tap on itself, so this test proves nothing"
    );

    // 18pt from the centre is 8pt outside a 20x20 box, and well inside the
    // 48x48 the theme asks for.
    let outside = driver.hit_test(centre + Offset::new(18.0, 18.0));
    assert!(
        !outside.is_empty(),
        "a 20x20 third-party control did not take a tap 8pt outside itself — \
         touch-target expansion is still opt-in"
    );

    // And the pixels did not move: the control still lays out at 20x20, so the
    // expansion bought reach and cost nothing visible. This is the assertion
    // that distinguishes it from "make small controls bigger".
    let semantics = driver.semantics();
    let control = semantics
        .nodes()
        .iter()
        .find(|node| node.label.as_deref() == Some("Tiny"))
        .expect("the third-party control declares itself");
    assert_eq!(
        (control.bounds.width(), control.bounds.height()),
        (20.0, 20.0),
        "the control's own box grew, which is a visual regression rather than an \
         accessibility fix"
    );

    // Far enough out to be past even the expanded area.
    let far = driver.hit_test(centre + Offset::new(40.0, 40.0));
    assert!(
        far.is_empty(),
        "the expansion is unbounded — a tap 40pt away should reach nothing"
    );
}
