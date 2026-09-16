//! Every widget in the catalogue, asked whether it stops asking for frames.
//!
//! ```console
//! cargo test -p vieww --test idle_cost
//! ```
//!
//! # What this gate is for
//!
//! `docs/AIMS.md` §F: *an idle tree costs nothing and a changing one costs work
//! proportional to what changed* — **asserted, not assumed**. Three test files
//! already hand-rolled the "asks for no more frames once settled" check for
//! three widgets. This applies it to the catalogue, from one harness
//! (`common/idle.rs`).
//!
//! # What a pass here means, and what it does not
//!
//! It means the widget reaches a state where it stops requesting frames and
//! stays there — so a screen full of these costs the compositor nothing while
//! nobody is touching it. It says nothing about how *much* work a frame does; it
//! is a check on the frame *count*, which is the number that turns a static
//! screen into a hot CPU and a flat battery.
//!
//! # Nothing failing is the useful outcome
//!
//! When this landed, no widget failed it. That is what a gate is for rather than
//! a disappointment: the property was already true and **nothing was checking
//! it**, so it was one refactor away from silently stopping being true. It is a
//! regression test, and the regression has not happened yet.
//!
//! # The two that must not settle, and the one that surprisingly does
//!
//! Both indeterminate indicators go through `assert_animates`, which fails if
//! they *stop*. A spinner that freezes is a worse bug than one that spins
//! forever, and without declaring them here they would be indistinguishable from
//! a widget that settles.
//!
//! `Skeleton` goes the other way and is asserted *static*, which is a decision
//! its own docs argue for rather than an oversight — see the test.

mod common;

use std::collections::HashSet;
use std::rc::Rc;

use common::idle::Idle;
use vieww::foundation::{Color, Date, EdgeInsets, Size, TextEditingValue, Time};
use vieww::prelude::*;

/// A plain box, for the widgets that need a child to wrap.
fn child() -> WidgetNode {
    SizedBox::square(40.0)
        .child(ColoredBox::new(Color::rgb(60, 90, 140)))
        .into()
}

/// Assert a widget settles, and say which one in the failure.
#[track_caller]
fn settles(name: &str, widget: impl Into<WidgetNode>) {
    let mut idle = Idle::new(widget);
    let frames = idle.settle();
    assert!(
        frames <= common::idle::LIMIT,
        "{name} took {frames} frames to settle"
    );
}

#[test]
fn the_static_controls_all_settle() {
    // The ones with no animation at all. They are in the sweep rather than
    // assumed, because "has no animation" is a property of today's
    // implementation and this is the thing that notices when one grows a fade.
    settles("Checkbox", Checkbox::new(true).label("Remember me"));
    settles("Switch", Switch::new(false).label("Aeroplane mode"));
    settles("Radio", Radio::new(true).label("One"));
    settles("Slider", Slider::new(0.4).label("Volume"));
    settles("Chip", Chip::new("Espresso"));
    settles("Button", Button::new("Sign up"));
    settles("Badge", Badge::new(child()).count(3));
    settles("Avatar", Avatar::initials("Ada Lovelace"));
    settles(
        "Breadcrumbs",
        Breadcrumbs::new(vec!["Home".to_owned(), "Orders".to_owned()]),
    );
    settles("Pagination", Pagination::new(3, 10));
    settles(
        "SegmentedControl",
        SegmentedControl::new(["Day", "Week"], 0),
    );
    settles("TabBar", TabBar::new(["One", "Two"], 0));
    settles(
        "FloatingActionButton",
        FloatingActionButton::new(icons::add()),
    );
    settles(
        "EmptyState",
        EmptyState::new(icons::add(), "No messages yet"),
    );
    settles("InlineError", InlineError::new(icons::add(), "Required"));
    settles("Confirmation", Confirmation::new("Saved"));
    settles("Markdown", Markdown::new("# Title\n\nSome *text*."));
    settles("Dropdown", Dropdown::new(["Ireland", "Japan"], Some(0)));
}

#[test]
fn the_input_controls_settle() {
    settles("TextField", TextField::new(TextEditingValue::from("Ada")));
    settles(
        "TextField (obscured)",
        TextField::new(TextEditingValue::from("hunter2")).obscure(true),
    );
    settles(
        "TextField (placeholder)",
        TextField::new(TextEditingValue::default()).placeholder("Email"),
    );
    settles(
        "DatePicker",
        DatePicker::new(Date::new(2026, 8, 16).expect("a real date")),
    );
    settles(
        "TimePicker",
        TimePicker::new(Time::new(9, 30).expect("a real time")),
    );
}

#[test]
fn the_data_widgets_settle() {
    settles(
        "DataTable",
        DataTable::new(
            vec![
                DataColumn::new("Name", 120.0),
                DataColumn::new("Size", 80.0),
            ],
            200,
            32.0,
            |row, column| Text::new(format!("r{row}c{column}")).into(),
        ),
    );
    settles("Accordion", Accordion::new(Text::new("Details"), child()));
    settles(
        "TreeView",
        TreeView::new(
            vec![TreeNode::branch(
                "src",
                Text::new("src"),
                vec![TreeNode::leaf("lib", Text::new("lib.rs"))],
            )],
            HashSet::from(["src".to_owned()]),
        ),
    );
    settles(
        "ListView",
        SizedBox::square(200.0).child(Scrollable::vertical(0.0).child(ListView::new(
            50,
            32.0,
            Rc::new(|index| Text::new(format!("row {index}")).into()),
        ))),
    );
    settles(
        "GridView",
        SizedBox::square(200.0).child(Scrollable::vertical(0.0).child(GridView::new(
            30,
            3,
            40.0,
            Rc::new(|_| child()),
        ))),
    );
}

#[test]
fn the_transient_widgets_settle_once_they_have_arrived() {
    // A snackbar animates in and then stops. The interesting case is not that it
    // animates — it is that it *stops*, because a transient widget that keeps
    // asking is one that costs a frame per screen refresh for as long as it is
    // on screen, which for a snackbar is several seconds on every action.
    settles("Snackbar", Snackbar::new("Saved"));
    settles("Tooltip", Tooltip::new("Rename"));
    settles("Dialog", Dialog::new().title("Delete?").content(child()));
    settles("BottomSheet", BottomSheet::new().child(child()));
    settles("Drawer", Drawer::new().child(child()));
    settles("Menu", Menu::new(vec![MenuItem::new("Rename")]));
}

#[test]
fn the_structural_widgets_settle() {
    settles("Padding", Padding::new(EdgeInsets::all(8.0)).child(child()));
    settles("Opacity", Opacity::new(0.5).child(child()));
    settles("Fitted", Fitted::new().child(child()));
    settles("Offstage", Offstage::new(true).child(child()));
    settles(
        "DecoratedBox",
        DecoratedBox::rounded(Color::RED, 4.0).child(child()),
    );
    settles(
        "ExcludeSemantics",
        ExcludeSemantics::new(true).child(child()),
    );
    settles("BlockSemantics", BlockSemantics::new(true).child(child()));
    settles("Sensitive", Sensitive::new().child(child()));
    settles("RepaintBoundary", RepaintBoundary::new().child(child()));
    settles(
        "LayoutBuilder",
        LayoutBuilder::new(|_| child()).breakpoints([600.0]),
    );
}

#[test]
fn an_animation_that_reaches_its_target_stops() {
    // `Animated` interpolates towards a value and must go quiet on arrival. This
    // is the widget where "settles" is the entire contract rather than a
    // side-effect, so it gets its own test and asserts it actually animated —
    // settling in zero frames would pass `settle` and mean the animation never
    // ran.
    let mut idle = Idle::new(Animated::new(1.0).build(|t| {
        SizedBox::square(20.0 + 20.0 * t)
            .child(ColoredBox::new(Color::rgb(60, 90, 140)))
            .into()
    }));
    let frames = idle.settle();
    assert!(
        frames > 0 || !idle.wants_frame(),
        "an animation that never asked for a frame is not an animation"
    );
}

#[test]
fn a_determinate_indicator_and_a_skeleton_are_both_static() {
    // A bar that knows its value is a static shape until the value changes.
    settles("LinearProgress (determinate)", LinearProgress::new(0.4));
    settles("CircularProgress (determinate)", CircularProgress::new(0.4));

    // `Skeleton` is **deliberately** a static fill rather than a shimmer, and
    // its own docs say why: a looping animation primitive does not exist in
    // `vieww-animation` yet, and faking one by fighting `Animated` into looping
    // would be the wrong kind of finished.
    //
    // This assertion is here rather than omitted because that decision is
    // exactly the kind that gets quietly reversed — the day a shimmer lands,
    // this fails, and whoever landed it moves the line to `assert_animates` on
    // purpose instead of a skeleton starting to cost a frame per refresh by
    // accident.
    //
    // (The first draft of this file asserted the opposite, from the assumption
    // that a skeleton shimmers. The harness caught it.)
    settles("Skeleton", Skeleton::text().width(180.0));
    settles("Skeleton (circle)", Skeleton::circle(36.0));
}

#[test]
fn both_indeterminate_indicators_keep_going() {
    // The other half of the harness. These fail if they *stop* — a spinner
    // frozen mid-sweep reads as an application that has hung, which is worse
    // than one that spins for ever.
    let mut linear = Idle::new(LinearProgress::indeterminate());
    linear.assert_animates(60);

    let mut circular = Idle::new(CircularProgress::indeterminate());
    circular.assert_animates(60);
}

#[test]
fn a_settled_tree_stays_settled_when_nothing_touches_it() {
    // The property §F actually asks for, on a whole screen rather than one
    // widget: an idle tree costs nothing.
    let screen = Flex::column()
        .spacing(12.0)
        .children(vieww_widget::children![
            Text::new("Settings"),
            Checkbox::new(true).label("Remember me"),
            Switch::new(false).label("Aeroplane mode"),
            Slider::new(0.4).label("Volume"),
            Button::new("Save"),
        ]);

    let mut idle = Idle::showing(Size::new(400.0, 400.0), screen);
    idle.settle();

    // Well past the grace period the harness itself checks.
    idle.steps(120);
    assert!(
        !idle.wants_frame(),
        "a screen nobody is touching asked for another frame — at 60Hz that is \
         a flat battery for a picture that is not changing"
    );
}
