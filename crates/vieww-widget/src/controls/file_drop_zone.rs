use std::fmt;
use std::rc::Rc;

use vieww_foundation::{EdgeInsets, FileDrag, Key};

use crate::{
    widget_node_from, BuildContext, Center, DecoratedBox, Padding, SemanticRole, Semantics,
    ThemeData, Widget, WidgetKind, WidgetNode,
};

/// An area that shows it would accept a file dragged in from outside.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::FileDropZone;
///
/// let zone = FileDropZone::new().child(Text::new("Drop a file here"));
/// ```
///
/// # It is an affordance, not a hit-tested target
///
/// This lights up while a file is over the **window**, because that is the only
/// thing the platform reports. `winit` 0.30 delivers a path and nothing else —
/// no position — and during an OS drag the ordinary cursor stream stops, so
/// there is no honest way to say which of two zones a file is over.
///
/// `vieww_foundation::file_drop`'s module docs carry the full reasoning,
/// including why the plausible workaround — hit test with the last cursor
/// position seen before the drag began — was refused: it is right whenever the
/// user did not move, which means right in a demo and wrong in use, and its
/// failure is a file landing somewhere the user cannot tell it landed.
///
/// So **the files themselves are delivered to the application**, through
/// `FrameDriver::take_dropped_files` and whatever the platform layer offers on
/// top of it. This widget's job is the half that has to be on screen: telling
/// the user, before they let go, that letting go will work. A drop zone that
/// only reacts *after* the drop is not a drop zone.
///
/// # Two zones are allowed and both light up
///
/// Which is the honest consequence rather than an oversight. Since the platform
/// says nothing about position, two zones are equally the target, and dimming
/// one of them would be inventing a fact. An application that needs to
/// distinguish should show one zone at a time.
#[derive(Clone)]
pub struct FileDropZone {
    child: Option<WidgetNode>,
    label: Option<String>,
    on_hover: Option<Rc<dyn Fn(bool)>>,
    key: Option<Key>,
}

impl FileDropZone {
    #[must_use]
    pub fn new() -> Self {
        Self {
            child: None,
            label: None,
            on_hover: None,
            key: None,
        }
    }

    /// What is inside the zone.
    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// What a screen reader calls this zone.
    ///
    /// Defaults to "Drop files here". A zone with no announcement is invisible
    /// to somebody who cannot see the dashed outline, and drag-and-drop is
    /// already the least accessible interaction there is — so a zone should
    /// never be the *only* way to attach a file, and this label should say what
    /// the alternative is when there is one.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Told when a file starts or stops hovering the window.
    #[must_use]
    pub fn on_hover(mut self, handler: impl Fn(bool) + 'static) -> Self {
        self.on_hover = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Default for FileDropZone {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for FileDropZone {
    fn debug_name(&self) -> &'static str {
        "FileDropZone"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        // Read from the tree, so the zone reacts with no application code in
        // between — the same arrangement `Sensitive` uses for `Capture`.
        let drag = *ctx.inherit_or(FileDrag::default());
        let hovering = drag.is_hovering();

        if let Some(handler) = &self.on_hover {
            handler(hovering);
        }

        // Thicker and accented while hovering, dim otherwise. The border is the
        // whole affordance, so the two states have to differ by more than a
        // shade — a zone whose "yes, drop here" is a 10% lighter grey is one
        // nobody notices while concentrating on not dropping a file.
        let (color, width) = if hovering {
            (theme.colors.primary, 3.0)
        } else {
            (theme.colors.outline, 1.0)
        };

        let label = self
            .label
            .clone()
            .unwrap_or_else(|| "Drop files here".to_owned());

        Semantics::container(label)
            .role(SemanticRole::Custom("region"))
            .child(
                DecoratedBox::outlined(color, width, theme.metrics.corner).child(
                    Padding::new(EdgeInsets::all(theme.metrics.gap * 2.0)).child(
                        match &self.child {
                            Some(child) => Center::new().child(child.clone()).into(),
                            None => WidgetNode::from(Center::new()),
                        },
                    ),
                ),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("label", self.label.clone().unwrap_or_default())]
    }
}

impl fmt::Debug for FileDropZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileDropZone")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

widget_node_from!(FileDropZone);
