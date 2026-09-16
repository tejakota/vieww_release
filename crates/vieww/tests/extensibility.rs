//! Every extension seam, exercised from outside the framework.
//!
//! ```console
//! cargo test -p vieww --test extensibility
//! ```
//!
//! # Why this file exists
//!
//! `FrameDriver::register` was public, documented in two places as *the* way to
//! extend vieww, unit-tested at its own level — and unreachable from an
//! application, because the only route to it returned `&RenderOwner`. Nothing
//! caught that for as long as it existed, and nothing could have: every test
//! that touched the registry lived inside `vieww-render`, where an owner is
//! there for the taking.
//!
//! The lesson is not about that seam. It is that **a seam tested from inside is
//! not tested**, and vieww had ten more of them in exactly that state. So this
//! file stands where an application stands — nothing but what `vieww`
//! re-exports — and uses each one as a third party would.
//!
//! `third_party_render_widget.rs` is the eleventh, kept separate because it is
//! about the accessor that was missing rather than about a seam that was fine.
//!
//! # What each test is allowed to assume
//!
//! Nothing that a crate outside this repository could not. No `pub(crate)`, no
//! reaching into `vieww_render` past the facade, no constructing internals by
//! hand. If a test here needs something the facade does not export, that is the
//! finding, not an inconvenience to work around.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use vieww::animation::Lerp;
use vieww::element::ErrorPolicy;
use vieww::foundation::stream::Stream;
use vieww::foundation::task::{AsyncValue, FrameWaker, Spawn};
use vieww::foundation::{
    Color, Constraints, GlyphRun, Offset, Path, PointerEvent, PointerId, PointerPhase, Rect,
    Services, SharedServices, Size, Transform,
};
use vieww::gestures::{GestureDisposition, GestureRecognizer, Sink};
use vieww::paint::{Image, Paint};
use vieww::prelude::*;
use vieww::render::{LayoutCtx, PaintCtx, SliverConstraints, SliverGeometry};
use vieww::{AssetBundle, AssetError, RenderObject, RouteTransition};

const SURFACE: Size = Size {
    width: 200.0,
    height: 200.0,
};
const POINTER: PointerId = PointerId(1);

fn driver() -> FrameDriver {
    FrameDriver::new(SURFACE)
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

// ------------------------------------------------------------ 1. composed widget

/// The seam every application uses first, and the only one that needs no
/// registration: a widget that owns no pixels and just assembles built-ins.
#[derive(Debug)]
struct Panel {
    ink: Color,
}

impl Widget for Panel {
    fn debug_name(&self) -> &'static str {
        "Panel"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        ColoredBox::new(self.ink)
            .child(SizedBox::square(24.0))
            .into()
    }
}

vieww::widget::widget_node_from!(Panel);

#[test]
fn a_composed_widget_from_outside_builds_into_the_tree() {
    let mut driver = driver();
    driver.set_root(Center::new().child(Panel { ink: Color::RED }));
    driver.draw_frame();

    let fills = driver.scene().fills();
    assert_eq!(fills.len(), 1, "the panel's box");
    assert_eq!(fills[0].1.color, Color::RED);
    assert_eq!(
        fills[0].0.size(),
        Size::new(24.0, 24.0),
        "and it sized itself from what it built, not from the window"
    );
}

// -------------------------------------------------------------------- 2. sliver

/// A sliver of its own, which is the answer `HeaderBehaviour` gives to anybody
/// wanting a scroll effect this repository did not think of.
#[derive(Debug)]
struct Band {
    ink: Color,
    extent: f32,
}

impl Widget for Band {
    fn debug_name(&self) -> &'static str {
        "Band"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }
}

vieww::widget::widget_node_from!(Band);

#[derive(Debug)]
struct RenderBand {
    ink: Color,
    extent: f32,
}

impl RenderObject for RenderBand {
    fn debug_name(&self) -> &'static str {
        "RenderBand"
    }

    /// What it would be under box layout. A sliver still answers this, because
    /// the same object can be put somewhere that is not a viewport.
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(Size::new(constraints.max_width, self.extent))
    }

    /// The sliver protocol, through a **defaulted** trait method — no separate
    /// trait to implement and nothing to register beyond the widget itself.
    fn layout_sliver(
        &mut self,
        _ctx: &mut LayoutCtx<'_>,
        constraints: &SliverConstraints,
    ) -> Option<SliverGeometry> {
        let painted = (self.extent - constraints.scroll_offset)
            .clamp(0.0, constraints.remaining_paint_extent);
        Some(SliverGeometry {
            scroll_extent: self.extent,
            paint_extent: painted,
            layout_extent: painted,
            max_paint_extent: self.extent,
            paint_origin: 0.0,
            hit_test_extent: painted,
            visible: painted > 0.0,
            has_visual_overflow: false,
        })
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        ctx.canvas().fill_rect(bounds, self.ink.into());
    }
}

#[test]
fn a_sliver_from_outside_is_laid_out_by_a_built_in_scroll_view() {
    let mut driver = driver();
    driver.register::<Band, RenderBand>(|band| RenderBand {
        ink: band.ink,
        extent: band.extent,
    });

    driver.set_root(CustomScrollView::vertical().child(Band {
        ink: Color::GREEN,
        extent: 60.0,
    }));
    driver.draw_frame();

    let band = driver
        .scene()
        .fills()
        .into_iter()
        .find(|(_, paint)| paint.color == Color::GREEN)
        .expect("the foreign sliver painted inside a built-in viewport");
    assert_eq!(
        band.0.height(),
        60.0,
        "at the extent its own `layout_sliver` reported — the scroll view asks \
         the protocol rather than asking what type this is"
    );
}

// ------------------------------------------------------------------ 3. gestures

/// A recogniser for a gesture vieww has no name for: two touches, with no timing
/// rule. The logic is not the point — being *in the arena* is, since that is what
/// lets it compete with a scroll instead of fighting it.
///
/// # A recogniser is created per gesture, so its own fields are not memory
///
/// `PointerRouter::open_route` calls `RenderObject::gesture_recognizers` on
/// **every pointer-down** and builds a fresh contest from what comes back, so
/// each gesture gets a new instance. That is right — a recogniser belongs to one
/// contest — and it means a `self.seen += 1` counting across gestures silently
/// never gets anywhere. The first version of this test did exactly that and
/// failed at 0, which is worth keeping written down because nothing about the
/// trait says it.
///
/// So anything spanning gestures lives outside, here in an `Rc<Cell<_>>` the
/// widget holds.
///
/// # The half that is not open, and the way round it
///
/// [`Recognized`](vieww::gestures::Recognized) is a closed enum of the gestures
/// this repository ships, so a foreign recogniser cannot *report* through the
/// sink — there is no variant for a gesture nobody has written. It calls its own
/// callback instead, which is what this does, and the sink stays for built-ins.
/// Arena participation is the valuable part and that is fully open; the reporting
/// vocabulary is not, and an application should know which half it is using.
#[derive(Debug)]
struct DoubleTouch {
    touches: Rc<Cell<usize>>,
    fired: Rc<Cell<usize>>,
}

impl GestureRecognizer for DoubleTouch {
    fn debug_name(&self) -> &'static str {
        "DoubleTouch"
    }

    fn handle(&mut self, event: &PointerEvent, _sink: Sink<'_>) -> Option<GestureDisposition> {
        if event.phase == PointerPhase::Down {
            self.touches.set(self.touches.get() + 1);
            if self.touches.get() == 2 {
                self.fired.set(self.fired.get() + 1);
                return Some(GestureDisposition::Accepted);
            }
        }
        None
    }
}

#[derive(Debug)]
struct Touchable {
    touches: Rc<Cell<usize>>,
    fired: Rc<Cell<usize>>,
}

impl Widget for Touchable {
    fn debug_name(&self) -> &'static str {
        "Touchable"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }
}

vieww::widget::widget_node_from!(Touchable);

#[derive(Debug)]
struct RenderTouchable {
    touches: Rc<Cell<usize>>,
    fired: Rc<Cell<usize>>,
}

impl RenderObject for RenderTouchable {
    fn debug_name(&self) -> &'static str {
        "RenderTouchable"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(Size::new(100.0, 100.0))
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        true
    }

    /// The seam: whatever this returns joins the arena for pointers that land
    /// here, alongside the built-in recognisers of any ancestor.
    fn gesture_recognizers(&self) -> Vec<Box<dyn GestureRecognizer>> {
        vec![Box::new(DoubleTouch {
            touches: Rc::clone(&self.touches),
            fired: Rc::clone(&self.fired),
        })]
    }
}

#[test]
fn a_gesture_recognizer_from_outside_joins_the_arena() {
    let touches = Rc::new(Cell::new(0));
    let fired = Rc::new(Cell::new(0));

    let mut driver = driver();
    // Built from the widget, which is what the constructor is handed and what an
    // application would use — the closure capturing the handles instead would
    // ignore the widget entirely and work by accident.
    driver.register::<Touchable, RenderTouchable>(|widget| RenderTouchable {
        touches: Rc::clone(&widget.touches),
        fired: Rc::clone(&widget.fired),
    });
    driver.set_root(Touchable {
        touches: Rc::clone(&touches),
        fired: Rc::clone(&fired),
    });
    driver.draw_frame();

    let at = Offset::new(50.0, 50.0);
    driver.handle_pointer(&PointerEvent::down(POINTER, at, ms(0)));
    assert_eq!(fired.get(), 0, "one touch is not the gesture");

    driver.handle_pointer(&PointerEvent::up(POINTER, at, ms(10)));
    driver.handle_pointer(&PointerEvent::down(POINTER, at, ms(20)));

    assert_eq!(
        fired.get(),
        1,
        "the second touch completed a gesture this framework has no name for, \
         through a recogniser it has never seen"
    );
}

// ------------------------------------------------------------------ 4. services

/// A capability vieww has never heard of. The registry is `TypeId`-keyed over
/// `dyn Trait`, so this needs no entry anywhere in the framework.
trait Telemetry {
    fn record(&self, event: &str);
}

#[derive(Default)]
struct Recorder {
    events: RefCell<Vec<String>>,
}

impl Telemetry for Recorder {
    fn record(&self, event: &str) {
        self.events.borrow_mut().push(event.to_owned());
    }
}

/// Reads the service the way a real screen does: out of the inherited scope,
/// during its own build.
#[derive(Debug)]
struct Instrumented;

impl Widget for Instrumented {
    fn debug_name(&self) -> &'static str {
        "Instrumented"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        if let Some(services) = ctx.inherit::<SharedServices>() {
            if let Some(telemetry) = services.get::<dyn Telemetry>() {
                telemetry.record("built");
            }
        }
        SizedBox::square(10.0).into()
    }
}

vieww::widget::widget_node_from!(Instrumented);

#[test]
fn a_service_vieww_has_never_heard_of_reaches_a_widget() {
    let recorder = Rc::new(Recorder::default());

    let mut registry = Services::new();
    registry.provide::<dyn Telemetry>(Rc::clone(&recorder) as Rc<dyn Telemetry>);

    let mut driver = driver();
    driver.set_root(Inherited::new(SharedServices::new(registry), Instrumented));
    driver.draw_frame();

    assert_eq!(
        recorder.events.borrow().as_slice(),
        ["built"],
        "the widget found a service defined entirely outside vieww, through the \
         same path `Storage` and `DeepLinks` take"
    );
}

// -------------------------------------------------------------- 5. spawn + wake

/// An executor of the application's own. Runs inline, which is all a test wants
/// — the seam is that vieww never picks one.
struct HereAndNow;

impl Spawn for HereAndNow {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        work();
    }
}

/// A waker of the application's own, counting instead of asking a window.
#[derive(Default)]
struct Counting(AtomicUsize);

impl FrameWaker for Counting {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_spawner_and_waker_from_outside_carry_a_value_to_the_screen() {
    let waker = Arc::new(Counting::default());

    let mut driver = driver();
    driver.set_root(Center::new().child(AsyncBuilder::new(
        Arc::new(HereAndNow),
        Arc::clone(&waker) as Arc<dyn FrameWaker>,
        || Ok::<_, String>(Color::BLUE),
        |value: &AsyncValue<Color, String>| match value {
            AsyncValue::Ready(color) => {
                ColoredBox::new(*color).child(SizedBox::square(12.0)).into()
            }
            _ => SizedBox::square(12.0).into(),
        },
    )));

    // Two frames: the first mounts and starts the work, the second shows what
    // `take_pending` collected. That is the same shape every async widget has.
    driver.draw_frame();
    driver.draw_frame();

    let fills = driver.scene().fills();
    assert_eq!(
        fills.first().map(|fill| fill.1.color),
        Some(Color::BLUE),
        "a value produced by the application's own executor reached the screen"
    );
    assert!(
        waker.0.load(Ordering::SeqCst) >= 1,
        "and the application's own waker was asked for the frame that showed it"
    );
}

#[test]
fn a_stream_runs_on_the_same_foreign_seam() {
    // Streams reuse `Spawn` and `FrameWaker` rather than introducing their own,
    // which is only worth anything if a foreign implementation of those works
    // unchanged. This is that, with no vieww-supplied spawner in sight.
    let waker = Arc::new(Counting::default());
    let mut stream: Stream<u32, ()> = Stream::spawn(
        &HereAndNow,
        Arc::clone(&waker) as Arc<dyn FrameWaker>,
        |emit| {
            emit.emit(1);
            emit.emit(2);
            Ok(())
        },
    );

    assert!(stream.poll());
    assert_eq!(stream.latest(), Some(&2));
    assert!(waker.0.load(Ordering::SeqCst) >= 2, "one wake per value");
}

// --------------------------------------------------------------------- 6. asset

/// Assets from wherever the application keeps them — a zip, a server, a map.
struct InMemory(Vec<(&'static str, &'static [u8])>);

impl AssetBundle for InMemory {
    fn open(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        self.0
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| bytes.to_vec())
            .ok_or_else(|| AssetError::NotFound(path.to_owned()))
    }
}

#[test]
fn an_asset_bundle_from_outside_is_reachable_through_the_registry() {
    let bundle = Rc::new(InMemory(vec![("greeting.txt", b"hello")]));

    let mut registry = Services::new();
    registry.provide::<dyn AssetBundle>(Rc::clone(&bundle) as Rc<dyn AssetBundle>);
    let services = SharedServices::new(registry);

    let found = services
        .get::<dyn AssetBundle>()
        .expect("the bundle is registered")
        .open("greeting.txt")
        .expect("and it opens");
    assert_eq!(found, b"hello");

    assert!(
        services
            .get::<dyn AssetBundle>()
            .expect("still registered")
            .contains("greeting.txt"),
        "and `contains` comes free from the trait's default"
    );
}

// ---------------------------------------------------------------------- 7. lerp

/// A type of the application's own, animated by the framework.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Celsius(f32);

impl Lerp for Celsius {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self(self.0.lerp(other.0, t))
    }
}

#[test]
fn a_type_from_outside_is_animated_by_implementing_one_method() {
    let warming = Tween::new(Celsius(10.0), Celsius(20.0));

    assert_eq!(warming.at(0.0), Celsius(10.0));
    assert_eq!(warming.at(0.5), Celsius(15.0));
    assert_eq!(warming.at(1.0), Celsius(20.0));
}

// -------------------------------------------------------------------- 8. canvas

/// A backend of the application's own: records instead of rasterising.
#[derive(Default)]
struct Recording {
    fills: Vec<(Rect, Paint)>,
    saves: usize,
}

impl Canvas for Recording {
    fn save(&mut self) {
        self.saves += 1;
    }
    fn restore(&mut self) {}
    fn transform(&mut self, _transform: Transform) {}
    fn clip_rect(&mut self, _rect: Rect) {}
    fn fill_rect(&mut self, rect: Rect, paint: Paint) {
        self.fills.push((rect, paint));
    }
    fn fill_path(&mut self, _path: &Path, _paint: Paint) {}
    fn draw_glyphs(&mut self, _run: &GlyphRun) {}
    fn draw_image(&mut self, _rect: Rect, _image: &Image) {}
}

#[test]
fn a_canvas_from_outside_receives_the_painted_tree() {
    let mut driver = driver();
    driver.set_root(Center::new().child(ColoredBox::new(Color::RED).child(SizedBox::square(30.0))));
    driver.draw_frame();

    let mut recording = Recording::default();
    driver.owner().paint(&mut recording);

    assert_eq!(recording.fills.len(), 1, "one box, one fill");
    assert_eq!(recording.fills[0].1.color, Color::RED);
    assert_eq!(
        recording.fills[0].0.size(),
        Size::new(30.0, 30.0),
        "in the coordinates layout decided, with no vello anywhere near it"
    );
}

// ----------------------------------------------------------------- 9. inherited

/// A value of the application's own, published to a subtree.
#[derive(Debug, PartialEq)]
struct Branding {
    accent: Color,
}

/// Reads it from wherever it happens to be in the tree.
#[derive(Debug)]
struct Accented;

impl Widget for Accented {
    fn debug_name(&self) -> &'static str {
        "Accented"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let accent = ctx
            .inherit::<Branding>()
            .map_or(Color::BLACK, |branding| branding.accent);
        ColoredBox::new(accent).child(SizedBox::square(16.0)).into()
    }
}

vieww::widget::widget_node_from!(Accented);

#[test]
fn a_value_from_outside_is_published_to_a_subtree() {
    let mut driver = driver();
    driver.set_root(Inherited::new(
        Branding {
            accent: Color::GREEN,
        },
        Center::new().child(Accented),
    ));
    driver.draw_frame();

    assert_eq!(
        driver.scene().fills()[0].1.color,
        Color::GREEN,
        "`Inherited<T>` is generic over the payload, so a type defined here \
         travels the same path `Theme` does"
    );
}

// --------------------------------------------------- 10. the two custom hatches

#[test]
fn a_curve_and_an_error_policy_from_outside_are_both_reachable() {
    // A curve nobody here wrote. Through `custom` rather than the literal, which
    // the variant asks for: `flipped` is bookkeeping the type owns, and building
    // it by hand means guessing at it.
    //
    // It takes a `fn` rather than a closure, so it cannot capture — a curve
    // parameterised by a value read from a theme cannot be expressed, and a
    // lookup table has to be a `static` rather than a `Vec`. `Curve::custom`'s
    // own documentation says exactly this, which is the right way for a limit to
    // be recorded.
    let squared = Curve::custom(|t| t * t);
    assert_eq!(squared.transform(0.5), 0.25);
    assert_eq!(squared.transform(1.0), 1.0);

    // An error policy nobody here wrote, with the same `fn` limit. A build that
    // panics is caught and this decides what stands in for it.
    let mut driver = driver();
    driver
        .elements()
        .set_error_policy(ErrorPolicy::Custom(|_error| {
            ColoredBox::new(Color::RED)
                .child(SizedBox::square(8.0))
                .into()
        }));
    driver.set_root(Center::new().child(Exploding));
    driver.draw_frame();

    assert_eq!(
        driver.scene().fills()[0].1.color,
        Color::RED,
        "the application's own placeholder is what a broken build shows"
    );
    assert_eq!(driver.scene().fills()[0].0.size(), Size::new(8.0, 8.0));
}

/// A widget whose build is a bug, so the policy above has something to catch.
#[derive(Debug)]
struct Exploding;

impl Widget for Exploding {
    fn debug_name(&self) -> &'static str {
        "Exploding"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        panic!("this build is broken on purpose")
    }
}

vieww::widget::widget_node_from!(Exploding);

// ------------------------------------------------------- 11. route transitions

#[test]
fn a_page_transition_from_outside_wraps_the_screen_it_is_given() {
    // **The last seam that was genuinely closed.** `RouteTransition` had four
    // variants and no way in: `t` exists only inside `wrap`, and the navigator
    // publishes no progress, so a route's own builder could not see how far in
    // it was. A custom page transition was impossible without editing vieww.
    //
    // It takes a closure rather than a `fn` because the transitions people
    // actually write capture something — a hero captures the rect it flies from,
    // this one captures a colour. A `fn` hatch would have closed the seam on
    // paper and left it useless.
    let veil = Color::rgb(7, 7, 7);
    let transition = RouteTransition::custom(move |screen, t, _surface| {
        Stack::new()
            .push(screen)
            .push(Opacity::new(1.0 - t).child(ColoredBox::new(veil)))
            .into()
    });

    let screen = ColoredBox::new(Color::BLUE)
        .child(SizedBox::square(20.0))
        .into();

    // Half way in: the screen is there and the application's own veil is over it
    // at half strength. `wrap` is what the navigator calls every frame of a push.
    let wrapped = transition.wrap(screen, 0.5, SURFACE);

    let mut driver = driver();
    driver.set_root(Center::new().child(wrapped));
    driver.draw_frame();

    let fills = driver.scene().fills();
    assert!(
        fills.iter().any(|(_, paint)| paint.color == Color::BLUE),
        "the screen the navigator handed the transition is still drawn"
    );
    assert!(
        fills.iter().any(|(_, paint)| paint.color == veil),
        "and so is a layer that exists nowhere in vieww, put there by a closure \
         that captured it"
    );
}

#[test]
fn a_custom_transition_is_not_instant_and_compares_by_identity() {
    // Two properties the navigator depends on. `is_instant` decides whether the
    // leaving mechanism runs at all, and a custom transition must not be
    // mistaken for `None` or a screen would vanish rather than animate out.
    let fade_to_nothing = RouteTransition::custom(|screen, _t, _surface| screen);
    assert!(!fade_to_nothing.is_instant());
    assert!(RouteTransition::None.is_instant());

    // And equality is identity, since two closures cannot be compared for
    // behaviour. A shared one is equal to its clones; an identical one written
    // twice is not.
    assert_eq!(fade_to_nothing, fade_to_nothing.clone());
    assert_ne!(
        fade_to_nothing,
        RouteTransition::custom(|screen, _t, _surface| screen)
    );
}
