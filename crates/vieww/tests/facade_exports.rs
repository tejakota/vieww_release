//! The facade re-exports every widget, and keeps doing so.
//!
//! ```console
//! cargo test -p vieww --test facade_exports
//! ```
//!
//! # Why this file exists
//!
//! `crates/vieww/src/lib.rs` used to mirror `vieww_widget`'s prelude by hand.
//! The two lists diverged by 62 names — `vieww::Button` resolved, `vieww::Text`
//! did not; `vieww::OverlayPosition` resolved, `vieww::Overlay` did not — and
//! nothing in this repository could see it, because a name missing from a
//! re-export list only fails to compile in a *downstream* crate.
//!
//! So the assertion has to be made from outside the facade's own source, which
//! is what an integration test is. This file names types through `vieww::`
//! only, never through `vieww::prelude::*` or `vieww_widget::` — a glob at the
//! top would make every path below resolve for the wrong reason and the test
//! would pass while proving nothing.
//!
//! It is a *compile-time* test. There are no assertions worth making about a
//! type path: either it resolves and this file builds, or it does not and the
//! build fails with the name that went missing. The one runtime assertion at
//! the bottom exists only so `cargo test` reports something.

/// One name from each family, named the long way round.
///
/// Not exhaustive by design — an exhaustive list is the hand-maintained list
/// this test was written to retire. What it covers is one representative per
/// module in `vieww_widget::widgets` and `::controls`, so a module dropping out
/// of the glob is caught while a single new widget is not asked to be listed
/// twice.
#[allow(dead_code, unreachable_pub)]
mod resolves {
    // Layout — the four that were missing and are the most-used in the crate.
    pub type Text = vieww::Text;
    pub type Container = vieww::Container;
    pub type Flex = vieww::Flex;
    pub type Stack = vieww::Stack;
    pub type Padding = vieww::Padding;
    pub type Align = vieww::Align;
    pub type Center = vieww::Center;
    pub type SizedBox = vieww::SizedBox;
    pub type Constrained = vieww::Constrained;
    pub type AspectRatio = vieww::AspectRatio;
    pub type Fitted = vieww::Fitted;
    pub type Positioned = vieww::Positioned;
    pub type PositionedDirectional = vieww::PositionedDirectional;
    pub type Viewport = vieww::Viewport;
    pub type GridView = vieww::GridView;

    // Painting and compositing.
    pub type Opacity = vieww::Opacity;
    pub type ColoredBox = vieww::ColoredBox;
    pub type RepaintBoundary = vieww::RepaintBoundary;
    pub type Transformed = vieww::Transformed;
    pub type Svg = vieww::Svg;

    // Navigation and overlays — `Overlay` is the one whose `OverlayPosition`
    // was exported without it.
    pub type Overlay = vieww::Overlay;
    pub type OverlayHandle = vieww::OverlayHandle;
    pub type OverlayId = vieww::OverlayId;
    pub type OverlayPosition = vieww::OverlayPosition;
    pub type Drawer = vieww::Drawer;
    pub type DrawerSide = vieww::DrawerSide;
    pub type Breadcrumbs = vieww::Breadcrumbs;
    pub type Pagination = vieww::Pagination;

    // Controls.
    pub type Button = vieww::Button;
    pub type Dropdown = vieww::Dropdown;
    pub type FloatingActionButton = vieww::FloatingActionButton;
    pub type Accordion = vieww::Accordion;
    pub type TreeView = vieww::TreeView;
    pub type Markdown = vieww::Markdown;
    pub type Avatar = vieww::Avatar;
    pub type Badge = vieww::Badge;
    pub type Skeleton = vieww::Skeleton;
    pub type EmptyState = vieww::EmptyState;
    pub type InlineError = vieww::InlineError;
    pub type Confirmation = vieww::Confirmation;
    pub type FileDropZone = vieww::FileDropZone;

    // Semantics and privacy.
    pub type Sensitive = vieww::Sensitive;
    pub type SensitiveMask = vieww::SensitiveMask;
    pub type SemanticLiveness = vieww::SemanticLiveness;

    // Localisation and direction.
    pub type Localizations = vieww::Localizations;
    pub type Directionality = vieww::Directionality;

    // The layout enums, which are named as often as the widgets are.
    pub type MainAxisAlignment = vieww::MainAxisAlignment;
    pub type CrossAxisAlignment = vieww::CrossAxisAlignment;
    pub type MainAxisSize = vieww::MainAxisSize;
    pub type FlexFit = vieww::FlexFit;
    pub type StackFit = vieww::StackFit;

    // Not in the prelude, and named explicitly by the facade — listed here so
    // that shortening that list is also caught.
    // Named at a concrete parameter rather than left generic: a bound on a type
    // alias is not enforced, so `FormField<T>` here would warn without adding
    // anything the concrete form does not already prove.
    pub type FormField = vieww::FormField<String>;
    pub type PressState = vieww::PressState;
    pub type SortDirection = vieww::SortDirection;
    pub type RouteTransition = vieww::RouteTransition;

    // The pixel type, which the prelude's `Image` (the widget) shadows. Named
    // `ImageData` at the facade so a previewed buffer can construct one — see
    // `crates/vieww/src/lib.rs`'s comment on the alias for why the widget and
    // the pixel type cannot share a name.
    pub type ImageData = vieww::ImageData;
}

#[test]
fn the_facade_carries_the_widget_prelude() {
    // The real assertion is that this file compiled at all. This one is here so
    // the test binary reports a pass rather than "0 tests", which reads as a
    // suite that was skipped.
    let _ = vieww::Text::new("resolved through `vieww::`, not `vieww::prelude`");
    let _ = vieww::Container::new();
}
