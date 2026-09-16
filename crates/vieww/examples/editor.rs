//! The smallest thing that is still a text editor.
//!
//! ```text
//! cargo run -p vieww --example editor
//! ```
//!
//! # Why this exists
//!
//! `viewwstudio` is a large application — a compile pipeline, a fold
//! projection, a highlighter, a watcher, a preview — and when text goes missing
//! in it there are a dozen places to look. Almost none of them are the text
//! editing itself.
//!
//! This is the text editing itself, with nothing else in the window: one
//! multiline field, a counter that says how many characters and how much is
//! selected, and three buttons. It is a **bisect**. If a defect reproduces
//! here, it is in the framework — `RenderEditableText`, the gesture arena, the
//! controlled-value round trip — and the studio is innocent. If it does not
//! reproduce here, the framework is doing its job and the defect is in what the
//! studio wraps around it.
//!
//! `tests/editor.rs` drives this same widget through a real `FrameDriver`, so
//! the bisect is a test rather than something to look at.
//!
//! # The one thing this demonstrates on purpose
//!
//! The field is **controlled**: it holds no text of its own. The signal below
//! is the document, `on_changed` is the only way it changes, and `on_selection`
//! carries a selection and no text at all.
//!
//! That last part is the whole lesson. A pointer can move a caret and it cannot
//! type — so the pointer path must not be able to express a text change. When
//! it can, a drag hands back a value assembled from whatever text the widget
//! was built with, which during a drag is a frame behind, and the document is
//! silently replaced by an older copy of itself. Wiring `on_selection` is what
//! makes that unrepresentable rather than merely avoided.

use vieww::prelude::*;
use vieww::{BuildContext, Widget, WidgetKind, WidgetNode};
use vieww_foundation::{Alignment, Border, EdgeInsets, TextEditingValue, TextSelection, TextStyle};
use vieww_widget::{Container, Flexible, SizedBox, TextField};

/// The starting document. Long enough to drag across and to scroll.
pub const SAMPLE: &str = "\
The quick brown fox jumps over the lazy dog.
Select this line by dragging across it.
Then drag from here down to the line below.
Nothing you do with the pointer may change this text.
Only typing, deleting and the buttons may.
";

/// The editor, and everything it knows.
///
/// One signal. A field that held its own text would be a second copy of the
/// document, and the two would disagree the first time anything else wrote to
/// it — which is exactly the bug class this example exists to rule out.
#[derive(Debug, Clone)]
pub struct Editor {
    pub value: Signal<TextEditingValue>,
}

impl Editor {
    #[must_use]
    pub fn new(runtime: &Runtime) -> Self {
        let mut value = TextEditingValue::new(SAMPLE);
        // At the top, like a file being opened — `TextEditingValue::new` puts
        // the caret at the end because that is what a *form field* wants.
        value.selection = TextSelection::collapsed(0);
        Self {
            value: runtime.signal(value),
        }
    }

    /// Replace the whole document. What the Clear button does.
    pub fn set_text(&self, text: impl Into<String>) {
        let text: String = text.into();
        let at = text.len();
        self.value.set(TextEditingValue {
            text,
            selection: TextSelection::collapsed(at),
            composing: None,
            secondary: Vec::new(),
        });
    }

    /// Select everything, without touching the text.
    pub fn select_all(&self) {
        let mut value = self.value.get();
        value.selection = TextSelection::new(0, value.text.len());
        self.value.set(value);
    }

    /// How many characters are selected.
    #[must_use]
    pub fn selected_len(&self) -> usize {
        let value = self.value.get();
        let (start, end) = (
            value.selection.base.min(value.selection.extent),
            value.selection.base.max(value.selection.extent),
        );
        value.text.get(start..end).map_or(0, str::len)
    }
}

/// The widget under test.
#[derive(Debug)]
pub struct EditorScreen {
    pub editor: Editor,
}

impl Widget for EditorScreen {
    fn debug_name(&self) -> &'static str {
        "EditorScreen"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let colors = ThemeData::of(ctx).colors;
        let value = self.editor.value.get();
        let selected = self.editor.selected_len();

        let field = {
            let editor = self.editor.clone();
            let selecting = self.editor.clone();
            TextField::new(value.clone())
                .style(
                    TextStyle::new(14.0)
                        .family(vieww_foundation::FontFamily::Monospace)
                        .color(colors.on_surface),
                )
                // Multiline is the default; `single_line()` is the opt-out.
                .selection_color(colors.primary.with_alpha(60))
                .cursor(colors.primary, 2.0)
                // The keyboard reports a whole value: a key can change the
                // text, the selection and the composing region at once, and
                // there is nothing to assemble.
                .on_changed(std::rc::Rc::new(move |next| editor.value.set(next)))
                // **The pointer reports a selection and nothing else.**
                // See this file's header for why that is the entire point.
                .on_selection(std::rc::Rc::new(move |selection: TextSelection| {
                    let mut value = selecting.value.get();
                    value.selection = selection;
                    value.secondary.clear();
                    selecting.value.set(value);
                }))
        };

        let status = format!(
            "{} characters, {} lines, {selected} selected",
            value.text.len(),
            value.text.split('\n').count()
        );

        Container::new()
            .color(colors.surface)
            .padding(EdgeInsets::all(16.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children![
                        Text::new("editor — the smallest thing that is still one")
                            .style(TextStyle::new(13.0).color(colors.on_surface_variant)),
                        SizedBox::height(12.0),
                        Flexible::expanded(1).child(
                            Container::new()
                                .color(colors.surface_variant)
                                .radius(8.0)
                                .padding(EdgeInsets::all(12.0))
                                .border(Border {
                                    color: colors.outline,
                                    width: 1.0,
                                })
                                .child(field)
                        ),
                        SizedBox::height(12.0),
                        Text::new(status)
                            .style(TextStyle::new(12.0).color(colors.on_surface_variant)),
                        SizedBox::height(12.0),
                        self.buttons(colors),
                    ]),
            )
            .into()
    }
}

vieww_widget::widget_node_from!(EditorScreen);

impl EditorScreen {
    fn buttons(&self, colors: ColorScheme) -> WidgetNode {
        let clearing = self.editor.clone();
        let restoring = self.editor.clone();
        let selecting = self.editor.clone();
        Flex::row()
            .children(children![
                button("Clear", colors, move || clearing.set_text("")),
                SizedBox::width(8.0),
                button("Restore sample", colors, move || restoring.set_text(SAMPLE)),
                SizedBox::width(8.0),
                button("Select all", colors, move || selecting.select_all()),
            ])
            .into()
    }
}

fn button(label: &'static str, colors: ColorScheme, on_tap: impl Fn() + 'static) -> WidgetNode {
    Pressable::new(move |press| {
        Container::new()
            .color(if press > 0.5 {
                colors.primary
            } else {
                colors.surface_variant
            })
            .radius(6.0)
            .padding(EdgeInsets::symmetric(14.0, 8.0))
            .alignment(Alignment::CENTER)
            .border(Border {
                color: colors.outline,
                width: 1.0,
            })
            .child(Text::new(label).style(TextStyle::new(12.5).color(colors.on_surface)))
            .into()
    })
    .on_tap(on_tap)
    .into()
}

fn main() {
    // Headless by default so this runs anywhere, including a container with no
    // graphics adapter. `--window` opens a real one.
    let windowed = std::env::args().any(|arg| arg == "--window");
    if windowed {
        println!("this example is headless; run the studio for a window");
    }

    let mut driver = vieww_render::FrameDriver::new(vieww_foundation::Size::new(800.0, 600.0));
    let runtime = driver.elements().runtime().clone();
    let editor = Editor::new(&runtime);
    driver.set_root(EditorScreen {
        editor: editor.clone(),
    });
    driver.draw_frame();

    println!(
        "editor mounted with {} characters",
        editor.value.get().text.len()
    );
    println!("{}", driver.elements().debug_tree());

    editor.select_all();
    driver.draw_frame();
    println!("select all → {} characters selected", editor.selected_len());
    println!(
        "text after selecting all → {} characters (a selection is not an edit)",
        editor.value.get().text.len()
    );

    editor.set_text("");
    driver.draw_frame();
    println!("clear → {} characters", editor.value.get().text.len());

    editor.set_text(SAMPLE);
    driver.draw_frame();
    println!("restore → {} characters", editor.value.get().text.len());
}
