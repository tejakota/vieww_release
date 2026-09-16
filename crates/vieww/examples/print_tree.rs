//! Phase 1 exit test: build a widget tree in memory, with no window and no GPU,
//! and print it.
//!
//! ```text
//! cargo run -p vieww --example print_tree
//! ```
//!
//! What to look for in the output:
//!
//! - `Container` and `ProfileCard` are composed, so each shows the subtree it
//!   built underneath it — you see both what was written and what it expanded to.
//! - The `Theme` published at the root is read inside `ProfileCard::build`
//!   without being threaded through any intermediate widget.
//! - Nothing has a size or a position. Layout is Phase 3.

use std::rc::Rc;

use vieww::prelude::*;
use vieww::{BuildContext, Widget, WidgetKind, WidgetNode};

/// App-wide styling, published once at the root and read wherever it is needed.
#[derive(Debug, Clone)]
struct Theme {
    surface: Color,
    on_surface: Color,
    accent: Color,
    gutter: f32,
}

impl Theme {
    fn light() -> Self {
        Self {
            surface: Color::hex(0xFFFFFF),
            on_surface: Color::hex(0x1A1A1A),
            accent: Color::hex(0x3B6EF5),
            gutter: 16.0,
        }
    }

    /// Read the nearest theme, falling back to light so a widget can be
    /// rendered in a test without standing up a whole app.
    fn of(ctx: &BuildContext) -> Rc<Self> {
        ctx.inherit_or(Self::light())
    }
}

/// A user-defined composed widget — the shape most application code takes.
#[derive(Debug)]
struct ProfileCard {
    name: String,
    role: String,
    key: Option<Key>,
}

impl ProfileCard {
    fn new(name: &str, role: &str) -> Self {
        Self {
            name: name.to_owned(),
            role: role.to_owned(),
            key: None,
        }
    }

    fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for ProfileCard {
    fn debug_name(&self) -> &'static str {
        "ProfileCard"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = Theme::of(ctx);

        Container::new()
            .padding(EdgeInsets::all(theme.gutter))
            .color(theme.surface)
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .children(children![
                        // Avatar placeholder: a fixed square of accent color.
                        Container::new().size(48.0, 48.0).color(theme.accent),
                        SizedBox::width(theme.gutter),
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .children(children![
                                Text::new(self.name.as_str())
                                    .size(16.0)
                                    .bold()
                                    .color(theme.on_surface),
                                SizedBox::height(4.0),
                                Text::new(self.role.as_str()).size(13.0).color(theme.accent),
                            ]),
                    ]),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("name", self.name.clone())]
    }
}

widget_node_from!(ProfileCard);

fn app() -> impl Into<WidgetNode> {
    Inherited::new(
        Theme::light(),
        Flex::column().children(children![
            Container::new()
                .padding(EdgeInsets::symmetric(16.0, 12.0))
                .child(Text::new("Contributors").size(20.0).bold()),
            ProfileCard::new("Ada Lovelace", "Analyst").key("ada"),
            ProfileCard::new("Grace Hopper", "Rear Admiral").key("grace"),
            Center::new().child(Text::new("2 of 2").size(12.0)),
        ]),
    )
}

fn main() {
    let tree = vieww::inflate(app());

    println!("{tree}\n");
    println!("{} nodes, {} deep", tree.len(), max_depth(&tree));
}

fn max_depth(node: &vieww::DebugNode) -> usize {
    node.iter().map(|n| n.depth).max().unwrap_or(0)
}
