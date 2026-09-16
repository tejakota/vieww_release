//! An incrementally synced render tree is identical to one built from scratch.
//!
//! ```console
//! cargo test -p vieww-render --test incremental_sync
//! ```
//!
//! # What is being guarded
//!
//! `RenderOwner::sync` skips any subtree whose
//! `Element::subtree_revision` has not advanced since the last sync. That makes
//! an idle frame nearly free, and it makes the element tree responsible for a
//! new invariant: **every path that changes what the render tree should look
//! like must stamp a revision.** Miss one, and the symptom is a frame that
//! silently keeps showing the previous state — the hardest kind of bug to see,
//! because nothing fails and nothing logs.
//!
//! So the assertion is not "the skip works". It is that *skipping is
//! unobservable*: after any sequence of mutations, an owner that has been
//! syncing incrementally the whole time must produce exactly the render tree a
//! fresh owner produces from the same widget tree. Compared through the paint
//! command list, which is the only thing the render tree exists to produce and
//! which folds in geometry, order and every object's own configuration.
//!
//! The mutation sequence is pseudo-random but **deterministic**: the generator
//! is a fixed-seed LCG, so a failure reproduces exactly and a run that passes
//! today passes tomorrow. A test that shuffled differently on each run would
//! report a defect once and never again.

use vieww_element::ElementTree;
use vieww_foundation::{Color, Constraints, EdgeInsets, Size};
use vieww_render::{Command, RenderOwner, Scene};
use vieww_widget::prelude::*;

const VIEWPORT: Size = Size::new(400.0, 800.0);

// ---------------------------------------------------------------- the model

/// A tiny application state, rich enough to exercise every reconcile path.
///
/// Each row is a keyed item, so reordering is a real move rather than a
/// rewrite; the `boxed` flag toggles a wrapper *around* a row, which is the
/// case that changes tree depth; and `heading` changes a node that is a sibling
/// of the list rather than inside it, so a change there must not be reported as
/// a change to a row.
#[derive(Clone, PartialEq, Eq)]
struct Model {
    heading: String,
    rows: Vec<Row>,
}

#[derive(Clone, PartialEq, Eq)]
struct Row {
    key: u32,
    label: String,
    shade: u8,
    boxed: bool,
}

fn view(model: &Model) -> WidgetNode {
    let rows: Vec<WidgetNode> = model.rows.iter().map(row_view).collect();
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(children![
            Text::new(model.heading.clone()),
            Flex::column().children(rows),
        ])
        .into()
}

fn row_view(row: &Row) -> WidgetNode {
    let body = Container::new()
        .height(20.0)
        .color(Color::rgb(row.shade, 40, 40))
        .child(Text::new(row.label.clone()));

    let inner: WidgetNode = if row.boxed {
        Padding::new(EdgeInsets::all(4.0)).child(body).into()
    } else {
        body.into()
    };

    // Keyed, so a reorder moves the element rather than rebuilding it in place
    // — which is the reconcile path that moves children between parents and the
    // one most likely to leave a revision unstamped.
    Container::new()
        .key(Key::from(i64::from(row.key)))
        .child(inner)
        .into()
}

// ------------------------------------------------------------- the generator

/// A fixed-seed linear congruential generator.
///
/// Deliberately not `rand`: this test's value depends on the sequence being the
/// same on every machine and every run, and a dependency that reseeds from the
/// clock would turn a reproducible failure into a rumour.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        // Numerical Recipes' constants. Any decent multiplier does; what
        // matters is that it is written down here rather than imported.
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, limit: usize) -> usize {
        if limit == 0 {
            0
        } else {
            (self.next() as usize) % limit
        }
    }
}

/// Apply one random mutation, returning a name for the failure message.
fn mutate(model: &mut Model, rng: &mut Lcg, next_key: &mut u32) -> &'static str {
    match rng.below(7) {
        0 => {
            let at = rng.below(model.rows.len() + 1);
            model.rows.insert(
                at,
                Row {
                    key: *next_key,
                    label: format!("row {next_key}"),
                    shade: (*next_key % 200) as u8,
                    boxed: rng.below(2) == 0,
                },
            );
            *next_key += 1;
            "insert"
        }
        1 if !model.rows.is_empty() => {
            let at = rng.below(model.rows.len());
            model.rows.remove(at);
            "remove"
        }
        2 if model.rows.len() > 1 => {
            let (from, to) = (rng.below(model.rows.len()), rng.below(model.rows.len()));
            let row = model.rows.remove(from);
            model.rows.insert(to, row);
            "reorder"
        }
        3 if !model.rows.is_empty() => {
            let at = rng.below(model.rows.len());
            model.rows[at].shade = model.rows[at].shade.wrapping_add(37);
            "recolour"
        }
        4 if !model.rows.is_empty() => {
            let at = rng.below(model.rows.len());
            model.rows[at].label = format!("{}!", model.rows[at].label);
            "relabel"
        }
        5 if !model.rows.is_empty() => {
            let at = rng.below(model.rows.len());
            model.rows[at].boxed = !model.rows[at].boxed;
            // The interesting one: the depth of the subtree changes, so the
            // render object's *child* changes without the row's own widget
            // changing type.
            "rewrap"
        }
        _ => {
            model.heading = format!("heading {}", rng.next() % 1000);
            "reheading"
        }
    }
}

// --------------------------------------------------------------- comparison

/// Everything a render tree can say about itself, as a flat list.
///
/// Paint commands rather than a structural dump, because the commands are what
/// the tree is *for*: they fold in each object's configuration, its resolved
/// geometry and the order it draws in. Two trees with the same commands are
/// indistinguishable to everything downstream.
fn commands(owner: &mut RenderOwner, elements: &mut ElementTree) -> Vec<String> {
    owner.draw_frame(elements, Constraints::tight(VIEWPORT));
    let mut scene = Scene::new();
    owner.paint(&mut scene);
    scene.commands().iter().map(describe).collect()
}

/// A stable one-line form of a command.
///
/// `Command` is not `Display` and its `Debug` includes a clip whose formatting
/// is noisy; this keeps what distinguishes two trees and drops what does not.
fn describe(command: &Command) -> String {
    match command {
        Command::FillRect { rect, paint, .. } => {
            format!("fill {rect:?} {paint:?}")
        }
        Command::DrawGlyphs { run, .. } => format!(
            "glyphs {:?} n={} colour={:?}",
            run.origin,
            run.glyphs.len(),
            run.color
        ),
        other => format!("{other:?}"),
    }
}

/// The render tree a fresh owner produces for this model, with no history.
fn from_scratch(model: &Model) -> Vec<String> {
    let mut elements = ElementTree::new();
    elements.set_root(view(model));
    let mut owner = RenderOwner::new();
    commands(&mut owner, &mut elements)
}

// -------------------------------------------------------------------- tests

#[test]
fn an_incrementally_synced_tree_matches_one_built_from_scratch() {
    let mut model = Model {
        heading: String::from("heading"),
        rows: (0..6)
            .map(|key| Row {
                key,
                label: format!("row {key}"),
                shade: (key * 30) as u8,
                boxed: key % 2 == 0,
            })
            .collect(),
    };
    let mut next_key = 100_u32;
    let mut rng = Lcg(0x5eed_1234);

    let mut elements = ElementTree::new();
    elements.set_root(view(&model));
    let mut owner = RenderOwner::new();
    let _ = commands(&mut owner, &mut elements);

    for step in 0..200 {
        let what = mutate(&mut model, &mut rng, &mut next_key);
        elements.set_root(view(&model));
        let incremental = commands(&mut owner, &mut elements);
        let scratch = from_scratch(&model);

        assert_eq!(
            incremental,
            scratch,
            "step {step} ({what}, {} rows): the incremental sync skipped a \
             subtree that had in fact changed. A missing `ElementTree::touch` \
             on one of the mutation paths is the usual cause.",
            model.rows.len()
        );
    }
}

// ------------------------------------------------ the counted half, driven by
// ------------------------------------------------ signals rather than by hand

/// A row that reads its own signal, so a write to it rebuilds it and nothing
/// else.
///
/// This is how an application actually changes: a signal write marks one
/// element pending, `rebuild_pending` rebuilds that element's subtree, and
/// every other element's widget is still the same `Rc`. The tests above drive
/// the tree by rebuilding the root from a model, which is the harsher case and
/// the right one for a correctness property; these two are about *cost*, and
/// cost is only meaningful against the shape a real frame has.
#[derive(Debug)]
struct SignalRow {
    shade: vieww_element::Signal<u8>,
    key: Key,
}

impl Widget for SignalRow {
    fn debug_name(&self) -> &'static str {
        "SignalRow"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        Some(&self.key)
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Container::new()
            .height(20.0)
            .color(Color::rgb(self.shade.get(), 40, 40))
            .child(Padding::new(EdgeInsets::all(2.0)).child(Text::new("row")))
            .into()
    }
}

vieww_widget::widget_node_from!(SignalRow);

/// A tree of `count` signal-backed rows, and the signals that drive them.
fn signal_tree(count: usize) -> (ElementTree, Vec<vieww_element::Signal<u8>>, RenderOwner) {
    let mut elements = ElementTree::new();
    let signals: Vec<_> = (0..count)
        .map(|i| elements.runtime().signal((i % 200) as u8))
        .collect();

    let rows: Vec<WidgetNode> = signals
        .iter()
        .enumerate()
        .map(|(i, shade)| {
            SignalRow {
                shade: shade.clone(),
                key: Key::from(i as i64),
            }
            .into()
        })
        .collect();

    elements.mount(
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );

    let mut owner = RenderOwner::new();
    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));
    (elements, signals, owner)
}

#[test]
fn a_frame_that_changes_nothing_walks_nothing() {
    let (mut elements, _signals, mut owner) = signal_tree(50);

    let built = owner.visited_last_sync();
    assert!(
        built > 100,
        "the first sync has to walk the whole tree; it walked {built}"
    );

    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));

    assert_eq!(
        owner.visited_last_sync(),
        0,
        "an idle frame used to re-walk all {built} elements, cloning a widget \
         and building a child list per node, to find that nothing had moved"
    );
}

#[test]
fn one_changed_row_costs_one_path_and_not_the_tree() {
    let (mut elements, signals, mut owner) = signal_tree(80);
    let whole_tree = owner.visited_last_sync();

    signals[40].set(200);
    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));
    let one_row = owner.visited_last_sync();

    // The path from the root down to the changed row, plus that row's own few
    // nodes. Not eighty rows' worth, which is what it was.
    assert!(
        one_row < whole_tree / 8,
        "changing one row of eighty walked {one_row} elements of {whole_tree}"
    );

    // And it is still correct: the row that changed reached the screen.
    let scene = {
        let mut scene = Scene::new();
        owner.paint(&mut scene);
        scene
    };
    assert!(
        scene.commands().iter().any(|command| matches!(
            command,
            Command::FillRect { paint, .. } if paint.color == Color::rgb(200, 40, 40)
        )),
        "the skip made the frame cheap by not drawing the change"
    );
}

#[test]
fn removing_the_last_row_removes_its_render_objects() {
    let mut model = Model {
        heading: String::from("heading"),
        rows: (0..4)
            .map(|key| Row {
                key,
                label: format!("row {key}"),
                shade: 10,
                boxed: true,
            })
            .collect(),
    };

    let mut elements = ElementTree::new();
    elements.set_root(view(&model));
    let mut owner = RenderOwner::new();
    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));
    let full = owner.tree().len();

    model.rows.clear();
    elements.set_root(view(&model));
    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));

    assert!(
        owner.tree().len() < full,
        "render objects for removed elements were left detached in the tree: \
         {} of {full} remain",
        owner.tree().len()
    );
    assert_eq!(
        commands(&mut owner, &mut elements),
        from_scratch(&model),
        "and what is left draws the same as a tree that never had them"
    );
}

// ---------------------------------------------------------------------------
// Checklist item 6 — text shaping, measured where it is actually paid for.
// ---------------------------------------------------------------------------

/// A row whose **text** comes from a signal.
///
/// `SignalRow` above drives a *colour* and every one of its rows says the
/// literal `"row"`, so a whole tree of them is one distinct string. That is the
/// right fixture for the sync tests and the wrong one for this: against it a
/// shaping cache looks perfect no matter what its key is, because there is
/// nothing to tell apart.
#[derive(Debug)]
struct LabelRow {
    label: vieww_element::Signal<String>,
    key: Key,
}

impl Widget for LabelRow {
    fn debug_name(&self) -> &'static str {
        "LabelRow"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        Some(&self.key)
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Container::new()
            .height(16.0)
            .child(Text::new(self.label.get()))
            .into()
    }
}

vieww_widget::widget_node_from!(LabelRow);

/// `count` rows, each with its own text.
fn label_tree(count: usize) -> (ElementTree, Vec<vieww_element::Signal<String>>, RenderOwner) {
    let mut elements = ElementTree::new();
    let signals: Vec<_> = (0..count)
        .map(|i| elements.runtime().signal(format!("row number {i}")))
        .collect();

    let rows: Vec<WidgetNode> = signals
        .iter()
        .enumerate()
        .map(|(i, label)| {
            LabelRow {
                label: label.clone(),
                key: Key::from(i as i64),
            }
            .into()
        })
        .collect();

    elements.mount(
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );

    let mut owner = RenderOwner::new();
    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));
    (elements, signals, owner)
}

/// Where the shaping cache actually pays, measured.
///
/// # The test this replaced, and why it was worthless
///
/// The obvious frame-level test is "two idle frames shape nothing", and it
/// passes **with the cache switched off**: layout is already incremental, so an
/// idle frame does not lay anything out and therefore never reaches the shaper.
/// It measured the layout dirty set, not the cache. Deleting the cache's `get`
/// left it green, which is the definition of a test that cannot fail.
///
/// The two below are the cases where the shaper genuinely runs twice on the
/// same words, and each is checked by breaking the cache and watching it fail:
///
/// 1. **Repeated text.** A list of rows sharing a label — a status column, a
///    unit, a currency symbol — shapes one paragraph rather than one per row.
/// 2. **A relayout back to a width already seen.** Resizing a window and
///    resizing it back, a split pane dragged and released, a keyboard opening
///    and closing: layout runs in full and every string is one it has already
///    shaped at that width.
#[test]
fn identical_labels_are_shaped_once_between_them() {
    // `SignalRow`'s fifty rows all say "row" — fifty render objects, one
    // distinct string.
    let (_elements, _signals, mut owner) = signal_tree(50);

    let shaped = owner.tree_mut().fonts_mut().shape_count();
    assert!(
        shaped <= 3,
        "fifty rows with the same label shaped {shaped} paragraphs; they are \
         the same words at the same size and width"
    );
}

/// Laying out at a width, then another, then back at the first.
#[test]
fn returning_to_a_width_already_laid_out_shapes_nothing_new() {
    let (mut elements, _signals, mut owner) = label_tree(50);
    let first = owner.tree_mut().fonts_mut().shape_count();
    assert!(first >= 50, "fifty distinct labels shaped {first}");

    // A different width: everything genuinely has to be re-shaped, because the
    // wrap points can move.
    let narrow = Size::new(VIEWPORT.width / 2.0, VIEWPORT.height);
    owner.draw_frame(&mut elements, Constraints::tight(narrow));
    let after_narrow = owner.tree_mut().fonts_mut().shape_count();
    assert!(
        after_narrow > first,
        "a new width has to re-shape; it shaped {} more",
        after_narrow - first
    );

    // And back. Every string at this width has been shaped once already.
    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));
    assert_eq!(
        owner.tree_mut().fonts_mut().shape_count(),
        after_narrow,
        "going back to a width already laid out re-shaped every label"
    );
}

/// The cache must not make a *changed* label keep its old shape. A counter can
/// only say that work was skipped; this says the picture is still right.
#[test]
fn a_changed_label_reaches_the_screen() {
    let (mut elements, signals, mut owner) = label_tree(8);
    signals[3].set("brand new text".to_string());
    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));

    let mut scene = Scene::new();
    owner.paint(&mut scene);

    // A fresh owner, with a cold font store, from the same widget tree.
    let mut fresh = RenderOwner::new();
    fresh.draw_frame(&mut elements, Constraints::tight(VIEWPORT));
    let mut expected = Scene::new();
    fresh.paint(&mut expected);

    assert_eq!(
        format!("{:?}", scene.commands()),
        format!("{:?}", expected.commands()),
        "a cached frame differs from one drawn with no cache at all"
    );
}

/// Prints the numbers the tracker quotes. Not an assertion — a measurement, run
/// with `--nocapture` when the figures need refreshing.
#[test]
fn shaping_numbers_for_the_record() {
    let (_e, _s, mut owner) = signal_tree(50);
    println!(
        "50 identical labels: {} shaped",
        owner.tree_mut().fonts_mut().shape_count()
    );

    let (mut elements, _s, mut owner) = label_tree(50);
    let first = owner.tree_mut().fonts_mut().shape_count();
    let narrow = Size::new(VIEWPORT.width / 2.0, VIEWPORT.height);
    owner.draw_frame(&mut elements, Constraints::tight(narrow));
    let narrowed = owner.tree_mut().fonts_mut().shape_count();
    owner.draw_frame(&mut elements, Constraints::tight(VIEWPORT));
    let back = owner.tree_mut().fonts_mut().shape_count();
    println!(
        "50 distinct labels: {first} at first width, +{} at a new one, +{} going back",
        narrowed - first,
        back - narrowed
    );
    println!("cache hits: {}", owner.tree_mut().fonts_mut().shape_hits());
}
