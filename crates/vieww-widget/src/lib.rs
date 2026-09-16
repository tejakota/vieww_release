//! The widget layer — the first of `vieww`'s three trees.
//!
//! A widget is a **cheap, immutable, disposable description** of what the UI
//! should look like. Widgets are rebuilt constantly and thrown away; nothing
//! durable lives here. Identity across rebuilds belongs to the element tree
//! (Phase 2), and pixels belong to the render tree (Phase 3).
//!
//! Concretely, that means this crate must never depend on `vieww-render`, and a
//! widget must never hold mutable state. If you find yourself wanting either,
//! the thing you want is an element.
//!
//! ```
//! use vieww_widget::prelude::*;
//!
//! let tree = Container::new()
//!     .padding(EdgeInsets::all(16.0))
//!     .color(Color::WHITE)
//!     .child(Flex::column().children(children![
//!         Text::new("Ada Lovelace"),
//!         Text::new("Analyst"),
//!     ]));
//!
//! // Nothing has rendered — this is just a description, walkable in memory.
//! println!("{}", vieww_widget::debug_tree(tree));
//! ```

mod calendar_names;
mod context;
mod controls;
mod debug;
mod directionality;
pub mod forms;
pub mod icons;
mod localizations;
mod node;
mod theme;
mod tiers;
mod tokens;
mod widget;
mod widgets;

pub use calendar_names::CalendarNames;
pub use context::{BuildContext, Inherited, InheritedScope, Provision};
pub use controls::*;
pub use debug::{debug_tree, inflate, DebugNode};
pub use directionality::{AccessibilityOf, Directionality};
pub use localizations::Localizations;
pub use node::WidgetNode;
pub use theme::{
    ColorScheme, Metrics, Motion, Theme, ThemeData, Typography, DISABLED_ALPHA, PRESSED_ALPHA,
};
pub use tiers::{TierBudget, TierBudgetProvider, TierGate, Tiered, WidgetTier};
pub use tokens::{
    DesignToken, TokenSet, WindowSizeClass, WindowSizeClassProvider, COMPACT_MEDIUM_BOUNDARY,
    MEDIUM_EXPANDED_BOUNDARY,
};
pub use widget::{ElementState, Widget, WidgetKind};
pub use widgets::*;

pub use forms::{AllParsed, FormField, FormGroup, FromInput};

/// Write a composed widget's `build`, and let the attribute write the rest.
///
/// ```
/// use vieww_widget::prelude::*;
///
/// #[derive(Debug)]
/// struct Greeting {
///     name: String,
/// }
///
/// #[widget]
/// impl Greeting {
///     fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
///         Text::new(format!("Hello, {}!", self.name))
///     }
/// }
///
/// // `debug_name`, `kind` and `From<Greeting> for WidgetNode` are all there.
/// let node = WidgetNode::from(Greeting { name: "Ada".into() });
/// assert_eq!(node.debug_name(), "Greeting");
/// ```
///
/// See [`vieww_widget_macros`] for exactly what it generates, what it
/// deliberately does not (`#[derive(Debug)]`, notably), and how a widget that
/// is not composed opts out of the generated `kind`.
pub use vieww_widget_macros::widget;

pub use vieww_animation as animation;
pub use vieww_foundation as foundation;

/// Everything you need to describe a widget tree.
pub mod prelude {
    pub use crate::{
        children, icons, widget, widget_node_from, AccessibilityOf, Accordion, Align,
        AlignDirectional, Animated, AnimatedContainer, AspectRatio, AsyncBuilder, Autocomplete,
        Avatar, Badge, BarChart, BlockSemantics, BottomNavItem, BottomNavigation, BottomSheet,
        Breadcrumbs, BuildContext, Button, ButtonStyle, CalendarNames, Carousel, Center, Checkbox,
        Chip, Circle, CircularProgress, Clip, ClipShape, ColorPicker, ColorScheme, ColoredBox,
        Confirmation, Constrained, Container, CrossAxisAlignment, CursorArea, Curve, CustomPaint,
        CustomPainter, CustomScrollView, DataColumn, DataTable, DatePicker, DecoratedBox, Dialog,
        Directionality, DragTarget, Draggable, DrawInstruction, Drawer, DrawerSide, Dropdown,
        EmptyState, ExcludeSemantics, FileDropZone, Filtered, Fitted, Flex, FlexFactor, FlexFit,
        Flexible, FloatingActionButton, FocusTrap, GestureDetector, Grid, GridTrack, GridView,
        Icon, IgnorePointer, Image, InlineError, IntrinsicSize, LayoutBuilder, LineChart,
        LinearProgress, ListView, Localizations, MainAxisAlignment, MainAxisSize, Markdown,
        MatchMode, Measured, MeasuredConstraints, Menu, MenuItem, Metrics, ModalBarrier,
        MorphShape, Motion, Navigator, Offstage, Opacity, Overlay, OverlayHandle, OverlayId,
        OverlayLease, OverlayPosition, Padding, PaddingDirectional, Pagination, PaintWith, Painter,
        Painting, Positioned, PositionedDirectional, Pressable, Radio, RepaintBoundary, RichText,
        RoundedRectangle, Route, SafeArea, ScrollExtents, ScrollMetrics, Scrollable,
        SegmentedControl, SemanticLiveness, SemanticRole, Semantics, Sense, Sensitive,
        SensitiveMask, ShapeMorph, SizedBox, Skeleton, Slider, SliverAppBar, SliverList,
        SliverRefresh, Snackbar, Span, Stack, StackFit, StackPosition, StackPositionDirectional,
        StreamBuilder, Svg, Switch, TabBar, Text, TextField, Theme, ThemeData, TimePicker, Tooltip,
        Transformed, TreeNode, TreeView, Tween, Typography, Viewport, Widget, WidgetKind,
        WidgetNode, TREE_INDENT,
    };
    pub use vieww_foundation::{
        Accessibility, Alignment, AlignmentDirectional, Axis, BlendMode, Border, BoxDecoration,
        BoxFit, Brush, Capture, Color, Constraints, EdgeInsets, EdgeInsetsDirectional, FontWeight,
        Gradient, IconData, Key, Locale, Offset, Path, PluralCategory, Rect, Shadow, Size, Sketch,
        Sketchbook, TargetPlatform, TextAlign, TextDirection, TextEditingValue, TextPosition,
        TextRange, TextSelection, TextStyle, Transform, VectorImage, VectorShape, ViewMetrics,
    };
}
