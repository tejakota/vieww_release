//! **A DOM backend for vieww's web target.**
//!
//! The same widget tree, the same [`Signal`](vieww_element::Signal) runtime, the same element tree —
//! rendered by the browser instead of by `vieww-paint`.
//!
//! # Why this exists alongside `vieww-platform-web`
//!
//! `vieww-platform-web` puts vieww's own rasteriser on a `<canvas>`. That is
//! the right answer when the point *is* the rasteriser — an application, a
//! device preview, a demo that has to prove the pixels come from this
//! framework. It is the wrong answer for a page of text, and honestly so:
//!
//! - No `backdrop-filter`. A translucent bar over scrolling content needs the
//!   pixels behind it, which a scene graph does not have at composite time.
//! - No `background-clip: text`, so a gradient headline has to be faked.
//! - Text that cannot be selected, searched with the browser's find, or read by
//!   a screen reader, and that is invisible to a crawler.
//! - Glyph antialiasing this framework tunes, against one the platform tunes
//!   per display.
//! - Most of a megabyte of WebAssembly before the first pixel.
//!
//! So this backend translates instead of painting. It walks the **element
//! tree** — where every [`WidgetKind::Composed`](vieww_widget::WidgetKind::Composed) widget
//! has already been
//! resolved down to the primitive vocabulary — and emits elements and CSS, the
//! way `RenderObjectFactory` walks the same primitives and emits render
//! objects. It is a peer of the rasteriser, not a fork of it, and it needed no
//! change to any shared crate: [`Element::widget`](vieww_element::Element::widget),
//! [`Element::children`](vieww_element::Element::children) and
//! the primitives' own accessors were already public, because the factory needs
//! exactly the same things.
//!
//! # What a page gets back
//!
//! Real elements. Selectable text, the browser's find, a screen reader, a
//! crawler, the platform's own font rasteriser, `:hover`, `backdrop-filter`,
//! smooth `#fragment` scrolling and the browser's own scrollbar — none of which
//! the canvas version can have, and several of which it had hand-written and
//! worse.
//!
//! # And the island
//!
//! [`Canvas`] is the seam. It is a widget like any other, it lays out like any
//! other, and where it lands the page mounts a genuine `vieww-platform-web`
//! application into a real `<canvas>`. So a page can be DOM everywhere it
//! should be and still show a widget tree being painted by this framework, in
//! the reader's browser, a few hundred points below the headline that claims it.
//!
//! [`Signal`]: vieww_element::Signal
//! [`WidgetKind::Composed`]: vieww_widget::WidgetKind::Composed
//! [`Element::widget`]: vieww_element::Element::widget
//! [`Element::children`]: vieww_element::Element::children

pub mod images;
pub mod style;
pub mod tree;
pub mod widgets;

pub use style::Style;
pub use tree::VNode;
pub use widgets::{Canvas, Styled, Tag};

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
pub use app::{DomApp, DomError, DomHandle};
