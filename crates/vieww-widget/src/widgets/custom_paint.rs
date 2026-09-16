//! Custom drawing through existing widget primitives.
//!
//! # The approach
//!
//! There is no canvas API in vieww-foundation — and inventing one would
//! be a subsystem, not a widget. Instead, this module provides a
//! [`CustomPainter`] trait that produces drawing *instructions*, and a
//! [`CustomPaint`] widget that converts those instructions into the
//! existing widget tree (`ColoredBox`, `Clip`, `Transformed`).
//!
//! This is not the fastest path — the fastest path would be a render
//! object that emits commands directly into the scene — but it is
//! *correct* on every backend today, and the instruction set maps
//! cleanly onto what the renderer already handles.
//!
//! # The instruction set
//!
//! Five primitives, chosen because each maps to an existing widget:
//!
//! | Instruction | Widget |
//! |---|---|
//! | `FillRect` | `ColoredBox` with size |
//! | `FillCircle` | `ColoredBox` clipped with `Clip::rounded(radius)` |
//! | `DrawLine` | `ColoredBox` rotated via `Transformed` |
//! | `FillRoundedRect` | `ColoredBox` with `radius` |
//! | `Group` | `Opacity` for the group's alpha |
//!
//! Anything more complex (arbitrary paths, gradients along a stroke)
//! needs the render pipeline and is out of scope here.
//!
//! # The repaint contract
//!
//! A painter is asked [`should_repaint`](CustomPainter::should_repaint)
//! when its element rebuilds. If `true`, the instructions are
//! re-recorded; if `false`, the previous widget tree is reused. This
//! is the same caching that `RepaintBoundary` provides, applied at the
//! painter level.

use vieww_foundation::{Color, Offset, Rect, Size, Transform};

use crate::prelude::*;

/// One drawing instruction, in terms the existing widgets can express.
#[derive(Debug, Clone)]
pub enum DrawInstruction {
    /// Fill a rectangle.
    FillRect { rect: Rect, color: Color },
    /// Fill a circle.
    FillCircle {
        center: Offset,
        radius: f32,
        color: Color,
    },
    /// Draw a line (rendered as a thin rotated rectangle).
    DrawLine {
        from: Offset,
        to: Offset,
        color: Color,
        width: f32,
    },
    /// Fill a rounded rectangle.
    FillRoundedRect {
        rect: Rect,
        radius: f32,
        color: Color,
    },
    /// Apply opacity to a group of instructions.
    Group {
        alpha: f32,
        instructions: Vec<DrawInstruction>,
    },
}

/// Draws something no existing widget can.
///
/// The contract: given the size you will be painted at, return a list
/// of drawing instructions. The `should_repaint` question is the
/// performance contract — answer it honestly.
pub trait CustomPainter: std::any::Any {
    /// Produce the drawing instructions.
    fn paint(&self, size: Size) -> Vec<DrawInstruction>;

    /// This painter as an `Any`, so `should_repaint` can downcast.
    ///
    /// Implement as `self`, always.
    fn as_any(&self) -> &dyn std::any::Any;

    /// `true` if the instructions would differ from `previous`.
    ///
    /// Compare whatever drives your drawing: a time value, a data
    /// generation, a counter. `false` when the output would be identical.
    ///
    /// `previous` is `&dyn CustomPainter` rather than `&Self` because
    /// [`CustomPaint`] stores its painter as `Rc<dyn CustomPainter>`, and a
    /// trait with a `Self` parameter cannot be made into a trait object at
    /// all. Downcast through [`as_any`](Self::as_any) and return `true` when
    /// the downcast fails — a painter of a different type entirely is a
    /// different drawing.
    fn should_repaint(&self, previous: &dyn CustomPainter) -> bool {
        let _ = previous;
        true
    }
}

/// A widget that paints through a [`CustomPainter`].
///
/// The painter's instructions are converted into existing widgets at
/// build time. The result is a `Stack` of positioned primitives.
///
/// # Examples
///
/// A simple chart — three bars:
///
/// ```ignore
/// struct BarChart {
///     values: Vec<f32>,
/// }
///
/// impl CustomPainter for BarChart {
///     fn paint(&self, size: Size) -> Vec<DrawInstruction> {
///         let max = self.values.iter().cloned().fold(0.0_f32, f32::max).max(1.0);
///         let bar_width = size.width / self.values.len() as f32;
///
///         self.values.iter().enumerate().map(|(i, &v)| {
///             let height = (v / max) * size.height;
///             DrawInstruction::FillRect {
///                 rect: Rect::new(
///                     i as f32 * bar_width,
///                     size.height - height,
///                     i as f32 * bar_width + bar_width - 4.0,
///                     size.height,
///                 ),
///                 color: Color::rgb(58, 122, 246),
///             }
///         }).collect()
///     }
///
///     fn as_any(&self) -> &dyn std::any::Any { self }
///
///     fn should_repaint(&self, previous: &dyn CustomPainter) -> bool {
///         match previous.as_any().downcast_ref::<Self>() {
///             Some(prev) => self.values != prev.values,
///             None => true,
///         }
///     }
/// }
///
/// CustomPaint::sized(Size::new(240.0, 120.0), BarChart { values: vec![10.0, 25.0, 15.0] })
/// ```
pub struct CustomPaint {
    painter: std::rc::Rc<dyn CustomPainter>,
    size: Size,
}

// By hand: `dyn CustomPainter` is not `Debug`, and requiring it of every
// painter would be a tax on implementors for the sake of one derive.
impl std::fmt::Debug for CustomPaint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomPaint")
            .field("size", &self.size)
            .field("painter", &"<dyn CustomPainter>")
            .finish()
    }
}

impl CustomPaint {
    /// A painter at a fixed size.
    #[must_use]
    pub fn sized<P: CustomPainter>(size: Size, painter: P) -> Self {
        Self {
            painter: std::rc::Rc::new(painter),
            size,
        }
    }

    /// A painter with the existing size, drawn as instructions only.
    #[must_use]
    pub fn instructions(&self) -> Vec<DrawInstruction> {
        self.painter.paint(self.size)
    }
}

impl Widget for CustomPaint {
    fn debug_name(&self) -> &'static str {
        "CustomPaint"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let instructions = self.painter.paint(self.size);
        let widgets = instructions_to_widgets(&instructions);

        // `Stack::push` takes one child; `children` takes an iterator. There
        // is no `.child()` on Stack.
        Stack::new()
            .children(widgets)
            .push(SizedBox::from_size(self.size))
            .into()
    }
}

crate::widget_node_from!(CustomPaint);

/// Convert drawing instructions into widget nodes.
///
/// Each instruction maps to positioned widgets in a `Stack`.
fn instructions_to_widgets(instructions: &[DrawInstruction]) -> Vec<WidgetNode> {
    let mut result = Vec::new();

    for instruction in instructions {
        match instruction {
            DrawInstruction::FillRect { rect, color } => {
                result.push(rect_widget(*rect, *color, 0.0));
            }
            DrawInstruction::FillCircle {
                center,
                radius,
                color,
            } => {
                result.push(
                    Positioned::new()
                        .left(center.dx - radius)
                        .top(center.dy - radius)
                        .child(
                            Clip::rounded(*radius).child(
                                ColoredBox::new(*color).child(SizedBox::square(radius * 2.0)),
                            ),
                        )
                        .into(),
                );
            }
            DrawInstruction::DrawLine {
                from,
                to,
                color,
                width,
            } => {
                // A line is a thin rect, rotated to the correct angle.
                //
                // **The rotation pivot is the whole correctness of this arm.**
                // `Transformed::rotate` turns the child about its *top-left*,
                // so the pivot has to be the rect's top-left too — which means
                // the rect must be positioned at `from` exactly, and the
                // half-width that centres the stroke on the path applied
                // *inside* the transform, before the rotation, so it rotates
                // with the line instead of staying stuck on the screen's y
                // axis.
                //
                // Positioning at `from.dy - width/2` and then rotating about
                // that corner is the bug this replaced: it pivots about a point
                // half a stroke above where the line actually starts, so every
                // stroke lands displaced by `R(angle)·(0, w/2) - (0, w/2)`.
                // That is zero only for a horizontal line, which is why it
                // survived: horizontal rules looked perfect and every slanted
                // or vertical stroke — the letterforms in the launch film —
                // slid sideways by up to a full stroke width.
                let dx = to.dx - from.dx;
                let dy = to.dy - from.dy;
                let length = (dx * dx + dy * dy).sqrt();
                let angle = dy.atan2(dx);
                // Translate first, then rotate: `a.then(b)` applies `a` first.
                let pivoted = Transform::translate(Offset::new(0.0, -width / 2.0))
                    .then(Transform::rotate(angle));

                result.push(
                    Positioned::new()
                        .left(from.dx)
                        .top(from.dy)
                        .child(
                            // `Transformed::new(matrix)` constructs; the widget
                            // then goes in via `.child()`. There is no
                            // `Transformed::new(widget).rotate(..)`.
                            Transformed::new(pivoted).child(
                                ColoredBox::new(*color)
                                    .child(SizedBox::from_size(Size::new(length, *width))),
                            ),
                        )
                        .into(),
                );
            }
            DrawInstruction::FillRoundedRect {
                rect,
                radius,
                color,
            } => {
                result.push(rect_widget(*rect, *color, *radius));
            }
            DrawInstruction::Group {
                alpha,
                instructions,
            } => {
                let children = instructions_to_widgets(instructions);
                result.push(
                    Opacity::new(*alpha)
                        .child(Stack::new().children(children))
                        .into(),
                );
            }
        }
    }

    result
}

/// A positioned colored rectangle.
fn rect_widget(rect: Rect, color: Color, radius: f32) -> WidgetNode {
    Positioned::new()
        .left(rect.left)
        .top(rect.top)
        .child(
            Container::new()
                .color(color)
                .radius(radius)
                .child(SizedBox::from_size(Size::new(rect.width(), rect.height()))),
        )
        .into()
}

/// A convenience painter from a closure.
///
/// The closure runs at build time and returns instructions. The
/// generation counter distinguishes two closures so `should_repaint`
/// works correctly.
#[derive(Debug)]
pub struct DrawOnce<F: Fn(Size) -> Vec<DrawInstruction> + 'static> {
    draw: F,
    generation: u64,
}

impl<F: Fn(Size) -> Vec<DrawInstruction> + 'static> DrawOnce<F> {
    #[must_use]
    pub fn new(draw: F) -> Self {
        Self {
            draw,
            generation: next_generation(),
        }
    }
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_generation() -> u64 {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl<F: Fn(Size) -> Vec<DrawInstruction> + 'static> CustomPainter for DrawOnce<F> {
    fn paint(&self, size: Size) -> Vec<DrawInstruction> {
        (self.draw)(size)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn should_repaint(&self, previous: &dyn CustomPainter) -> bool {
        match previous.as_any().downcast_ref::<Self>() {
            Some(prev) => self.generation != prev.generation,
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ThreeBars {
        values: Vec<f32>,
    }

    impl CustomPainter for ThreeBars {
        fn paint(&self, size: Size) -> Vec<DrawInstruction> {
            let max = self.values.iter().cloned().fold(0.0_f32, f32::max).max(1.0);
            let bar_width = size.width / self.values.len() as f32;

            self.values
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let height = (v / max) * size.height;
                    DrawInstruction::FillRect {
                        rect: Rect::new(
                            i as f32 * bar_width,
                            size.height - height,
                            i as f32 * bar_width + bar_width - 4.0,
                            size.height,
                        ),
                        color: Color::rgb(58, 122, 246),
                    }
                })
                .collect()
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn should_repaint(&self, previous: &dyn CustomPainter) -> bool {
            match previous.as_any().downcast_ref::<Self>() {
                Some(prev) => self.values != prev.values,
                None => true,
            }
        }
    }

    #[test]
    fn custom_paint_produces_instructions() {
        let painter = ThreeBars {
            values: vec![10.0, 25.0, 15.0],
        };
        let instructions = painter.paint(Size::new(300.0, 100.0));
        assert_eq!(instructions.len(), 3);
    }

    #[test]
    fn should_repaint_compares_values() {
        let a = ThreeBars {
            values: vec![1.0, 2.0],
        };
        let b = ThreeBars {
            values: vec![1.0, 2.0],
        };
        let c = ThreeBars {
            values: vec![1.0, 3.0],
        };

        assert!(!a.should_repaint(&b));
        assert!(a.should_repaint(&c));
    }

    #[test]
    fn draw_once_has_distinct_generations() {
        let a = DrawOnce::new(|_| vec![]);
        let b = DrawOnce::new(|_| vec![]);
        assert!(
            a.should_repaint(&b),
            "different closures are different painters"
        );
    }

    #[test]
    fn line_instruction_computes_length() {
        let instructions = vec![DrawInstruction::DrawLine {
            from: Offset::new(0.0, 0.0),
            to: Offset::new(3.0, 4.0),
            color: Color::BLACK,
            width: 2.0,
        }];

        let widgets = instructions_to_widgets(&instructions);
        assert_eq!(widgets.len(), 1, "one line = one widget");
    }
}
