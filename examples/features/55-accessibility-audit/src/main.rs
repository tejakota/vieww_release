//! `vieww-accessibility` run over a small hand-built tree and a couple of
//! colour schemes — no window, no GPU, nothing to look at, only to read.
//!
//! ```console
//! cargo run -p feature-accessibility-audit
//! ```
//!
//! # Part 1: a semantics-tree audit
//!
//! Five controls are built directly on a [`vieww_render::RenderTree`] using
//! the real [`RenderSemantics`] and [`RenderConstrainedBox`] render objects —
//! the same ones a widget's `create_render_object` would produce — with four
//! of the five carrying a deliberate, distinct defect:
//!
//! 1. A "Save" button, properly labelled and sized. Clean.
//! 2. A button with **no label** — [`audit`](vieww_accessibility::audit()) can only
//!    say "Button" about it, nothing else.
//! 3. A "Close" button squeezed into a **10x10** target, well under the
//!    44x44 floor.
//! 4. A "Search" text field annotated with [`Role::TextField`] but nothing
//!    underneath it that answers a text-editing action — `RenderSemantics`
//!    derives its `actions` from role, and `TextField` is not one of the
//!    roles it wires an action to (that comes from `EditableText`
//!    elsewhere in a real widget tree, deliberately not reproduced here).
//! 5. A button that declares a **toggled** state — a role with no on/off
//!    announcement to give it.
//!
//! # Part 2: a theme contrast scan
//!
//! `vieww-widget`'s built-in `light` and `dark` schemes are scanned and
//! reported clean. Its `apple_light`/`apple_dark` schemes are scanned too —
//! and shown **failing** several pairs, a genuine property of those two
//! palettes documented in `vieww_accessibility::theme_audit`'s own tests.
//! Finally, a synthetic scheme with light-grey-on-white text is scanned to
//! show the failure path.

use vieww_accessibility::{audit, audit_color_scheme, Severity};
use vieww_foundation::{Color, Constraints, Offset, Size};
use vieww_render::{
    LayoutCtx, RenderConstrainedBox, RenderId, RenderObject, RenderSemantics, RenderTree, Role,
    SemanticsTree,
};
use vieww_widget::ColorScheme;

/// A plain vertical stack, existing only so this example has a root object
/// that can hold more than one child — the accessibility-relevant work
/// happens in the real `RenderSemantics`/`RenderConstrainedBox` children
/// underneath it, not in this type.
#[derive(Debug)]
struct Stack;

/// A bound each child can grow up to: `constraints`' own maximum where it is
/// finite, and a reasonable finite fallback where it is not — a `Stack` full
/// of fixed-size controls never needs an unbounded axis, and passing one
/// through would only risk a child that tries to fill it.
fn bounded_size(constraints: Constraints) -> Size {
    Size::new(
        if constraints.max_width.is_finite() {
            constraints.max_width
        } else {
            800.0
        },
        if constraints.max_height.is_finite() {
            constraints.max_height
        } else {
            600.0
        },
    )
}

impl RenderObject for Stack {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let mut y = 0.0_f32;
        let mut width = 0.0_f32;
        for child in ctx.children().to_vec() {
            let size = ctx.layout_child(child, Constraints::loose(bounded_size(constraints)));
            ctx.place_child(child, Offset::new(0.0, y));
            y += size.height;
            width = width.max(size.width);
        }
        constraints.constrain(Size::new(width, y))
    }

    fn debug_name(&self) -> &'static str {
        "Stack"
    }
}

/// A `RenderSemantics` node sized to exactly `width`x`height` via a
/// `RenderConstrainedBox` child — the same pattern a real control's
/// `touch_target`-style wrapper uses.
fn sized_control(
    tree: &mut RenderTree,
    parent: RenderId,
    node: RenderSemantics,
    width: f32,
    height: f32,
) -> RenderId {
    let id = tree.insert(Some(parent), Box::new(node));
    tree.insert(
        Some(id),
        Box::new(RenderConstrainedBox::new(Constraints::tight(Size::new(
            width, height,
        )))),
    );
    id
}

fn build_tree() -> (SemanticsTree, [RenderId; 5]) {
    let mut tree = RenderTree::new();
    let root = tree.insert(None, Box::new(Stack));

    let clean = sized_control(
        &mut tree,
        root,
        RenderSemantics::new(Role::Button).label("Save"),
        120.0,
        48.0,
    );
    let unlabelled = sized_control(
        &mut tree,
        root,
        RenderSemantics::new(Role::Button),
        120.0,
        48.0,
    );
    let undersized = sized_control(
        &mut tree,
        root,
        RenderSemantics::new(Role::Button).label("Close"),
        10.0,
        10.0,
    );
    let actionless_field = sized_control(
        &mut tree,
        root,
        RenderSemantics::new(Role::TextField).label("Search"),
        200.0,
        44.0,
    );
    let odd_toggle = sized_control(
        &mut tree,
        root,
        RenderSemantics::new(Role::Button)
            .label("Weird")
            .toggled(true),
        120.0,
        48.0,
    );

    let _ = tree.layout_root(Constraints::tight(Size::new(800.0, 600.0)));
    let semantics = SemanticsTree::build(&tree, None);
    (
        semantics,
        [clean, unlabelled, undersized, actionless_field, odd_toggle],
    )
}

fn print_findings(tree: &SemanticsTree) {
    let findings = audit(tree);
    if findings.is_empty() {
        println!("  (no findings)");
        return;
    }
    for finding in &findings {
        let tag = match finding.severity {
            Severity::Error => "ERROR  ",
            Severity::Warning => "WARNING",
        };
        println!("  [{tag}] node {}: {}", finding.node_id, finding.message);
    }
}

fn print_theme_report(name: &str, colors: &ColorScheme) {
    let report = audit_color_scheme(colors);
    if report.passes() {
        println!("  {name}: every pair clears AA");
        return;
    }
    println!("  {name}: {} pair(s) failed", report.failures.len());
    for failure in &report.failures {
        println!(
            "    {} — {:.2}:1, needed {:.1}:1",
            failure.pair, failure.ratio, failure.required
        );
    }
}

fn main() {
    println!("== semantics-tree audit ==");
    let (tree, ids) = build_tree();
    println!("tree of {} node(s), reading order:", tree.len());
    println!("{}", tree.describe());
    println!("findings:");
    print_findings(&tree);
    println!(
        "(node ids in order built: clean={} unlabelled={} undersized={} actionless_field={} odd_toggle={})",
        ids[0], ids[1], ids[2], ids[3], ids[4]
    );

    println!();
    println!("== theme contrast scan ==");
    print_theme_report("light", &ColorScheme::light());
    print_theme_report("dark", &ColorScheme::dark());
    print_theme_report("apple_light", &ColorScheme::apple_light());
    print_theme_report("apple_dark", &ColorScheme::apple_dark());

    let mut low_contrast = ColorScheme::light();
    low_contrast.on_surface = Color::hex(0xCC_CCCC);
    print_theme_report("synthetic (light grey on white)", &low_contrast);
}
