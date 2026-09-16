//! Phase 1 exit test: build a static widget tree in memory, with no window, and
//! confirm its structure is what was written.
//!
//! These also pin the two things Phase 2 will build directly on top of:
//! `can_update` identity, and the fact that a rebuild returning a cloned subtree
//! is pointer-identical.

use std::rc::Rc;

use vieww_widget::prelude::*;
use vieww_widget::{inflate, Inherited};

#[derive(Debug, Clone, PartialEq)]
struct Theme {
    accent: Color,
    gutter: f32,
}

impl Theme {
    fn dark() -> Self {
        Self {
            accent: Color::hex(0x8AB4F8),
            gutter: 12.0,
        }
    }
}

/// A composed widget that reads inherited state, as application code does.
#[derive(Debug)]
struct Badge {
    label: String,
}

impl Badge {
    fn new(label: &str) -> Self {
        Self {
            label: label.to_owned(),
        }
    }
}

impl Widget for Badge {
    fn debug_name(&self) -> &'static str {
        "Badge"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ctx.inherit_or(Theme::dark());
        Container::new()
            .padding(EdgeInsets::all(theme.gutter))
            .color(theme.accent)
            .child(Text::new(self.label.as_str()))
            .into()
    }
}

widget_node_from!(Badge);

fn sample_tree() -> WidgetNode {
    Inherited::new(
        Theme::dark(),
        Flex::column().children(children![
            Text::new("Header").bold(),
            Flex::row().children(children![
                Badge::new("one"),
                Badge::new("two"),
                Badge::new("three"),
            ]),
            Center::new().child(Text::new("Footer")),
        ]),
    )
    .into()
}

#[test]
fn the_declared_structure_survives_inflation() {
    let tree = inflate(sample_tree());

    assert_eq!(tree.name, "Inherited<Theme>");
    assert_eq!(tree.kind, "inherited");

    let column = &tree.children[0];
    assert_eq!(column.name, "Column");
    assert_eq!(column.children.len(), 3, "Column declared three children");

    let row = &column.children[1];
    assert_eq!(row.name, "Row");
    assert_eq!(row.children.len(), 3);

    let labels: Vec<&str> = tree
        .find_all("Text")
        .iter()
        .filter_map(|node| node.property("text"))
        .collect();
    assert_eq!(
        labels,
        [
            r#""Header""#,
            r#""one""#,
            r#""two""#,
            r#""three""#,
            r#""Footer""#
        ],
        "texts should appear in declaration order, depth-first"
    );
}

#[test]
fn composed_widgets_show_both_themselves_and_what_they_built() {
    let tree = inflate(sample_tree());
    let badge = tree.find("Badge").expect("Badge should be in the tree");

    assert_eq!(
        badge.spine(),
        ["Badge", "Container", "ColoredBox", "Padding", "Text"],
        "the composed widget stays visible above its expansion"
    );
}

#[test]
fn inherited_values_reach_descendants_without_being_threaded_through() {
    let tree = inflate(sample_tree());
    let theme = Theme::dark();

    // `Badge` never received a Theme argument; it read one from the context.
    let colored = tree.find("ColoredBox").expect("Badge built a ColoredBox");
    assert_eq!(
        colored.property("color"),
        Some(theme.accent.to_string()).as_deref()
    );

    let padding = tree.find("Padding").expect("Badge built a Padding");
    assert_eq!(padding.property("insets"), Some("all(12)"));
}

#[test]
fn an_inner_provider_shadows_an_outer_one() {
    let inner = Theme {
        accent: Color::RED,
        gutter: 2.0,
    };
    let tree = inflate(Inherited::new(
        Theme::dark(),
        Inherited::new(inner.clone(), Badge::new("shadowed")),
    ));

    let padding = tree.find("Padding").unwrap();
    assert_eq!(padding.property("insets"), Some("all(2)"));
    assert_eq!(
        tree.find("ColoredBox").unwrap().property("color"),
        Some(inner.accent.to_string()).as_deref()
    );
}

#[test]
fn a_widget_with_no_provider_falls_back_rather_than_panicking() {
    let tree = inflate(Badge::new("standalone"));
    assert_eq!(
        tree.find("Padding").unwrap().property("insets"),
        Some("all(12)")
    );
}

#[test]
fn elements_are_reusable_only_for_the_same_type_and_key() {
    let plain = WidgetNode::new(Text::new("a"));
    let other_text = WidgetNode::new(Text::new("b"));
    let keyed = WidgetNode::new(Text::new("a").key("id"));
    let same_key = WidgetNode::new(Text::new("different").key("id"));
    let other_key = WidgetNode::new(Text::new("a").key("elsewhere"));
    let other_type = WidgetNode::new(Padding::all(1.0));

    // Content is irrelevant to identity — only type and key decide.
    assert!(plain.can_update(&other_text));
    assert!(keyed.can_update(&same_key));

    assert!(!plain.can_update(&keyed), "keyed vs unkeyed must not match");
    assert!(!keyed.can_update(&other_key));
    assert!(!plain.can_update(&other_type));
}

#[test]
fn a_fresh_unique_key_forces_a_remount() {
    let before = WidgetNode::new(Text::new("same").key(Key::unique()));
    let after = WidgetNode::new(Text::new("same").key(Key::unique()));
    assert!(!before.can_update(&after));
}

#[test]
fn cloning_a_subtree_shares_it_rather_than_copying() {
    let shared = WidgetNode::new(Text::new("expensive"));
    let left = Flex::row().push(shared.clone());
    let right = Flex::row().push(shared.clone());

    let (left_child, right_child) = (
        left.kind().children()[0].clone(),
        right.kind().children()[0].clone(),
    );

    // The pointer-equality early-out Phase 2 reconciliation relies on: a
    // rebuild that hands back the same node cannot have changed it.
    assert!(left_child.ptr_eq(&right_child));
    assert!(left_child.ptr_eq(&shared));
    assert!(!left_child.ptr_eq(&WidgetNode::new(Text::new("expensive"))));
}

#[test]
fn a_shared_inherited_value_is_not_duplicated_per_provider() {
    let theme = Rc::new(Theme::dark());
    let tree = WidgetNode::from(Inherited::shared(
        Rc::clone(&theme),
        Inherited::shared(Rc::clone(&theme), Badge::new("x")),
    ));

    // Two providers hold two handles to one allocation, not two themes.
    assert_eq!(Rc::strong_count(&theme), 3);

    let inflated = inflate(tree.clone());
    assert!(inflated.find("ColoredBox").is_some());

    // Walking the tree publishes into scopes that are dropped with the walk,
    // so a build leaves no residual references behind.
    assert_eq!(Rc::strong_count(&theme), 3);
}

#[test]
fn downcasting_recovers_the_concrete_widget() {
    let node = WidgetNode::new(Text::new("hello").size(22.0));

    let text = node.downcast_ref::<Text>().expect("node holds a Text");
    assert_eq!(text.data(), "hello");
    assert_eq!(text.text_style().size, 22.0);

    assert!(node.downcast_ref::<Padding>().is_none());
}

#[test]
fn the_printed_tree_is_stable() {
    let dump = vieww_widget::debug_tree(Flex::column().children(children![
        Text::new("a"),
        Padding::all(4.0).child(Text::new("b")),
    ]));

    assert_eq!(
        dump,
        "\
Column
├─ Text (text: \"a\")
└─ Padding (insets: all(4))
   └─ Text (text: \"b\")"
    );
}
