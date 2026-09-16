//! Foundation types shared by every layer of `vieww`.
//!
//! This crate knows nothing about widgets, elements or pixels. It is the
//! vocabulary the other layers speak in.
//!
//! Its only dependency is `unicode-segmentation`, for the grapheme cluster
//! boundaries [`TextEditingValue`] moves a caret over. Those boundaries are a
//! Unicode table, not an algorithm — hand-rolling them is the failure mode where
//! backspace leaves half an emoji behind, and the crate has no dependencies of
//! its own.
//!
//! The one module that is not vocabulary is [`crash`], and it is here for the
//! same reason the vocabulary is: a panic hook is global, so the type
//! describing a panic has to be visible from every layer, and this is the only
//! crate all of them already depend on. It adds no dependency either.
//!
//! See `docs/DESIGN.md` §4 for why every scalar here is `f32`.

mod accessibility;
mod alignment;
pub mod app;
pub mod capability;
mod capture;
mod color;
mod constraints;
pub mod crash;
mod date;
mod decoration;
pub mod desktop;
mod edge_insets;
mod editing;
mod fast_hash;
mod file_drop;
pub mod filter;
mod geometry;
mod glyph;
pub mod hdr;
mod icon;
mod image;
pub mod intl;
mod key;
mod keyboard;
mod locale;
mod memory;
pub mod mobile;
mod paint;
mod path;
pub mod permission;
mod platform;
mod pointer;
pub mod service;
mod sketch;
pub mod stream;
mod svg;
pub mod task;
mod text;
mod text_layout;
mod transform;
mod vector;
mod view;

pub use accessibility::{Accessibility, MAX_TEXT_SCALE, MIN_TEXT_SCALE};
pub use alignment::{Alignment, AlignmentDirectional};
pub use capture::Capture;
pub use color::{Color, ColorSpace, Oklab};
pub use constraints::Constraints;
pub use date::{days_in_month, is_leap_year, Date, HalfDay, Time, Weekday};
pub use decoration::{Border, BoxDecoration};
pub use edge_insets::{EdgeInsets, EdgeInsetsDirectional};
pub use editing::{
    Affinity, Obscured, TextDecoration, TextDecorationShape, TextEditingValue, TextPosition,
    TextRange, TextSelection, DEFAULT_OBSCURING_CHARACTER,
};
pub use fast_hash::{FastHasher, FastMap, FastSet};
pub use file_drop::{DroppedFiles, FileDrag};
pub use filter::{
    apply_color_matrix_premultiplied, blur_rgba, brightness_matrix, compose_matrices,
    grayscale_matrix, identity_matrix, saturation_matrix, sepia_matrix, tint_matrix, ColorMatrix,
    ImageFilter,
};
pub use geometry::{Offset, Rect, Size};
pub use glyph::{FontData, FontVariation, Glyph, GlyphRun};
pub use icon::IconData;
pub use image::{BoxFit, Image};
pub use key::Key;
pub use keyboard::{ImeEvent, KeyEvent, KeyState, LogicalKey, Modifiers, NamedKey, TextIntent};
pub use locale::{Locale, PluralCategory};
pub use memory::{MemoryPressure, Trim};
pub use paint::{
    fade, BlendMode, Dash, Gradient, GradientGeometry, GradientStop, Shadow, StrokeCap, StrokeJoin,
    StrokeStyle, MAX_GRADIENT_STOPS,
};
pub use path::{Path, PathVerb};
pub use permission::{
    AlwaysDenied, AlwaysGranted, AlwaysRestricted, Gate, Grant, Guarded, Permission,
    PermissionState, Permissions, ScriptedPermissions,
};
pub use platform::TargetPlatform;
pub use pointer::{
    Cursor, DragDetails, LongPressDetails, PointerButton, PointerDeviceKind, PointerEvent,
    PointerId, PointerPhase, ScaleDetails, ScrollEvent, TapDetails, LONG_PRESS_TIMEOUT,
    MULTI_TAP_SLOP, MULTI_TAP_TIMEOUT, PRESS_TIMEOUT, TAP_TIMEOUT,
};
pub use service::{
    Clipboard, DeepLink, DeepLinks, KeyBacking, MemoryClipboard, MemorySecureStorage,
    MemoryStorage, SecureStorage, ServiceError, Services, SharedServices, Storage,
};
pub use sketch::{Brush, Sketch, Sketchbook};
pub use svg::{parse_path_data, SvgPathError};
pub use text::{FontFamily, FontWeight, TextAlign, TextDirection, TextStyle};
pub use text_layout::{TextLayoutProbe, TextLayoutReport};
pub use transform::Transform;
pub use vector::{VectorImage, VectorShape};
pub use view::ViewMetrics;

pub use app::{
    AccessibilityAnnouncements, Announcement, AppLifecycle, FeatureFlags, FlagValue,
    HeadlessLifecycle, InMemoryFlags, LifecycleState, LogLevel, RecordingAnnouncements,
    RecordingTelemetry, ScriptedUpdates, Telemetry, TelemetryEvent, UpdateChannel, UpdateState,
};

/// The axis a [`Flex`](https://docs.rs/vieww-widget)-style container lays out along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Children are laid out left-to-right.
    Horizontal,
    /// Children are laid out top-to-bottom.
    Vertical,
}

impl Axis {
    /// The axis perpendicular to this one.
    #[must_use]
    pub const fn cross(self) -> Self {
        match self {
            Self::Horizontal => Self::Vertical,
            Self::Vertical => Self::Horizontal,
        }
    }
}
