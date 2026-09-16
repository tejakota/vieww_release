//! The Phase 1 widget primitives.
//!
//! Deliberately small. These are the widgets every later widget is built out
//! of, so each one either owns a distinct piece of layout behaviour
//! ([`Padding`], [`Align`], [`Flex`], [`Stack`], [`Constrained`]) or composes
//! those that do ([`Container`], [`Center`], [`SizedBox`],
//! [`AnimatedContainer`]).

mod align;
mod animated;
mod animated_container;
mod aspect_ratio;
mod async_builder;
mod block_semantics;
mod carousel;
mod clip;
mod color_picker;
mod colored_box;
mod constrained;
mod container;
mod cursor_area;
mod custom_paint;
mod decorated_box;
mod directional;
mod error_placeholder;
mod exclude_semantics;
mod filtered;
mod fitted;
mod flex;
mod flexible;
mod focus_trap;
mod gesture_detector;
mod grid;
mod icon;
mod ignore_pointer;
mod image;
mod intrinsic;
mod layout_builder;
mod line_chart;
mod measured;
mod offstage;
mod opacity;
mod padding;
mod painting;
mod performance_overlay;
mod positioned;
mod pressable;
mod repaint_boundary;
mod rich_text;
mod safe_area;
mod semantics;
mod sensitive;
mod shape_morph;
mod stack;
mod stream_builder;
mod svg;
mod text;
mod text_field;
mod transformed;
mod viewport;

// `charts.rs` and `widgets/directionality.rs` were deleted in patch 5: the
// first is superseded by `line_chart.rs`, which draws the same `LineChart` and
// `BarChart` through `CustomPaint`; the second duplicated `src/directionality.rs`
// one level up, which is the one that builds.

pub use align::{Align, Center};
pub use animated::{Animated, AnimatedBuilder, AnimatedState, CONTROL_DURATION};
pub use animated_container::{AnimatedContainer, AnimatedContainerState, AnimatedProps};
pub use aspect_ratio::AspectRatio;
pub use async_builder::AsyncBuilder;
pub use block_semantics::BlockSemantics;
pub use carousel::{Carousel, CarouselState};
pub use clip::{Clip, ClipShape};
pub use color_picker::{ColorPicker, ColorPickerState};
pub use colored_box::ColoredBox;
pub use constrained::{Constrained, SizedBox};
pub use container::Container;
pub use cursor_area::CursorArea;
pub use custom_paint::{CustomPaint, CustomPainter, DrawInstruction, DrawOnce};
pub use decorated_box::DecoratedBox;
pub use directional::{AlignDirectional, PaddingDirectional, PositionedDirectional};
pub use error_placeholder::{ErrorPlaceholder, ERROR_PLACEHOLDER_COLOR, ERROR_PLACEHOLDER_SIZE};
pub use exclude_semantics::ExcludeSemantics;
pub use filtered::Filtered;
pub use fitted::Fitted;
pub use flex::{CrossAxisAlignment, Flex, FlexFactor, FlexFit, MainAxisAlignment, MainAxisSize};
pub use flexible::Flexible;
pub use focus_trap::FocusTrap;
pub use gesture_detector::{GestureDetector, GestureHandlers, Handler};
pub use grid::{Grid, GridTrack};
pub use icon::{Icon, DEFAULT_ICON_SIZE};
pub use ignore_pointer::IgnorePointer;
pub use image::Image;
pub use intrinsic::IntrinsicSize;
pub use layout_builder::{LayoutBuilder, LayoutBuilderState, MeasuredConstraints};
pub use line_chart::{BarChart, LineChart};
pub use measured::Measured;
pub use offstage::Offstage;
pub use opacity::Opacity;
pub use padding::Padding;
pub use painting::{PaintWith, Painter, Painting};
pub use performance_overlay::PerformanceOverlay;
pub use positioned::{Positioned, StackPosition, StackPositionDirectional};
pub use pressable::{PressBuilder, PressState, Pressable, Sense};
pub use repaint_boundary::RepaintBoundary;
pub use rich_text::{RichText, Span};
pub use safe_area::SafeArea;
pub use semantics::{SemanticLiveness, SemanticRole, Semantics};
pub use sensitive::{Sensitive, SensitiveMask};
pub use shape_morph::{Circle, MorphShape, RoundedRectangle, ShapeMorph};
pub use stack::{Stack, StackFit};
pub use stream_builder::StreamBuilder;
pub use svg::Svg;
pub use text::Text;
pub use text_field::TextField;
pub use transformed::Transformed;
pub use viewport::{ScrollExtents, Viewport};
pub use vieww_animation::{AnimationController, Curve, Lerp, Tween};
pub use vieww_foundation::{FontWeight, TextStyle};
