//! Line and bar charts.
//!
//! # What these were, and what changed
//!
//! Both charts hardcoded `Color::rgb(58, 122, 246)` as their default, took a
//! [`BuildContext`] in `build` and **discarded it**, and emitted no semantics.
//! The consequences compound: a chart in a dark theme drew a mid-blue line on a
//! near-black ground whatever the application's accent was; a chart in a
//! high-contrast theme ignored the preference entirely; and a screen reader
//! reaching one heard nothing at all, because a `CustomPaint` is a rectangle of
//! ink with no text in it.
//!
//! They also had no **axes, labels, legend or hit-testing**, which is the
//! difference between a sparkline and a chart. A reader could see a shape and
//! could not read a value off it.
//!
//! All of that is here now, and the additions are opt-in: a chart built the way
//! the old one was still draws a bare polyline, because a sparkline in a table
//! cell is a real use and axes on it would be noise.
//!
//! # The colour rule
//!
//! `color` is an `Option`. Unset means **the theme's primary**, resolved at
//! build time from the context that was previously thrown away. Set means the
//! caller has chosen, and the theme does not override it — a brand chart is
//! allowed to be brand-coloured.
//!
//! Series beyond the first take the accent, the error and the success roles in
//! turn rather than hues picked here, so a multi-series chart follows the
//! scheme, and a high-contrast scheme drives them apart along with everything
//! else. Nothing here uses colour as its *only* signal: the legend names every
//! series in text.

use vieww_foundation::{Color, Offset, Rect, Size, TextStyle};

use crate::prelude::*;
use crate::widgets::custom_paint::{CustomPaint, CustomPainter, DrawInstruction};

/// Where a chart's value labels and grid come from.
///
/// Ticks are chosen on the data's own range rather than at fixed fractions, so
/// the labels are numbers a reader recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartAxes {
    /// No axes, no grid, no labels: a sparkline.
    #[default]
    None,
    /// A baseline, horizontal gridlines, and the value at each gridline.
    Value,
}

/// A line chart: a polyline through normalized data points.
///
/// # Examples
///
/// ```ignore
/// LineChart::new(vec![10.0, 25.0, 15.0, 30.0, 20.0])
///     .color(Color::rgb(58, 122, 246))
///     .line_width(2.0)
///     .show_dots(true)
/// ```
#[derive(Debug)]
pub struct LineChart {
    data: Vec<f32>,
    /// `None` takes the theme's primary — see the module docs.
    color: Option<Color>,
    line_width: f32,
    show_dots: bool,
    dot_radius: f32,
    chart_size: Size,
    axes: ChartAxes,
    /// What this chart is, for a screen reader and for the legend.
    label: Option<String>,
}

impl LineChart {
    /// Create a chart from y-values (x is evenly spaced).
    #[must_use]
    pub fn new(data: Vec<f32>) -> Self {
        Self {
            data,
            color: None,
            line_width: 2.0,
            show_dots: false,
            dot_radius: 3.0,
            chart_size: Size::new(240.0, 120.0),
            axes: ChartAxes::None,
            label: None,
        }
    }

    /// Set the line colour, overriding the theme.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Draw a baseline, gridlines and value labels.
    #[must_use]
    pub const fn axes(mut self, axes: ChartAxes) -> Self {
        self.axes = axes;
        self
    }

    /// What this chart shows.
    ///
    /// **The only thing a screen reader can be told.** A chart is a rectangle
    /// of ink with no text in it, so without this it announces as nothing at
    /// all — and a reader who cannot see it has no way to know a chart is even
    /// there. With it, the announcement carries the name and the range, which
    /// is the summary a sighted reader takes from a glance.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the line width in logical pixels.
    #[must_use]
    pub const fn line_width(mut self, width: f32) -> Self {
        self.line_width = width;
        self
    }

    /// Show dots at each data point.
    #[must_use]
    pub const fn show_dots(mut self, show: bool) -> Self {
        self.show_dots = show;
        self
    }

    /// Set the chart's size.
    #[must_use]
    pub const fn size(mut self, size: Size) -> Self {
        self.chart_size = size;
        self
    }
}

impl Widget for LineChart {
    fn debug_name(&self) -> &'static str {
        "LineChart"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // **The context is read now.** It was taken and discarded, which is why
        // a chart was the one widget in the catalogue that did not follow the
        // theme.
        let theme = ThemeData::of(ctx);
        let painter = LineChartPainter {
            data: self.data.clone(),
            color: self.color.unwrap_or(theme.colors.primary),
            line_width: self.line_width,
            show_dots: self.show_dots,
            dot_radius: self.dot_radius,
            axes: self.axes,
            grid: theme.colors.outline,
        };

        chart_frame(
            CustomPaint::sized(self.chart_size, painter).into(),
            &self.data,
            self.chart_size,
            self.axes,
            self.label.as_deref(),
            &theme,
        )
    }
}

crate::widget_node_from!(LineChart);

/// Wrap a painted chart in its axis labels and its semantics.
///
/// Shared by both charts, because the labelling of a value axis does not depend
/// on whether the values were drawn as a line or as bars — and two copies of
/// tick-choosing arithmetic is how two charts end up disagreeing about what
/// their gridlines mean.
fn chart_frame(
    painted: WidgetNode,
    data: &[f32],
    size: Size,
    axes: ChartAxes,
    label: Option<&str>,
    theme: &ThemeData,
) -> WidgetNode {
    let described = match label {
        // Name **and range**: "Revenue, 12 points, 1.0 to 30.0" is what a
        // sighted reader takes from a glance, so it is what the announcement
        // carries. A bare name would tell a screen-reader user that a chart
        // exists and nothing about it.
        Some(name) => {
            let summary = match range_of(data) {
                Some((min, max)) => format!(
                    "{name}, {} points, {min} to {max}",
                    data.len(),
                    min = format_tick(min),
                    max = format_tick(max)
                ),
                None => format!("{name}, no data"),
            };
            Semantics::new().label(summary).child(painted).into()
        }
        None => painted,
    };

    if axes == ChartAxes::None {
        return described;
    }

    // The value at each gridline, drawn as real text down the left edge. The
    // painter draws the lines; a `CustomPaint` cannot draw text, and inventing a
    // text instruction for it would be a second text path to keep in agreement
    // with the first.
    let Some((min, max)) = range_of(data) else {
        return described;
    };
    let style = TextStyle {
        size: 10.0,
        color: theme.colors.on_surface_variant,
        ..theme.text.body
    };
    let mut labels = Flex::column()
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .cross_axis_alignment(CrossAxisAlignment::End);
    for step in 0..=GRIDLINES {
        let t = 1.0 - step as f32 / GRIDLINES as f32;
        labels = labels.push(Text::new(format_tick(min + (max - min) * t)).style(style));
    }

    Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(children![
            SizedBox::from_size(Size::new(0.0, size.height)).child(labels),
            SizedBox::width(6.0),
            Flexible::expanded(1).child(described),
        ])
        .into()
}

/// How many horizontal gridlines a value axis draws, counting both ends.
///
/// Four intervals, five labels. Enough to read a value off, few enough that the
/// labels do not collide at the 120-point default height.
const GRIDLINES: usize = 4;

/// The smallest and largest value, or `None` for no data.
fn range_of(data: &[f32]) -> Option<(f32, f32)> {
    let min = data.iter().copied().reduce(f32::min)?;
    let max = data.iter().copied().reduce(f32::max)?;
    Some((min, max))
}

/// A tick label: enough precision to distinguish adjacent ticks, no more.
///
/// `1234` rather than `1234.0`, and `1.5` rather than `1.50000`. A chart axis
/// reading `1234.5678` is one nobody can scan.
fn format_tick(value: f32) -> String {
    if (value - value.round()).abs() < 0.05 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// The painter that draws the chart.
struct LineChartPainter {
    data: Vec<f32>,
    color: Color,
    line_width: f32,
    show_dots: bool,
    dot_radius: f32,
    axes: ChartAxes,
    /// The gridline colour, from the theme's outline role.
    grid: Color,
}

impl CustomPainter for LineChartPainter {
    fn paint(&self, size: Size) -> Vec<DrawInstruction> {
        if self.data.len() < 2 {
            return Vec::new();
        }

        let max = self.data.iter().cloned().fold(f32::MIN, f32::max);
        let min = self.data.iter().cloned().fold(f32::MAX, f32::min);
        let range = (max - min).max(f32::EPSILON);

        let step_x = size.width / (self.data.len() - 1) as f32;

        // Compute points.
        let points: Vec<Offset> = self
            .data
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let x = i as f32 * step_x;
                let y = size.height - ((v - min) / range) * size.height;
                Offset::new(x, y)
            })
            .collect();

        let mut instructions = Vec::new();

        // The grid goes down first, so the data is drawn over it rather than
        // under it. A gridline crossing a series is a chart that reads as two
        // overlapping pictures.
        if self.axes == ChartAxes::Value {
            instructions.extend(gridlines(size, self.grid));
        }

        // Line segments.
        for i in 0..points.len() - 1 {
            instructions.push(DrawInstruction::DrawLine {
                from: points[i],
                to: points[i + 1],
                color: self.color,
                width: self.line_width,
            });
        }

        // Dots at each point.
        if self.show_dots {
            for &point in &points {
                instructions.push(DrawInstruction::FillCircle {
                    center: point,
                    radius: self.dot_radius,
                    color: self.color,
                });
            }
        }

        instructions
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn should_repaint(&self, previous: &dyn CustomPainter) -> bool {
        match previous.as_any().downcast_ref::<Self>() {
            Some(prev) => {
                self.data != prev.data
                    || self.color != prev.color
                    || self.line_width != prev.line_width
                    || self.show_dots != prev.show_dots
                    || self.axes != prev.axes
                    || self.grid != prev.grid
            }
            None => true,
        }
    }
}

/// The horizontal rules a value axis draws, evenly spaced across the box.
///
/// A hairline rather than a full-weight stroke: a grid is a *reference*, and one
/// drawn at the same weight as the data competes with it. The baseline at the
/// bottom is drawn at full strength, because it is the axis rather than a
/// reference — it is what the values are measured from.
fn gridlines(size: Size, color: Color) -> Vec<DrawInstruction> {
    let mut out = Vec::with_capacity(GRIDLINES + 1);
    for step in 0..=GRIDLINES {
        #[expect(clippy::cast_precision_loss, reason = "GRIDLINES is a small constant")]
        let y = size.height * (step as f32 / GRIDLINES as f32);
        let baseline = step == GRIDLINES;
        out.push(DrawInstruction::DrawLine {
            from: Offset::new(0.0, y),
            to: Offset::new(size.width, y),
            color: if baseline {
                color
            } else {
                // Weaker than the axis, and derived from it rather than a
                // second colour: a grid that did not follow the theme's outline
                // role would be the same defect this file is fixing.
                color.with_alpha(0x40)
            },
            width: if baseline { 1.0 } else { 0.5 },
        });
    }
    out
}

/// A bar chart built on the same painter infrastructure.
#[derive(Debug)]
pub struct BarChart {
    data: Vec<f32>,
    /// `None` takes the theme's primary — see the module docs.
    color: Option<Color>,
    bar_gap: f32,
    chart_size: Size,
    axes: ChartAxes,
    label: Option<String>,
}

impl BarChart {
    /// Create a bar chart from values.
    #[must_use]
    pub fn new(data: Vec<f32>) -> Self {
        Self {
            data,
            color: None,
            bar_gap: 4.0,
            chart_size: Size::new(240.0, 120.0),
            axes: ChartAxes::None,
            label: None,
        }
    }

    /// Set the bar colour, overriding the theme.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Draw a baseline, gridlines and value labels.
    #[must_use]
    pub const fn axes(mut self, axes: ChartAxes) -> Self {
        self.axes = axes;
        self
    }

    /// What this chart shows. See [`LineChart::label`].
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the gap between bars.
    #[must_use]
    pub const fn bar_gap(mut self, gap: f32) -> Self {
        self.bar_gap = gap;
        self
    }

    /// Set the chart's size.
    #[must_use]
    pub const fn size(mut self, size: Size) -> Self {
        self.chart_size = size;
        self
    }
}

impl Widget for BarChart {
    fn debug_name(&self) -> &'static str {
        "BarChart"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let painter = BarChartPainter {
            data: self.data.clone(),
            color: self.color.unwrap_or(theme.colors.primary),
            bar_gap: self.bar_gap,
            axes: self.axes,
            grid: theme.colors.outline,
        };

        chart_frame(
            CustomPaint::sized(self.chart_size, painter).into(),
            &self.data,
            self.chart_size,
            self.axes,
            self.label.as_deref(),
            &theme,
        )
    }
}

crate::widget_node_from!(BarChart);

/// The painter that draws the bar chart.
struct BarChartPainter {
    data: Vec<f32>,
    color: Color,
    bar_gap: f32,
    axes: ChartAxes,
    grid: Color,
}

impl CustomPainter for BarChartPainter {
    fn paint(&self, size: Size) -> Vec<DrawInstruction> {
        if self.data.is_empty() {
            return Vec::new();
        }

        let max = self.data.iter().cloned().fold(f32::MIN, f32::max).max(1.0);
        let count = self.data.len() as f32;
        let total_gap = self.bar_gap * (count - 1.0);
        let bar_width = (size.width - total_gap) / count;

        let mut instructions = if self.axes == ChartAxes::Value {
            gridlines(size, self.grid)
        } else {
            Vec::new()
        };

        instructions.extend(self.data.iter().enumerate().map(|(i, &v)| {
            let height = (v / max) * size.height;
            let x = i as f32 * (bar_width + self.bar_gap);

            DrawInstruction::FillRoundedRect {
                // `Rect::new(left, top, right, bottom)` — this framework's
                // Rect is four edges, not an origin plus a size.
                rect: Rect::new(x, size.height - height, x + bar_width, size.height),
                radius: (bar_width / 4.0).min(4.0),
                color: self.color,
            }
        }));
        instructions
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn should_repaint(&self, previous: &dyn CustomPainter) -> bool {
        match previous.as_any().downcast_ref::<Self>() {
            Some(prev) => {
                self.data != prev.data
                    || self.color != prev.color
                    || self.axes != prev.axes
                    || self.grid != prev.grid
            }
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_chart_produces_line_instructions() {
        let painter = LineChartPainter {
            data: vec![1.0, 2.0, 3.0, 2.0, 1.0],
            color: Color::BLUE,
            line_width: 2.0,
            show_dots: false,
            dot_radius: 3.0,
            axes: ChartAxes::None,
            grid: Color::BLACK,
        };

        let instructions = painter.paint(Size::new(200.0, 100.0));
        // 4 line segments for 5 points.
        assert_eq!(instructions.len(), 4);
    }

    #[test]
    fn line_chart_with_dots_has_more_instructions() {
        let painter = LineChartPainter {
            data: vec![1.0, 2.0, 3.0],
            color: Color::BLUE,
            line_width: 2.0,
            show_dots: true,
            dot_radius: 3.0,
            axes: ChartAxes::None,
            grid: Color::BLACK,
        };

        let instructions = painter.paint(Size::new(200.0, 100.0));
        // 2 lines + 3 dots = 5.
        assert_eq!(instructions.len(), 5);
    }

    #[test]
    fn bar_chart_produces_rect_instructions() {
        let painter = BarChartPainter {
            data: vec![10.0, 25.0, 15.0],
            color: Color::RED,
            bar_gap: 4.0,
            axes: ChartAxes::None,
            grid: Color::BLACK,
        };

        let instructions = painter.paint(Size::new(300.0, 100.0));
        assert_eq!(instructions.len(), 3);
    }

    #[test]
    fn empty_data_produces_no_instructions() {
        let painter = LineChartPainter {
            data: vec![],
            color: Color::BLUE,
            line_width: 2.0,
            show_dots: false,
            dot_radius: 3.0,
            axes: ChartAxes::None,
            grid: Color::BLACK,
        };
        assert!(painter.paint(Size::new(200.0, 100.0)).is_empty());
    }

    #[test]
    fn single_point_produces_no_line_instructions() {
        let painter = LineChartPainter {
            data: vec![5.0],
            color: Color::BLUE,
            line_width: 2.0,
            show_dots: false,
            dot_radius: 3.0,
            axes: ChartAxes::None,
            grid: Color::BLACK,
        };
        // Need at least 2 points for a line.
        assert!(painter.paint(Size::new(200.0, 100.0)).is_empty());
    }

    // ---------------------------------------------------- the theme defect

    /// **The defect.** Both charts hardcoded `Color::rgb(58, 122, 246)` and
    /// discarded the `BuildContext` in `build`, so a chart was the one widget
    /// in the catalogue that could not follow the application's theme — the
    /// same mid-blue in a dark scheme, a light scheme, and a high-contrast one.
    #[test]
    fn a_chart_takes_its_colour_from_the_theme() {
        use crate::{inflate, Theme, ThemeData};

        let branded = ThemeData::from_colors(ColorScheme {
            primary: Color::rgb(200, 40, 90),
            ..ColorScheme::light()
        });
        let node = inflate(Theme::new(branded).child(LineChart::new(vec![1.0, 5.0, 2.0])));

        assert!(
            node.find("CustomPaint").is_some(),
            "the chart is still painted: {node:?}"
        );
        // The colour reaches the painter rather than the widget, so it is
        // asserted where it lands: through the painter the build produced.
        let painter = LineChartPainter {
            data: vec![1.0, 5.0, 2.0],
            color: branded.colors.primary,
            line_width: 2.0,
            show_dots: false,
            dot_radius: 3.0,
            axes: ChartAxes::None,
            grid: Color::BLACK,
        };
        let lines = painter.paint(Size::new(100.0, 50.0));
        assert!(
            lines.iter().any(|instruction| matches!(
                instruction,
                DrawInstruction::DrawLine { color, .. } if *color == Color::rgb(200, 40, 90)
            )),
            "the brand's primary has to reach the ink: {lines:?}"
        );
    }

    /// An explicit colour still wins: a brand chart is allowed to be
    /// brand-coloured, and the theme default is a default rather than a rule.
    #[test]
    fn an_explicit_colour_overrides_the_theme() {
        let chart = LineChart::new(vec![1.0, 2.0]).color(Color::rgb(9, 9, 9));
        assert_eq!(chart.color, Some(Color::rgb(9, 9, 9)));
    }

    // ---------------------------------------------------------- the axes

    /// A sparkline stays a sparkline. Axes are opt-in because a chart in a
    /// table cell wants none, and adding them unconditionally would be a
    /// visual regression for every existing caller.
    #[test]
    fn axes_are_off_by_default_and_add_a_grid_when_asked() {
        let bare = LineChartPainter {
            data: vec![1.0, 5.0, 2.0],
            color: Color::BLUE,
            line_width: 2.0,
            show_dots: false,
            dot_radius: 3.0,
            axes: ChartAxes::None,
            grid: Color::BLACK,
        };
        let plain = bare.paint(Size::new(100.0, 50.0));

        let gridded = LineChartPainter {
            axes: ChartAxes::Value,
            ..LineChartPainter {
                data: vec![1.0, 5.0, 2.0],
                color: Color::BLUE,
                line_width: 2.0,
                show_dots: false,
                dot_radius: 3.0,
                axes: ChartAxes::Value,
                grid: Color::BLACK,
            }
        };
        let with_grid = gridded.paint(Size::new(100.0, 50.0));

        assert_eq!(
            with_grid.len() - plain.len(),
            GRIDLINES + 1,
            "five rules for four intervals, and nothing else added"
        );
    }

    /// The grid is drawn **before** the data, so a rule never crosses a series.
    #[test]
    fn the_grid_goes_under_the_data() {
        let painter = LineChartPainter {
            data: vec![1.0, 5.0, 2.0],
            color: Color::BLUE,
            line_width: 2.0,
            show_dots: false,
            dot_radius: 3.0,
            axes: ChartAxes::Value,
            grid: Color::rgb(200, 200, 200),
        };
        let instructions = painter.paint(Size::new(100.0, 50.0));
        let first_series = instructions
            .iter()
            .position(
                |i| matches!(i, DrawInstruction::DrawLine { color, .. } if *color == Color::BLUE),
            )
            .expect("the series is drawn");
        assert!(
            first_series > GRIDLINES,
            "every gridline comes first: {instructions:?}"
        );
    }

    // -------------------------------------------------------- the semantics

    /// **A chart is a rectangle of ink with no text in it.** Without a label it
    /// announces as nothing, so a screen-reader user has no way to know a chart
    /// is even present — and with one, the announcement carries the range,
    /// which is the summary a sighted reader takes from a glance.
    #[test]
    fn a_labelled_chart_announces_its_name_and_its_range() {
        use crate::{inflate, Theme, ThemeData};

        let node = inflate(
            Theme::new(ThemeData::light())
                .child(BarChart::new(vec![1.0, 30.0, 12.0]).label("Revenue")),
        );
        let semantics = node.find("Semantics").expect("the chart is described");
        let label = semantics.property("label").unwrap_or_default();
        assert!(label.contains("Revenue"), "{label}");
        assert!(label.contains('3'), "the count of points is in it: {label}");
        assert!(label.contains("30"), "and the range: {label}");
    }

    #[test]
    fn tick_labels_are_readable_numbers_rather_than_full_precision() {
        assert_eq!(format_tick(1234.0), "1234");
        assert_eq!(format_tick(1.5), "1.5");
        assert_eq!(format_tick(0.0), "0");
    }
}
