//! The controls — the part of the library an application actually assembles a
//! screen from.
//!
//! Everything here is [`Composed`](crate::WidgetKind::Composed): a control owns
//! no layout and no painting of its own, it arranges the primitives in
//! [`widgets`](crate::widgets) that do. That is not a stylistic choice — a
//! control that needed its own render object would have to be registered in
//! `vieww-render`'s factory, and the point of this layer is that an application
//! can write its own controls the same way these are written.
//!
//! # Controlled, not self-managing
//!
//! A control does not own its value. It is handed one and reports the value it
//! would like to become; the caller writes that to a signal and hands the result
//! back down. [`TextField`](crate::TextField) explains why at length, and the
//! same reasoning applies to every control here: durable state that a *gesture*
//! can change needs something that both mutates it and marks the element pending,
//! and that is a signal in the element tree, not a field on a widget.
//!
//! A control with no handler is **disabled**, rather than being read-only or
//! panicking. One rule, no separate `enabled` flag to keep in step with it.
//!
//! # What a control reads from the theme
//!
//! Colours, the type scale, the corner radius and the minimum touch target, all
//! through [`ThemeData::of`](crate::ThemeData::of) — exactly once per build. No
//! control here hard-codes a colour or a size, so restyling an application is
//! one [`Theme`](crate::Theme) at its root.
//!
//! # Pressed state is the one thing a control owns
//!
//! Every interactive control here highlights while a finger is on it, and that
//! flag is the single exception to "controlled, not self-managing" above. It
//! lives in a [`Pressable`](crate::Pressable)'s own
//! [`ElementState`](crate::ElementState) rather than in a signal, because no
//! application wants to own a highlight — see `Pressable` for the argument, and
//! for the rule that keeps the exception from spreading.
//!
//! The highlight itself is one rule in one place, [`pressed_fill`]: the fill is
//! washed towards whatever colour is drawn *on* it. A filled control darkens, an
//! outlined one picks up a tint of its own ink, and one with no fill at all
//! grows a faint background — three right answers from one line, and no pressed
//! palette to keep in step with the resting one.
//!
//! It fades rather than appearing, over
//! [`CONTROL_DURATION`](crate::CONTROL_DURATION), which is why every `body`
//! here takes a *fraction* rather than a flag.

mod accordion;
mod autocomplete;
mod avatar;
mod badge;
mod bottom_navigation;
mod breadcrumbs;
mod button;
mod checkbox;
mod chip;
mod confirmation;
mod data_table;
mod date_picker;
mod dialog;
mod draggable;
mod drawer;
mod dropdown;
mod empty_state;
mod fab;
mod file_drop_zone;
mod grid_view;
mod inline_error;
mod list_view;
mod markdown;
mod menu;
mod navigator;
mod overlay;
mod pagination;
mod progress;
mod radio;
mod scrollable;
mod segmented;
mod skeleton;
mod slider;
mod sliver;
mod switch;
mod tab_bar;
mod time_picker;
mod tooltip;
mod tree;

pub use accordion::Accordion;
pub use autocomplete::{Autocomplete, MatchFn, MatchMode};
pub use avatar::{Avatar, DEFAULT_AVATAR_SIZE};
pub use badge::Badge;
pub use bottom_navigation::{BottomNavItem, BottomNavigation};
pub use breadcrumbs::Breadcrumbs;
pub use button::{Button, ButtonStyle};
pub use checkbox::Checkbox;
pub use chip::Chip;
pub use confirmation::Confirmation;
pub use data_table::{DataColumn, DataTable, SelectionMode, SortDirection};
pub use date_picker::DatePicker;
pub use dialog::{BottomSheet, Dialog, Snackbar};
pub use draggable::{DragKind, DragPayload, DragSession, DragTarget, Draggable};
pub use drawer::{Drawer, DrawerSide, DRAWER_MAX_WIDTH};
pub use dropdown::Dropdown;
pub use empty_state::EmptyState;
pub use fab::{FloatingActionButton, FAB_SIZE};
pub use file_drop_zone::FileDropZone;
pub use grid_view::GridView;
pub use inline_error::InlineError;
pub use list_view::{ExtentBuilder, ItemBuilder, ListView};
pub use markdown::{LinkFn, Markdown};
pub use menu::{Menu, MenuItem, MENU_MAX_WIDTH, MENU_MIN_WIDTH};
pub use navigator::{
    ModalBarrier, Navigator, OverlayPosition, Route, RouteBuilder, RouteTransition, ROUTE_DURATION,
};
pub use overlay::{Overlay, OverlayBuilder, OverlayHandle, OverlayId, OverlayLease, OverlayState};
pub use pagination::Pagination;
pub use progress::{CircularProgress, CircularProgressArc, LinearProgress, SpinState, SweepState};
pub use radio::Radio;
pub use scrollable::{ScrollMetrics, Scrollable};
pub use segmented::SegmentedControl;
pub use skeleton::{Skeleton, TEXT_LINE_HEIGHT};
pub use slider::{Slider, SliderBar};
pub use sliver::{CustomScrollView, SliverAppBar, SliverList, SliverRefresh};
pub use switch::Switch;
pub use tab_bar::TabBar;
pub use time_picker::TimePicker;
pub use tooltip::{Tooltip, TOOLTIP_MAX_WIDTH, TOOLTIP_OFFSET};
pub use tree::{TreeNode, TreeView, TREE_INDENT};

use vieww_foundation::{BoxDecoration, Color, Constraints};

use crate::{
    ColorScheme, Constrained, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize, WidgetNode,
};

/// `decoration` with a finger on it, `press` far into the fade.
///
/// The same shape, the same border, its fill washed towards `ink` — the colour
/// of whatever the control draws *on* that fill. That is the only colour
/// guaranteed to contrast with it, which is what makes one rule work for a
/// filled control, an outlined one and one with no fill at all.
///
/// The alternative — a second translucent layer painted over the first — would
/// mean a second decoration with the same radius and the same border to keep in
/// step, and a second command in the stream for every control on screen. See
/// [`Color::over`], which is why it does not have to be one.
pub(crate) fn pressed_fill(decoration: BoxDecoration, ink: Color, press: f32) -> BoxDecoration {
    BoxDecoration {
        color: ColorScheme::pressed(decoration.color, ink, press),
        ..decoration
    }
}

/// Centre `child` in a box at least `min` logical pixels on each side.
///
/// A minimum rather than a fixed size, so a control grows with its content — a
/// button with a long label, a row that fills a screen — but never shrinks below
/// what a finger can hit.
///
/// The centring is a shrink-wrapping row rather than an
/// [`Align`](crate::Align), deliberately: `Align` expands to fill whatever
/// bounded space it is given, so a button inside one would be as tall as the
/// screen. A row with [`MainAxisSize::Min`] takes its size from its child and
/// then centres that child within whatever minimum the constraints impose,
/// which is exactly what a touch target wants.
///
/// Where it sits relative to a control's decoration is the control's own call,
/// and both answers are right: a button puts it *inside* its decoration, because
/// the fill is meant to cover everything a finger can hit; a switch puts it
/// *outside*, because a track stretched to 48 pixels is not a switch.
pub(crate) fn touch_target(min: f32, child: impl Into<WidgetNode>) -> Constrained {
    Constrained::new(Constraints::new(min, f32::INFINITY, min, f32::INFINITY)).child(
        Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .push(child.into()),
    )
}

/// Raise `child`'s minimum height to `min`, leaving its width exactly as its
/// parent gave it.
///
/// [`touch_target`] is wrong for a row that is already meant to span whatever
/// width it is given — an accordion header spread with
/// [`MainAxisAlignment::SpaceBetween`], a segmented control's tab — because its
/// inner `Flex::row` shrink-wraps to the child's own width, which would collapse
/// exactly the span the row exists to keep. `Constrained`'s extra constraints
/// are *enforced against* the incoming ones rather than replacing them (see
/// `vieww-render::objects::constrained`), so a `0..∞` width here changes
/// nothing about the width the parent already settled, and only the height
/// floor is new.
pub(crate) fn min_height(min: f32, child: impl Into<WidgetNode>) -> Constrained {
    Constrained::new(Constraints::new(0.0, f32::INFINITY, min, f32::INFINITY)).child(child.into())
}
