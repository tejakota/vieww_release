//! A recorded canvas.
//!
//! A [`Scene`] is what a paint pass produces: an ordered list of drawing
//! commands with the transform and clip already resolved into absolute
//! coordinates. Backends consume it; tests assert against it.
//!
//! Resolving state at record time rather than replay time is the important
//! choice. It means a backend never has to maintain its own state stack, a
//! command can be inspected in isolation, and — crucially for damage tracking —
//! every command already knows its own screen-space bounds.
//!
//! # The one exception, and why it is not a regression
//!
//! [`Command::PushLayer`] and [`Command::PopLayer`] are a *pair*, so a scene is
//! no longer a flat list in the strictest sense. Everything the flat list bought
//! is kept regardless: each command inside a layer still carries its own
//! resolved transform and clip and still reports its own absolute bounds, and
//! the `PushLayer` itself is patched on pop to carry the union of what it
//! encloses. So damage still measures a layer without replaying it, and a
//! backend still needs no state stack of its own — it needs a *layer* stack,
//! which is a different thing and which every backend already has.

use std::fmt;
use std::ops::Range;

use vieww_foundation::{BlendMode, Color, GlyphRun, ImageFilter, Offset, Rect, Shadow, Transform};

use crate::{Canvas, Damage, Image, Paint, Path, Stroke};

/// Counted work from a damage-culling pass.
///
/// A backend or test uses this to assert that damage culling actually skipped
/// commands, following the framework rule: every optimization needs a
/// regression test where the work is countable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DamageCullStats {
    /// Commands copied into the output scene — these were translated.
    pub copied_commands: usize,
    /// Commands skipped because their bounds could not touch the damaged
    /// region.
    pub skipped_commands: usize,
}

impl DamageCullStats {
    /// Total commands that were candidates (copied + skipped).
    #[must_use]
    pub const fn total(&self) -> usize {
        self.copied_commands + self.skipped_commands
    }
}

#[derive(Default)]
pub(crate) struct DamageAppendStats {
    pub copied_commands: usize,
    pub skipped_commands: usize,
}

impl From<DamageAppendStats> for DamageCullStats {
    fn from(s: DamageAppendStats) -> Self {
        Self {
            copied_commands: s.copied_commands,
            skipped_commands: s.skipped_commands,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingLayer<'a> {
    command: &'a Command,
    active: bool,
    emitted: bool,
}

/// The clip in force when a command was recorded, in absolute coordinates.
///
/// # Why this is not just a `Rect`
///
/// It was, and rounded corners are the reason it is not. A clip is stored
/// resolved, so a rectangle was enough for as long as every clip was a
/// rectangle — but an avatar is a circle and a card is a rounded rectangle, and
/// both were previously impossible to express at all.
///
/// So a clip is a bounding rectangle *plus* a list of shapes it must also fall
/// inside. The rectangle is exact and always present, which is what
/// [`Damage`](crate::Damage) reads — over-reporting damage costs fill rate,
/// while the shapes only ever remove pixels from it. The shapes are what a
/// backend pushes as real clip layers.
///
/// The list is almost always empty, and an empty `Vec` allocates nothing, so a
/// rectangular clip costs exactly what it did before.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    bounds: Option<Rect>,
    /// Absolute-space shapes, outermost first. Empty for a plain rectangle.
    shapes: Vec<Path>,
}

/// The clip that clips nothing.
///
/// A `static` rather than a `const` because [`Command::clip`] hands out a
/// reference to it, and a type holding a `Vec` cannot be promoted to `'static`
/// from a constant.
static NO_CLIP: Clip = Clip::NONE;

impl Clip {
    /// Clips nothing.
    pub const NONE: Self = Self {
        bounds: None,
        shapes: Vec::new(),
    };

    /// The bounding box of everything this clip lets through, if it clips at
    /// all.
    #[must_use]
    pub const fn bounds(&self) -> Option<Rect> {
        self.bounds
    }

    /// The shapes that further cut into [`bounds`](Self::bounds), outermost
    /// first.
    #[must_use]
    pub fn shapes(&self) -> &[Path] {
        &self.shapes
    }

    /// This clip in a space scaled by `factor` about the origin.
    ///
    /// A clip's geometry is stored **absolute** — already through whatever
    /// transform was in force when it was recorded — so it does not go through
    /// a command's `transform` and has to be scaled here instead. See
    /// [`Scene::scaled`].
    #[must_use]
    pub fn scaled(&self, factor: f32) -> Self {
        let matrix = Transform::scale(factor, factor);
        Self {
            bounds: self.bounds.map(|rect| {
                Rect::new(
                    rect.left * factor,
                    rect.top * factor,
                    rect.right * factor,
                    rect.bottom * factor,
                )
            }),
            shapes: self.shapes.iter().map(|p| p.transformed(matrix)).collect(),
        }
    }

    /// `true` if this clip lets everything through.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        self.bounds.is_none()
    }

    /// `true` if this clip lets *nothing* through.
    ///
    /// Only ever true for the bounding box: a shape that happens to enclose no
    /// area is not detected here, because doing so means rasterising it.
    #[must_use]
    pub fn is_everything_clipped(&self) -> bool {
        self.bounds.is_some_and(Rect::is_empty)
    }

    /// Intersect with an absolute-space rectangle.
    pub fn add_rect(&mut self, rect: Rect) {
        self.bounds = Some(match self.bounds {
            Some(existing) => existing.intersect(rect),
            None => rect,
        });
    }

    /// Intersect with an absolute-space shape.
    ///
    /// The shape's bounding box tightens [`bounds`](Self::bounds) as well, so
    /// damage gets the benefit of the clip even though it never looks at the
    /// shape itself.
    pub fn add_path(&mut self, path: Path) {
        self.add_rect(path.bounds());
        self.shapes.push(path);
    }

    /// `rect` with this clip applied.
    #[must_use]
    pub fn clamp(&self, rect: Rect) -> Rect {
        match self.bounds {
            Some(bounds) => rect.intersect(bounds),
            None => rect,
        }
    }

    /// This clip as seen from a space that `by` maps this one into.
    ///
    /// The bounding box is carried through as a box, so under a rotation or a
    /// skew it grows — letting a little more through than it should. The shapes
    /// are transformed exactly, so a rotated *rounded* clip stays exact where
    /// it matters; the box is only ever used by damage, which is allowed to be
    /// generous.
    #[must_use]
    pub fn transformed(&self, by: Transform) -> Self {
        Self {
            bounds: self.bounds.map(|bounds| by.apply_rect(bounds)),
            shapes: self
                .shapes
                .iter()
                .map(|path| path.transformed(by))
                .collect(),
        }
    }
}

/// One resolved drawing operation.
///
/// # Closed on purpose, and it is the deepest closure in the framework
///
/// No third party can add a drawing primitive, because every backend matches
/// this exhaustively. That is not an oversight, and it is the one place this
/// framework deliberately answers *"what type is this?"* rather than *"what can
/// you do?"*.
///
/// **A closed set is what makes "every backend renders every scene" a checkable
/// claim.** Open it and a backend can silently fail to render somebody's
/// primitive — and the failure surfaces as a missing shape on one platform,
/// found by whoever happens to own that device. Adding a variant here, by
/// contrast, breaks every backend at compile time until each has an answer.
///
/// The extension points that *are* open sit above this: [`Canvas`](crate::Canvas)
/// for a whole backend, `RenderObject` for anything that draws, and
/// [`Path`](vieww_foundation::Path) for arbitrary geometry — which between them
/// cover what a new primitive would usually be wanted for.
///
/// If it is ever opened, the shape that preserves the guarantee is a
/// `Command::Custom` that backends may **skip**, with [`Damage`](crate::Damage)
/// treating it as opaque bounds. The worst case is then a shape missing
/// everywhere rather than on one device, which is at least consistent and
/// visible to whoever wrote it.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    FillRect {
        rect: Rect,
        paint: Paint,
        transform: Transform,
        clip: Clip,
    },
    FillPath {
        path: Path,
        paint: Paint,
        transform: Transform,
        clip: Clip,
    },
    StrokePath {
        path: Path,
        stroke: Stroke,
        paint: Paint,
        transform: Transform,
        clip: Clip,
    },
    /// A blurred silhouette behind a rounded rectangle.
    ///
    /// The *caster* is recorded rather than the shadow's own geometry, because
    /// a backend with a closed-form blurred-box primitive wants exactly this and
    /// deriving it back out of a pre-offset rectangle loses which part was the
    /// spread.
    DrawShadow {
        rect: Rect,
        radius: f32,
        shadow: Shadow,
        transform: Transform,
        clip: Clip,
    },
    DrawGlyphs {
        run: GlyphRun,
        transform: Transform,
        clip: Clip,
    },
    DrawImage {
        rect: Rect,
        image: Image,
        transform: Transform,
        clip: Clip,
    },
    /// Begin a group composited as one, closed by [`Command::PopLayer`].
    ///
    /// `bounds` is **absolute**, unlike every other command's geometry, and is
    /// patched on pop to the union of what the layer actually encloses. Storing
    /// it resolved is what lets damage measure a layer without walking into it,
    /// and matches how `clip` is stored for the same reason.
    PushLayer {
        bounds: Rect,
        alpha: f32,
        blend: BlendMode,
        clip: Clip,
        /// What the group's rasterised pixels go through before compositing.
        ///
        /// [`ImageFilter::NONE`] for every layer that is only a group opacity,
        /// which is almost all of them — a backend checks
        /// [`ImageFilter::is_noop`] and takes the path it always took, so
        /// adding filters costs an existing tree nothing.
        ///
        /// On the same command as `alpha` and `blend` rather than a command of
        /// its own, because all three are properties of *the group*, and a
        /// second marker would need a second pop to stay balanced. See
        /// `vieww_foundation::filter`.
        filter: ImageFilter,
    },
    /// Composite the group opened by the matching [`Command::PushLayer`].
    PopLayer,
}

impl Command {
    /// The transform in force when this command was recorded.
    #[must_use]
    pub fn transform(&self) -> Transform {
        match self {
            Self::FillRect { transform, .. }
            | Self::FillPath { transform, .. }
            | Self::StrokePath { transform, .. }
            | Self::DrawShadow { transform, .. }
            | Self::DrawGlyphs { transform, .. }
            | Self::DrawImage { transform, .. } => *transform,
            // A layer's bounds are already absolute, so there is nothing left
            // for a transform to do.
            Self::PushLayer { .. } | Self::PopLayer => Transform::IDENTITY,
        }
    }

    /// The clip in force when this command was recorded.
    #[must_use]
    pub fn clip(&self) -> &Clip {
        match self {
            Self::FillRect { clip, .. }
            | Self::FillPath { clip, .. }
            | Self::StrokePath { clip, .. }
            | Self::DrawShadow { clip, .. }
            | Self::DrawGlyphs { clip, .. }
            | Self::DrawImage { clip, .. }
            | Self::PushLayer { clip, .. } => clip,
            Self::PopLayer => &NO_CLIP,
        }
    }

    /// The clip, to tighten it further.
    ///
    /// `None` for `PopLayer`, which carries no clip because it draws nothing —
    /// the caller has to treat that as "cannot be clipped" rather than as "not
    /// clipped", or a marker gets dropped and its pair does not.
    fn clip_mut(&mut self) -> Option<&mut Clip> {
        match self {
            Self::FillRect { clip, .. }
            | Self::FillPath { clip, .. }
            | Self::StrokePath { clip, .. }
            | Self::DrawShadow { clip, .. }
            | Self::DrawGlyphs { clip, .. }
            | Self::DrawImage { clip, .. }
            | Self::PushLayer { clip, .. } => Some(clip),
            Self::PopLayer => None,
        }
    }

    /// The clip's bounding box, if this command was clipped at all.
    ///
    /// What damage tracking reads. The shapes never widen it, so this is a
    /// sound over-approximation of what the command can touch.
    #[must_use]
    pub fn clip_bounds(&self) -> Option<Rect> {
        self.clip().bounds()
    }

    /// `true` for the two commands that draw nothing themselves.
    #[must_use]
    pub const fn is_layer_marker(&self) -> bool {
        matches!(self, Self::PushLayer { .. } | Self::PopLayer)
    }

    /// This command as seen from a space that `by` maps this one into.
    ///
    /// Compositing uses this to lift a layer's commands into its parent's
    /// coordinates without replaying the layer's paint pass.
    #[must_use]
    pub fn transformed(&self, by: Transform) -> Self {
        let mut command = self.clone();
        match &mut command {
            Self::FillRect {
                transform, clip, ..
            }
            | Self::FillPath {
                transform, clip, ..
            }
            | Self::StrokePath {
                transform, clip, ..
            }
            | Self::DrawShadow {
                transform, clip, ..
            }
            | Self::DrawGlyphs {
                transform, clip, ..
            }
            | Self::DrawImage {
                transform, clip, ..
            } => {
                *transform = transform.then(by);
                *clip = clip.transformed(by);
            }
            Self::PushLayer { bounds, clip, .. } => {
                *bounds = by.apply_rect(*bounds);
                *clip = clip.transformed(by);
            }
            Self::PopLayer => {}
        }
        command
    }

    /// The screen-space area this command can affect.
    ///
    /// Every variant is measured rather than estimated, and every approximation
    /// rounds *outward* — a stroke by half its width, a shadow by three
    /// standard deviations of its blur. Under-reporting leaves stale pixels on
    /// the screen; over-reporting merely repaints more than necessary.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let local = match self {
            Self::FillRect { rect, .. } | Self::DrawImage { rect, .. } => *rect,
            Self::FillPath { path, .. } => path.bounds(),
            Self::StrokePath { path, stroke, .. } => path.bounds().inflate(stroke.reach()),
            Self::DrawShadow { rect, shadow, .. } => shadow.bounds(*rect),
            Self::DrawGlyphs { run, .. } => run.bounds(),
            // Already absolute, and already the union of its contents.
            Self::PushLayer { bounds, .. } => return self.clip().clamp(*bounds),
            // A pop draws nothing of its own; the layer it closes reported for
            // both of them.
            Self::PopLayer => return Rect::ZERO,
        };

        self.clip().clamp(self.transform().apply_rect(local))
    }
}

#[derive(Debug, Clone)]
struct State {
    transform: Transform,
    clip: Clip,
    /// Multiplied into every colour recorded under it, 0.0 to 1.0.
    ///
    /// Resolved at record time exactly as the transform and the clip are, which
    /// is what keeps a command independently inspectable and self-bounding —
    /// the property `Damage` is built on. See [`Canvas::push_alpha`] for what
    /// that costs and [`Canvas::push_layer`] for the version that does not.
    alpha: f32,
}

impl State {
    const fn initial() -> Self {
        Self {
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
            alpha: 1.0,
        }
    }

    /// `paint` as it should be recorded under this state.
    ///
    /// Mutated rather than rebuilt so that a `Paint` gaining a third field keeps
    /// it, instead of this silently resetting it to the default.
    fn apply(&self, mut paint: Paint) -> Paint {
        paint.color = self.faded(paint.color);
        // A gradient fades stop by stop. Fading only the representative colour
        // would leave the ramp itself at full strength, so a faded gradient
        // would not fade at all — and the bug would look like "opacity does
        // nothing on gradients" rather than like a missing line here.
        paint.gradient = paint.gradient.map(|gradient| gradient.faded(self.alpha));
        paint
    }

    fn faded(&self, color: Color) -> Color {
        vieww_foundation::fade(color, self.alpha)
    }
}

/// An ordered recording of drawing commands.
///
/// Implements [`Canvas`], so anything that can paint can paint into one.
#[derive(Debug, Clone)]
pub struct Scene {
    commands: Vec<Command>,
    state: State,
    stack: Vec<State>,
    /// Indices of the `PushLayer` commands still waiting for their pop.
    layers: Vec<usize>,
}

impl Scene {
    /// An empty scene with an identity transform and no clip.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
            state: State::initial(),
            stack: Vec::new(),
            layers: Vec::new(),
        }
    }

    /// Every command, in paint order — later entries draw on top.
    #[must_use]
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// The command list, to splice into.
    ///
    /// `pub(crate)` and no further: a command carries its own resolved
    /// transform and clip, so the list can be cut and rejoined anywhere — but
    /// only by something that keeps the `PushLayer`/`PopLayer` pairs balanced.
    /// [`SceneFlattener`](crate::SceneFlattener) is that something, and it is
    /// the only caller.
    pub(crate) fn commands_mut(&mut self) -> &mut Vec<Command> {
        &mut self.commands
    }

    /// The recorded commands, giving up the scene.
    ///
    /// For a scratch recording whose commands are about to be spliced into
    /// another list: the state stack is finished with by then, and moving the
    /// `Vec` out is what keeps the splice free of a second copy.
    pub(crate) fn into_commands(self) -> Vec<Command> {
        self.commands
    }

    /// This scene, recorded as if every command had been painted through a
    /// scale of `factor` about the origin.
    ///
    /// # Why a backend needs this
    ///
    /// A window on a 2x screen has to lay out in logical points and rasterise
    /// in device pixels. Wrapping the application's root in
    /// [`Transformed::scale`](https://docs.rs/vieww) is the version of that
    /// which needs no backend support, and it is **wrong below a repaint
    /// boundary**: a boundary records into a scene of its own that starts from
    /// an identity canvas, and composites back into its parent through an
    /// `Offset`. A translation survives that; a scale does not. So a page whose
    /// content sits inside a `Scrollable` — every page — is laid out at the
    /// logical size and painted at 1:1, which on screen is a window's worth of
    /// content drawn at half size in the corner of the buffer.
    ///
    /// Doing it here instead applies the ratio *after* the layer tree has been
    /// composited, where there is one flat command list and no boundaries left
    /// to lose it. Geometry that a command's `transform` maps — a rect, a path,
    /// a glyph run's origin — goes through the matrix, so glyph outlines are
    /// scan-converted at device resolution rather than magnified; geometry that
    /// is stored absolute — a clip, and a layer's bounds — is scaled directly.
    #[must_use]
    pub fn scaled(&self, factor: f32) -> Self {
        if (factor - 1.0).abs() < f32::EPSILON {
            return self.clone();
        }
        let matrix = Transform::scale(factor, factor);
        let scale_rect = |rect: Rect| {
            Rect::new(
                rect.left * factor,
                rect.top * factor,
                rect.right * factor,
                rect.bottom * factor,
            )
        };
        let commands = self
            .commands
            .iter()
            .map(|command| match command.clone() {
                Command::FillRect {
                    rect,
                    paint,
                    transform,
                    clip,
                } => Command::FillRect {
                    rect,
                    paint,
                    transform: transform.then(matrix),
                    clip: clip.scaled(factor),
                },
                Command::FillPath {
                    path,
                    paint,
                    transform,
                    clip,
                } => Command::FillPath {
                    path,
                    paint,
                    transform: transform.then(matrix),
                    clip: clip.scaled(factor),
                },
                Command::StrokePath {
                    path,
                    stroke,
                    paint,
                    transform,
                    clip,
                } => Command::StrokePath {
                    path,
                    stroke,
                    paint,
                    transform: transform.then(matrix),
                    clip: clip.scaled(factor),
                },
                Command::DrawShadow {
                    rect,
                    radius,
                    shadow,
                    transform,
                    clip,
                } => Command::DrawShadow {
                    rect,
                    radius,
                    shadow,
                    transform: transform.then(matrix),
                    clip: clip.scaled(factor),
                },
                Command::DrawGlyphs {
                    run,
                    transform,
                    clip,
                } => Command::DrawGlyphs {
                    run,
                    transform: transform.then(matrix),
                    clip: clip.scaled(factor),
                },
                Command::DrawImage {
                    rect,
                    image,
                    transform,
                    clip,
                } => Command::DrawImage {
                    rect,
                    image,
                    transform: transform.then(matrix),
                    clip: clip.scaled(factor),
                },
                Command::PushLayer {
                    bounds,
                    alpha,
                    blend,
                    clip,
                    filter,
                } => Command::PushLayer {
                    bounds: scale_rect(bounds),
                    alpha,
                    blend,
                    clip: clip.scaled(factor),
                    filter,
                },
                Command::PopLayer => Command::PopLayer,
            })
            .collect();
        Self {
            commands,
            state: State::initial(),
            stack: Vec::new(),
            layers: Vec::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Discard everything recorded, and reset the state stack.
    pub fn clear(&mut self) {
        self.commands.clear();
        self.stack.clear();
        self.layers.clear();
        self.state = State::initial();
    }

    /// The union of every command's bounds — the area this scene touches.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.commands
            .iter()
            .map(Command::bounds)
            .fold(Rect::ZERO, Rect::union)
    }

    /// Every filled rectangle, as `(screen bounds, paint)`.
    ///
    /// A convenience for tests and for asserting paint order.
    #[must_use]
    pub fn fills(&self) -> Vec<(Rect, Paint)> {
        self.commands
            .iter()
            .filter_map(|command| match command {
                Command::FillRect { paint, .. } => Some((command.bounds(), *paint)),
                _ => None,
            })
            .collect()
    }

    /// Every glyph run, as `(screen baseline origin, run)`.
    ///
    /// A run carries no source text — by the time it exists, characters have
    /// stopped being the unit of anything. Assert on *what text is displayed* at
    /// the widget layer, where the string still exists; assert here on where the
    /// ink lands.
    #[must_use]
    pub fn glyph_runs(&self) -> Vec<(Offset, &GlyphRun)> {
        self.commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawGlyphs { run, transform, .. } => {
                    Some((transform.apply(run.origin), run))
                }
                _ => None,
            })
            .collect()
    }

    /// Append every command of `other`, mapped through `transform`.
    ///
    /// This is how compositing flattens a layer into its parent: because each
    /// command already carries its own resolved transform, the whole recording
    /// can be lifted into another coordinate space by composing, with no replay
    /// of the paint pass that produced it.
    ///
    /// The current save/restore state is untouched — appended commands keep the
    /// state they were recorded under, not this scene's.
    pub fn append(&mut self, other: &Self, transform: Transform) {
        self.commands.reserve(other.commands.len());
        for command in &other.commands {
            self.commands.push(command.transformed(transform));
        }
    }

    /// [`append`](Self::append), with every command additionally confined to
    /// `clip` — an **absolute-space** rectangle, or `None` to append unchanged.
    ///
    /// # Why a clip cannot simply be in force around an `append`
    ///
    /// Because `append` documents that it does not read this scene's state, and
    /// that contract is load-bearing: it is what lets a recording be lifted into
    /// another coordinate space without replaying the paint pass. So a clip an
    /// ancestor could not record — one that has to reach a *layer* rather than a
    /// scene — has to be handed over explicitly. See
    /// [`LayerTree::composite`](crate::LayerTree::composite).
    ///
    /// # An empty clip drops the whole recording, and does so all at once
    ///
    /// Per-command dropping would be wrong: a `PushLayer` skipped while its
    /// `PopLayer` was kept unbalances the stack, and `PopLayer` carries no clip
    /// to be judged by. The rectangle applies uniformly to every command here,
    /// so the only safe granularity is all or nothing.
    pub fn append_clipped(&mut self, other: &Self, transform: Transform, clip: Option<Rect>) {
        self.append_slice(&other.commands, transform, clip);
    }

    /// [`append_clipped`](Self::append_clipped), for one contiguous run of
    /// `other`'s commands rather than all of them.
    ///
    /// # Why a recording can be cut anywhere at all
    ///
    /// Because every command carries its own resolved transform and clip. The
    /// save/restore stack is *state* consulted while recording and leaves
    /// nothing in the list, so commands 4..9 mean exactly what they meant in
    /// place — there is no prefix that has to be replayed to make them legible.
    /// That is the property [`append`](Self::append) already relies on to lift a
    /// whole recording into another space, applied to a piece of one.
    ///
    /// # Except across a `PushLayer`
    ///
    /// A group's markers are commands, so a range that opened a layer without
    /// closing it hands the backend an unbalanced stack. Callers must cut at
    /// [`layer_depth`](Self::layer_depth) zero;
    /// [`LayerTree::composite`](crate::LayerTree::composite) is where that rule
    /// is enforced, because it is the only thing that knows where the cuts came
    /// from.
    ///
    /// # Panics
    ///
    /// If `range` is not within `other`'s commands.
    pub fn append_range_clipped(
        &mut self,
        other: &Self,
        range: Range<usize>,
        transform: Transform,
        clip: Option<Rect>,
    ) {
        self.append_slice(&other.commands[range], transform, clip);
    }

    fn append_slice(&mut self, commands: &[Command], transform: Transform, clip: Option<Rect>) {
        let Some(clip) = clip else {
            self.commands.reserve(commands.len());
            for command in commands {
                self.commands.push(command.transformed(transform));
            }
            return;
        };
        if clip.is_empty() {
            return;
        }

        self.commands.reserve(commands.len());
        for command in commands {
            let mut command = command.transformed(transform);
            if let Some(existing) = command.clip_mut() {
                existing.add_rect(clip);
            }
            self.commands.push(command);
        }
    }

    /// Produce a new scene containing only the commands that can touch
    /// `damage`.
    ///
    /// This is the public face of `append_damage_filtered`,
    /// which the backends call internally. A frame driver or test uses this
    /// when it needs to see how many commands damage culling would skip, or to
    /// hand a pre-filtered scene to a backend that does not do its own culling.
    ///
    /// When the damage is clean or the scene is empty, returns an empty scene
    /// with zero stats — no allocation is done for a frame that has nothing
    /// to repaint.
    ///
    /// The returned scene is a separate allocation; this scene is not modified.
    #[must_use]
    pub fn damage_cull(&self, damage: &Damage) -> (Scene, DamageCullStats) {
        let mut out = Scene::new();
        if damage.is_clean() || self.commands.is_empty() {
            return (out, DamageCullStats::default());
        }
        // One bounding box for all regions, as the CPU backend does.
        // A per-region pass would emit a command twice where two regions
        // overlap it, and over-rendering is the safe direction.
        let bounds = if damage.is_everything() {
            damage.surface()
        } else {
            damage
                .regions()
                .iter()
                .copied()
                .fold(Rect::ZERO, Rect::union)
        };
        if bounds.is_empty() {
            return (out, DamageCullStats::default());
        }
        let stats = out.append_damage_filtered(self, Transform::IDENTITY, bounds);
        (out, stats.into())
    }

    pub(crate) fn append_damage_filtered(
        &mut self,
        other: &Self,
        transform: Transform,
        damage: Rect,
    ) -> DamageAppendStats {
        let mut stats = DamageAppendStats::default();
        let mut layers: Vec<PendingLayer<'_>> = Vec::new();

        self.commands.reserve(other.commands.len());

        for command in &other.commands {
            match command {
                Command::PushLayer { .. } => {
                    let parent_active = layers.last().is_none_or(|layer| layer.active);
                    layers.push(PendingLayer {
                        command,
                        active: parent_active && command.bounds().overlaps(damage),
                        emitted: false,
                    });
                    if !layers.last().expect("pushed layer").active {
                        stats.skipped_commands += 1;
                    }
                }
                Command::PopLayer => {
                    let layer = layers.pop().expect("balanced scene layer stack");
                    if layer.emitted {
                        self.commands.push(command.transformed(transform));
                        stats.copied_commands += 1;
                    } else {
                        stats.skipped_commands += 1;
                    }
                }
                _ => {
                    let active = layers.last().is_none_or(|layer| layer.active);
                    if !active || !command.bounds().overlaps(damage) {
                        stats.skipped_commands += 1;
                        continue;
                    }

                    for layer in &mut layers {
                        if layer.active && !layer.emitted {
                            self.commands.push(layer.command.transformed(transform));
                            layer.emitted = true;
                            stats.copied_commands += 1;
                        }
                    }

                    self.commands.push(command.transformed(transform));
                    stats.copied_commands += 1;
                }
            }
        }

        stats
    }

    /// How deep the save/restore stack currently is.
    ///
    /// Should be zero once a paint pass finishes; anything else means a widget
    /// saved without restoring.
    #[must_use]
    pub fn save_depth(&self) -> usize {
        self.stack.len()
    }

    /// How many layers are open.
    ///
    /// Should be zero once a paint pass finishes, for the reason
    /// [`save_depth`](Self::save_depth) should be.
    #[must_use]
    pub fn layer_depth(&self) -> usize {
        self.layers.len()
    }

    /// Append an already-resolved command verbatim.
    ///
    /// Bypasses the recording state — no transform, no clip, no alpha is
    /// applied — because the command being pushed already carries its own,
    /// resolved when it was first recorded. For a backend pass that rewrites a
    /// scene into another scene, which is what `CpuRenderer::resolve_filters`
    /// does; anything *authoring* a scene wants the [`Canvas`] methods.
    pub fn push_command(&mut self, command: Command) {
        self.commands.push(command);
    }

    fn push(&mut self, command: Command) {
        self.commands.push(command);
    }

    /// `true` if nothing recorded under the current state could be seen.
    fn fully_clipped(&self) -> bool {
        self.state.clip.is_everything_clipped()
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas for Scene {
    fn save(&mut self) {
        self.stack.push(self.state.clone());
    }

    fn restore(&mut self) {
        // An unbalanced restore means some widget's clip is about to leak into
        // an unrelated subtree. Fail here, where the culprit is on the stack.
        self.state = self
            .stack
            .pop()
            .expect("Canvas::restore with no matching save");
    }

    fn transform(&mut self, transform: Transform) {
        self.state.transform = transform.then(self.state.transform);
    }

    fn clip_rect(&mut self, rect: Rect) {
        let global = self.state.transform.apply_rect(rect);
        self.state.clip.add_rect(global);
    }

    fn clip_rrect(&mut self, rect: Rect, radius: f32) {
        // A radius of zero is a rectangle, and a rectangle clip costs no shape
        // at all. Worth checking because `Container` passes its decoration's
        // radius straight through, and most decorations have none.
        if radius <= 0.0 {
            self.clip_rect(rect);
            return;
        }
        self.clip_path(&Path::rounded_rect(rect, radius));
    }

    fn clip_path(&mut self, path: &Path) {
        if path.is_empty() {
            return;
        }
        self.state
            .clip
            .add_path(path.transformed(self.state.transform));
    }

    fn push_alpha(&mut self, alpha: f32) {
        // Multiplied into whatever is already in force, so nesting two halves
        // gives a quarter rather than the inner one winning.
        self.state.alpha = (self.state.alpha * alpha).clamp(0.0, 1.0);
    }

    fn push_layer(&mut self, bounds: Rect, alpha: f32, blend: BlendMode) {
        self.push_filtered_layer(bounds, alpha, blend, ImageFilter::NONE);
    }

    fn push_filtered_layer(
        &mut self,
        bounds: Rect,
        alpha: f32,
        blend: BlendMode,
        filter: ImageFilter,
    ) {
        let bounds = self.state.transform.apply_rect(bounds);
        // Grown by the blur's reach, or a blurred panel is cut off square at
        // its own edge — the "why does my blur have a hard border" bug. Zero
        // for an unfiltered layer, so nothing changes for the common case.
        let reach = filter.bounds_expansion();
        let bounds = if reach > 0.0 {
            Rect::new(
                bounds.left - reach,
                bounds.top - reach,
                bounds.right + reach,
                bounds.bottom + reach,
            )
        } else {
            bounds
        };
        self.layers.push(self.commands.len());
        self.push(Command::PushLayer {
            bounds,
            // The enclosing per-primitive fade multiplies in, so a layer inside
            // a faded subtree fades with it rather than escaping the fade.
            alpha: (alpha * self.state.alpha).clamp(0.0, 1.0),
            blend,
            clip: self.state.clip.clone(),
            filter,
        });
    }

    fn pop_layer(&mut self) {
        let opened = self
            .layers
            .pop()
            .expect("Canvas::pop_layer with no matching push_layer");

        // Either nothing was drawn into it, or what was drawn cannot be seen.
        // Both are a compositing target allocated to hold nothing, so the
        // markers go and anything between them goes with them.
        //
        // # The alpha case is not an optimisation
        //
        // `Opacity::new(0.0)` is how a subtree is hidden while staying
        // hit-testable, and that is a documented, load-bearing behaviour. A
        // scene that recorded its commands anyway would report damage every
        // time something invisible changed underneath it — a list rebuilding
        // behind a fully faded overlay would repaint the screen, forever, for
        // no picture.
        //
        // Sound for every blend mode, not just the normal one: a source
        // contributing zero alpha changes nothing whatever it is mixed with.
        let invisible = matches!(
            self.commands[opened],
            Command::PushLayer { alpha, .. } if alpha <= 0.0
        );

        // **A backdrop layer is never empty.**
        //
        // The rule below drops a layer nothing was drawn into, on the sound
        // argument that it is a compositing target allocated to hold nothing.
        // That argument does not survive contact with a *backdrop* filter,
        // whose entire content is what is already on the destination
        // underneath it: it draws nothing of its own **by definition**, and a
        // frosted panel with no children — `BackdropBlur::new(frosted(6.0))`
        // over a page, which is exactly how the widget reads — is the most
        // ordinary way to ask for one.
        //
        // Dropped here, that panel produced no `PushLayer` at all, so the
        // renderer never sampled, never blurred, and the page showed through
        // untouched. It failed silently and it failed *only* in the empty
        // case, so a panel with a label in it worked and the same panel
        // without one did not.
        let backdrop = matches!(
            self.commands[opened],
            Command::PushLayer { filter, .. } if filter.backdrop
        );
        if invisible || (!backdrop && opened + 1 == self.commands.len()) {
            self.commands.truncate(opened);
            return;
        }

        let contents = self.commands[opened + 1..]
            .iter()
            .map(Command::bounds)
            .fold(Rect::ZERO, Rect::union);
        if let Command::PushLayer { bounds, filter, .. } = &mut self.commands[opened] {
            // Replaced outright rather than intersected with what the caller
            // declared. A layer's declared bounds is an *estimate* — see
            // `Canvas::push_layer` — and treating it as a ceiling would make an
            // `Opacity` clip any child that paints outside its box, which a
            // shadow now routinely does. The contents are the truth, and by the
            // time anything reads this the group is closed.
            //
            // **Except for a filter's reach.** A blur samples outward from the
            // contents, so the region a backend has to rasterise is genuinely
            // larger than anything recorded inside — no command has those
            // pixels' bounds, because no command drew them. Replacing outright
            // here undid the expansion `push_filtered_layer` applied and cut
            // every blur off square at the edge of its own content, which is
            // the single most recognisable way a blur is implemented wrong.
            //
            // **And except for a backdrop filter, whose region is its own
            // box.** "The contents are the truth" is an argument about ink,
            // and a backdrop filter's subject is not its ink — it is the
            // destination underneath its bounds. Deriving its region from its
            // children says a frosted panel is only as wide as the label
            // inside it, and says a panel with no label at all is nothing:
            // `contents` folds from `Rect::ZERO`, so an empty group collapses
            // the layer to a speck at the origin and the blur lands nowhere
            // near the panel. The declared bounds are the truth here, and
            // they are already grown by the filter's reach in
            // `push_filtered_layer`.
            let reach = filter.bounds_expansion();
            if filter.backdrop {
                // Left exactly as declared.
            } else if reach > 0.0 {
                *bounds = Rect::new(
                    contents.left - reach,
                    contents.top - reach,
                    contents.right + reach,
                    contents.bottom + reach,
                );
            } else {
                *bounds = contents;
            }
        }
        self.push(Command::PopLayer);
    }

    fn fill_rect(&mut self, rect: Rect, paint: Paint) {
        let paint = self.state.apply(paint);
        if paint.is_invisible() || self.fully_clipped() {
            return;
        }
        let (transform, clip) = (self.state.transform, self.state.clip.clone());
        self.push(Command::FillRect {
            rect,
            paint,
            transform,
            clip,
        });
    }

    fn fill_path(&mut self, path: &Path, paint: Paint) {
        let paint = self.state.apply(paint);
        if paint.is_invisible() || path.is_empty() || self.fully_clipped() {
            return;
        }
        let (transform, clip) = (self.state.transform, self.state.clip.clone());
        self.push(Command::FillPath {
            path: path.clone(),
            paint,
            transform,
            clip,
        });
    }

    fn stroke_path(&mut self, path: &Path, stroke: Stroke, paint: Paint) {
        let paint = self.state.apply(paint);
        if paint.is_invisible() || stroke.is_invisible() || path.is_empty() || self.fully_clipped()
        {
            return;
        }
        let (transform, clip) = (self.state.transform, self.state.clip.clone());
        self.push(Command::StrokePath {
            path: path.clone(),
            stroke,
            paint,
            transform,
            clip,
        });
    }

    fn draw_shadow(&mut self, rect: Rect, radius: f32, shadow: Shadow) {
        let shadow = Shadow {
            color: self.state.faded(shadow.color),
            ..shadow
        };
        if shadow.is_invisible() || self.fully_clipped() {
            return;
        }
        let (transform, clip) = (self.state.transform, self.state.clip.clone());
        self.push(Command::DrawShadow {
            rect,
            radius,
            shadow,
            transform,
            clip,
        });
    }

    fn draw_glyphs(&mut self, run: &GlyphRun) {
        if run.is_empty() || self.fully_clipped() {
            return;
        }
        let (transform, clip) = (self.state.transform, self.state.clip.clone());
        let mut run = run.clone();
        run.color = self.state.faded(run.color);
        if run.color.is_transparent() {
            return;
        }
        self.push(Command::DrawGlyphs {
            run,
            transform,
            clip,
        });
    }

    fn draw_image(&mut self, rect: Rect, image: &Image) {
        if self.fully_clipped() {
            return;
        }
        let (transform, clip) = (self.state.transform, self.state.clip.clone());
        self.push(Command::DrawImage {
            rect,
            image: image.clone(),
            transform,
            clip,
        });
    }
}

impl fmt::Display for Scene {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut depth = 0_usize;
        for command in &self.commands {
            if matches!(command, Command::PopLayer) {
                depth = depth.saturating_sub(1);
            }
            for _ in 0..depth {
                f.write_str("  ")?;
            }
            match command {
                Command::FillRect { paint, .. } => {
                    writeln!(f, "fill {} {}", command.bounds(), paint.color)?;
                }
                Command::FillPath { paint, .. } => {
                    writeln!(f, "path {} {}", command.bounds(), paint.color)?;
                }
                Command::StrokePath { paint, stroke, .. } => {
                    writeln!(
                        f,
                        "stroke {} {} w{}",
                        command.bounds(),
                        paint.color,
                        stroke.width
                    )?;
                }
                Command::DrawShadow { shadow, .. } => {
                    writeln!(
                        f,
                        "shadow {} {} blur{}",
                        command.bounds(),
                        shadow.color,
                        shadow.blur
                    )?;
                }
                Command::DrawGlyphs { run, .. } => {
                    writeln!(
                        f,
                        "glyphs {} x{}{}",
                        command.bounds(),
                        run.glyphs.len(),
                        if run.is_rtl { " rtl" } else { "" }
                    )?;
                }
                Command::DrawImage { image, .. } => {
                    writeln!(
                        f,
                        "image {} {}x{}",
                        command.bounds(),
                        image.width(),
                        image.height()
                    )?;
                }
                Command::PushLayer {
                    bounds,
                    alpha,
                    blend,
                    ..
                } => {
                    writeln!(f, "layer {bounds} alpha{alpha} {blend:?}")?;
                }
                Command::PopLayer => f.write_str("end\n")?,
            }
            if matches!(command, Command::PushLayer { .. }) {
                depth += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Damage;
    use vieww_foundation::{Color, Gradient};

    use super::*;

    #[test]
    fn scaled_matches_recording_the_same_scene_through_a_scale() {
        // The property the web backend depends on: applying the device-pixel
        // ratio to a finished scene has to give the same commands as painting
        // the tree through a `Transformed::scale` would have — with the
        // difference that this one survives a repaint boundary, which the
        // transform does not.
        let mut recorded = Scene::new();
        recorded.save();
        recorded.transform(Transform::scale(2.0, 2.0));
        recorded.clip_rect(Rect::new(0.0, 0.0, 20.0, 20.0));
        recorded.fill_rect(Rect::new(5.0, 5.0, 15.0, 15.0), Color::RED.into());
        recorded.restore();

        let mut plain = Scene::new();
        plain.clip_rect(Rect::new(0.0, 0.0, 20.0, 20.0));
        plain.fill_rect(Rect::new(5.0, 5.0, 15.0, 15.0), Color::RED.into());
        let scaled = plain.scaled(2.0);

        assert_eq!(scaled.len(), recorded.len());
        assert_eq!(
            scaled.commands()[0].transform(),
            recorded.commands()[0].transform()
        );
        assert_eq!(
            scaled.commands()[0].clip().bounds(),
            recorded.commands()[0].clip().bounds(),
            "a clip is stored absolute, so it is scaled rather than transformed"
        );
        assert_eq!(scaled.fills()[0].0, recorded.fills()[0].0);
    }

    #[test]
    fn scaling_by_one_changes_nothing() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        assert_eq!(scene.scaled(1.0).fills()[0].0, scene.fills()[0].0);
    }

    #[test]
    fn commands_record_in_paint_order() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLUE.into());

        let fills = scene.fills();
        assert_eq!(fills[0].1.color, Color::RED);
        assert_eq!(fills[1].1.color, Color::BLUE);
    }

    #[test]
    fn the_transform_is_resolved_when_the_command_is_recorded() {
        let mut scene = Scene::new();
        scene.translate(Offset::new(100.0, 50.0));
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());

        assert_eq!(
            scene.fills()[0].0,
            Rect::new(100.0, 50.0, 110.0, 60.0),
            "bounds are absolute, so a backend needs no state stack"
        );
    }

    #[test]
    fn restore_undoes_a_transform() {
        let mut scene = Scene::new();
        scene.save();
        scene.translate(Offset::new(100.0, 0.0));
        scene.restore();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());

        assert_eq!(scene.fills()[0].0, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(scene.save_depth(), 0);
    }

    #[test]
    fn nested_transforms_compose_outermost_last() {
        let mut scene = Scene::new();
        scene.translate(Offset::new(10.0, 10.0));
        scene.translate(Offset::new(5.0, 5.0));
        scene.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Color::RED.into());

        assert_eq!(scene.fills()[0].0, Rect::new(15.0, 15.0, 16.0, 16.0));
    }

    #[test]
    fn a_clip_shrinks_recorded_bounds() {
        let mut scene = Scene::new();
        scene.clip_rect(Rect::new(0.0, 0.0, 20.0, 20.0));
        scene.fill_rect(Rect::new(10.0, 10.0, 100.0, 100.0), Color::RED.into());

        assert_eq!(scene.fills()[0].0, Rect::new(10.0, 10.0, 20.0, 20.0));
    }

    #[test]
    fn nested_clips_intersect() {
        let mut scene = Scene::new();
        scene.clip_rect(Rect::new(0.0, 0.0, 50.0, 50.0));
        scene.clip_rect(Rect::new(25.0, 0.0, 100.0, 30.0));
        scene.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED.into());

        assert_eq!(scene.fills()[0].0, Rect::new(25.0, 0.0, 50.0, 30.0));
    }

    // ------------------------------------------------------------- shape clips

    #[test]
    fn a_rounded_clip_keeps_a_shape_as_well_as_a_box() {
        let mut scene = Scene::new();
        scene.clip_rrect(Rect::new(0.0, 0.0, 40.0, 40.0), 8.0);
        scene.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED.into());

        let clip = scene.commands()[0].clip();
        assert_eq!(clip.bounds(), Some(Rect::new(0.0, 0.0, 40.0, 40.0)));
        assert_eq!(clip.shapes().len(), 1, "the curve survives for the backend");
    }

    #[test]
    fn a_square_rounded_clip_costs_no_shape() {
        let mut scene = Scene::new();
        scene.clip_rrect(Rect::new(0.0, 0.0, 40.0, 40.0), 0.0);
        scene.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED.into());

        assert!(
            scene.commands()[0].clip().shapes().is_empty(),
            "a radius of zero is a rectangle, and most decorations have none"
        );
    }

    #[test]
    fn a_shape_clip_is_resolved_into_absolute_space_like_a_rect_one() {
        let mut scene = Scene::new();
        scene.translate(Offset::new(100.0, 0.0));
        scene.clip_rrect(Rect::new(0.0, 0.0, 40.0, 40.0), 8.0);
        scene.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED.into());

        let clip = scene.commands()[0].clip();
        assert_eq!(clip.bounds(), Some(Rect::new(100.0, 0.0, 140.0, 40.0)));
        assert_eq!(
            clip.shapes()[0].bounds(),
            Rect::new(100.0, 0.0, 140.0, 40.0)
        );
    }

    #[test]
    fn a_clip_that_excludes_everything_records_nothing_at_all() {
        let mut scene = Scene::new();
        scene.clip_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        scene.clip_rect(Rect::new(90.0, 90.0, 100.0, 100.0));
        scene.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED.into());

        assert!(
            scene.is_empty(),
            "two disjoint clips let nothing through, so the fill is not worth \
             carrying to the backend"
        );
    }

    // ------------------------------------------------------------------ layers

    #[test]
    fn a_layer_reports_the_union_of_what_it_encloses() {
        let mut scene = Scene::new();
        scene.push_layer(Rect::new(0.0, 0.0, 1000.0, 1000.0), 0.5, BlendMode::Normal);
        scene.fill_rect(Rect::new(10.0, 10.0, 20.0, 20.0), Color::RED.into());
        scene.fill_rect(Rect::new(40.0, 40.0, 50.0, 50.0), Color::BLUE.into());
        scene.pop_layer();

        assert_eq!(
            scene.commands()[0].bounds(),
            Rect::new(10.0, 10.0, 50.0, 50.0),
            "the declared bounds was an estimate; the union is what it cost"
        );
        assert_eq!(scene.layer_depth(), 0);
    }

    #[test]
    fn a_child_painting_outside_the_declared_bounds_is_not_cut_off() {
        // The case this rule exists for: an `Opacity` declares its own box, and
        // a child inside it casts a shadow that reaches past it. Treating the
        // declaration as a ceiling would clip the shadow, and the bug would look
        // like "shadows disappear when something fades".
        let mut scene = Scene::new();
        scene.push_layer(Rect::new(0.0, 0.0, 10.0, 10.0), 0.5, BlendMode::Normal);
        scene.fill_rect(Rect::new(-20.0, -20.0, 30.0, 30.0), Color::RED.into());
        scene.pop_layer();

        assert_eq!(
            scene.commands()[0].bounds(),
            Rect::new(-20.0, -20.0, 30.0, 30.0)
        );
    }

    #[test]
    fn a_layer_faded_to_nothing_takes_its_contents_with_it() {
        // `Opacity::new(0.0)` hides a subtree while keeping it hit-testable.
        // Recording its commands anyway would damage the screen every time
        // something invisible changed — a list rebuilding behind a faded
        // overlay would repaint, forever, for no picture.
        let mut scene = Scene::new();
        scene.push_layer(Rect::new(0.0, 0.0, 100.0, 100.0), 0.0, BlendMode::Normal);
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        scene.fill_rect(Rect::new(20.0, 20.0, 30.0, 30.0), Color::BLUE.into());
        scene.pop_layer();

        assert!(scene.is_empty());
        assert_eq!(scene.layer_depth(), 0);
    }

    #[test]
    fn a_faded_layer_inside_a_visible_one_leaves_the_outer_one_intact() {
        // The truncation is to the *inner* push, not to the scene.
        let mut scene = Scene::new();
        scene.push_layer(Rect::new(0.0, 0.0, 100.0, 100.0), 0.5, BlendMode::Normal);
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        scene.push_layer(Rect::new(0.0, 0.0, 100.0, 100.0), 0.0, BlendMode::Normal);
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLUE.into());
        scene.pop_layer();
        scene.pop_layer();

        assert_eq!(scene.fills().len(), 1, "the visible fill survives");
        assert_eq!(scene.fills()[0].1.color, Color::RED);
        assert_eq!(scene.layer_depth(), 0);
    }

    #[test]
    fn a_layer_nothing_was_drawn_into_leaves_no_trace() {
        let mut scene = Scene::new();
        scene.push_layer(Rect::new(0.0, 0.0, 100.0, 100.0), 0.5, BlendMode::Normal);
        scene.pop_layer();

        assert!(
            scene.is_empty(),
            "an empty subtree behind an Opacity must not allocate a target"
        );
    }

    #[test]
    fn a_layers_contents_are_not_faded_at_record_time() {
        let mut scene = Scene::new();
        scene.push_layer(Rect::new(0.0, 0.0, 100.0, 100.0), 0.5, BlendMode::Normal);
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        scene.pop_layer();

        assert_eq!(
            scene.fills()[0].1.color,
            Color::RED,
            "group opacity fades the composited result, not each primitive — \
             that difference is the whole reason this exists"
        );
        let Command::PushLayer { alpha, .. } = scene.commands()[0] else {
            panic!("expected a layer");
        };
        assert_eq!(alpha, 0.5);
    }

    #[test]
    fn an_enclosing_fade_multiplies_into_a_layer_rather_than_being_lost() {
        let mut scene = Scene::new();
        scene.save();
        scene.push_alpha(0.5);
        scene.push_layer(Rect::new(0.0, 0.0, 100.0, 100.0), 0.5, BlendMode::Normal);
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        scene.pop_layer();
        scene.restore();

        let Command::PushLayer { alpha, .. } = scene.commands()[0] else {
            panic!("expected a layer");
        };
        assert_eq!(alpha, 0.25);
    }

    #[test]
    #[should_panic(expected = "no matching push_layer")]
    fn an_unbalanced_pop_layer_fails_loudly() {
        let mut scene = Scene::new();
        scene.pop_layer();
    }

    // ------------------------------------------------------ strokes and shadows

    #[test]
    fn a_stroke_reports_bounds_grown_by_half_its_width() {
        let mut scene = Scene::new();
        scene.stroke_rect(
            Rect::new(10.0, 10.0, 50.0, 50.0),
            Stroke::new(4.0),
            Color::RED.into(),
        );

        // **Grown by the stroke's `reach`, which is not half its width.** A
        // rectangle has four right-angled corners and the default join is a
        // miter, so the ink reaches `miter_limit` half-widths out from each of
        // them. Bounds grown by half the width alone would leave the corners
        // outside the damaged region — see `Stroke::reach`.
        let reach = Stroke::new(4.0).reach();
        assert_eq!(
            scene.commands()[0].bounds(),
            Rect::new(10.0, 10.0, 50.0, 50.0).inflate(reach),
            "a stroke is centred on its path, so its reach is outside it"
        );

        // Round joins have nothing to reach past, so those bounds are the
        // classic half-width.
        let mut rounded = Scene::new();
        rounded.stroke_rect(
            Rect::new(10.0, 10.0, 50.0, 50.0),
            Stroke::new(4.0).rounded(),
            Color::RED.into(),
        );
        assert_eq!(
            rounded.commands()[0].bounds(),
            Rect::new(8.0, 8.0, 52.0, 52.0)
        );
    }

    #[test]
    fn a_shadow_reports_bounds_larger_than_the_box_casting_it() {
        let mut scene = Scene::new();
        scene.draw_shadow(
            Rect::new(20.0, 20.0, 80.0, 80.0),
            8.0,
            Shadow::new(Color::BLACK, Offset::new(0.0, 4.0), 12.0),
        );

        let bounds = scene.commands()[0].bounds();
        assert!(
            bounds.top < 20.0 && bounds.bottom > 84.0,
            "a blur that damage under-reports smears on the next scroll: {bounds}"
        );
    }

    #[test]
    fn invisible_draws_are_dropped_at_record_time() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::TRANSPARENT.into());
        scene.draw_glyphs(&glyph_run(Offset::ZERO, 0));
        scene.fill_path(&Path::new(), Color::RED.into());
        scene.stroke_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Stroke::new(0.0),
            Color::RED.into(),
        );
        scene.draw_shadow(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
            Shadow::new(Color::TRANSPARENT, Offset::ZERO, 4.0),
        );
        assert!(scene.is_empty());
    }

    // --------------------------------------------------------------- gradients

    #[test]
    fn a_gradient_fades_stop_by_stop_under_an_alpha() {
        let mut scene = Scene::new();
        scene.save();
        scene.push_alpha(0.5);
        scene.fill_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Paint::gradient(Gradient::vertical().between(Color::RED, Color::BLUE)),
        );
        scene.restore();

        let gradient = scene.fills()[0]
            .1
            .gradient
            .expect("the gradient survives recording");
        assert!(
            gradient.stops().iter().all(|stop| stop.color.a == 128),
            "fading only the representative colour would leave the ramp at \
             full strength, and opacity would appear to do nothing"
        );
    }

    /// A run of `count` glyphs on a baseline at `origin`, 6px apart.
    fn glyph_run(origin: Offset, count: usize) -> GlyphRun {
        use std::rc::Rc;
        GlyphRun {
            font: vieww_foundation::FontData::new(Rc::new(vec![0_u8; 4]), 0),
            size: 10.0,
            color: Color::BLACK,
            origin,
            glyphs: (0..count)
                .map(|i| vieww_foundation::Glyph::new(i as u16, Offset::new(i as f32 * 6.0, 0.0)))
                .collect(),
            is_rtl: false,
            ascent: 8.0,
            descent: 2.0,
        }
    }

    #[test]
    fn a_glyph_runs_baseline_is_resolved_to_screen_space() {
        let mut scene = Scene::new();
        scene.translate(Offset::new(30.0, 40.0));
        scene.draw_glyphs(&glyph_run(Offset::new(5.0, 20.0), 3));

        let runs = scene.glyph_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].0,
            Offset::new(35.0, 60.0),
            "baseline in screen space"
        );
    }

    #[test]
    fn glyph_bounds_are_measured_from_the_run_rather_than_estimated() {
        let mut scene = Scene::new();
        scene.draw_glyphs(&glyph_run(Offset::new(0.0, 100.0), 3));

        // Baseline at y=100 with ascent 8 and descent 2, so the box is 92..102 —
        // not a guess derived from the font size.
        let bounds = scene.commands()[0].bounds();
        assert_eq!(bounds.top, 92.0, "{bounds}");
        assert_eq!(bounds.bottom, 102.0, "{bounds}");
        assert!(bounds.right >= 12.0, "the last glyph is included: {bounds}");
    }

    #[test]
    #[should_panic(expected = "no matching save")]
    fn an_unbalanced_restore_fails_loudly() {
        let mut scene = Scene::new();
        scene.restore();
    }

    #[test]
    fn scene_bounds_cover_every_command() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        scene.fill_rect(Rect::new(90.0, 90.0, 100.0, 100.0), Color::BLUE.into());
        assert_eq!(scene.bounds(), Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn appending_lifts_layers_and_shapes_together() {
        let mut inner = Scene::new();
        inner.clip_rrect(Rect::new(0.0, 0.0, 10.0, 10.0), 2.0);
        inner.push_layer(Rect::new(0.0, 0.0, 10.0, 10.0), 0.5, BlendMode::Normal);
        inner.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        inner.pop_layer();

        let mut outer = Scene::new();
        outer.append(&inner, Transform::translate(Offset::new(50.0, 0.0)));

        assert_eq!(outer.len(), 3);
        assert_eq!(
            outer.commands()[0].bounds(),
            Rect::new(50.0, 0.0, 60.0, 10.0),
            "the layer moved with its contents"
        );
        assert_eq!(
            outer.commands()[0].clip().shapes()[0].bounds(),
            Rect::new(50.0, 0.0, 60.0, 10.0),
            "and so did the clip shape, exactly rather than as a box"
        );
    }

    #[test]
    fn damage_filtered_append_skips_untouched_commands() {
        let mut source = Scene::new();
        source.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        source.fill_rect(Rect::new(90.0, 90.0, 100.0, 100.0), Color::BLUE.into());

        let mut culled = Scene::new();
        let stats = culled.append_damage_filtered(
            &source,
            Transform::IDENTITY,
            Rect::new(0.0, 0.0, 20.0, 20.0),
        );

        assert_eq!(culled.len(), 1);
        assert_eq!(culled.fills()[0].1.color, Color::RED);
        assert_eq!(stats.copied_commands, 1);
        assert_eq!(stats.skipped_commands, 1);
    }

    #[test]
    fn damage_filtered_append_keeps_layers_balanced_around_surviving_content() {
        let mut source = Scene::new();
        source.push_layer(Rect::new(0.0, 0.0, 120.0, 120.0), 0.5, BlendMode::Normal);
        source.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        source.fill_rect(Rect::new(90.0, 90.0, 100.0, 100.0), Color::BLUE.into());
        source.pop_layer();

        let mut culled = Scene::new();
        let stats = culled.append_damage_filtered(
            &source,
            Transform::IDENTITY,
            Rect::new(0.0, 0.0, 20.0, 20.0),
        );

        assert_eq!(culled.len(), 3, "push, surviving draw, pop");
        assert!(matches!(culled.commands()[0], Command::PushLayer { .. }));
        assert!(matches!(culled.commands()[2], Command::PopLayer));
        assert_eq!(culled.fills().len(), 1);
        assert_eq!(culled.fills()[0].1.color, Color::RED);
        assert_eq!(stats.copied_commands, 3);
        assert_eq!(stats.skipped_commands, 1);
    }

    #[test]
    fn damage_filtered_append_drops_an_unaffected_layer_wholesale() {
        let mut source = Scene::new();
        source.push_layer(Rect::new(80.0, 80.0, 120.0, 120.0), 0.5, BlendMode::Normal);
        source.fill_rect(Rect::new(90.0, 90.0, 100.0, 100.0), Color::BLUE.into());
        source.pop_layer();

        let mut culled = Scene::new();
        let stats = culled.append_damage_filtered(
            &source,
            Transform::IDENTITY,
            Rect::new(0.0, 0.0, 20.0, 20.0),
        );

        assert!(culled.is_empty());
        assert_eq!(stats.copied_commands, 0);
        assert_eq!(stats.skipped_commands, 3, "push, draw and pop all skipped");
    }

    // ------------------------------------------------- damage_cull (public)

    #[test]
    fn damage_cull_on_clean_damage_returns_empty() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        let damage = Damage::new(Rect::new(0.0, 0.0, 100.0, 100.0));

        let (culled, stats) = scene.damage_cull(&damage);
        assert!(culled.is_empty());
        assert_eq!(stats, DamageCullStats::default());
    }

    #[test]
    fn damage_cull_skips_commands_outside_the_region() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        scene.fill_rect(Rect::new(90.0, 90.0, 100.0, 100.0), Color::BLUE.into());

        let mut damage = Damage::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        damage.add(Rect::new(5.0, 5.0, 15.0, 15.0));

        let (culled, stats) = scene.damage_cull(&damage);
        assert_eq!(culled.len(), 1);
        assert_eq!(stats.copied_commands, 1);
        assert_eq!(stats.skipped_commands, 1);
        assert_eq!(stats.total(), 2);
    }

    #[test]
    fn damage_cull_keeps_everything_for_full_repaint() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        scene.fill_rect(Rect::new(90.0, 90.0, 100.0, 100.0), Color::BLUE.into());

        let damage = Damage::everything(Rect::new(0.0, 0.0, 100.0, 100.0));

        let (culled, stats) = scene.damage_cull(&damage);
        assert_eq!(culled.len(), 2);
        assert_eq!(stats.skipped_commands, 0);
    }
}
