//! `vieww` — a Rust UI framework with a three-tree architecture.
//!
//! Three trees, each with one job:
//!
//! | tree | crate | lifetime | job |
//! |---|---|---|---|
//! | Widget | [`vieww_widget`] | one frame | describe the UI |
//! | Element | [`vieww_element`] | across frames | hold identity and state |
//! | Render | [`vieww_render`] | across frames | lay out and paint |
//!
//! plus [`vieww_paint`], which sits *below* the render tree and supplies the
//! canvas it paints onto, the layers it composites into, and the scheduler that
//! decides when a frame runs.
//!
//! plus [`vieww_gestures`], which turns pointer events into gestures, and
//! [`vieww_animation`], which turns frames into moving values. Both sit beside
//! the paint layer, above foundation and below everything else.
//!
//! Current status: **all ten phases complete except hot reload.** Widget trees
//! build, mount into a persistent element tree, rebuild only where state
//! changed, lay out into sized, positioned, hit-testable geometry, shape real
//! text, rasterise on the GPU repainting only what changed, disambiguate
//! gestures through a contest arena, animate — by controller, by spring, or
//! implicitly through [`AnimatedContainer`] — open a window, take keys and an
//! input method, publish a semantics tree to AccessKit, and assemble a real
//! screen out of themed controls: buttons, switches, sliders, chips, a
//! virtualised list, a navigation stack and dialogs.
//!
//! [`FrameDriver`] joins every layer; enable the `gpu` feature for the vello
//! backend.
//!
//! **It runs on Android, at speed.** `android_main` is wired, and a real phone
//! — a Redmi Note 7 Pro, Adreno 612 — renders **59.3fps over ten seconds with
//! 4.09ms of median frame work against a 16.67ms budget**, respects its display
//! cutout, answers a finger (taps land, and a swipe across a button is
//! correctly *not* a tap), and reports its soft keyboard as `view_insets`
//! separately from the safe area, so content scrolls out from under it.
//! `cargo apk run -p vieww-platform-winit --example android --release`, and
//! `ci/mobile/device-suite.sh` is what checks all of that on the device rather than
//! asserting it here.
//!
//! **iOS is disputed.** This page, `vieww-platform-winit`'s page and
//! `platform-winit/src/ios.rs` all say no frame has reached an iPhone and that
//! the UIKit calls behind the safe area and the keyboard have never executed.
//! `docs/PRODUCTION-GAPS.md` says the opposite, with a device and a date. Three
//! files against one document is not a vote, and it is not settled here — see
//! the note under that document's platform table. Treat iOS as unverified until
//! somebody with the hardware deletes whichever of the two is wrong.
//!
//! **Neither phone's screen reader has been heard.** The semantics tree is
//! built and current, and `accesskit_winit` 0.33 does have an Android backend
//! and does wire the iOS one — this page used to say otherwise, and
//! `vieww-platform-winit`'s own docs corrected it while this one kept the old
//! claim, which is the kind of disagreement between two documents that is worse
//! than either being wrong alone. What is missing is not code. It is that
//! nobody has switched TalkBack or VoiceOver on and listened.
//!
//! See `docs/ROADMAP.md` for the phase plan, `docs/DESIGN.md` for the decisions
//! behind it, and `docs/STABILITY.md` for what any of this promises about not
//! breaking — which today is deliberately nothing.
//!
//! ```
//! use vieww::prelude::*;
//!
//! let tree = Container::new()
//!     .padding(EdgeInsets::all(12.0))
//!     .child(Flex::column().children(children![
//!         Text::new("Hello").bold(),
//!         Text::new("world"),
//!     ]));
//!
//! assert!(vieww::debug_tree(tree).contains("Hello"));
//! ```

pub use vieww_animation as animation;
pub use vieww_effects as effects;
pub use vieww_element as element;
pub use vieww_foundation as foundation;
pub use vieww_gestures as gestures;
pub use vieww_paint as paint;
pub use vieww_render as render;
pub use vieww_text as text;
pub use vieww_widget as widget;

pub use vieww_animation::{
    AnimationController, AnimationStatus, Curve, Fling, Lerp, Motion, Simulation, Spring, Ticker,
    Tickers, Tween,
};
pub use vieww_asset::{AssetBundle, AssetError, DirectoryBundle, EmbeddedBundle, ImageCache};
pub use vieww_element::{
    Animation, BuildError, DragController, DragTargetId, Dropped, Element, ElementId, ElementTree,
    ErrorPolicy, Hotspot, Memo, NavigatorController, Runtime, ScrollController, Signal,
};
/// The decoded-pixel type [`vieww_foundation::Image`], under a name that does
/// not collide with the [`Image`] *widget* the prelude
/// re-exports.
///
/// The facade glob-imports `vieww_widget::prelude::*`, whose `Image` is the
/// widget. Without this alias the pixel type is unreachable from a previewed
/// buffer: `vieww::Image` resolves to the widget, and a buffer that calls
/// `Image::new(vieww::Image::from_rgba8(...))` is naming the same widget twice
/// and never names the pixels at all. `vieww::foundation::Image` still resolves
/// — the foundation crate is re-exported by name above — but a buffer that
/// has only `use vieww::prelude::*;` cannot name it. `ImageData` is the one
/// the prelude-less buffer reaches for.
///
/// `Image as ImageData` rather than the other way round, because the widget
/// is what a buffer describes and the pixel type is what it names on purpose —
/// the same division as `vieww_foundation`'s own module docs, which put the
/// widget behind a `pub use` and the pixel type behind a `pub`.
pub use vieww_foundation::Image as ImageData;
pub use vieww_foundation::{
    Affinity, BlendMode, BoxFit, Date, DeepLink, DeepLinks, DragDetails, Gradient, HalfDay,
    IconData, ImeEvent, KeyEvent, KeyState, LogicalKey, LongPressDetails, MemoryStorage, Modifiers,
    NamedKey, PointerEvent, PointerId, PointerPhase, ScaleDetails, ServiceError, Services, Shadow,
    SharedServices, Storage, TapDetails, TextEditingValue, TextIntent, TextPosition, TextRange,
    TextSelection, Time, ViewMetrics, Weekday,
};
pub use vieww_gestures::{Overscroll, Recognized, ScrollPhysics, ScrollPosition};
pub use vieww_paint::{
    Canvas, Damage, FramePhase, FrameScheduler, FrameSink, Layer, LayerId, LayerTree, Paint, Path,
    Scene,
};
pub use vieww_render::{
    set_render_panic_sink, set_unregistered_render_object_sink, Announcement, Dispatched,
    FocusManager, FrameDriver, HitTestResult, Liveness, PointerRouter, RenderId, RenderObject,
    RenderOwner, RenderTree, Role, ScrollDirection, SemanticsNode, SemanticsTree,
    SliverConstraints, SliverGeometry, OVERLAY_HEIGHT,
};
// Everything `vieww_widget::prelude` exports, re-exported at the facade's root
// by *glob* rather than by name.
//
// It used to be a hand-written list, and the two lists diverged the way two
// hand-written lists do: 62 names had been added to the widget prelude and
// never mirrored here, so `vieww::Text`, `vieww::Container`, `vieww::Flex` and
// `vieww::Stack` did not resolve while `vieww::Button` did — and
// `vieww::OverlayPosition` resolved while `vieww::Overlay` did not. Nothing
// caught it: a name missing from a re-export list is not a compile error here,
// it is a compile error in somebody else's crate.
//
// A glob cannot go stale. Explicit `use` beats a glob in Rust's name
// resolution, so the named re-exports above still win on collision — `Path`
// stays `vieww_paint::Path`, exactly as before.
//
// `tests/facade_exports.rs` names one widget from each family through `vieww::`
// and stops compiling if the glob stops carrying it.
pub use vieww_widget::prelude::*;

/// The parts of `vieww_widget` that are deliberately not in its prelude.
///
/// A prelude is what you glob into a file that builds widgets. These are the
/// types you name on purpose: the debug and error surfaces, the drag protocol,
/// the form parser, and the state a control hands to a builder.
/// Write a composed widget's `build`, and let the attribute write the rest —
/// `debug_name`, `kind`, and the `WidgetNode` conversion.
///
/// Re-exported here rather than only from `vieww-widget` because an
/// application depends on this facade alone; see `vieww-widget-macros` for
/// what it generates and why its expansion names the widget types
/// unqualified (which is what makes it work through this re-export).
pub use vieww_widget::widget;

pub use vieww_widget::{
    debug_tree, inflate, AnimatedProps, DebugNode, DragKind, DragPayload, DragSession,
    ElementState, ErrorPlaceholder, FormField, FromInput, Inherited, InheritedScope, ItemBuilder,
    PerformanceOverlay, PressState, RouteBuilder, RouteTransition, SliderBar, SortDirection,
    CONTROL_DURATION, ROUTE_DURATION,
};

/// Everything you need to describe a widget tree and drive it.
pub mod prelude {
    pub use vieww_animation::{Curve, Tween};
    pub use vieww_element::{Animation, ElementTree, Memo, Runtime, Signal};
    pub use vieww_gestures::{ScrollPhysics, ScrollPosition};
    pub use vieww_paint::{Canvas, FrameScheduler, Scene, Stroke};
    pub use vieww_render::{FrameDriver, RenderOwner};
    pub use vieww_widget::prelude::*;
    pub use vieww_widget::{widget, ElementState, Inherited};
}
