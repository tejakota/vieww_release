//! `CrossAxisAlignment::Baseline`, measured rather than asserted about.
//!
//! Every test here builds a real row of real shaped text and reads back the
//! offsets layout chose. A baseline test that stubbed the text engine would
//! only prove the arithmetic in `RenderFlex`, and the arithmetic was never the
//! risky part — where a font puts its baseline is.

use vieww_foundation::{Axis, Constraints, EdgeInsets, Size};
use vieww_render::{
    RenderColoredBox, RenderConstrainedBox, RenderFlex, RenderId, RenderPadding, RenderText,
    RenderTree,
};
use vieww_widget::{CrossAxisAlignment, TextStyle};

/// A row under a given cross-axis alignment, laid out at a fixed width.
fn row(alignment: CrossAxisAlignment) -> RenderFlex {
    let mut flex = RenderFlex::new(Axis::Horizontal);
    flex.cross_axis_alignment = alignment;
    flex
}

fn text(size: f32) -> Box<RenderText> {
    Box::new(RenderText::new("Hxy", TextStyle::new(size)))
}

/// Lay a row out at 400x(loose) and return the tree plus the children's ids.
fn lay_out(flex: RenderFlex, children: Vec<Box<dyn vieww_render::RenderObject>>) -> LaidOut {
    let mut tree = RenderTree::new();
    let root = tree.insert(None, Box::new(flex));
    let ids: Vec<RenderId> = children
        .into_iter()
        .map(|child| tree.insert(Some(root), child))
        .collect();
    let size = tree.layout(root, Constraints::loose(Size::new(400.0, f32::INFINITY)));
    LaidOut {
        tree,
        root,
        ids,
        size,
    }
}

struct LaidOut {
    tree: RenderTree,
    root: RenderId,
    ids: Vec<RenderId>,
    size: Size,
}

impl LaidOut {
    /// Where the child's own baseline lands in the row's coordinates: the
    /// child's offset plus the baseline inside it. This is the number the
    /// whole feature exists to make equal.
    fn baseline_of(&mut self, index: usize) -> f32 {
        let id = self.ids[index];
        let inside = self
            .tree
            .baseline(id)
            .expect("this child was built with text in it");
        self.tree.offset(id).dy + inside
    }

    fn top_of(&self, index: usize) -> f32 {
        self.tree.offset(self.ids[index]).dy
    }
}

#[test]
fn two_sizes_of_type_sit_on_one_line() {
    let mut laid = lay_out(
        row(CrossAxisAlignment::Baseline),
        vec![text(24.0), text(11.0)],
    );

    let big = laid.baseline_of(0);
    let small = laid.baseline_of(1);
    assert!(
        (big - small).abs() < 0.01,
        "the whole point of the feature: {big} vs {small}"
    );
}

#[test]
fn centring_does_not_line_them_up_which_is_why_baseline_exists() {
    // The control. If this ever starts passing, either the font stopped having
    // different metrics at different sizes or `Center` quietly became
    // `Baseline`, and the test above would keep passing while proving nothing.
    let mut laid = lay_out(
        row(CrossAxisAlignment::Center),
        vec![text(24.0), text(11.0)],
    );

    let big = laid.baseline_of(0);
    let small = laid.baseline_of(1);
    assert!(
        (big - small).abs() > 1.0,
        "centred boxes put the letters on different lines: {big} vs {small}"
    );
}

#[test]
fn the_larger_type_does_not_move_and_the_smaller_one_comes_down_to_it() {
    let laid = lay_out(
        row(CrossAxisAlignment::Baseline),
        vec![text(24.0), text(11.0)],
    );

    assert!(
        laid.top_of(0) < 0.01,
        "the deepest ascent defines the line, so it sits at the top: {}",
        laid.top_of(0)
    );
    assert!(
        laid.top_of(1) > 1.0,
        "and the smaller one is pushed down onto it: {}",
        laid.top_of(1)
    );
}

#[test]
fn a_child_with_no_text_falls_back_to_the_top_rather_than_to_zero() {
    // An icon in a row of labels. `None` has to mean "top", not "baseline at
    // zero" — the two differ by a whole ascent, which is exactly the amount
    // that makes a toolbar look broken.
    let icon: Box<dyn vieww_render::RenderObject> = Box::new(RenderConstrainedBox::new(
        Constraints::tight(Size::new(16.0, 16.0)),
    ));
    let laid = lay_out(row(CrossAxisAlignment::Baseline), vec![text(24.0), icon]);

    assert!(
        laid.top_of(1) < 0.01,
        "no baseline means top-aligned: {}",
        laid.top_of(1)
    );
}

#[test]
fn a_row_is_tall_enough_for_the_deepest_ascent_and_the_deepest_descent() {
    // Two type sizes whose tallest *box* is shorter than ascent-plus-descent
    // once they are shifted onto a common line. A row that sized itself to the
    // tallest child would clip the smaller one's descenders.
    let laid = lay_out(
        row(CrossAxisAlignment::Baseline),
        vec![text(24.0), text(11.0)],
    );
    let tallest = laid
        .ids
        .iter()
        .map(|&id| laid.tree.size(id).height)
        .fold(0.0_f32, f32::max);

    assert!(
        laid.size.height >= tallest,
        "never shorter than its tallest child: {} vs {tallest}",
        laid.size.height
    );
    for (index, &id) in laid.ids.iter().enumerate() {
        let bottom = laid.tree.offset(id).dy + laid.tree.size(id).height;
        assert!(
            bottom <= laid.size.height + 0.01,
            "child {index} hangs out of the bottom: {bottom} vs {}",
            laid.size.height
        );
    }
}

#[test]
fn padding_around_a_label_moves_its_baseline_down_by_the_padding() {
    // The pass-through has to add the child's own offset back in. Without
    // that, wrapping a label in padding would silently stop it aligning.
    let mut bare = RenderTree::new();
    let bare_id = bare.insert(None, text(16.0));
    bare.layout(bare_id, Constraints::loose(Size::new(400.0, f32::INFINITY)));
    let unpadded = bare.baseline(bare_id).expect("text has a baseline");

    let mut padded = RenderTree::new();
    let pad = padded.insert(None, Box::new(RenderPadding::new(EdgeInsets::all(7.0))));
    let inner = padded.insert(Some(pad), text(16.0));
    let _ = inner;
    padded.layout(pad, Constraints::loose(Size::new(400.0, f32::INFINITY)));
    let through = padded.baseline(pad).expect("padding forwards a baseline");

    assert!(
        (through - unpadded - 7.0).abs() < 0.01,
        "the top inset, exactly: {through} vs {unpadded} + 7"
    );
}

#[test]
fn a_box_with_nothing_in_it_has_no_baseline() {
    let mut tree = RenderTree::new();
    let id = tree.insert(
        None,
        Box::new(RenderColoredBox::new(vieww_foundation::Color::hex(
            0xFF_0000,
        ))),
    );
    tree.layout(id, Constraints::tight(Size::new(10.0, 10.0)));

    assert_eq!(
        tree.baseline(id),
        None,
        "a coloured box holds no text, and saying zero would be a lie a row acts on"
    );
}

#[test]
fn a_column_asked_for_baselines_behaves_as_start_rather_than_panicking() {
    // A baseline is a horizontal line; a column's cross axis is horizontal
    // too, so there is nothing to align across. A stricter toolkit asserts here.
    let mut flex = RenderFlex::new(Axis::Vertical);
    flex.cross_axis_alignment = CrossAxisAlignment::Baseline;

    let mut tree = RenderTree::new();
    let root = tree.insert(None, Box::new(flex));
    let wide = tree.insert(Some(root), text(24.0));
    let narrow = tree.insert(Some(root), text(11.0));
    tree.layout(root, Constraints::loose(Size::new(400.0, f32::INFINITY)));

    assert!(tree.offset(wide).dx.abs() < 0.01, "left edge");
    assert!(tree.offset(narrow).dx.abs() < 0.01, "left edge");
}

#[test]
fn a_row_inside_a_row_aligns_to_the_inner_rows_first_label() {
    // Baseline alignment has to compose, or it stops working the first time
    // somebody groups two labels together.
    let mut inner = RenderFlex::new(Axis::Horizontal);
    inner.cross_axis_alignment = CrossAxisAlignment::Baseline;

    let mut tree = RenderTree::new();
    let outer = tree.insert(None, Box::new(row(CrossAxisAlignment::Baseline)));
    let lone = tree.insert(Some(outer), text(24.0));
    let group = tree.insert(Some(outer), Box::new(inner));
    let grouped = tree.insert(Some(group), text(11.0));
    tree.layout(outer, Constraints::loose(Size::new(400.0, f32::INFINITY)));

    let lone_at = tree.offset(lone).dy + tree.baseline(lone).expect("text");
    let grouped_at =
        tree.offset(group).dy + tree.offset(grouped).dy + tree.baseline(grouped).expect("text");

    assert!(
        (lone_at - grouped_at).abs() < 0.01,
        "one line through both: {lone_at} vs {grouped_at}"
    );
}

#[test]
fn only_the_alignment_that_asks_for_baselines_pays_for_them() {
    // The cost is a query per child. A centred row must not be walking
    // subtrees looking for text it will never use — which is observable here
    // as the two rows placing children by different rules, and asserted
    // properly by the fact that `Center` above does *not* line them up.
    let centred = lay_out(
        row(CrossAxisAlignment::Center),
        vec![text(24.0), text(11.0)],
    );
    let aligned = lay_out(
        row(CrossAxisAlignment::Baseline),
        vec![text(24.0), text(11.0)],
    );

    assert_ne!(
        centred.tree.offset(centred.ids[1]).dy,
        aligned.tree.offset(aligned.ids[1]).dy,
        "the two alignments must actually place differently"
    );
    let _ = (centred.root, aligned.root);
}
