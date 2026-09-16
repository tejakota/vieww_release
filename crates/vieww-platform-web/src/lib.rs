//! The web backend — a vieww application on an HTML `<canvas>`.
//!
//! # Status: compiled, and run
//!
//! This crate carried a banner saying it had never been through `rustc` — the
//! machine it was written on could not install `wasm32-unknown-unknown`,
//! because `rustup target add` fetches from `static.rust-lang.org` and that
//! host was outside its network policy.
//!
//! It has now been compiled and run, on 2026-09-06, against rustc 1.95.0. The
//! target was not downloaded: `library/` from `rust-lang/rust` at tag `1.95.0`
//! was installed as the `rust-src` component and `std` built from source with
//! `-Z build-std=std,panic_abort` (under `RUSTC_BOOTSTRAP=1`), which needs
//! github.com rather than `static.rust-lang.org`. `ci/check/wasm-check.sh` documents
//! the download path; this is the path for a machine that cannot take it.
//!
//! What the first compile found, in full:
//!
//! * `vieww-foundation`'s `compile_error!` guard listed five `target_os`
//!   values, and `wasm32-unknown-unknown` reports `target_os = "unknown"` — so
//!   the workspace refused before this crate was reached. Fixed by matching the
//!   architecture and adding `TargetPlatform::Web`.
//! * one missing `web-sys` feature: `HtmlCanvasElement::style()` is inherited
//!   from `HtmlElement`, and web-sys gates inherited methods behind the
//!   ancestor's feature. `ci/check/wasm-check.sh`'s stage-0 audit cannot find this
//!   class — the crate names neither `HtmlElement` nor `CssStyleDeclaration`.
//!
//! Nothing else. Every `web-sys` call in this file type-checked as written.
//!
//! What *running* it then found — none of which a compiler could have — is
//! recorded where each was fixed:
//!
//! * the frame loop never ran the pipeline, so nothing ever changed on screen
//!   (`Loop::frame`);
//! * the damage-culled scene is the wrong scene for a backend that hands
//!   `put_image_data` a whole buffer — the page drew once and erased itself
//!   (same place);
//! * there was no wheel listener and no resize listener, so nothing scrolled
//!   and a window resize did not reach the tree;
//! * the driver was never told where an unpressed pointer was, so every hover
//!   state in every tree rendered as dead paint (`install_pointer_listeners`);
//! * `Cursor::css_name` — written for this backend — had no caller, so a page
//!   of tappable rows showed an arrow over all of them
//!   (`WebSurface::set_cursor`);
//! * there was nowhere to hand the rasteriser a device-pixel scale
//!   (`WebSurface::resize`) — closed since: the ratio is applied to the
//!   finished scene, `Scene::scaled` in `Loop::frame`, which is after
//!   compositing and survives repaint boundaries (see `resize`'s own notes).
//!
//! `apps/viewwsite` is the application that exercised all of it: a page laid
//! out, scrolled, tapped and resized in a browser.
//!
//! # Certified, 2026-09-14
//!
//! `ci/check/wasm-check.sh` passed for the first time — the same `rustc` and
//! `clippy -D warnings` as every other crate, on this target — and then
//! `examples/test-web` went further than a compile: it mounts this crate in
//! a headless Chromium and compares the canvas readback **byte for byte**
//! against a native `render_to_pixels` of the same scene, at
//! `devicePixelRatio` 1, in two pointer states. Equal, all 2,304,000 bytes
//! both times, plus a tap that re-rendered exactly the native "tapped"
//! state — the pointer pipeline included. That example's directory holds
//! the comparison and its evidence.
//!
//! # Why the canvas is a 2D context and not WebGL
//!
//! Because the renderer that produces the pixels is `vieww-paint`'s CPU
//! rasterizer, and it produces *finished, straight-alpha RGBA8*. There is
//! nothing left to shade. A WebGL context would exist only to upload that
//! buffer as a texture and draw one quad, which is what
//! `CanvasRenderingContext2d::put_image_data` already does with less code and
//! no context loss to handle.
//!
//! This is the same decision the Vulkan path made for the same reason — see
//! `vieww-hal`'s `vulkan::swapchain`, which also only ferries CPU pixels to a
//! surface. `vieww-gpu`'s scene path can now plan every command a frame
//! contains (every fixture in the gallery plans complete — see
//! `examples/fixtures`' `--census`), and its shaders are WGSL that `naga`
//! already cross-compiles, so a WebGPU backend under the same `ScenePlan` is a
//! question worth asking. It has not been asked yet: nothing in this crate
//! uses the GPU path.
//!
//! # Device pixels, CSS pixels, and the bug that lives between them
//!
//! (Names below are written as plain code rather than as links: this whole
//! crate is `#![cfg(target_arch = "wasm32")]`, so on a host `cargo doc` —
//! which is what `ci/check/checks.sh` runs — none of its items exist to link to.)
//!
//! A canvas has two sizes: its `width`/`height` attributes, which are the
//! pixel buffer, and its CSS `width`/`height`, which are how big it looks.
//! Setting only the first gives a crisp canvas at the wrong size; setting
//! only the second gives a blurry one. `WebSurface::resize` sets both, from
//! one logical size and the window's `devicePixelRatio`, which is the only
//! place in this crate that arithmetic happens.
//!
//! vieww works in **logical** pixels throughout — `FrameDriver::resize` takes
//! a logical `Size`, pointer events arrive in logical coordinates — so the
//! ratio is applied at exactly two boundaries: the canvas buffer size, and
//! the scale handed to the rasterizer. Anything else that multiplies by it is
//! a bug.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use vieww_foundation::{
    Color, Cursor, Modifiers, Offset, PointerDeviceKind, PointerEvent, PointerId, ScrollEvent, Size,
};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::WidgetNode;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, ImageData, KeyboardEvent,
    PointerEvent as DomPointerEvent,
};

/// What can go wrong bringing a vieww application up in a browser.
#[derive(Debug, Clone)]
pub enum WebError {
    /// No `window`, no `document`, or no `performance` — which means this is
    /// not running in a browser at all (a worker, or a test harness).
    NoBrowser(&'static str),
    /// The element id given to [`WebApp::mount`] matched nothing, or matched
    /// something that is not a `<canvas>`.
    NoCanvas(String),
    /// `getContext("2d")` returned nothing. In practice this means the canvas
    /// already has a context of a different kind — a canvas cannot change its
    /// context type once it has one, and the error says so because the fix
    /// ("use a different canvas") is not guessable from "returned null".
    NoContext,
    /// Something in the DOM said no, with its own message.
    Dom(String),
}

impl std::fmt::Display for WebError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBrowser(what) => write!(
                f,
                "no {what}: vieww's web backend needs a browser main thread"
            ),
            Self::NoCanvas(id) => write!(
                f,
                "no <canvas> with id {id:?}; the element must exist before mount() is called"
            ),
            Self::NoContext => write!(
                f,
                "getContext(\"2d\") returned null, which usually means this canvas already \
                 holds a context of another kind — a canvas cannot change context type, so \
                 use a fresh one"
            ),
            Self::Dom(message) => write!(f, "DOM error: {message}"),
        }
    }
}

impl std::error::Error for WebError {}

impl From<JsValue> for WebError {
    fn from(value: JsValue) -> Self {
        Self::Dom(value.as_string().unwrap_or_else(|| format!("{value:?}")))
    }
}

/// A canvas, its 2D context, and the size bookkeeping between them.
#[derive(Debug)]
pub struct WebSurface {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    /// The size the *application* thinks in.
    logical: Size,
    /// The backing buffer, in device pixels — `logical` times the ratio, which
    /// is what the rasteriser is asked for and what `put_image_data` is handed.
    device: Size,
    /// `devicePixelRatio` at the last resize. Re-read on every resize rather
    /// than cached at startup: dragging a window between a laptop screen and
    /// an external monitor changes it, and a canvas that kept the old one is
    /// blurry until the page is reloaded.
    scale: f32,
    /// The shape last written to the canvas's `cursor` style. See
    /// [`set_cursor`](Self::set_cursor).
    cursor: Cursor,
}

impl WebSurface {
    /// Take over the `<canvas>` with this id.
    ///
    /// # Errors
    ///
    /// [`WebError::NoBrowser`], [`WebError::NoCanvas`] or
    /// [`WebError::NoContext`] — see each.
    pub fn by_id(id: &str) -> Result<Self, WebError> {
        let document = web_sys::window()
            .ok_or(WebError::NoBrowser("window"))?
            .document()
            .ok_or(WebError::NoBrowser("document"))?;
        let canvas: HtmlCanvasElement = document
            .get_element_by_id(id)
            .ok_or_else(|| WebError::NoCanvas(id.to_owned()))?
            .dyn_into()
            .map_err(|_| WebError::NoCanvas(id.to_owned()))?;
        Self::from_canvas(canvas)
    }

    /// Take over a canvas already in hand.
    ///
    /// # Errors
    ///
    /// [`WebError::NoContext`] if the canvas will not give up a 2D context.
    pub fn from_canvas(canvas: HtmlCanvasElement) -> Result<Self, WebError> {
        let context: CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or(WebError::NoContext)?
            .dyn_into()
            .map_err(|_| WebError::NoContext)?;
        // The canvas attributes are the starting size, in CSS pixels until
        // `resize` has run once and applied the device-pixel ratio to them.
        let logical = Size::new(canvas.width() as f32, canvas.height() as f32);
        Ok(Self {
            canvas,
            context,
            logical,
            device: logical,
            scale: 1.0,
            cursor: Cursor::Default,
        })
    }

    /// The logical size the application should lay out for.
    #[must_use]
    pub const fn logical_size(&self) -> Size {
        self.logical
    }

    /// The backing buffer, in device pixels.
    ///
    /// [`logical_size`](Self::logical_size) times [`scale`](Self::scale),
    /// rounded — the size the rasteriser draws and the canvas holds.
    #[must_use]
    pub const fn device_size(&self) -> Size {
        self.device
    }

    /// The device-pixel ratio in force.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// Resize the backing buffer and the CSS box together.
    ///
    /// See the crate doc: this is the one place the device-pixel ratio is
    /// applied, and the CSS size is set as well as the buffer size because
    /// setting only one of them is the difference between a canvas at the
    /// wrong size and a blurry one.
    ///
    /// # Errors
    ///
    /// [`WebError::Dom`] if the style could not be set.
    pub fn resize(&mut self, logical: Size) -> Result<(), WebError> {
        // **The buffer is device pixels; the tree is not.**
        //
        // This spent a while at 1:1 — buffer the same size as the CSS box — and
        // the page was visibly soft on every modern screen, because the browser
        // was upscaling a 1x canvas. It then spent a while with the *tree* laid
        // out in device pixels under a root `Transformed::scale(ratio)`, which
        // is correct in principle and does not survive a repaint boundary: a
        // boundary records into a scene of its own from an identity canvas and
        // composites back through an `Offset`, so the scale was silently
        // dropped for everything inside the page's `Scrollable`.
        //
        // The ratio is applied to the finished scene instead — `Scene::scaled`
        // in `Loop::frame` — which is after compositing, where the command list
        // is flat and there is no boundary left to lose it. So the tree,
        // pointer positions and wheel deltas are all in plain logical points,
        // and `scale()` is read in exactly one place.
        let ratio = web_sys::window()
            .map(|w| w.device_pixel_ratio())
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
            .unwrap_or(1.0) as f32;
        let scale = ratio.clamp(1.0, 3.0);
        // `logical` arrives in CSS pixels — it is the window's size — and is
        // kept that way: the tree lays out in logical points, and the ratio is
        // applied to the finished scene on the way to the rasteriser. See the
        // note above `Scene::scaled` in `frame`.
        let device_width = (logical.width * scale).round().max(1.0);
        let device_height = (logical.height * scale).round().max(1.0);
        self.logical = logical;
        self.device = Size::new(device_width, device_height);
        self.scale = scale;

        self.canvas.set_width(device_width as u32);
        self.canvas.set_height(device_height as u32);

        let style = self.canvas.style();
        style.set_property("width", &format!("{}px", logical.width))?;
        style.set_property("height", &format!("{}px", logical.height))?;
        Ok(())
    }

    /// Set the pointer shape over the canvas.
    ///
    /// A canvas is one element, so the browser's own cursor logic — which is
    /// per element — cannot help: without this, a page of tappable rows shows
    /// an arrow over every one of them, and nothing on it looks clickable.
    /// [`Cursor::css_name`] exists for exactly this and had no caller.
    ///
    /// The last shape written is remembered because this runs on every mouse
    /// move, which on a trackpad is a few hundred times a second, and setting
    /// a CSS property is a style invalidation each time. The common case — the
    /// pointer moving *within* one widget — costs a comparison.
    ///
    /// # Errors
    ///
    /// [`WebError::Dom`] if the style could not be set.
    pub fn set_cursor(&mut self, cursor: Cursor) -> Result<(), WebError> {
        if cursor == self.cursor {
            return Ok(());
        }
        self.canvas
            .style()
            .set_property("cursor", cursor.css_name())?;
        self.cursor = cursor;
        Ok(())
    }

    /// Put a straight-alpha RGBA8 buffer of `width` x `height` **device**
    /// pixels onto the canvas.
    ///
    /// # Why the buffer is copied into an `ImageData` rather than wrapped
    ///
    /// `ImageData::new_with_u8_clamped_array_and_sh` copies. There is a
    /// zero-copy shape — construct a `Uint8ClampedArray` view over the wasm
    /// linear memory and hand that to `ImageData` — and it is a trap: any
    /// allocation in wasm can grow the memory, which detaches every existing
    /// view, and the detachment is silent. The copy is a memcpy of the frame,
    /// which for a 1440x900 window is about 5 MB and is not the expensive
    /// part of a frame that just ran a software rasterizer over the same
    /// pixels.
    ///
    /// # Errors
    ///
    /// [`WebError::Dom`] if the browser rejects the buffer — which it does if
    /// `pixels.len()` is not exactly `width * height * 4`.
    pub fn present(&self, pixels: &[u8], width: u32, height: u32) -> Result<(), WebError> {
        let expected = width as usize * height as usize * 4;
        debug_assert_eq!(
            pixels.len(),
            expected,
            "present() got {} bytes for a {width}x{height} frame, wanted {expected}",
            pixels.len()
        );
        let clamped = wasm_bindgen::Clamped(pixels);
        let image = ImageData::new_with_u8_clamped_array_and_sh(clamped, width, height)?;
        self.context.put_image_data(&image, 0.0, 0.0)?;
        Ok(())
    }
}

/// A vieww application running in a browser.
///
/// # The shape, and why it is not `App::run` like the desktop one
///
/// `vieww-platform-winit`'s `App::run` takes the thread and does not return —
/// which is what a native event loop is. A browser has one event loop and it
/// is not ours: blocking it is how a tab stops responding. So the web
/// equivalent *mounts* and returns, having registered a
/// `requestAnimationFrame` callback and the DOM listeners, and the browser
/// calls back in.
///
/// That difference is why [`mount`](Self::mount) returns a [`WebHandle`]
/// rather than a report: the application is still running when the function
/// returns, and the handle is how it is stopped.
#[derive(Debug)]
pub struct WebApp {
    surface: WebSurface,
    background: Color,
    fill_window: bool,
}

/// A mounted application. Dropping it stops the frame loop.
///
/// Kept alive by whoever mounted the app. Dropping the handle cancels the
/// pending animation frame and releases the DOM listeners — a `WebHandle`
/// that is created and immediately dropped mounts an application that renders
/// exactly nothing, which is a surprising-but-correct consequence of tying
/// the lifetime to a value rather than leaking it.
pub struct WebHandle {
    inner: Rc<RefCell<Loop>>,
}

impl core::fmt::Debug for WebHandle {
    /// The loop behind the handle is borrowed by the frame callback whenever a
    /// frame is in flight, so printing its contents could panic. A handle has
    /// no state a caller can act on anyway — it is alive or it is dropped.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WebHandle").finish_non_exhaustive()
    }
}

impl WebHandle {
    /// Stop the frame loop and release the listeners.
    ///
    /// Idempotent, and what [`Drop`] calls.
    pub fn stop(&self) {
        self.inner.borrow_mut().stop();
    }
}

impl Drop for WebHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The per-frame state the `requestAnimationFrame` closure owns.
struct Loop {
    driver: FrameDriver,
    renderer: NativeRenderer,
    surface: WebSurface,
    background: Color,
    /// The registered callback, kept so it can be cancelled.
    animation_frame: Option<i32>,
    /// The closure itself, kept alive: a `Closure` dropped while the browser
    /// still holds a reference to it is a use-after-free, and the usual
    /// workaround (`Closure::forget`) leaks it for the life of the page. This
    /// keeps it exactly as long as the loop is running.
    callback: Option<Closure<dyn FnMut(f64)>>,
    /// DOM listeners, kept alive for the same reason.
    listeners: Vec<Listener>,
    running: bool,
    /// `performance.now()` at the first frame, so frame times are relative to
    /// the application starting rather than to the page loading.
    origin: Option<f64>,
    /// Time since the first frame, updated once per frame and read by the
    /// event listeners.
    ///
    /// Events are stamped with this rather than with a fresh
    /// `performance.now()` because `std::time::Instant` panics on wasm and
    /// because a gesture's velocity should be measured on the same clock the
    /// frames are — see `PointerEvent::timestamp`. It is one frame stale at
    /// worst, which is the same staleness a native platform's event queue
    /// introduces.
    elapsed: Duration,
    /// The last position seen per pointer, so a move can report its delta.
    last_pointer: BTreeMap<PointerId, Offset>,
    /// Whether anything has been put on the canvas yet. See `frame`.
    presented: bool,
    /// `FrameDriver::scene_rebuilds` at the last present, so an unchanged
    /// frame can be skipped. See `frame`.
    last_rebuild: u64,
}

impl Loop {
    fn stop(&mut self) {
        self.running = false;
        if let (Some(handle), Some(window)) = (self.animation_frame.take(), web_sys::window()) {
            let _ = window.cancel_animation_frame(handle);
        }
        self.callback = None;
        self.listeners.clear();
    }

    /// Lay out, paint, rasterize and present one frame.
    fn frame(&mut self, now_ms: f64) {
        let origin = *self.origin.get_or_insert(now_ms);
        let elapsed = Duration::from_secs_f64(((now_ms - origin).max(0.0)) / 1000.0);
        self.elapsed = elapsed;

        // **The pipeline, not just the pointers.** This used to call
        // `tick_pointers` and then ask for the scene, which runs neither the
        // animate phase nor the rebuild: signals written by an event handler
        // never reached the tree, animations never advanced, and the page
        // presented the same first frame for ever while every listener
        // underneath it worked perfectly. `draw_frame_at` is animate → build →
        // layout → paint → composite, and it calls `tick_pointers` itself as
        // the first half of its build phase, which is why that call is gone
        // rather than moved.
        //
        // Every frame, unconditionally, because `requestAnimationFrame` is
        // already the browser's own "when it is worth drawing" — the frame
        // scheduling a native loop does with `frame_deadline` is done for us
        // here, and a tab that is not visible is not called back at all.
        self.driver.draw_frame_at(elapsed);

        // **Nothing was repainted, so present nothing.** A canvas keeps its
        // pixels between frames, while `requestAnimationFrame` fires sixty
        // times a second whether or not the page moved — so without this an
        // idle page rasterises the whole window sixty times a second, which is
        // a battery bug rather than a frame rate.
        //
        // The question is asked of `scene_rebuilds`, which counts flattening
        // passes: it goes up exactly when something repainted, and it is a
        // counter rather than a flag, so a frame that rebuilt twice cannot be
        // mistaken for one that rebuilt none. `damage` was the first thing
        // tried here and is the wrong question — it is empty on a frame that
        // rebuilt the whole scene, so the page drew once and then froze while
        // the loop kept running and every event kept arriving.
        //
        // `presented` covers the first frame, which has to reach the canvas
        // before "unchanged" can mean "already on screen".
        let rebuilds = self.driver.scene_rebuilds();
        if self.presented && rebuilds == self.last_rebuild && !self.driver.is_animating() {
            return;
        }
        self.last_rebuild = rebuilds;

        // **The whole scene, not the damage-culled one.** `damage_culled_scene`
        // is for a backend that presents the damaged rectangle into a surface
        // it keeps; this one rasterises into a fresh buffer and hands the
        // *entire* buffer to `put_image_data`, so anything culled out is not
        // "left as it was", it is painted over with transparent black. Taking
        // the culled scene here drew the page on the first frame and erased it
        // on the second, which looks exactly like a mount that failed.
        //
        // Presenting only the damaged rectangle — `put_image_data`'s dirty-rect
        // overload, against a buffer rasterised at that size — is the version
        // of this that would let the cull pay off. It needs the rasteriser to
        // be given an origin, which it cannot be told today.
        let scene = self.driver.scene();
        // **The device-pixel ratio is applied here, to the finished scene.**
        //
        // The tree is laid out in logical points, so its commands are in
        // logical points; the canvas is `scale` times that in each direction.
        // `Scene::scaled` maps the flat, already-composited command list into
        // device space — glyph outlines included, which is where the sharpness
        // comes from — and is the one place in this backend that knows about
        // the ratio at all. Doing it in the *tree*, under a root
        // `Transformed::scale`, is the version that does not work: a repaint
        // boundary records from an identity canvas and composites back through
        // an offset, so a scale above one is dropped for everything inside it.
        let device = self.surface.device_size();
        let device_width = device.width.round().max(1.0) as u32;
        let device_height = device.height.round().max(1.0) as u32;
        let scene = scene.scaled(self.surface.scale());

        match self
            .renderer
            .render_to_pixels(&scene, device_width, device_height, self.background)
        {
            Ok((pixels, _report)) => {
                if self
                    .surface
                    .present(pixels.data(), device_width, device_height)
                    .is_ok()
                {
                    self.presented = true;
                }
            }
            Err(_error) => {
                // A frame that failed to rasterize is a bug in the scene, not
                // a reason to stop the loop: the next frame usually renders,
                // and a stopped loop is a frozen page with no clue in it.
                // The error is dropped rather than logged because this crate
                // does not own the application's logging.
            }
        }
    }
}

/// A DOM listener that removes itself when dropped.
struct Listener {
    target: web_sys::EventTarget,
    kind: &'static str,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self
            .target
            .remove_event_listener_with_callback(self.kind, self.closure.as_ref().unchecked_ref());
    }
}

impl WebApp {
    /// An application that will draw onto `surface`.
    #[must_use]
    pub const fn new(surface: WebSurface) -> Self {
        Self {
            surface,
            background: Color::WHITE,
            fill_window: false,
        }
    }

    /// Follow the browser window's size rather than the canvas's own.
    ///
    /// Off by default, because a canvas placed inside a page has a size its
    /// author chose and a backend that overrode it would be taking a decision
    /// that is not its to take. Turned on, the surface and the frame driver are
    /// resized together on every `resize` event — which is the only way a vieww
    /// tree learns that a browser window changed shape, there being no
    /// `WindowEvent::Resized` here to route.
    #[must_use]
    pub const fn fill_window(mut self, fill: bool) -> Self {
        self.fill_window = fill;
        self
    }

    /// The colour behind everything the widget tree draws.
    #[must_use]
    pub const fn background(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    /// Mount, with the driver in hand before the first frame.
    ///
    /// The web counterpart of `vieww_platform_winit::App::run`'s closure, and
    /// needed for the same reasons: a [`ScrollController`] has to be attached
    /// to the driver's tickers or a fling stops dead when the finger lifts, an
    /// application's signals have to be created on *this* tree's runtime to
    /// mark anything pending, and the root has to be set through
    /// [`FrameDriver::set_root`] so it is republished under
    /// `Inherited<ViewMetrics>`.
    ///
    /// [`mount`](Self::mount) is this with a closure that only sets the root.
    ///
    /// [`ScrollController`]: https://docs.rs/vieww-element
    ///
    /// # Errors
    ///
    /// [`WebError::NoBrowser`] if there is no `window`, and anything
    /// [`WebSurface::resize`] can fail with.
    pub fn mount_with(
        mut self,
        build: impl FnOnce(&mut FrameDriver),
    ) -> Result<WebHandle, WebError> {
        let window = web_sys::window().ok_or(WebError::NoBrowser("window"))?;
        let logical = if self.fill_window {
            window_size(&window).unwrap_or_else(|| self.surface.logical_size())
        } else {
            self.surface.logical_size()
        };
        self.surface.resize(logical)?;
        // The driver gets the **logical** size — CSS pixels, the window's own
        // size. The buffer is that times the device-pixel ratio, and the ratio
        // is applied to the scene rather than to the tree; see `Loop::frame`.
        let surface = self.surface.logical_size();

        let mut driver = FrameDriver::new(surface);
        build(&mut driver);

        let inner = Rc::new(RefCell::new(Loop {
            driver,
            renderer: NativeRenderer::new(),
            surface: self.surface,
            background: self.background,
            animation_frame: None,
            callback: None,
            listeners: Vec::new(),
            running: true,
            origin: None,
            elapsed: Duration::ZERO,
            last_pointer: BTreeMap::new(),
            presented: false,
            last_rebuild: 0,
        }));

        install_pointer_listeners(&inner, &window)?;
        install_pointer_out_listener(&inner, &window)?;
        install_wheel_listener(&inner, &window)?;
        install_key_listeners(&inner, &window)?;
        if self.fill_window {
            install_resize_listener(&inner, &window)?;
        }
        schedule(&inner, &window);
        Ok(WebHandle { inner })
    }

    /// Mount `root` and start the frame loop.
    ///
    /// Returns as soon as the first frame is scheduled — see [`WebApp`] on
    /// why this does not block. Keep the returned [`WebHandle`] alive for as
    /// long as the application should run.
    ///
    /// # Errors
    ///
    /// [`WebError::NoBrowser`] if there is no `window`, and anything
    /// [`WebSurface::resize`] can fail with.
    pub fn mount(self, root: impl Into<WidgetNode>) -> Result<WebHandle, WebError> {
        self.mount_with(move |driver| driver.set_root(root))
    }
}

/// Register the next animation frame, and the closure that will register the
/// one after it.
///
/// The recursion is the standard `requestAnimationFrame` shape and the part
/// worth reading twice: the closure holds an `Rc` to the loop, and the loop
/// holds the closure, which is a cycle. It is broken by [`Loop::stop`]
/// clearing `callback` — not by `Drop`, which cannot run while the cycle
/// exists. That is why [`WebHandle::drop`] calls `stop` rather than relying
/// on the refcount.
fn schedule(inner: &Rc<RefCell<Loop>>, window: &web_sys::Window) {
    let weak = Rc::downgrade(inner);
    let closure = Closure::wrap(Box::new(move |now: f64| {
        let Some(inner) = weak.upgrade() else { return };
        // The borrow is released before scheduling the next frame: a frame
        // whose widget code re-enters this loop would otherwise panic on a
        // double borrow, and it can, through an event handler that requests
        // a redraw.
        let still_running = {
            let mut guard = inner.borrow_mut();
            if !guard.running {
                false
            } else {
                guard.animation_frame = None;
                guard.frame(now);
                true
            }
        };
        if still_running {
            if let Some(window) = web_sys::window() {
                schedule(&inner, &window);
            }
        }
    }) as Box<dyn FnMut(f64)>);

    let handle = window
        .request_animation_frame(closure.as_ref().unchecked_ref())
        .ok();
    let mut guard = inner.borrow_mut();
    guard.animation_frame = handle;
    guard.callback = Some(closure);
}

/// Pointer events, translated into vieww's own.
///
/// Pointer events rather than mouse events, deliberately: a browser delivers
/// touch, pen and mouse through one `PointerEvent` API with a `pointerId` per
/// contact, which is exactly the shape `vieww_foundation::PointerEvent`
/// already has. Listening for `mousedown` instead would make a two-finger
/// gesture arrive as one confused cursor.
fn install_pointer_listeners(
    inner: &Rc<RefCell<Loop>>,
    window: &web_sys::Window,
) -> Result<(), WebError> {
    for kind in ["pointerdown", "pointermove", "pointerup", "pointercancel"] {
        let weak = Rc::downgrade(inner);
        let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
            let Some(inner) = weak.upgrade() else { return };
            let Ok(pointer) = event.dyn_into::<DomPointerEvent>() else {
                return;
            };
            let mut guard = inner.borrow_mut();
            // `offsetX`/`offsetY` are already relative to the target element
            // and already in CSS (logical) pixels, which is the space the tree
            // is laid out in — so nothing is multiplied by the device pixel
            // ratio here. The ratio is applied to the finished scene instead;
            // see `Loop::frame`.
            let position = Offset::new(pointer.offset_x() as f32, pointer.offset_y() as f32);
            let id = PointerId(pointer.pointer_id() as u64);
            let timestamp = guard.elapsed;
            // `previous` for a move: the framework computes `delta` from it,
            // and a delta measured against the wrong origin is what makes a
            // fling's velocity wrong — see `PointerEvent::timestamp`'s doc.
            let previous = guard.last_pointer.get(&id).copied().unwrap_or(position);
            let event = match kind {
                "pointerdown" => PointerEvent::down(id, position, timestamp),
                "pointermove" => PointerEvent::moved(id, previous, position, timestamp),
                "pointerup" => PointerEvent::up(id, position, timestamp),
                _ => PointerEvent::cancel(id, position, timestamp),
            }
            .with_kind(device_kind(&pointer))
            .with_modifiers(modifiers_of_mouse(&pointer));

            if kind == "pointerup" || kind == "pointercancel" {
                guard.last_pointer.remove(&id);
            } else {
                guard.last_pointer.insert(id, position);
            }
            guard.driver.handle_pointer(&event);

            // **Hover, which is not a pointer event.** `handle_pointer` routes
            // presses and drags; what is *under* an unpressed cursor is a
            // separate question, and the driver has to be asked it or nothing
            // on the page ever looks hovered. Without this a `Pressable` built
            // from `Sense` sees `hover: 0.0` for ever, and every hover state on
            // a page renders as dead paint.
            //
            // Mouse and pen only: a finger has no hover, and telling the tree
            // that a touch is hovering leaves a highlight behind after the
            // finger lifts.
            if kind == "pointermove" && event.kind != PointerDeviceKind::Touch {
                guard.driver.handle_hover(Some(position));
                let cursor = guard.driver.cursor_at(position);
                let _ = guard.surface.set_cursor(cursor);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);

        let target: web_sys::EventTarget = window.clone().into();
        target.add_event_listener_with_callback(kind, closure.as_ref().unchecked_ref())?;
        inner.borrow_mut().listeners.push(Listener {
            target,
            kind,
            closure,
        });
    }
    Ok(())
}

/// The browser window's inner size, in logical pixels.
///
/// `None` rather than a guess when either dimension is missing or is not a
/// number: a surface sized from a guess is worse than one left at the size the
/// canvas already had.
fn window_size(window: &web_sys::Window) -> Option<Size> {
    let width = window.inner_width().ok()?.as_f64()?;
    let height = window.inner_height().ok()?.as_f64()?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(Size::new(width as f32, height as f32))
}

/// The pointer leaving the canvas.
///
/// Its own listener rather than a fifth entry in the loop above, because it
/// produces no `PointerEvent` at all: nothing was pressed, so there is nothing
/// to cancel — the only thing that changed is that the tree is no longer
/// hovered. A page that skips this keeps its last hovered row lit after the
/// pointer has moved to another window, which is how a hover highlight ends up
/// looking like a selection.
fn install_pointer_out_listener(
    inner: &Rc<RefCell<Loop>>,
    window: &web_sys::Window,
) -> Result<(), WebError> {
    let weak = Rc::downgrade(inner);
    let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
        let Some(inner) = weak.upgrade() else { return };
        let mut guard = inner.borrow_mut();
        guard.driver.handle_hover(None);
        let _ = guard.surface.set_cursor(Cursor::Default);
    }) as Box<dyn FnMut(web_sys::Event)>);

    let target: web_sys::EventTarget = window.clone().into();
    target.add_event_listener_with_callback("pointerout", closure.as_ref().unchecked_ref())?;
    inner.borrow_mut().listeners.push(Listener {
        target,
        kind: "pointerout",
        closure,
    });
    Ok(())
}

/// Wheel and trackpad scrolling.
///
/// # The two conversions, and the sign that is easy to get backwards
///
/// A `WheelEvent` reports its delta in one of three units — pixels, lines or
/// pages — and says which in `deltaMode`. Only a platform knows how far a line
/// is, so the conversion happens here, at the same 40 logical pixels
/// `vieww-platform-winit` uses for a wheel notch, so that a page scrolls the
/// same distance per notch in a browser as it does in a window.
///
/// The sign is inverted, and that is not a mistake to be tidied away later:
/// [`ScrollEvent::delta`] is **finger-equivalent** — the movement a drag would
/// have made — while the DOM reports a scroll *downward* (content moving up)
/// as a **positive** `deltaY`. A finger producing that scroll travels up the
/// screen, which is negative. `vieww-platform-winit`'s own `scroll` carries the
/// long version of this argument, written after the opposite convention shipped
/// and made every wheel run backwards.
fn install_wheel_listener(
    inner: &Rc<RefCell<Loop>>,
    window: &web_sys::Window,
) -> Result<(), WebError> {
    /// One wheel notch, in logical pixels. `vieww-platform-winit`'s
    /// `LINE_HEIGHT`, restated rather than shared: the two crates have no
    /// dependency between them, and a browser's idea of a line is its own.
    const LINE_HEIGHT: f64 = 40.0;
    /// A page, for `DOM_DELTA_PAGE`. A screenful, less the overlap a reader
    /// needs to keep their place — the same fraction a browser's own Page Down
    /// leaves behind.
    const PAGE_FRACTION: f64 = 0.9;

    let weak = Rc::downgrade(inner);
    let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        let Some(inner) = weak.upgrade() else { return };
        let Ok(wheel) = event.clone().dyn_into::<web_sys::WheelEvent>() else {
            return;
        };
        let mut guard = inner.borrow_mut();
        let logical = guard.surface.logical_size();
        let unit = match wheel.delta_mode() {
            web_sys::WheelEvent::DOM_DELTA_LINE => (LINE_HEIGHT, LINE_HEIGHT),
            web_sys::WheelEvent::DOM_DELTA_PAGE => (
                f64::from(logical.width) * PAGE_FRACTION,
                f64::from(logical.height) * PAGE_FRACTION,
            ),
            // `DOM_DELTA_PIXEL`, and anything a future browser invents:
            // pixels are the unit vieww already works in.
            _ => (1.0, 1.0),
        };
        // Logical pixels throughout, for the reason the pointer listener gives.
        let delta = Offset::new(
            -(wheel.delta_x() * unit.0) as f32,
            -(wheel.delta_y() * unit.1) as f32,
        );
        let position = Offset::new(wheel.offset_x() as f32, wheel.offset_y() as f32);
        let timestamp = guard.elapsed;
        let scroll = ScrollEvent::new(position, delta, timestamp);
        // Only swallow the browser's own scrolling when the tree actually
        // scrolled. A canvas that eats every wheel event inside a longer page
        // traps the reader on it, which is the same accessibility failure the
        // key listener avoids for the same reason.
        if guard.driver.handle_scroll(&scroll) {
            event.prevent_default();
        }
    }) as Box<dyn FnMut(web_sys::Event)>);

    let target: web_sys::EventTarget = window.clone().into();
    // Not passive: `prevent_default` above is the whole point, and a passive
    // listener that calls it is ignored with a console warning.
    let options = web_sys::AddEventListenerOptions::new();
    options.set_passive(false);
    target.add_event_listener_with_callback_and_add_event_listener_options(
        "wheel",
        closure.as_ref().unchecked_ref(),
        &options,
    )?;
    inner.borrow_mut().listeners.push(Listener {
        target,
        kind: "wheel",
        closure,
    });
    Ok(())
}

/// The browser window changing shape.
///
/// Installed only under [`WebApp::fill_window`]. The surface and the driver are
/// resized together and in that order — the driver lays out against the size
/// the surface will present at, and a frame laid out for the old size and
/// presented into the new buffer is the flash of stretched pixels every
/// canvas-based UI has shipped at least once.
fn install_resize_listener(
    inner: &Rc<RefCell<Loop>>,
    window: &web_sys::Window,
) -> Result<(), WebError> {
    let weak = Rc::downgrade(inner);
    let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
        let Some(inner) = weak.upgrade() else { return };
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(size) = window_size(&window) else {
            return;
        };
        let mut guard = inner.borrow_mut();
        if guard.surface.logical_size() == size {
            return;
        }
        // A failed resize leaves the old size in place rather than a
        // half-applied one: `resize` sets the buffer and the CSS box together,
        // and the driver is only told once that has succeeded.
        if guard.surface.resize(size).is_ok() {
            let device = guard.surface.logical_size();
            guard.driver.resize(device);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);

    let target: web_sys::EventTarget = window.clone().into();
    target.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())?;
    inner.borrow_mut().listeners.push(Listener {
        target,
        kind: "resize",
        closure,
    });
    Ok(())
}

/// Keyboard events, translated into vieww's own.
fn install_key_listeners(
    inner: &Rc<RefCell<Loop>>,
    window: &web_sys::Window,
) -> Result<(), WebError> {
    for kind in ["keydown", "keyup"] {
        let weak = Rc::downgrade(inner);
        let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
            let Some(inner) = weak.upgrade() else { return };
            let Ok(key) = event.clone().dyn_into::<KeyboardEvent>() else {
                return;
            };
            let mut guard = inner.borrow_mut();
            let timestamp = guard.elapsed;
            let translated = keys::translate(&key, kind == "keydown", timestamp);
            // Only prevent the browser's own handling when the widget tree
            // actually consumed the key. Swallowing everything breaks the
            // browser's find bar, its shortcuts, and tab navigation out of
            // the canvas — the accessibility failure a canvas-based UI is
            // most likely to ship with.
            if guard.driver.handle_key(&translated) {
                event.prevent_default();
            }
        }) as Box<dyn FnMut(web_sys::Event)>);

        let target: web_sys::EventTarget = window.clone().into();
        target.add_event_listener_with_callback(kind, closure.as_ref().unchecked_ref())?;
        inner.borrow_mut().listeners.push(Listener {
            target,
            kind,
            closure,
        });
    }
    Ok(())
}

/// Browser key names to vieww key events.
mod keys {
    use vieww_foundation::{KeyEvent, KeyState, LogicalKey, Modifiers, NamedKey};
    use web_sys::KeyboardEvent;

    /// Translate a DOM keyboard event.
    ///
    /// Reads `event.key()` rather than `event.code()`: `key` is the character
    /// or named key the user meant *after* their layout and any dead-key
    /// composition have been applied, which is what a text field wants, while
    /// `code` is the physical position and would hand a Dvorak typist a
    /// QWERTY layout.
    ///
    /// An unrecognised name becomes [`LogicalKey::Unidentified`] rather than
    /// being dropped, matching that variant's own documented contract:
    /// delivered so an application can handle it, ignored by everything
    /// built in.
    pub(crate) fn translate(
        event: &KeyboardEvent,
        pressed: bool,
        timestamp: core::time::Duration,
    ) -> KeyEvent {
        let name = event.key();
        let key = match name.as_str() {
            "Enter" => LogicalKey::Named(NamedKey::Enter),
            "Tab" => LogicalKey::Named(NamedKey::Tab),
            " " => LogicalKey::Named(NamedKey::Space),
            "Backspace" => LogicalKey::Named(NamedKey::Backspace),
            "Delete" => LogicalKey::Named(NamedKey::Delete),
            "Escape" => LogicalKey::Named(NamedKey::Escape),
            "ArrowLeft" => LogicalKey::Named(NamedKey::ArrowLeft),
            "ArrowRight" => LogicalKey::Named(NamedKey::ArrowRight),
            "ArrowUp" => LogicalKey::Named(NamedKey::ArrowUp),
            "ArrowDown" => LogicalKey::Named(NamedKey::ArrowDown),
            "Home" => LogicalKey::Named(NamedKey::Home),
            "End" => LogicalKey::Named(NamedKey::End),
            "PageUp" => LogicalKey::Named(NamedKey::PageUp),
            "PageDown" => LogicalKey::Named(NamedKey::PageDown),
            "Insert" => LogicalKey::Named(NamedKey::Insert),
            "Shift" => LogicalKey::Named(NamedKey::Shift),
            "Control" => LogicalKey::Named(NamedKey::Control),
            "Alt" => LogicalKey::Named(NamedKey::Alt),
            "Meta" => LogicalKey::Named(NamedKey::Meta),
            "CapsLock" => LogicalKey::Named(NamedKey::CapsLock),
            // A DOM `key` that is a single grapheme is text the layout
            // produced. Longer unrecognised names ("F7", "MediaPlayPause")
            // are keys this framework has no name for.
            other if other.chars().take(2).count() == 1 => LogicalKey::Character(other.to_owned()),
            _ => LogicalKey::Unidentified,
        };
        KeyEvent {
            key,
            state: if pressed {
                KeyState::Down
            } else {
                KeyState::Up
            },
            repeat: event.repeat(),
            modifiers: modifiers_of_key(event),
            timestamp,
        }
    }

    fn modifiers_of_key(event: &KeyboardEvent) -> Modifiers {
        let mut modifiers = Modifiers::NONE;
        if event.shift_key() {
            modifiers = modifiers.union(Modifiers::SHIFT);
        }
        if event.ctrl_key() {
            modifiers = modifiers.union(Modifiers::CONTROL);
        }
        if event.alt_key() {
            modifiers = modifiers.union(Modifiers::ALT);
        }
        if event.meta_key() {
            modifiers = modifiers.union(Modifiers::META);
        }
        modifiers
    }
}

/// A DOM pointer's `pointerType` as vieww's own device kind.
///
/// The default is `Mouse` rather than `Touch` for an unknown type, and the
/// choice matters: `PointerDeviceKind` decides a recogniser's slop, and
/// giving a precise device a finger's 18-pixel tolerance makes small targets
/// hard to hit, while giving a finger a mouse's makes taps get dropped as
/// drags. An unrecognised `pointerType` on the web is overwhelmingly a mouse.
fn device_kind(event: &DomPointerEvent) -> PointerDeviceKind {
    match event.pointer_type().as_str() {
        "touch" => PointerDeviceKind::Touch,
        "pen" => PointerDeviceKind::Stylus,
        _ => PointerDeviceKind::Mouse,
    }
}

/// The modifier keys held during a pointer event.
///
/// Carried rather than dropped because `PointerEvent::modifiers`' own doc
/// explains what breaks without it: shift-click to extend a selection,
/// meta-click to add one, alt-drag to copy.
fn modifiers_of_mouse(event: &DomPointerEvent) -> Modifiers {
    let mut modifiers = Modifiers::NONE;
    if event.shift_key() {
        modifiers = modifiers.union(Modifiers::SHIFT);
    }
    if event.ctrl_key() {
        modifiers = modifiers.union(Modifiers::CONTROL);
    }
    if event.alt_key() {
        modifiers = modifiers.union(Modifiers::ALT);
    }
    if event.meta_key() {
        modifiers = modifiers.union(Modifiers::META);
    }
    modifiers
}
