//! The three widgets a DOM target needs and a rasteriser does not.
//!
//! All three are [`WidgetKind::Composed`] and build to something ordinary, so a
//! tree using them still renders on the canvas backend, on a phone and in a
//! test. The DOM walk recognises them on the way past and does something better
//! with them; every other backend sees straight through.

use std::rc::Rc;

use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

/// Raw CSS declarations attached to the subtree below.
///
/// # Why an escape hatch is not a defeat
///
/// A browser can do things this framework's rasteriser deliberately does not:
/// `backdrop-filter`, which needs the pixels *behind* a box; `background-clip:
/// text`, which needs the glyph coverage as a mask for a gradient; a repeating
/// `background-image` for a grid; `:hover`, which is a state no widget tree
/// holds; `@media (prefers-reduced-motion)`. Those are the difference between a
/// page that looks designed and one that looks drawn, and pretending otherwise
/// is how the canvas version ended up flat.
///
/// On any other backend this is a pass-through, so the escape hatch costs a
/// tree nothing when it is not on the web.
#[derive(Debug, Clone)]
pub struct Styled {
    pub(crate) css: String,
    pub(crate) class: Option<String>,
    /// Emit an element for *this*, rather than folding into the one below.
    pub(crate) block: bool,
    /// Emit an element with **no subtree at all**.
    pub(crate) leaf: bool,
    child: WidgetNode,
}

impl Styled {
    /// Declarations, `a:b;c:d` — semicolon optional on the last.
    #[must_use]
    pub fn css(css: impl Into<String>, child: impl Into<WidgetNode>) -> Self {
        Self {
            css: css.into(),
            class: None,
            block: false,
            leaf: false,
            child: child.into(),
        }
    }

    /// A box with declarations and nothing inside it.
    ///
    /// # Why this is not `Styled::css(.., Container::new())`
    ///
    /// It is the obvious spelling and it does not work. An empty `Container`
    /// resolves to a `Constrained` with tight zero constraints — correct for a
    /// rasteriser, where a box with no child and no size occupies nothing —
    /// which reaches CSS as `width:0;height:0`. Those beat a later
    /// `aspect-ratio` or `inset:0`, so the box is there, styled, and zero
    /// pixels tall. It cost an invisible hero backdrop and an invisible
    /// screenshot before it was worth a constructor.
    ///
    /// This emits the element and stops: no child, no size, nothing to
    /// override the declarations. It is what a decorative box is —
    /// a background, a gradient rule, a picture placed with `background-image`.
    #[must_use]
    pub fn empty(css: impl Into<String>) -> Self {
        Self {
            css: css.into(),
            class: None,
            block: false,
            leaf: true,
            child: SizedBox::shrink().into(),
        }
    }

    /// [`empty`](Self::empty), styled from the document's stylesheet.
    #[must_use]
    pub fn empty_class(class: impl Into<String>) -> Self {
        Self {
            css: String::new(),
            class: Some(class.into()),
            block: false,
            leaf: true,
            child: SizedBox::shrink().into(),
        }
    }

    /// Declarations on a box of **this widget's own**, rather than folded into
    /// the one below.
    ///
    /// The default folds, which is what makes `Styled` free: a rule set on a
    /// `Flex` lands on the same element the flex does, so the page pays no
    /// extra node for it. That is wrong exactly when the declarations describe
    /// a *container* for the subtree rather than the subtree itself — a
    /// centring wrapper around a box that has its own `display`, say, where
    /// folding produces one element trying to be both and the inner rule wins.
    #[must_use]
    pub fn block(css: impl Into<String>, child: impl Into<WidgetNode>) -> Self {
        Self {
            css: css.into(),
            class: None,
            block: true,
            leaf: false,
            child: child.into(),
        }
    }

    /// A class name, for rules that need a selector — `:hover`, `::selection`,
    /// a keyframe animation — declared in the document's own stylesheet.
    #[must_use]
    pub fn class(class: impl Into<String>, child: impl Into<WidgetNode>) -> Self {
        Self {
            css: String::new(),
            class: Some(class.into()),
            block: false,
            leaf: false,
            child: child.into(),
        }
    }

    /// Both at once.
    #[must_use]
    pub fn with(mut self, class: impl Into<String>) -> Self {
        self.class = Some(class.into());
        self
    }
}

impl Widget for Styled {
    fn debug_name(&self) -> &'static str {
        "Styled"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        self.child.clone()
    }
}
widget_node_from!(Styled);

/// The HTML element a box should be, when `div` is the wrong answer.
///
/// A product page that is one long `<div>` soup is unreadable to a screen
/// reader and invisible to a search engine, and both of those are things the
/// canvas version simply could not have. Naming the element is most of what
/// fixes it: a heading is an `<h1>`, a link is an `<a href>` that works with
/// middle-click and shows its target in the status bar, a button is a
/// `<button>` that answers the keyboard.
#[derive(Debug, Clone)]
pub struct Tag {
    pub(crate) tag: &'static str,
    pub(crate) href: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) id: Option<String>,
    child: WidgetNode,
}

impl Tag {
    #[must_use]
    pub fn new(tag: &'static str, child: impl Into<WidgetNode>) -> Self {
        Self {
            tag,
            href: None,
            label: None,
            id: None,
            child: child.into(),
        }
    }

    /// An `<a>` pointing somewhere. `#section` scrolls, and the browser does
    /// the smooth scrolling and the history entry for free.
    #[must_use]
    pub fn link(href: impl Into<String>, child: impl Into<WidgetNode>) -> Self {
        Self {
            tag: "a",
            href: Some(href.into()),
            label: None,
            id: None,
            child: child.into(),
        }
    }

    /// `aria-label`, for a control whose visible content is a picture.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The `id`, which is what a `#fragment` link lands on.
    #[must_use]
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl Widget for Tag {
    fn debug_name(&self) -> &'static str {
        "Tag"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        self.child.clone()
    }
}
widget_node_from!(Tag);

/// A `<canvas>` with a real vieww application painted into it.
///
/// **The island.** Everything around it is DOM, because a browser sets type and
/// composites translucency better than any rasteriser shipped in a page can.
/// This is the part that is not a claim: a widget tree, running in the reader's
/// browser, drawn by `vieww-paint` into these pixels.
///
/// `mount` is handed the canvas's element id once it is in the document. It is
/// called exactly once per mount — the DOM patcher never recreates a `Canvas`
/// node in place, because doing so would restart the application inside it.
#[derive(Clone)]
pub struct Canvas {
    pub(crate) id: String,
    pub(crate) width: f32,
    pub(crate) height: f32,
    #[allow(clippy::type_complexity)]
    pub(crate) mount: Rc<dyn Fn(&str)>,
}

impl Canvas {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        width: f32,
        height: f32,
        mount: impl Fn(&str) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            width,
            height,
            mount: Rc::new(mount),
        }
    }
}

impl std::fmt::Debug for Canvas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Canvas")
            .field("id", &self.id)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Widget for Canvas {
    fn debug_name(&self) -> &'static str {
        "Canvas"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }
    /// Off the web this is a hole of the right size, so a layout built around
    /// one still measures correctly in a test or a host render.
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        SizedBox::from_size(vieww_foundation::Size::new(self.width, self.height)).into()
    }
}
widget_node_from!(Canvas);
