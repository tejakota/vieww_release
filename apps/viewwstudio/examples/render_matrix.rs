//! What the Render button can actually put on the stage.
//!
//! Not a reading of the catalogue — every case below goes through the real
//! `compile` + `dlopen` + `build()` path the button uses, and reports what came
//! back. Run it from the workspace root after `cargo build`:
//!
//! ```text
//! cargo run -p viewwstudio --example render_matrix
//! ```

use std::path::{Path, PathBuf};

use viewwstudio::compile::{compile, Session, Toolchain};
use viewwstudio::loaded::Preview;

fn target() -> PathBuf {
    let exe = std::env::current_exe().expect("a binary");
    exe.parent()
        .and_then(Path::parent)
        .expect("target/debug")
        .to_path_buf()
}

/// One probe: a name, and the body of a buffer.
struct Case {
    group: &'static str,
    name: &'static str,
    body: &'static str,
}

fn case(group: &'static str, name: &'static str, body: &'static str) -> Case {
    Case { group, name, body }
}

fn main() {
    let toolchain = match Toolchain::discover(&target()) {
        Ok(toolchain) => toolchain,
        Err(error) => {
            eprintln!("no toolchain: {error}");
            std::process::exit(1);
        }
    };

    let cases = cases();
    let mut ok = 0usize;
    let mut bad = 0usize;

    for (index, probe) in cases.iter().enumerate() {
        let session = Session::new(0x9000 + index as u64).expect("a temp dir");
        let text = format!("use vieww::prelude::*;\n{}", probe.body);
        let compiled = match compile(&toolchain, &session, &text, "screen.rs") {
            Ok(compiled) => compiled,
            Err(error) => {
                println!(
                    "[{}] {}: RUSTC DID NOT RUN — {error}",
                    probe.group, probe.name
                );
                bad += 1;
                continue;
            }
        };

        if compiled.failed() {
            let first = compiled
                .diagnostics
                .first()
                .map(|d| format!("{}: {}", d.code, d.message))
                .unwrap_or_else(|| "no diagnostic".to_owned());
            println!("[{}] {}: COMPILE FAILED — {first}", probe.group, probe.name);
            bad += 1;
            continue;
        }

        let library = compiled.library.expect("a library");
        match Preview::load(&library) {
            Ok(preview) => {
                let dump = vieww_widget::debug_tree(preview.node());
                let lines = dump.lines().count();
                println!("[{}] {}: ok — {lines} nodes", probe.group, probe.name);
                ok += 1;
            }
            Err(error) => {
                println!("[{}] {}: LOAD FAILED — {error:?}", probe.group, probe.name);
                bad += 1;
            }
        }
    }

    println!("\nrendered: {ok}   refused: {bad}   total: {}", cases.len());
}

fn cases() -> Vec<Case> {
    vec![
        // -------------------------------------------------------------- layout
        case(
            "layout",
            "Container / Flex / Stack / Positioned",
            r##"
pub fn screen() -> impl Widget {
    Stack::new()
        .children(children![
            Container::new().color(Color::hex(0x10_1418)),
            Positioned::new().left(12.0).top(12.0).child(
                Flex::column().children(children![Text::new("a"), Text::new("b")])),
        ])
}
"##,
        ),
        case(
            "layout",
            "LayoutBuilder / Measured / IntrinsicSize / AspectRatio",
            r##"
pub fn screen() -> impl Widget {
    LayoutBuilder::new(|constraints| {
        AspectRatio::new(16.0 / 9.0)
            .child(Text::new(format!("{}", constraints.max_width as i32)))
            .into()
    })
}
"##,
        ),
        case(
            "layout",
            "Grid / SafeArea / Offstage",
            r##"
pub fn screen() -> impl Widget {
    SafeArea::new().child(Grid::columns(2).children(children![
        Text::new("x"),
        Offstage::new(true).child(Text::new("y")),
    ]))
}
"##,
        ),
        // ---------------------------------------------------------------- text
        case(
            "text",
            "Text with style and elision",
            r##"
pub fn screen() -> impl Widget {
    SizedBox::width(80.0).child(
        Text::new("a long label that will not fit at all").max_lines(1))
}
"##,
        ),
        case(
            "text",
            "TextField",
            r##"
pub fn screen() -> impl Widget {
    Container::new()
        .padding(EdgeInsets::all(16.0))
        .child(TextField::new(TextEditingValue::default()))
}
"##,
        ),
        case(
            "text",
            "Markdown",
            r##"
pub fn screen() -> impl Widget {
    Markdown::new("# Title\n\nA paragraph with **bold** in it.\n\n* one\n* two\n")
}
"##,
        ),
        // ------------------------------------------------------------ controls
        case(
            "controls",
            "Button / Checkbox / Switch / Slider",
            r##"
pub fn screen() -> impl Widget {
    Flex::column().children(children![
        Button::new("Press").on_pressed(|| {}),
        Checkbox::new(true),
        Switch::new(false),
        Slider::new(0.4).range(0.0, 1.0),
    ])
}
"##,
        ),
        case(
            "controls",
            "DataTable / TreeView / Accordion",
            r##"
pub fn screen() -> impl Widget {
    Flex::column().children(children![
        DataTable::new(
            vec![DataColumn::new("Name", 120.0)],
            1,
            28.0,
            |_row, _column| Text::new("one").into(),
        ),
        Accordion::new(Text::new("Section"), Text::new("body")),
    ])
}
"##,
        ),
        case(
            "controls",
            "Dialog / Snackbar / EmptyState",
            r##"
pub fn screen() -> impl Widget {
    Flex::column().children(children![
        EmptyState::new(icons::add(), "Nothing here"),
        Snackbar::new("Saved"),
    ])
}
"##,
        ),
        case(
            "controls",
            "DatePicker / TimePicker / ColorPicker",
            r##"
pub fn screen() -> impl Widget {
    TimePicker::new(vieww::Time::new(9, 30).unwrap())
}
"##,
        ),
        // ----------------------------------------------------------- scrolling
        case(
            "scrolling",
            "ListView / GridView / CustomScrollView",
            r##"
pub fn screen() -> impl Widget {
    ListView::new(200, 28.0, std::rc::Rc::new(
        |index| Text::new(format!("row {index}")).into()))
}
"##,
        ),
        // --------------------------------------------------------- painting/fx
        case(
            "paint",
            "CustomPaint",
            r##"
#[derive(Debug)]
struct Dots;

impl CustomPainter for Dots {
    fn paint(&self, size: Size) -> Vec<DrawInstruction> {
        vec![DrawInstruction::FillRect {
            rect: Rect::new(0.0, 0.0, size.width, size.height),
            color: Color::hex(0x20_2530),
        }]
    }
    fn as_any(&self) -> &dyn std::any::Any { self }
}

pub fn screen() -> impl Widget {
    CustomPaint::sized(Size::new(200.0, 120.0), Dots)
}
"##,
        ),
        case(
            "paint",
            "Opacity / Clip / Transformed / DecoratedBox",
            r##"
pub fn screen() -> impl Widget {
    Opacity::new(0.6).child(
        Clip::rounded(12.0).child(
            Container::new().color(Color::hex(0x33_88FF)).child(SizedBox::square(80.0))))
}
"##,
        ),
        case(
            "paint",
            "Image from raw RGBA (no decoder)",
            r##"
// The widget is in the prelude; the pixel type, `vieww::ImageData`, is the
// `vieww_foundation::Image` alias the facade re-exports. The prelude's `Image`
// is the widget, and `vieww::Image` resolves to it through the glob — so the
// pixel type needs a name of its own, which is what `ImageData` is.
pub fn screen() -> impl Widget {
    Image::new(vieww::ImageData::from_rgba8(vec![255, 0, 0, 255], 1, 1))
}
"##,
        ),
        case(
            "paint",
            "LineChart / BarChart",
            r##"
pub fn screen() -> impl Widget {
    LineChart::new(vec![1.0, 4.0, 2.0, 8.0, 5.0])
}
"##,
        ),
        // ----------------------------------------------------------- animation
        case(
            "animation",
            "Animated + Curve",
            r##"
pub fn screen() -> impl Widget {
    Animated::new(1.0).from(0.0).curve(Curve::FAST_OUT_SLOW_IN).build(|value| {
        Opacity::new(value).child(Text::new("fading in")).into()
    })
}
"##,
        ),
        case(
            "animation",
            "AnimatedContainer",
            r##"
pub fn screen() -> impl Widget {
    AnimatedContainer::new().color(Color::hex(0x22_88FF)).width(120.0).height(60.0)
}
"##,
        ),
        // ------------------------------------------------------------- gesture
        case(
            "gesture",
            "GestureDetector with state",
            r##"
#[derive(Debug)]
pub struct Counter;

#[derive(Debug, Default)]
struct CounterState { taps: u32 }

impl vieww::ElementState for CounterState {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

impl Widget for Counter {
    fn debug_name(&self) -> &'static str { "Counter" }
    fn kind(&self) -> WidgetKind<'_> { WidgetKind::Composed }
    fn create_state(&self) -> Option<Box<dyn vieww::ElementState>> {
        Some(Box::new(CounterState::default()))
    }
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let taps = ctx.state(|state: &CounterState| state.taps).unwrap_or(0);
        GestureDetector::new()
            .on_tap(|_| {})
            .child(Text::new(format!("{taps} taps")))
            .into()
    }
}

pub fn screen() -> impl Widget { Counter }
"##,
        ),
        // ------------------------------------------------------------- theming
        case(
            "theme",
            "Theme / ColorScheme / adaptive",
            r##"
pub fn screen() -> impl Widget {
    Theme::new(ThemeData::adaptive(TargetPlatform::Android, true))
        .child(Button::new("Themed").on_pressed(|| {}))
}
"##,
        ),
        case(
            "a11y",
            "Semantics / Directionality / Localizations",
            r##"
pub fn screen() -> impl Widget {
    Directionality::new(TextDirection::Rtl).child(
        Semantics::new().label("a greeting").child(Text::new("hello")))
}
"##,
        ),
        // -------------------------------------------------------- ordinary Rust
        case(
            "rust",
            "plain fns, iterators, collections, tests",
            r##"
use std::collections::HashMap;

fn tally(words: &[&str]) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for word in words { *out.entry((*word).to_owned()).or_insert(0) += 1; }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_counts() { assert_eq!(super::tally(&["a", "a"])["a"], 2); }
}

pub fn screen() -> impl Widget {
    let counts = tally(&["a", "b", "a"]);
    let mut keys: Vec<_> = counts.keys().cloned().collect();
    keys.sort();
    Flex::column().children(
        keys.into_iter().map(|k| Text::new(k).into()).collect::<Vec<WidgetNode>>())
}
"##,
        ),
        case(
            "rust",
            "traits, generics, closures, Result",
            r##"
trait Label { fn label(&self) -> String; }
struct Item(u32);
impl Label for Item { fn label(&self) -> String { format!("#{}", self.0) } }

fn first<T: Label>(items: &[T]) -> Result<String, &'static str> {
    items.first().map(Label::label).ok_or("empty")
}

pub fn screen() -> impl Widget {
    Text::new(first(&[Item(7)]).unwrap_or_default())
}
"##,
        ),
        // ---------------------------------------------------------- the limits
        case(
            "limits",
            "a third-party crate (serde)",
            r##"
use serde::Serialize;
pub fn screen() -> impl Widget { Text::new("never gets here") }
"##,
        ),
        case(
            "limits",
            "vieww_widget by its real name",
            r##"
pub fn screen() -> impl Widget { vieww_widget::Text::new("direct") }
"##,
        ),
        case(
            "limits",
            "edition 2024 syntax (let-chains)",
            r##"
pub fn screen() -> impl Widget {
    let a = Some(1);
    if let Some(x) = a && x > 0 { Text::new("yes") } else { Text::new("no") }
}
"##,
        ),
        case(
            "limits",
            "std::thread and std::fs actually run",
            r##"
pub fn screen() -> impl Widget {
    let handle = std::thread::spawn(|| 40 + 2);
    let answer = handle.join().unwrap_or(0);
    let cwd = std::env::current_dir().map(|p| p.display().to_string())
        .unwrap_or_default();
    Flex::column().children(children![
        Text::new(format!("thread said {answer}")),
        Text::new(format!("cwd is {} chars", cwd.len())),
    ])
}
"##,
        ),
        case(
            "limits",
            "no screen() at all",
            r##"
pub fn other() -> impl Widget { Text::new("wrong name") }
"##,
        ),
        case(
            "limits",
            "screen() panics",
            r##"
pub fn screen() -> impl Widget {
    panic!("boom");
    #[allow(unreachable_code)]
    Text::new("never")
}
"##,
        ),
        case(
            "limits",
            "build() panics",
            r##"
#[derive(Debug)]
pub struct Bad;

impl Widget for Bad {
    fn debug_name(&self) -> &'static str { "Bad" }
    fn kind(&self) -> WidgetKind<'_> { WidgetKind::Composed }
    fn build(&self, _ctx: &BuildContext) -> WidgetNode { panic!("boom in build") }
}

pub fn screen() -> impl Widget { Bad }
"##,
        ),
    ]
}
