//! What `#[widget]` writes, checked against what a hand-written `impl Widget`
//! produces.
//!
//! The attribute's whole claim is that it is a *typing* shortcut and not a
//! semantic one: a widget written with it must be indistinguishable, to
//! everything downstream, from the same widget written out. So every test here
//! is a comparison against the long form rather than an assertion about the
//! attribute in isolation — a macro that generated something subtly different
//! would pass the second kind of test and fail these.

use vieww_widget::prelude::*;
use vieww_widget::{ElementState, WidgetKind};

// --- the long form, as the counter example writes it today -------------------

#[derive(Debug)]
struct Longhand {
    label: &'static str,
}

impl Widget for Longhand {
    fn debug_name(&self) -> &'static str {
        "Longhand"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new(self.label).into()
    }
}

widget_node_from!(Longhand);

// --- the same widget, with the attribute -------------------------------------

#[derive(Debug)]
struct Shorthand {
    label: &'static str,
}

#[widget]
impl Shorthand {
    fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
        Text::new(self.label)
    }
}

#[test]
fn the_attribute_names_the_widget_after_its_type() {
    let long = Longhand { label: "x" };
    let short = Shorthand { label: "x" };
    assert_eq!(long.debug_name(), "Longhand");
    assert_eq!(short.debug_name(), "Shorthand");
}

#[test]
fn the_attribute_assumes_composed() {
    let short = Shorthand { label: "x" };
    assert!(matches!(short.kind(), WidgetKind::Composed));
}

#[test]
fn the_attribute_writes_the_widgetnode_conversion() {
    // The `widget_node_from!` half. Without it this line does not compile,
    // which is the entire point of folding it in.
    let node = WidgetNode::from(Shorthand { label: "x" });
    assert_eq!(node.debug_name(), "Shorthand");
}

#[test]
fn both_forms_build_the_same_tree() {
    let long = vieww_widget::debug_tree(Longhand { label: "hello" });
    let short = vieww_widget::debug_tree(Shorthand { label: "hello" });
    // Only the root's own name differs; everything under it must match.
    let long_body = long.split_once('\n').map(|(_, rest)| rest);
    let short_body = short.split_once('\n').map(|(_, rest)| rest);
    assert_eq!(long_body, short_body, "{long}\n---\n{short}");
}

#[test]
fn a_build_returning_widgetnode_directly_still_works() {
    // `impl Into<WidgetNode>` is the ergonomic case; the trait's own
    // `WidgetNode` must keep working, through core's reflexive `From<T> for T`.
    #[derive(Debug)]
    struct Direct;

    #[widget]
    impl Direct {
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            Text::new("direct").into()
        }
    }

    assert_eq!(WidgetNode::from(Direct).debug_name(), "Direct");
}

#[test]
fn a_key_written_in_the_block_reaches_the_trait() {
    #[derive(Debug)]
    struct Keyed {
        key: Key,
    }

    #[widget]
    impl Keyed {
        fn key(&self) -> Option<&Key> {
            Some(&self.key)
        }

        fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
            Text::new("keyed")
        }
    }

    let keyed = Keyed {
        key: Key::from("row-3"),
    };
    assert_eq!(keyed.key(), Some(&Key::from("row-3")));
}

#[test]
fn a_written_debug_name_wins_over_the_generated_one() {
    #[derive(Debug)]
    struct Renamed;

    #[widget]
    impl Renamed {
        fn debug_name(&self) -> &'static str {
            "SomethingElse"
        }

        fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
            Text::new("x")
        }
    }

    assert_eq!(Renamed.debug_name(), "SomethingElse");
}

#[test]
fn helper_methods_stay_inherent() {
    #[derive(Debug)]
    struct WithHelpers {
        count: u32,
    }

    #[widget]
    impl WithHelpers {
        // Not a trait method: must stay callable as an ordinary constructor.
        fn new(count: u32) -> Self {
            Self { count }
        }

        fn doubled(&self) -> u32 {
            self.count * 2
        }

        fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
            Text::new(self.doubled().to_string())
        }
    }

    let widget = WithHelpers::new(21);
    assert_eq!(widget.doubled(), 42);
    assert_eq!(WidgetNode::from(widget).debug_name(), "WithHelpers");
}

#[test]
fn create_state_written_in_the_block_reaches_the_trait() {
    #[derive(Debug)]
    struct Counting;

    impl ElementState for Counting {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[derive(Debug)]
    struct Stateful;

    #[widget]
    impl Stateful {
        fn create_state(&self) -> Option<Box<dyn ElementState>> {
            Some(Box::new(Counting))
        }

        fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
            Text::new("stateful")
        }
    }

    // Reached through the trait, which is where the element tree asks.
    let as_widget: &dyn Widget = &Stateful;
    assert!(
        as_widget.create_state().is_some(),
        "create_state written in the block must land in the Widget impl, \
         not in an inherent one where the element tree cannot see it"
    );
}

#[test]
fn no_into_suppresses_the_generated_conversion() {
    #[derive(Debug)]
    struct Manual;

    #[widget(no_into)]
    impl Manual {
        fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
            Text::new("manual")
        }
    }

    // The attribute wrote no `From`, so this one is the only one and compiles.
    // Two would be E0119, which is what `no_into` exists to avoid for the
    // catalogue's own batch `widget_node_from!` calls.
    widget_node_from!(Manual);

    assert_eq!(WidgetNode::from(Manual).debug_name(), "Manual");
}

#[test]
fn a_written_kind_opts_out_of_composed() {
    // The escape hatch that lets a render or inherited widget use the
    // attribute for the naming half without being told it is composed.
    #[derive(Debug)]
    struct NotComposed;

    #[widget]
    impl NotComposed {
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::RenderLeaf
        }
    }

    assert!(matches!(NotComposed.kind(), WidgetKind::RenderLeaf));
    assert_eq!(NotComposed.debug_name(), "NotComposed");
}
