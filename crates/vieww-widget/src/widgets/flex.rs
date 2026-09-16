use vieww_foundation::{Axis, Key, TextDirection};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// How leftover space is distributed along the main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainAxisAlignment {
    #[default]
    Start,
    End,
    Center,
    /// Free space split evenly *between* children, none at the ends.
    SpaceBetween,
    /// Free space split evenly around each child, so the end gaps are half the
    /// gaps between children.
    SpaceAround,
    /// Free space split evenly, including full-size gaps at both ends.
    SpaceEvenly,
}

/// How children are sized and positioned across the main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CrossAxisAlignment {
    Start,
    End,
    #[default]
    Center,
    /// Every child is forced to the full cross-axis extent.
    Stretch,
    /// Children are shifted so their first lines of text sit on one line.
    ///
    /// This is the difference between a row that looks designed and one that
    /// looks assembled. [`Center`](Self::Center) aligns the children's
    /// *boxes*, and a box is as tall as its font's line height — so a 24pt
    /// heading beside a 13pt label has its letters riding high, by a few
    /// pixels that nobody can name and nobody can unsee. `Baseline` aligns the
    /// letters instead.
    ///
    /// # What happens to children that have no text
    ///
    /// An icon, a divider, a coloured box: there is no baseline in there to
    /// align to, so those children fall back to [`Start`](Self::Start)
    /// individually and the rest of the row still lines up. That is deliberate
    /// and it is the only sane answer — treating "no baseline" as a baseline
    /// of zero would hang every icon from the row's top edge and read as a bug
    /// somewhere else entirely.
    ///
    /// # Only rows
    ///
    /// A column's cross axis is horizontal, and a baseline is a horizontal
    /// line; there is nothing to align across. A column asked for `Baseline`
    /// behaves as [`Start`](Self::Start) rather than panicking the way a stricter
    /// check would, because the situation arises from a shared alignment constant
    /// being reused in a column, not from a misunderstanding worth crashing
    /// over.
    ///
    /// # What it costs
    ///
    /// One extra query per child after layout, walking down to whatever holds
    /// the text. It is cheaper than an intrinsic pass — nothing is laid out
    /// twice — but it is not free, which is why it is a choice rather than the
    /// default.
    Baseline,
}

impl CrossAxisAlignment {
    /// Whether this alignment needs each child's baseline measured after
    /// layout.
    #[must_use]
    pub const fn needs_baselines(self) -> bool {
        matches!(self, Self::Baseline)
    }
}

/// Whether a flexible child is *forced* to its share of the space or merely
/// allowed up to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexFit {
    /// Fill the share exactly. The child is laid out under a tight main-axis
    /// constraint, so it cannot be smaller. This is what `Expanded` means, and
    /// what a divider or a background needs.
    #[default]
    Tight,
    /// Take up to the share, and less if that is all the child wants. A label
    /// that fits in half its allowance stays that wide, and the rest is free
    /// space for the alignment to distribute.
    Loose,
}

/// How much of the leftover main-axis space a child takes, and whether it must.
///
/// Read by [`RenderFlex`](https://docs.rs/vieww-render) off the child itself —
/// see `Flexible` for why that is stored on the child rather than in a
/// parent-owned slot the way the classic design does it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlexFactor {
    /// The child's share, relative to its flexible siblings. A child with 2
    /// against a sibling with 1 gets two thirds of what is left over.
    ///
    /// Zero means "flexible, but claim nothing", which is a legal way to say
    /// *loose with no share* and is why this is not a `NonZero`.
    pub flex: u16,
    pub fit: FlexFit,
}

impl FlexFactor {
    /// Fill this share exactly.
    #[must_use]
    pub const fn tight(flex: u16) -> Self {
        Self {
            flex,
            fit: FlexFit::Tight,
        }
    }

    /// Take up to this share.
    #[must_use]
    pub const fn loose(flex: u16) -> Self {
        Self {
            flex,
            fit: FlexFit::Loose,
        }
    }
}

/// Whether the flex shrink-wraps its children or fills the main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainAxisSize {
    /// Take only as much main-axis space as the children need.
    Min,
    /// Expand to the incoming maximum, if it is bounded.
    #[default]
    Max,
}

/// Lays out children in a single row or column.
///
/// One type covers both axes, unlike the `Row`/`Column` subclass split of
/// `Flex`. That means changing a `Flex` from horizontal to vertical reuses its
/// element and just updates the axis, instead of unmounting and remounting the
/// whole subtree — the axis is state, not identity. See `docs/DESIGN.md` §2.
#[derive(Debug, Clone)]
pub struct Flex {
    direction: Axis,
    main_axis_alignment: MainAxisAlignment,
    cross_axis_alignment: CrossAxisAlignment,
    main_axis_size: MainAxisSize,
    spacing: f32,
    /// `None` means "follow the ambient [`Directionality`](crate::Directionality)",
    /// which is why this is an `Option` rather than a defaulted
    /// [`TextDirection`]: `Ltr` is a real answer an application can pin a
    /// subtree to, and it has to be distinguishable from never having said.
    /// Same shape, and the same reason, as [`Text`](crate::Text)'s own
    /// direction override.
    text_direction: Option<TextDirection>,
    children: Vec<WidgetNode>,
    key: Option<Key>,
}

impl Flex {
    #[must_use]
    pub fn new(direction: Axis) -> Self {
        Self {
            direction,
            main_axis_alignment: MainAxisAlignment::default(),
            cross_axis_alignment: CrossAxisAlignment::default(),
            main_axis_size: MainAxisSize::default(),
            spacing: 0.0,
            text_direction: None,
            children: Vec::new(),
            key: None,
        }
    }

    /// A horizontal flex.
    #[must_use]
    pub fn row() -> Self {
        Self::new(Axis::Horizontal)
    }

    /// A vertical flex.
    #[must_use]
    pub fn column() -> Self {
        Self::new(Axis::Vertical)
    }

    /// Replace the children. Use the [`children!`](crate::children) macro to
    /// build the vector from widgets of differing types.
    #[must_use]
    pub fn children(mut self, children: impl IntoIterator<Item = WidgetNode>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// Append one child.
    #[must_use]
    pub fn push(mut self, child: impl Into<WidgetNode>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Set the main-axis distribution.
    #[must_use]
    pub const fn main_axis_alignment(mut self, alignment: MainAxisAlignment) -> Self {
        self.main_axis_alignment = alignment;
        self
    }

    /// Set the cross-axis alignment.
    #[must_use]
    pub const fn cross_axis_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.cross_axis_alignment = alignment;
        self
    }

    /// Shrink-wrap the main axis instead of filling it.
    #[must_use]
    pub const fn main_axis_size(mut self, size: MainAxisSize) -> Self {
        self.main_axis_size = size;
        self
    }

    /// A fixed gap between adjacent children, on the main axis.
    ///
    /// Spacing is *reserved*, not distributed: it comes off the top, before
    /// flexible children divide what is left and before
    /// [`MainAxisAlignment`] places the slack. So a row of three with
    /// `.spacing(8.0)` keeps its two 8px gaps whatever the alignment does, and
    /// an [`Expanded`](crate::Flexible) sibling shrinks to make room rather
    /// than the gaps collapsing.
    ///
    /// The alternative — inserting a [`SizedBox`](crate::SizedBox) between
    /// every pair — puts n-1 extra render objects in the tree to express one
    /// number, and gets the spacing wrong at the ends the moment a child is
    /// added or removed conditionally.
    ///
    /// Negative values are clamped to zero: a gap that pulls children into
    /// each other has no meaning in a layout that sums extents, and the
    /// alternative is a flex whose children overlap and whose reported size
    /// is smaller than its contents.
    #[must_use]
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing.max(0.0);
        self
    }

    /// Which way the interface reads, mirroring the horizontal axis.
    ///
    /// In a row this reverses the main axis, so `MainAxisAlignment::Start`
    /// packs children against the right edge; in a column it flips the cross
    /// axis, so `CrossAxisAlignment::Start` aligns them right. Only placement
    /// moves — children keep their order in the tree, so paint, hit testing and
    /// the order a screen reader walks are unchanged.
    ///
    /// **Setting this pins the flex.** Leave it alone and the flex follows the
    /// nearest [`Directionality`](crate::Directionality) above it, which is what
    /// an application normally wants; set it to hold one row the other way — a
    /// phone number, a code snippet, a licence plate — inside a screen that
    /// reads the opposite direction.
    #[must_use]
    pub fn text_direction(mut self, direction: TextDirection) -> Self {
        self.text_direction = Some(direction);
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The pinned direction, or `None` to follow the ambient one.
    ///
    /// Named differently from the [`text_direction`](Self::text_direction)
    /// builder for the same reason [`child_spacing`](Self::child_spacing) is:
    /// a builder and an accessor cannot share a name.
    #[must_use]
    pub const fn reading_direction(&self) -> Option<TextDirection> {
        self.text_direction
    }

    /// The axis children are laid out along.
    #[must_use]
    pub const fn direction(&self) -> Axis {
        self.direction
    }

    /// How leftover main-axis space is distributed.
    ///
    /// Named differently from the `main_axis_alignment` builder because a
    /// getter and a setter cannot share one name.
    #[must_use]
    pub const fn main_axis(&self) -> MainAxisAlignment {
        self.main_axis_alignment
    }

    /// How children are aligned across the main axis.
    #[must_use]
    pub const fn cross_axis(&self) -> CrossAxisAlignment {
        self.cross_axis_alignment
    }

    /// Whether the flex fills the main axis or shrink-wraps it.
    #[must_use]
    pub const fn axis_size(&self) -> MainAxisSize {
        self.main_axis_size
    }

    /// The fixed gap held between adjacent children.
    ///
    /// Named differently from the [`spacing`](Self::spacing) builder for the
    /// same reason [`main_axis`](Self::main_axis) is — a getter and a setter
    /// cannot share one name.
    #[must_use]
    pub const fn child_spacing(&self) -> f32 {
        self.spacing
    }
}

impl Widget for Flex {
    fn debug_name(&self) -> &'static str {
        match self.direction {
            Axis::Horizontal => "Row",
            Axis::Vertical => "Column",
        }
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderMultiChild(&self.children)
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = Vec::new();
        if self.main_axis_alignment != MainAxisAlignment::default() {
            props.push(("main", format!("{:?}", self.main_axis_alignment)));
        }
        if self.cross_axis_alignment != CrossAxisAlignment::default() {
            props.push(("cross", format!("{:?}", self.cross_axis_alignment)));
        }
        if self.main_axis_size != MainAxisSize::default() {
            props.push(("mainSize", format!("{:?}", self.main_axis_size)));
        }
        if self.spacing != 0.0 {
            props.push(("spacing", format!("{:?}", self.spacing)));
        }
        if let Some(direction) = self.text_direction {
            props.push(("textDirection", format!("{direction:?}")));
        }
        props
    }
}

widget_node_from!(Flex);
