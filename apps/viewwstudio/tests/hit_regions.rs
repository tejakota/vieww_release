//! Where a control is drawn, and where it can actually be pressed.
//!
//! # Why this file exists
//!
//! Reported from use, against the tab strip's "+" button: *"when tried to click
//! on it, it didn't work at the location but a few mm left."*
//!
//! That is a class of defect nothing else in this suite could catch.
//! `tests/input.rs` drives real pointers at points chosen by hand, so it proves
//! a control responds *somewhere* — and its points were themselves chosen by
//! finding somewhere that worked. A control whose hit region sits beside its
//! picture passes every one of those tests, and passes every screenshot test
//! too, because the picture is right. Only a person clicking finds it.
//!
//! So this makes the comparison a person makes: find where a control is
//! **painted**, and ask whether pressing *there* reaches it.
//!
//! # What it found
//!
//! The divider between the editor and the preview is a six-point line in a
//! six-point gutter, and `GestureDetector` expands every control to the theme's
//! 48-point minimum touch target. That put an invisible 48-point-wide grab
//! strip on top of the controls at both cards' edges — including the "+", which
//! ends flush against the editor card's right edge. Measured: the "+" is
//! painted across x 948.9–957.1 and was reachable only across 929–944.
//! Everything from 945 rightwards went to the divider. See
//! `ui::divider::GRAB`.

use std::time::Duration;

use vieww_foundation::{Offset, PointerEvent, PointerId, Size, TargetPlatform};
use vieww_render::{FrameDriver, RenderId};
use viewwstudio::state::{View, OPEN_SIDEBAR};
use viewwstudio::{Shell, Studio};

fn shell() -> (FrameDriver, Studio) {
    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime).with_host(TargetPlatform::Linux);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    viewwstudio::seed_focus(&mut driver);
    (driver, studio)
}

fn click(driver: &mut FrameDriver, at: Offset, ms: u64) {
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        at,
        Duration::from_millis(ms),
    ));
    driver.draw_frame_at(Duration::from_millis(ms));
    driver.draw_frame_at(Duration::from_millis(ms + 110));
    driver.handle_pointer(&PointerEvent::up(
        PointerId(1),
        at,
        Duration::from_millis(ms + 120),
    ));
    driver.draw_frame_at(Duration::from_millis(ms + 120));
}

/// Every icon in the tree, with its painted centre and the nearest gesture
/// detector above it.
///
/// The ancestry is reconstructed from `describe_subtree`'s depths: the walk is
/// depth-first in paint order, so the last node seen at a shallower depth is
/// the parent.
fn icons_with_their_buttons(driver: &FrameDriver) -> Vec<(RenderId, Offset, RenderId)> {
    let tree = driver.renders();
    let Some(root) = tree.root() else {
        return Vec::new();
    };
    let mut ancestry: Vec<RenderId> = Vec::new();
    let mut out = Vec::new();
    for (depth, node) in tree.describe_subtree(root, 20_000) {
        ancestry.truncate(depth);
        ancestry.push(node.id);
        if !node.name.contains("Icon") {
            continue;
        }
        let origin = tree.global_offset(node.id);
        let centre = Offset::new(
            origin.dx + node.size.width / 2.0,
            origin.dy + node.size.height / 2.0,
        );
        // Only what is on screen. A tree holds render objects that no clip lets
        // through — a row scrolled out of a viewport, a pane behind the bottom
        // panel — and an icon nobody can see is not a control anybody can fail
        // to press. Containment in every ancestor's box is the cheap proxy for
        // "not clipped away", and it is exact for this shell, whose panes are
        // clipped to their cards.
        let visible = ancestry.iter().all(|id| {
            tree.describe(*id).is_some_and(|d| {
                let at = tree.global_offset(*id);
                (at.dx..=at.dx + d.size.width).contains(&centre.dx)
                    && (at.dy..=at.dy + d.size.height).contains(&centre.dy)
            })
        });
        if !visible {
            continue;
        }
        // The nearest detector above it. `rev` because a button nested inside
        // another clickable — a tab's close button inside its tab — belongs to
        // the inner one.
        let Some(&button) = ancestry.iter().rev().find(|id| {
            tree.describe(**id)
                .is_some_and(|d| d.name.contains("Gesture"))
        }) else {
            continue;
        };
        out.push((node.id, centre, button));
    }
    out
}

#[test]
fn every_icon_button_is_reachable_where_its_icon_is_drawn() {
    let (driver, _studio) = shell();
    let tree = driver.renders();

    let mut wrong: Vec<String> = Vec::new();
    for (icon, centre, button) in icons_with_their_buttons(&driver) {
        let hit = driver.hit_test(centre);
        let reached = hit.entries().iter().any(|entry| entry.id == button);
        if !reached {
            let stole = hit
                .entries()
                .iter()
                .find(|entry| {
                    tree.describe(entry.id)
                        .is_some_and(|d| d.name.contains("Gesture"))
                })
                .map(|entry| {
                    let offset = tree.global_offset(entry.id);
                    let size = tree.describe(entry.id).map(|d| d.size).unwrap_or_default();
                    format!(
                        "{:?} painted x {:.1}..{:.1}",
                        entry.id,
                        offset.dx,
                        offset.dx + size.width
                    )
                })
                .unwrap_or_else(|| "nothing".to_owned());
            let origin = tree.global_offset(icon);
            wrong.push(format!(
                "icon {icon:?} drawn at {origin:?}, centre {centre:?}: its button \
                 {button:?} is not under that point — {stole} is"
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "a control that cannot be pressed where it is drawn:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn the_new_file_button_opens_a_file_when_its_plus_is_pressed() {
    // The reported case, end to end and through the real gesture path rather
    // than through `hit_test` alone: press the centre of the drawn glyph and a
    // new buffer must appear.
    let (mut driver, studio) = shell();
    let before = studio.buffers.get().len();

    // The "+" is the rightmost icon in the tab strip. Found rather than
    // hard-coded, so this keeps testing the button and not a number that was
    // true once.
    let (_, centre, _) = icons_with_their_buttons(&driver)
        .into_iter()
        .filter(|(_, centre, _)| (40.0..90.0).contains(&centre.dy))
        .max_by(|a, b| a.1.dx.total_cmp(&b.1.dx))
        .expect("an icon in the tab strip");

    click(&mut driver, centre, 1);

    assert_eq!(
        studio.buffers.get().len(),
        before + 1,
        "pressing the centre of the drawn + must open a new file; it was pressed at {centre:?}"
    );
}

/// The activity bar's icons, top to bottom, with their painted centres and
/// the detectors that own them.
///
/// The bar is the leftmost column on screen — every icon in it shares one x,
/// a centred 48-point column — so the cluster is found by taking everything
/// within a step of the smallest x rather than a hard-coded window padding.
fn activity_bar_icons(driver: &FrameDriver) -> Vec<(RenderId, Offset, RenderId)> {
    let all = icons_with_their_buttons(driver);
    let left = all
        .iter()
        .map(|(_, centre, _)| centre.dx)
        .fold(f32::INFINITY, f32::min);
    let mut icons: Vec<(RenderId, Offset, RenderId)> = all
        .into_iter()
        .filter(|(_, centre, _)| centre.dx <= left + 8.0)
        .collect();
    icons.sort_by(|a, b| a.1.dy.total_cmp(&b.1.dy));
    icons
}

#[test]
fn the_folder_icon_folds_and_unfolds_the_sidebar() {
    // The Explorer press used to be a plain switcher, so pressing the view
    // that was already showing did nothing at all: the pane had no mouse
    // trigger, and the keyboard chord was the only way to fold it. VS Code's
    // rule — press the showing view's icon to fold, press any icon to unfold
    // — now belongs to the whole column, and the folder is tested here as
    // the first of its icons, through the real pointer path rather than by
    // setting the width by hand.
    let (mut driver, studio) = shell();

    // A fresh shell: Explorer showing in an open pane, folder at the top.
    assert_eq!(studio.sidebar_width.get(), OPEN_SIDEBAR);
    assert_eq!(studio.view.get(), View::Explorer);
    let icons = activity_bar_icons(&driver);
    assert!(
        icons.len() >= 2,
        "the activity bar's icons must be on screen for this to test anything"
    );

    // Press one: Explorer is the view already showing, so the pane folds
    // and the view is left alone.
    let (_, centre, _) = icons[0];
    click(&mut driver, centre, 1);
    assert_eq!(
        studio.sidebar_width.get(),
        0.0,
        "pressing the showing view's folder must fold the pane"
    );
    assert_eq!(
        studio.view.get(),
        View::Explorer,
        "the folding press moves the width, not the view"
    );

    // Press two: the same icon brings the pane back at its open width.
    let (_, centre, _) = activity_bar_icons(&driver)[0];
    click(&mut driver, centre, 800);
    assert_eq!(
        studio.sidebar_width.get(),
        OPEN_SIDEBAR,
        "the same icon unfolds the pane"
    );

    // Press three folds it again; a press on a different icon while folded
    // reveals the pane with that icon's view instead.
    click(&mut driver, centre, 1_600);
    assert_eq!(studio.sidebar_width.get(), 0.0);
    let (_, centre, _) = activity_bar_icons(&driver)[1];
    click(&mut driver, centre, 2_400);
    assert_eq!(
        studio.view.get(),
        View::Search,
        "a folded pane opens on the pressed view"
    );
    assert_eq!(studio.sidebar_width.get(), OPEN_SIDEBAR);
}

#[test]
fn every_activity_bar_icon_hides_and_reveals_the_pane() {
    // "The folder toggles it" was never the spec — the column does. Two more
    // icons past the first, driven through the real pointer path at their
    // drawn glyphs: the search glass and the problems marker, so the claim
    // "any icon" rests on more than the one the rule was first noticed on.
    let (mut driver, studio) = shell();
    let icons = activity_bar_icons(&driver);
    assert!(
        icons.len() >= 4,
        "the bar's upper group must be on screen for this to test anything"
    );

    // Search, second from the top. The first press finds Search not showing,
    // so it switches; the pane stays up. The press after that finds Search
    // showing and folds the pane, and the press after that unfolds it.
    let (_, search, _) = icons[1];
    click(&mut driver, search, 1);
    assert_eq!(
        studio.view.get(),
        View::Search,
        "the first press switches to the pressed view"
    );
    assert_eq!(
        studio.sidebar_width.get(),
        OPEN_SIDEBAR,
        "switching keeps the pane up"
    );
    click(&mut driver, search, 800);
    assert_eq!(
        studio.sidebar_width.get(),
        0.0,
        "Search's own icon folds the pane, like the folder does"
    );
    click(&mut driver, search, 1_600);
    assert_eq!(
        studio.sidebar_width.get(),
        OPEN_SIDEBAR,
        "Search's own icon unfolds it again"
    );

    // Problems, further down the column, same rule from a fresh open pane:
    // switch, fold, unfold — and a folding press never switched the view.
    let (_, problems, _) = activity_bar_icons(&driver)[3];
    click(&mut driver, problems, 2_400);
    assert_eq!(studio.view.get(), View::Problems);
    assert_eq!(studio.sidebar_width.get(), OPEN_SIDEBAR);
    click(&mut driver, problems, 3_200);
    assert_eq!(
        studio.sidebar_width.get(),
        0.0,
        "Problems' own icon folds the pane"
    );
    click(&mut driver, problems, 4_000);
    assert_eq!(
        studio.sidebar_width.get(),
        OPEN_SIDEBAR,
        "Problems' own icon unfolds it"
    );
    assert_eq!(
        studio.view.get(),
        View::Problems,
        "folding never switched the view"
    );
}
