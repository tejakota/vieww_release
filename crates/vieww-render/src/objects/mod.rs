//! The core render objects.
//!
//! Each owns exactly one layout behaviour. Between them they cover everything
//! the Phase 1 widget primitives describe.

mod align;
mod aspect_ratio;
mod block_semantics;
mod circular_progress;
mod clip;
mod colored_box;
mod constrained;
mod cursor_area;
mod decorated_box;
mod editable_text;
mod exclude_semantics;
mod filter;
mod fitted;
mod flex;
mod flexible;
mod focus_trap;
mod gesture_detector;
mod icon;
mod ignore_pointer;
mod image;
mod intrinsic;
mod measured;
mod measured_constraints;
mod offstage;
mod opacity;
mod padding;
mod painting;
mod performance_overlay;
mod positioned;
mod repaint_boundary;
mod rich_text;
mod scroll_view;
mod semantics;
mod sensitive;
mod slider;
mod sliver_app_bar;
mod sliver_list;
mod sliver_refresh;
mod stack;
mod text;
mod transform;
mod vector_image;
mod viewport;

pub use align::RenderAlign;
pub use aspect_ratio::{RenderAspectRatio, FALLBACK_RATIO};
pub use block_semantics::RenderBlockSemantics;
pub use circular_progress::RenderCircularProgress;
pub use clip::{ClipShape, RenderClip};
pub use colored_box::RenderColoredBox;
pub use constrained::RenderConstrainedBox;
pub use decorated_box::RenderDecoratedBox;
pub use editable_text::{
    RenderEditableText, SelectionChanged, Submitted, ValueChanged, DEFAULT_PAGE_LINES,
};
pub use exclude_semantics::RenderExcludeSemantics;
pub use fitted::RenderFitted;
pub use flex::RenderFlex;
pub use flexible::RenderFlexible;
pub use focus_trap::RenderFocusTrap;
pub use gesture_detector::RenderGestureDetector;
pub use icon::RenderIcon;
pub use rich_text::{LinkHandler, RenderRichText};
// `RenderImage` holds `vieww_foundation::Image`; the pixels are re-exported by
// `vieww-paint` too, so nothing here needs its own name for them.
pub use cursor_area::RenderCursorArea;
pub use filter::RenderFilter;
pub use ignore_pointer::RenderIgnorePointer;
pub use image::RenderImage;
pub use intrinsic::RenderIntrinsicSize;
pub use measured::RenderMeasured;
pub use measured_constraints::RenderMeasuredConstraints;
pub use offstage::RenderOffstage;
pub use opacity::RenderOpacity;
pub use padding::RenderPadding;
pub use painting::RenderPainting;
pub use performance_overlay::{RenderPerformanceOverlay, BUDGET_FRACTION, OVERLAY_HEIGHT};
pub use positioned::RenderPositioned;
pub use repaint_boundary::RenderRepaintBoundary;
pub use scroll_view::RenderScrollView;
pub use semantics::RenderSemantics;
pub use sensitive::RenderSensitive;
pub use slider::RenderSlider;
pub use sliver_app_bar::{HeaderBehaviour, RenderSliverAppBar};
pub use sliver_list::RenderSliverFixedList;
pub use sliver_refresh::RenderSliverRefresh;
pub use stack::RenderStack;
pub use text::RenderText;
pub use transform::RenderTransform;
pub use vector_image::RenderVectorImage;
pub use viewport::RenderViewport;
