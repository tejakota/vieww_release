//! Code generation: the program tree into one deterministic Rust file.
//!
//! Every generated statement carries a `// say: file:line  phrase` marker, so
//! the studio can point a rustc diagnostic at the `.say` line that produced
//! it and so "show me the Rust" can put the two side by side. The output is
//! self-contained: state, the `ElementState` impl, one function per screen,
//! and a small `mod say` of handler helpers — `--extern vieww` is the only
//! thing it needs beyond `std`.
//!
//! # Where the state lives, and why
//!
//! A previewed screen is compiled to a cdylib and mounted by the host; there
//! is no `Runtime` across that boundary and no way for guest code to
//! subscribe to one. So the state lives in an `ElementState` on the generated
//! root (`SayApp`): created once at mount, written by handlers through the
//! element's own handle, polled once a frame by the tree via
//! `take_pending`, and carried across every Render by `snapshot`/`restore` —
//! the half the studio already runs around its guest root. It is the same
//! pattern the studio's own scaffold template ships.

use crate::parse::{
    AKind, ActionLine, Align, CmpOp, ColorSpec, Cond, Cross, EventKind, Expr, Init, Node, Part,
    Pin, Program, Prop, Screen, StateDecl, StateKind, ThemeSlot, Value, WKind, Widget,
};

pub(crate) fn generate(file: &std::rc::Rc<String>, program: &Program) -> String {
    let multi_screen = program.screens.len() > 1;
    let uses_dialog = !program.modal_fields.is_empty();
    let uses_toast = program_has_toast(program);

    let mut g = Gen {
        file: file.to_string(),
        app_name: program.app.name.clone(),
        out: String::new(),
        states: &program.states,
        modal_fields: &program.modal_fields,
        screens: &program.screens,
        needs_text: program.states.iter().any(|s| s.kind == StateKind::Text) || uses_dialog,
        multi_screen,
        uses_dialog,
        uses_toast,
        unique: 0,
    };
    g.header();
    g.type_alias();
    g.state_struct();
    g.say_mod();
    g.app_widget();
    for screen in program.screens.iter() {
        g.screen_fn(screen);
    }
    if g.uses_dialog {
        g.dialog_dispatch();
        for dialog in g.dialog_list() {
            let screen = program
                .screens
                .iter()
                .find(|s| find_dialog(&s.body, &dialog.title))
                .expect("the dialog was found when it was listed");
            g.dialog_fn(&dialog, screen);
        }
    }
    g.out
}

// ---------------------------------------------------------------------------
// The generator

struct Gen<'a> {
    file: String,
    app_name: Option<String>,
    out: String,
    states: &'a [StateDecl],
    modal_fields: &'a [String],
    screens: &'a [Screen],
    needs_text: bool,
    multi_screen: bool,
    uses_dialog: bool,
    uses_toast: bool,
    unique: usize,
}

/// One `open a dialog` in the file, with its generated function name.
#[derive(Debug, Clone)]
struct DialogFn {
    title: String,
    slug: String,
    line: u32,
}

impl<'a> Gen<'a> {
    fn push(&mut self, line: impl AsRef<str>) {
        self.out.push_str(line.as_ref());
        self.out.push('\n');
    }

    fn indent(&mut self, depth: usize, line: impl AsRef<str>) {
        for _ in 0..depth {
            self.out.push_str("    ");
        }
        self.out.push_str(line.as_ref());
        self.out.push('\n');
    }

    fn unique(&mut self, stem: &str) -> String {
        self.unique += 1;
        format!("{stem}_{}", self.unique)
    }

    fn state(&self, name: &str) -> Option<&'a StateDecl> {
        self.states.iter().find(|s| s.name == name)
    }

    // -- header -------------------------------------------------------------

    fn header(&mut self) {
        let app_name = self
            .app_name
            .clone()
            .unwrap_or_else(|| "A Say app".to_owned());
        // Plain comments, not `//!`: the generated file compiles both as a
        // crate root (the studio's buffer) and included as a module (the
        // tests), and inner doc comments are only legal in the first.
        self.push("// Generated from a `.say` file by Say — edit the `.say` file, not this.");
        self.push("// This pane is what it compiles to; every line below points back at the");
        self.push("// `.say` line that produced it with a `// say:` marker.");
        self.push("//");
        self.push(format!("// Say: {app_name}"));
        self.push("//");
        self.push("// say-language: 1 · say-codegen 0.0.1 · the same file, byte for byte, on every Render");
        self.push("");
        self.push("use std::cell::RefCell;");
        self.push("use std::rc::Rc;");
        self.push("");
        self.push("use vieww::prelude::*;");
        if self.needs_text {
            self.push("use vieww::foundation::TextEditingValue;");
        }
        self.push("");
    }

    fn type_alias(&mut self) {
        self.push("/// The handle a handler reaches its screen's state through.");
        self.push("type Handle = Option<Rc<RefCell<dyn ElementState>>>;");
        self.push("");
    }

    // -- state ---------------------------------------------------------------

    fn field_ty(&self, state: &StateDecl) -> &'static str {
        match state.kind {
            StateKind::Whole => "i64",
            StateKind::Number => "f32",
            StateKind::Flag => "bool",
            StateKind::Text => "TextEditingValue",
            StateKind::ListText => "Vec<String>",
            StateKind::ListNumber => "Vec<f32>",
        }
    }

    fn default_expr(&self, state: &StateDecl) -> String {
        match &state.init {
            Init::Whole(v) => format!("{v}"),
            Init::Number(v) => rust_f32(*v),
            Init::Flag(v) => format!("{v}"),
            Init::Text(text) => format!("TextEditingValue::new({})", rust_str(text)),
            Init::ListText(items) => format!(
                "vec![{}]",
                items
                    .iter()
                    .map(|i| rust_str(i))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Init::ListNumber(items) => format!(
                "vec![{}]",
                items
                    .iter()
                    .map(|i| rust_f32(*i))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn state_struct(&mut self) {
        self.push("// -- state ------------------------------------------------------------------");
        self.push("");
        self.push("/// Everything the screens keep, one field per `keep` line, plus the");
        self.push("/// navigation stack. This is the state that survives a Render: it lives on");
        self.push("/// the screen's own element, the studio carries it across every");
        self.push("/// recompile, and a handler writes it through the element's handle.");
        self.push("#[derive(Debug)]");
        self.push("struct SayState {");
        for state in self.states {
            self.push(format!(
                "    // say: {}:{}  keep {}",
                self.file, state.line, state.name
            ));
            self.push(format!(
                "    {},",
                declare(&state.name, self.field_ty(state))
            ));
        }
        if self.multi_screen {
            self.push("    stack: Vec<SayScreens>,");
        }
        if self.uses_dialog {
            self.push("    modal: Option<SayModal>,");
        }
        if self.uses_toast {
            self.push("    toast: Option<String>,");
            self.push("    toast_start: Option<std::time::Duration>,");
        }
        self.push("    dirty: bool,");
        self.push("}");
        self.push("");
        self.push("#[allow(clippy::derivable_impls)] // the defaults come from the keep lines");
        self.push("impl Default for SayState {");
        self.push("    fn default() -> Self {");
        self.push("        Self {");
        for state in self.states {
            self.push(format!(
                "            {},",
                declare(&state.name, &self.default_expr(state))
            ));
        }
        if self.multi_screen {
            self.push("            stack: Vec::new(),");
        }
        if self.uses_dialog {
            self.push("            modal: None,");
        }
        if self.uses_toast {
            self.push("            toast: None,");
            self.push("            toast_start: None,");
        }
        self.push("            dirty: false,");
        self.push("        }");
        self.push("    }");
        self.push("}");
        self.push("");
        // **Generated code must not warn.**
        //
        // What is emitted here is a small fixed runtime, and which parts of it
        // a given screen uses depends on what the screen says. `counter.say`
        // writes its field directly and never calls `mark`, so a project
        // scaffolded from it built with `warning: method `mark` is never used`,
        // pointing into a file under `target/` that the author did not write
        // and cannot edit.
        //
        // A warning nobody can act on is worse than none: it teaches the person
        // that this project's build is expected to be noisy, and the next
        // warning — one that *is* theirs — arrives into a build they have
        // stopped reading.
        //
        // On the impl rather than the file, and for the same reason `mod say`
        // below carries one: it covers the generated scaffold and leaves every
        // other lint in the file live.
        self.push("#[allow(dead_code)] // helpers a screen may not call, kept for uniformity");
        self.push("impl SayState {");
        self.push("    /// A handler finished writing; ask the tree for one rebuild. The tree");
        self.push("    /// takes this flag once a frame — see `take_pending` below.");
        self.push("    fn mark(&mut self) {");
        self.push("        self.dirty = true;");
        self.push("    }");
        if self.multi_screen {
            self.push("");
            self.push("    /// The screen on top of the stack; the home screen is the floor.");
            self.push("    fn current(&self) -> SayScreens {");
            self.push("        *self.stack.last().unwrap_or(&SayScreens::Home)");
            self.push("    }");
            self.push("");
            self.push("    /// `open the screen \"X\"`");
            self.push("    fn open_screen(&mut self, screen: SayScreens) {");
            self.push("        self.stack.push(screen);");
            self.push("        self.mark();");
            self.push("    }");
            self.push("");
            self.push("    /// `go back` — closes a dialog first; the home screen is the floor.");
            self.push("    fn go_back(&mut self) {");
            self.push("        if self.modal.is_some() {");
            self.push("            self.modal = None;");
            self.push("        } else if !self.stack.is_empty() {");
            self.push("            self.stack.pop();");
            self.push("        }");
            self.push("        self.mark();");
            self.push("    }");
        }
        if self.uses_dialog {
            self.push("");
            self.push("    /// `close this dialog`");
            self.push("    fn close_dialog(&mut self) {");
            self.push("        self.modal = None;");
            self.push("        self.mark();");
            self.push("    }");
        }
        if self.uses_toast {
            self.push("");
            self.push("    /// `show a snackbar \"M\"` — a note that dismisses itself.");
            self.push("    fn show_toast(&mut self, message: String) {");
            self.push("        self.toast = Some(message);");
            self.push("        self.toast_start = None;");
            self.push("        self.mark();");
            self.push("    }");
        }
        self.push("}");
        self.push("");
        self.state_trait_impl();
        if self.multi_screen {
            self.screens_enum();
        }
        if self.uses_dialog {
            self.modal_struct();
        }
    }

    fn state_trait_impl(&mut self) {
        let kept: Vec<&StateDecl> = self.states.iter().collect();
        let has_snapshot = !kept.is_empty() || self.multi_screen;

        self.push("impl ElementState for SayState {");
        self.push("    fn as_any(&self) -> &dyn std::any::Any {");
        self.push("        self");
        self.push("    }");
        self.push("");
        self.push("    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {");
        self.push("        self");
        self.push("    }");
        self.push("");
        self.push("    fn take_pending(&mut self) -> bool {");
        self.push("        std::mem::take(&mut self.dirty)");
        self.push("    }");

        if has_snapshot {
            self.push("");
            self.push("    /// The state as a string, so it can survive the recompile a Render");
            self.push("    /// performs. Fields are joined with U+001E, list items with U+001F.");
            self.push("    fn snapshot(&self) -> Option<String> {");
            self.push("        Some(format!(");
            let mut fmt = String::new();
            let mut args: Vec<String> = Vec::new();
            for state in &kept {
                if !fmt.is_empty() {
                    fmt.push_str("\\u{1e}");
                }
                fmt.push_str(&format!("{}={{}}", state.name));
                args.push(match state.kind {
                    StateKind::Whole | StateKind::Number | StateKind::Flag => {
                        format!("self.{}", state.name)
                    }
                    StateKind::Text => format!("say::esc(&self.{}.text)", state.name),
                    StateKind::ListText | StateKind::ListNumber => {
                        format!("say::join(&self.{})", state.name)
                    }
                });
            }
            if self.multi_screen {
                if !fmt.is_empty() {
                    fmt.push_str("\\u{1e}");
                }
                fmt.push_str("stack={}");
                args.push(
                    "self.stack.iter().map(|s| s.tag()).collect::<Vec<_>>().join(\"|\")".to_owned(),
                );
            }
            self.push("            \"".to_owned() + &fmt + "\",");
            for arg in args {
                self.push(format!("                {arg},"));
            }
            self.push("            )");
            self.push("        )");
            self.push("    }");
            self.push("");
            self.push("    /// Put back what `snapshot` saved. The input is untrusted — it may be");
            self.push("    /// from a version of this file that kept different things — so this");
            self.push("    /// reads into locals and applies only when all of it made sense.");
            self.push("    fn restore(&mut self, saved: &str) -> bool {");
            for state in &kept {
                let init = match state.kind {
                    StateKind::Whole | StateKind::Number | StateKind::Flag => {
                        format!("        let mut {n} = self.{n};", n = state.name)
                    }
                    _ => format!("        let mut {n} = self.{n}.clone();", n = state.name),
                };
                self.push(init);
            }
            if self.multi_screen {
                self.push("        let mut stack = self.stack.clone();");
            }
            self.push("        for field in saved.split('\\u{1e}') {");
            self.push("            let Some((key, value)) = field.split_once('=') else {");
            self.push("                return false;");
            self.push("            };");
            self.push("            match key {");
            for state in &kept {
                let n = &state.name;
                match state.kind {
                    StateKind::Whole | StateKind::Number => {
                        self.push(format!(
                            "                \"{}\" => match value.parse() {{ Ok(v) => {n} = v, Err(_) => return false }},",
                            n
                        ));
                    }
                    StateKind::Flag => {
                        self.push(format!(
                            "                \"{n}\" => match value {{ \"true\" => {n} = true, \"false\" => {n} = false, _ => return false }},"
                        ));
                    }
                    StateKind::Text => {
                        self.push(format!(
                            "                \"{n}\" => {n} = TextEditingValue::new(value.to_owned()),"
                        ));
                    }
                    StateKind::ListText => {
                        self.push(format!(
                            "                \"{n}\" => {n} = say::split_list(value).collect(),"
                        ));
                    }
                    StateKind::ListNumber => {
                        self.push(format!("                \"{n}\" => {{"));
                        self.push("                    let mut list = Vec::new();");
                        self.push("                    for item in say::split_list(value) {");
                        self.push(
                            "                        match item.parse() { Ok(v) => list.push(v), Err(_) => return false }",
                        );
                        self.push("                    }");
                        self.push(format!("                    {n} = list;"));
                        self.push("                }");
                    }
                }
            }
            if self.multi_screen {
                self.push("                \"stack\" => {");
                self.push("                    stack = Vec::new();");
                self.push("                    for tag in value.split('|') {");
                self.push("                        match SayScreens::from_tag(tag) {");
                self.push("                            Some(screen) => stack.push(screen),");
                self.push("                            None => return false,");
                self.push("                        }");
                self.push("                    }");
                self.push("                }");
            }
            self.push("                _ => return false,");
            self.push("            }");
            self.push("        }");
            for state in &kept {
                self.push(format!("        self.{n} = {n};", n = state.name));
            }
            if self.multi_screen {
                self.push("        self.stack = stack;");
            }
            self.push("        true");
            self.push("    }");
        }

        if self.uses_toast {
            self.push("");
            self.push("    /// A snackbar asks for frames until it has been up long enough. The");
            self.push("    /// handler that showed it cannot know the time — the tree supplies");
            self.push("    /// it here, in the frame's animate phase.");
            self.push("    fn tick(&mut self, now: std::time::Duration) -> bool {");
            self.push("        match (self.toast.as_ref(), self.toast_start) {");
            self.push("            (Some(_), Some(start)) => {");
            self.push("                if now - start >= TOAST_FOR {");
            self.push("                    self.toast = None;");
            self.push("                    self.toast_start = None;");
            self.push("                    self.dirty = true;");
            self.push("                }");
            self.push("                true");
            self.push("            }");
            self.push("            (Some(_), None) => {");
            self.push("                self.toast_start = Some(now);");
            self.push("                true");
            self.push("            }");
            self.push("            (None, _) => {");
            self.push("                self.toast_start = None;");
            self.push("                false");
            self.push("            }");
            self.push("        }");
            self.push("    }");
            self.push("");
            self.push("    fn is_animating(&self) -> bool {");
            self.push("        self.toast.is_some()");
            self.push("    }");
        }
        self.push("}");
        if self.uses_toast {
            self.push("");
            self.push("/// How long a snackbar stays up.");
            self.push("const TOAST_FOR: std::time::Duration = std::time::Duration::from_secs(3);");
        }
    }

    fn screens_enum(&mut self) {
        self.push("");
        self.push("/// The screens of this file, in the order they are declared.");
        self.push("#[derive(Debug, Clone, Copy, PartialEq, Eq)]");
        self.push("enum SayScreens {");
        for screen in self.screens {
            self.push(format!("    {},", snake(&screen.title)));
        }
        self.push("}");
        self.push("");
        self.push("impl SayScreens {");
        self.push("    fn tag(self) -> &'static str {");
        self.push("        match self {");
        for screen in self.screens {
            let name = snake(&screen.title);
            self.push(format!("            Self::{name} => \"{name}\","));
        }
        self.push("        }");
        self.push("    }");
        self.push("");
        self.push("    /// An unreadable tag means the snapshot is not for this file any");
        self.push("    /// more — `restore` refuses rather than landing somewhere arbitrary.");
        self.push("    fn from_tag(tag: &str) -> Option<Self> {");
        self.push("        match tag {");
        for screen in self.screens {
            let name = snake(&screen.title);
            self.push(format!("            \"{name}\" => Some(Self::{name}),"));
        }
        self.push("            _ => None,");
        self.push("        }");
        self.push("    }");
        self.push("}");
    }

    fn modal_struct(&mut self) {
        self.push("");
        self.push("/// The value a dialog edits while it is open. Transient by design:");
        self.push("/// closing a dialog throws the draft away, which is what Cancel means");
        self.push("/// and what Save has already copied into state.");
        self.push("#[derive(Debug)]");
        self.push("struct SayModal {");
        self.push("    title: String,");
        for field in self.modal_fields {
            self.push(format!("    {}: TextEditingValue,", field));
        }
        self.push("}");
        self.push("");
        self.push("impl SayModal {");
        self.push("    fn new(title: &str) -> Self {");
        self.push("        Self {");
        self.push("            title: title.to_owned(),");
        for field in self.modal_fields {
            self.push(format!(
                "            {}: TextEditingValue::new(String::new()),",
                field
            ));
        }
        self.push("        }");
        self.push("    }");
        self.push("}");
    }

    // -- the say helper module ----------------------------------------------

    fn say_mod(&mut self) {
        self.push("");
        self.push(
            "// -- the say helpers ---------------------------------------------------------",
        );
        self.push("");
        self.push("/// The two moves that make Say work inside a previewed screen. A handler");
        self.push("/// cannot reach a runtime — there is none across the preview boundary —");
        self.push("/// so it reaches the screen's own state instead, mutates it, and raises");
        self.push("/// the flag the element tree polls once a frame.");
        self.push("#[allow(dead_code)] // helpers a small file may not call, kept for uniformity");
        self.push("mod say {");
        self.push("    use super::SayState;");
        self.push("    use std::cell::RefCell;");
        self.push("    use std::rc::Rc;");
        self.push("    use vieww::prelude::*;");
        self.push("");
        self.push("    type Handle = Option<Rc<RefCell<dyn ElementState>>>;");
        self.push("");
        self.push("    /// Run `f` against the screen's state from a tap handler.");
        self.push("    pub(crate) fn act<F: Fn(&mut SayState) + 'static>(handle: &Handle, f: F) -> impl Fn() + 'static {");
        self.push("        let handle = handle.clone();");
        self.push("        move || {");
        self.push("            let Some(state) = handle.as_ref() else { return };");
        self.push("            let mut borrowed = state.borrow_mut();");
        self.push(
            "            if let Some(s) = borrowed.as_any_mut().downcast_mut::<SayState>() {",
        );
        self.push("                f(s);");
        self.push("                s.dirty = true;");
        self.push("            }");
        self.push("        }");
        self.push("    }");
        self.push("");
        self.push("    /// The same, for handlers that receive a value: a field's new text, a");
        self.push("    /// switch's new position.");
        self.push("    pub(crate) fn on<T: 'static, F: Fn(&mut SayState, T) + 'static>(");
        self.push("        handle: &Handle,");
        self.push("        f: F,");
        self.push("    ) -> impl Fn(T) + 'static {");
        self.push("        let handle = handle.clone();");
        self.push("        move |value| {");
        self.push("            let Some(state) = handle.as_ref() else { return };");
        self.push("            let mut borrowed = state.borrow_mut();");
        self.push(
            "            if let Some(s) = borrowed.as_any_mut().downcast_mut::<SayState>() {",
        );
        self.push("                f(s, value);");
        self.push("                s.dirty = true;");
        self.push("            }");
        self.push("        }");
        self.push("    }");
        self.push("");
        self.push("    /// Escape one value for the snapshot string.");
        self.push("    pub(crate) fn esc(text: &str) -> String {");
        self.push("        text.replace(['\\u{1e}', '\\u{1f}', '|'], \" \")");
        self.push("    }");
        self.push("");
        self.push("    /// Join list values for the snapshot string.");
        self.push("    pub(crate) fn join(items: &[String]) -> String {");
        self.push("        items.iter().map(|i| esc(i)).collect::<Vec<_>>().join(\"\\u{1f}\")");
        self.push("    }");
        self.push("");
        self.push("    /// Split list values back out.");
        self.push(
            "    pub(crate) fn split_list(saved: &str) -> impl Iterator<Item = String> + '_ {",
        );
        self.push("        saved.split('\\u{1f}').map(str::to_owned)");
        self.push("    }");
        self.push("}");
        self.push("");
    }

    // -- the app widget -------------------------------------------------------

    fn app_widget(&mut self) {
        self.push(
            "// -- the app -----------------------------------------------------------------",
        );
        self.push("");
        self.push("/// The root widget: it owns the state (see `create_state`), builds the");
        self.push("/// screen on top of the stack, and lays the dialog and the snackbar over");
        self.push("/// it when they are up.");
        self.push("#[derive(Debug)]");
        self.push("struct SayApp;");
        self.push("");
        self.push("impl Widget for SayApp {");
        self.push("    fn debug_name(&self) -> &'static str {");
        self.push("        \"SayApp\"");
        self.push("    }");
        self.push("");
        self.push("    fn kind(&self) -> WidgetKind<'_> {");
        self.push("        WidgetKind::Composed");
        self.push("    }");
        self.push("");
        self.push("    fn create_state(&self) -> Option<Box<dyn ElementState>> {");
        self.push("        Some(Box::new(SayState::default()))");
        self.push("    }");
        self.push("");
        self.push("    fn build(&self, ctx: &BuildContext) -> WidgetNode {");
        self.push("        let handle = ctx.state_handle();");
        if self.uses_toast {
            self.push("        let theme = ThemeData::of(ctx);");
        }
        if self.multi_screen {
            self.push("        let current = ctx.state::<SayState, _>(|s| s.current()).unwrap_or(SayScreens::Home);");
            self.push("        let page: WidgetNode = match current {");
            for screen in self.screens {
                let name = snake(&screen.title);
                self.push(format!(
                    "            SayScreens::{name} => screen_{name}(ctx, &handle),"
                ));
            }
            self.push("        };");
        } else {
            let name = snake(
                &self
                    .screens
                    .first()
                    .expect("the parser guarantees one screen")
                    .title,
            );
            self.push(format!("        let page = screen_{name}(ctx, &handle);"));
        }
        self.push(
            "        let mut layers: Vec<WidgetNode> = vec![SafeArea::new().child(page).into()];",
        );
        if self.uses_dialog {
            self.push("        if let Some(m) = ctx.state::<SayState, _>(|s| s.modal.clone()) {");
            self.push("            layers.push(vieww::widget::ModalBarrier::new().into());");
            self.push(
                "            layers.push(Center::new().child(dialog(ctx, &handle, &m)).into());",
            );
            self.push("        }");
        }
        if self.uses_toast {
            self.push(
                "        if let Some(message) = ctx.state::<SayState, _>(|s| s.toast.clone()) {",
            );
            self.push("            layers.push(");
            self.push("                Positioned::new()");
            self.push("                    .left(24.0)");
            self.push("                    .right(24.0)");
            self.push("                    .bottom(24.0)");
            self.push("                    .child(");
            self.push("                        Container::new()");
            self.push("                            .color(theme.colors.on_surface)");
            self.push("                            .radius(8.0)");
            self.push("                            .padding(EdgeInsets::symmetric(16.0, 12.0))");
            self.push("                            .child(");
            self.push("                                Text::new(message).size(13.0).color(theme.colors.surface),");
            self.push("                            ),");
            self.push("                    )");
            self.push("                    .into(),");
            self.push("            );");
            self.push("        }");
        }
        self.push("        if layers.len() == 1 {");
        self.push("            layers.remove(0)");
        self.push("        } else {");
        self.push("            Stack::new().children(layers).into()");
        self.push("        }");
        self.push("    }");
        self.push("}");
        self.push("");
        self.push("/// The entry point the preview looks for.");
        self.push(
            "#[allow(unreachable_pub)] // pub is the contract when the file compiles standalone",
        );
        self.push("pub fn screen() -> impl Widget {");
        self.push("    SayApp");
        self.push("}");
        self.push("");
    }

    // -- one function per screen ----------------------------------------------

    fn screen_fn(&mut self, screen: &Screen) {
        let name = snake(&screen.title);
        let nodes: Vec<NodeRef<'_>> = screen.body.iter().map(NodeRef::of).collect();
        let reads = collect_reads(&nodes, &[]);
        let has_handlers = subtree_has_handlers(&screen.body);

        self.push(format!("// -- the {name} screen ---------------------------------------------------------------"));
        self.push("");
        self.push(format!(
            "fn screen_{name}(ctx: &BuildContext, handle: &Handle) -> WidgetNode {{"
        ));
        self.push(format!(
            "    // say: {}:{}  screen \"{}\"",
            self.file, screen.line, screen.title
        ));
        if let Some(transition) = screen.transition {
            self.push(format!(
                "    // the {} transition is accepted and reserved for Say 1.1",
                match transition {
                    crate::parse::Transition::Fade => "fade",
                    crate::parse::Transition::Slide => "slide",
                }
            ));
        }
        for read in &reads {
            self.emit_read(read, None);
        }
        if uses_theme(&screen.body) {
            self.push("    let theme = ThemeData::of(ctx);");
        }
        if !has_handlers {
            self.push("    let _ = handle; // this screen has no events");
        }
        self.push("    let mut children: Vec<WidgetNode> = Vec::new();");
        let mut scope = Scope::default();
        for node in &screen.body {
            self.emit_node(node, &mut scope, "children", 1);
        }
        self.push("    // One root line returns itself; more wrap in a column, which is what");
        self.push("    // a stack of root lines almost always means.");
        self.push("    if children.len() == 1 {");
        self.push("        children.remove(0)");
        self.push("    } else {");
        self.push("        Flex::column()");
        self.push("            .cross_axis_alignment(CrossAxisAlignment::Stretch)");
        self.push("            .children(children)");
        self.push("            .into()");
        self.push("    }");
        self.push("}");
        self.push("");
    }

    // -- dialogs ---------------------------------------------------------------

    fn dialog_list(&self) -> Vec<DialogFn> {
        let mut list: Vec<DialogFn> = Vec::new();
        for screen in self.screens {
            collect_dialogs(&screen.body, &mut list);
        }
        let mut seen: Vec<String> = Vec::new();
        for dialog in list.iter_mut() {
            let taken = seen.iter().filter(|slug| **slug == dialog.slug).count();
            if taken > 0 {
                dialog.slug = format!("{}_{}", dialog.slug, taken + 1);
            }
            seen.push(dialog.slug.clone());
        }
        list
    }

    fn dialog_dispatch(&mut self) {
        let list = self.dialog_list();
        self.push(
            "// -- dialogs -----------------------------------------------------------------",
        );
        self.push("");
        self.push("/// The dialog on top, by title. A dialog is state, not a route: it is");
        self.push("/// laid over the screen by `SayApp::build` and closed by writing None.");
        self.push("fn dialog(ctx: &BuildContext, handle: &Handle, m: &SayModal) -> WidgetNode {");
        self.push("    match m.title.as_str() {");
        for dialog in &list {
            self.push(format!(
                "        {} => {}(ctx, handle, m),",
                rust_str(&dialog.title),
                dialog.slug
            ));
        }
        self.push("        _ => SizedBox::shrink().into(),");
        self.push("    }");
        self.push("}");
        self.push("");
    }

    fn dialog_fn(&mut self, dialog: &DialogFn, screen: &Screen) {
        let body = find_dialog_body(&screen.body, &dialog.title)
            .expect("the dialog body was found when it was listed");
        let nodes: Vec<NodeRef<'_>> = body.iter().map(NodeRef::of).collect();
        let reads = collect_reads(&nodes, self.modal_fields);
        let has_handlers = subtree_has_handlers(body);

        self.push(format!(
            "/// The \"{}\" dialog. Its bound fields live on `SayModal` for as long",
            dialog.title
        ));
        self.push("/// as the dialog is open; a Save action copies them into state and a");
        self.push("/// Cancel closes without reading them.");
        self.push(format!(
            "fn {}(ctx: &BuildContext, handle: &Handle, m: &SayModal) -> WidgetNode {{",
            dialog.slug
        ));
        self.push(format!(
            "    // say: {}:{}  open a dialog titled \"{}\"",
            self.file, dialog.line, dialog.title
        ));
        for read in &reads {
            self.emit_read(read, None);
        }
        for field in self.modal_fields {
            if body_uses_field(body, field) {
                self.push(format!("    let {field} = m.{field}.clone();"));
            }
        }
        if uses_theme(body) {
            self.push("    let theme = ThemeData::of(ctx);");
        }
        if !has_handlers {
            self.push("    let _ = handle;");
        }
        self.push("    let mut children: Vec<WidgetNode> = Vec::new();");
        let mut scope = Scope {
            is_dialog: true,
            ..Scope::default()
        };
        for node in body {
            self.emit_node(node, &mut scope, "children", 1);
        }
        self.push("    let content: WidgetNode = if children.len() == 1 {");
        self.push("        children.remove(0)");
        self.push("    } else {");
        self.push("        Flex::column()");
        self.push("            .main_axis_size(MainAxisSize::Min)");
        self.push("            .cross_axis_alignment(CrossAxisAlignment::Start)");
        self.push("            .spacing(12.0)");
        self.push("            .children(children)");
        self.push("            .into()");
        self.push("    };");
        self.push("    vieww::widget::Dialog::new()");
        self.push("        .title(m.title.clone())");
        self.push("        .content(content)");
        self.push("        .actions(Vec::new())");
        self.push("        .into()");
        self.push("}");
        self.push("");
    }

    // -- reads ------------------------------------------------------------------

    fn emit_read(&mut self, read: &Read, _scope: Option<&Scope>) {
        let Some(state) = self.state(&read.name) else {
            // A loop variable or dialog field — the caller holds it already.
            return;
        };
        let n = &state.name;
        match state.kind {
            StateKind::Whole | StateKind::Number | StateKind::Flag => {
                let default = self.default_expr(state);
                self.push(format!(
                    "    let {n} = ctx.state::<SayState, _>(|s| s.{n}).unwrap_or({default});"
                ));
            }
            _ => {
                let default = self.default_expr(state);
                self.push(format!(
                    "    let {n} = ctx.state::<SayState, _>(|s| s.{n}.clone()).unwrap_or({default});"
                ));
            }
        }
    }
}

/// One node, borrowed, so collection can walk screens and dialog bodies the
/// same way.
#[derive(Debug, Clone, Copy)]
enum NodeRef<'a> {
    Widget(&'a Widget),
    ForEach { list: &'a str, body: &'a [Node] },
}

impl<'a> NodeRef<'a> {
    fn of(node: &'a Node) -> Self {
        match node {
            Node::Widget(w) => NodeRef::Widget(w),
            Node::ForEach { list, body, .. } => NodeRef::ForEach { list, body },
        }
    }
}

/// Names in scope while emitting: the loop variables open, whether the
/// enclosing builder is a list row (where `item` exists), and whether the
/// function being emitted is a dialog (whose bound fields live on `SayModal`).
#[derive(Debug, Default, Clone)]
struct Scope {
    loop_vars: Vec<(String, StateKind)>,
    item: Option<StateKind>,
    is_dialog: bool,
}

// -- collection ---------------------------------------------------------------

/// The state a function's *build* reads: interpolation, conditions, list
/// sources, bound controls. Handlers are excluded — they reach the state at
/// tap time through the handle.
fn collect_reads(nodes: &[NodeRef<'_>], exclude: &[String]) -> Vec<Read> {
    let mut reads = Vec::new();
    for node in nodes {
        collect_node(node, &mut reads);
    }
    reads.retain(|read| !exclude.contains(&read.name));
    reads.dedup_by(|a, b| a.name == b.name);
    reads
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Read {
    name: String,
}

fn collect_node(node: &NodeRef<'_>, reads: &mut Vec<Read>) {
    match node {
        NodeRef::Widget(widget) => collect_widget(widget, reads),
        NodeRef::ForEach { list, body, .. } => {
            reads.push(Read {
                name: (*list).to_owned(),
            });
            for inner in *body {
                collect_node(&NodeRef::of(inner), reads);
            }
        }
    }
}

fn collect_widget(widget: &Widget, reads: &mut Vec<Read>) {
    if let Some(cond) = &widget.only_if {
        collect_cond(cond, reads);
    }
    match &widget.kind {
        WKind::Text { value, .. } | WKind::Button { label: value } => collect_value(value, reads),
        WKind::EmptyState { title, .. } => collect_value(title, reads),
        WKind::ListView { list, .. } => reads.push(Read { name: list.clone() }),
        WKind::TextField { bound }
        | WKind::Switch { bound }
        | WKind::Checkbox { bound }
        | WKind::Slider { bound, .. } => {
            reads.push(Read {
                name: bound.clone(),
            });
        }
        _ => {}
    }
    for child in &widget.children {
        collect_node(&NodeRef::of(child), reads);
    }
}

fn collect_value(value: &Value, reads: &mut Vec<Read>) {
    if let Value::Parts(parts) = value {
        for part in parts {
            if let Part::Expr(expr) = part {
                collect_expr(expr, reads);
            }
        }
    }
}

fn collect_expr(expr: &Expr, reads: &mut Vec<Read>) {
    match expr {
        Expr::Ref { name, .. } => reads.push(Read { name: name.clone() }),
        Expr::Len { list, .. } => reads.push(Read { name: list.clone() }),
        Expr::Bin { lhs, rhs, .. } => {
            collect_expr(lhs, reads);
            collect_expr(rhs, reads);
        }
        _ => {}
    }
}

fn collect_cond(cond: &Cond, reads: &mut Vec<Read>) {
    match cond {
        Cond::Cmp { lhs, rhs, .. } => {
            collect_expr(lhs, reads);
            collect_expr(rhs, reads);
        }
        Cond::Flag { name, .. } => reads.push(Read { name: name.clone() }),
        Cond::Contains { of, .. } => reads.push(Read { name: of.clone() }),
        Cond::And(a, b) | Cond::Or(a, b) => {
            collect_cond(a, reads);
            collect_cond(b, reads);
        }
        Cond::Not(a) => collect_cond(a, reads),
    }
}

// -- pre-passes -----------------------------------------------------------------

fn program_has_toast(program: &Program) -> bool {
    let mut found = false;
    for screen in &program.screens {
        subtree_has_toast(&screen.body, &mut found);
    }
    found
}

fn subtree_has_toast(nodes: &[Node], found: &mut bool) {
    for node in nodes {
        match node {
            Node::Widget(widget) => {
                if let Some(event) = &widget.event {
                    for action in &event.actions {
                        if matches!(action.kind, AKind::Snackbar { .. }) {
                            *found = true;
                        }
                        if let AKind::OpenDialog { body, .. } = &action.kind {
                            subtree_has_toast(body, found);
                        }
                    }
                }
                subtree_has_toast(&widget.children, found);
            }
            Node::ForEach { body, .. } => subtree_has_toast(body, found),
        }
    }
}

fn find_dialog(nodes: &[Node], title: &str) -> bool {
    find_dialog_body(nodes, title).is_some()
}

fn find_dialog_body<'a>(nodes: &'a [Node], title: &str) -> Option<&'a [Node]> {
    for node in nodes {
        match node {
            Node::Widget(widget) => {
                if let Some(event) = &widget.event {
                    for action in &event.actions {
                        if let AKind::OpenDialog { title: found, body } = &action.kind {
                            if found == title {
                                return Some(body);
                            }
                            if let Some(deeper) = find_dialog_body(body, title) {
                                return Some(deeper);
                            }
                        }
                    }
                }
                if let Some(deeper) = find_dialog_body(&widget.children, title) {
                    return Some(deeper);
                }
            }
            Node::ForEach { body, .. } => {
                if let Some(deeper) = find_dialog_body(body, title) {
                    return Some(deeper);
                }
            }
        }
    }
    None
}

fn collect_dialogs(nodes: &[Node], into: &mut Vec<DialogFn>) {
    for node in nodes {
        match node {
            Node::Widget(widget) => {
                if let Some(event) = &widget.event {
                    for action in &event.actions {
                        if let AKind::OpenDialog { title, body } = &action.kind {
                            into.push(DialogFn {
                                title: title.clone(),
                                slug: format!("dialog_{}", snake(title)),
                                line: action.line,
                            });
                            collect_dialogs(body, into);
                        }
                    }
                }
                collect_dialogs(&widget.children, into);
            }
            Node::ForEach { body, .. } => collect_dialogs(body, into),
        }
    }
}

fn subtree_has_handlers(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::Widget(widget) => {
            widget.event.is_some()
                || matches!(widget.kind, WKind::TextField { .. })
                || matches!(widget.kind, WKind::Switch { .. })
                || matches!(widget.kind, WKind::Checkbox { .. })
                || matches!(widget.kind, WKind::Slider { .. })
                || subtree_has_handlers(&widget.children)
        }
        Node::ForEach { body, .. } => subtree_has_handlers(body),
    })
}

fn body_uses_field(nodes: &[Node], field: &str) -> bool {
    nodes.iter().any(|node| match node {
        Node::Widget(widget) => {
            let binds = match &widget.kind {
                WKind::TextField { bound }
                | WKind::Switch { bound }
                | WKind::Checkbox { bound }
                | WKind::Slider { bound, .. } => bound == field,
                _ => false,
            };
            binds || body_uses_field(&widget.children, field)
        }
        Node::ForEach { body, .. } => body_uses_field(body, field),
    })
}

fn uses_theme(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::Widget(widget) => {
            let themed = match &widget.kind {
                WKind::Text { .. } => true,
                _ => widget
                    .props
                    .iter()
                    .any(|p| matches!(p, Prop::Color(ColorSpec::Theme(_)))),
            };
            themed || uses_theme(&widget.children)
        }
        Node::ForEach { body, .. } => uses_theme(body),
    })
}

// -- names and literals ---------------------------------------------------------

/// `Home` → `home`; anything a Rust identifier cannot carry becomes `_`.
fn snake(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_owned();
    if trimmed.is_empty() {
        "screen".to_owned()
    } else if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("s{trimmed}")
    } else {
        trimmed
    }
}

fn declare(name: &str, ty: &str) -> String {
    format!("{name}: {ty}")
}

/// A Rust string literal, escaped.
fn rust_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A Rust f32 literal — always with a decimal point so it stays an f32.
fn rust_f32(value: f32) -> String {
    if value.fract() == 0.0 && value.is_finite() && value.abs() < 1e19 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

// ---------------------------------------------------------------------------
// Emitting widgets

impl<'a> Gen<'a> {
    /// Emit one node: pushes into `target`, a `Vec<WidgetNode>` variable.
    fn emit_node(&mut self, node: &Node, scope: &mut Scope, target: &str, depth: usize) {
        match node {
            Node::Widget(widget) => self.emit_widget(widget, scope, target, depth),
            Node::ForEach {
                var,
                list,
                body,
                line,
            } => {
                let rows = self.unique("rows");
                self.indent(
                    depth,
                    format!("// say: {}:{}  for each {var} in {list}", self.file, line),
                );
                self.indent(depth, "{");
                self.indent(
                    depth + 1,
                    format!(
                        "let {rows}: Vec<WidgetNode> = {list}.iter().enumerate().flat_map(|(_i, {var})| {{"
                    ),
                );
                let mut inner = scope.clone();
                let elem = self
                    .state(list)
                    .map(|s| match s.kind {
                        StateKind::ListNumber => StateKind::Number,
                        _ => StateKind::Text,
                    })
                    .unwrap_or(StateKind::Text);
                inner.loop_vars.push(((*var).to_owned(), elem));
                self.indent(depth + 2, "let mut children: Vec<WidgetNode> = Vec::new();");
                for child in body {
                    self.emit_node(child, &mut inner, "children", depth + 2);
                }
                self.indent(depth + 2, "children");
                self.indent(depth + 1, "}).collect();");
                self.indent(depth + 1, format!("{target}.extend({rows});"));
                self.indent(depth, "}");
            }
        }
    }

    fn emit_widget(&mut self, widget: &Widget, scope: &mut Scope, target: &str, depth: usize) {
        let expr = self.widget_expr(widget, scope, depth);
        self.indent(
            depth,
            format!("// say: {}:{}  {}", self.file, widget.line, widget.phrase),
        );
        if let Some(cond) = &widget.only_if {
            let guard = self.cond_rust(cond, scope);
            self.indent(depth, format!("if {guard} {{"));
            self.indent(depth + 1, format!("{target}.push({expr}.into());"));
            self.indent(depth, "}");
        } else {
            self.indent(depth, format!("{target}.push({expr}.into());"));
        }
    }

    /// The Rust expression for one widget line, possibly spanning lines.
    fn widget_expr(&mut self, widget: &Widget, scope: &mut Scope, depth: usize) -> String {
        let mut used: Vec<usize> = Vec::new();
        let mut expr = match &widget.kind {
            WKind::Card => self.container_expr(widget, scope, depth, &mut used),
            WKind::Column => self.flex_expr(widget, scope, depth, true, &mut used),
            WKind::Row => self.flex_expr(widget, scope, depth, false, &mut used),
            WKind::Stack => {
                used.push(0);
                let children = self.children_vec(widget, scope, depth + 1, false);
                format!("Stack::new().children({children})")
            }
            WKind::Center => {
                used.push(0);
                let inner = self.children_vec(widget, scope, depth + 1, false);
                format!("Center::new().child({inner})")
            }
            WKind::Fixed { width, height } => {
                used.push(0);
                let mut out = String::from("SizedBox");
                if let Some(w) = width {
                    out.push_str(&format!(".width({})", rust_f32(*w)));
                }
                if let Some(h) = height {
                    out.push_str(&format!(".height({})", rust_f32(*h)));
                }
                let inner = self.children_vec(widget, scope, depth + 1, false);
                format!("{out}.child({inner})")
            }
            WKind::Text { value, heading } => {
                used.push(0);
                let mut out = format!("Text::new({})", self.value_rust(value, scope));
                out.push_str(if *heading {
                    ".style(theme.text.headline)"
                } else {
                    ".style(theme.text.body)"
                });
                for (index, prop) in widget.props.iter().enumerate() {
                    match prop {
                        Prop::Size(n) => {
                            used.push(index);
                            out.push_str(&format!(".size({})", rust_f32(*n)));
                        }
                        Prop::Bold => {
                            used.push(index);
                            out.push_str(".bold()");
                        }
                        Prop::Color(spec) => {
                            used.push(index);
                            out.push_str(&format!(".color({})", self.color_rust(spec)));
                        }
                        _ => {}
                    }
                }
                out
            }
            WKind::Button { label } => {
                used.push(0);
                let mut out = format!("Button::new({})", self.value_rust(label, scope));
                if let Some(event) = &widget.event {
                    let body = self.action_stmts(&event.actions, scope, 3);
                    out.push_str(&format!(
                        ".on_pressed(say::act(handle, |s: &mut SayState| {{\n{body}    }}))"
                    ));
                }
                out
            }
            WKind::Fab { icon } => {
                used.push(0);
                let mut out = format!("FloatingActionButton::new({})", icon.rust());
                if let Some(Prop::Labelled(text)) =
                    widget.props.iter().find(|p| matches!(p, Prop::Labelled(_)))
                {
                    used.push(
                        widget
                            .props
                            .iter()
                            .position(|p| matches!(p, Prop::Labelled(_)))
                            .unwrap_or(0),
                    );
                    out.push_str(&format!(".label({})", rust_str(text)));
                }
                if let Some(event) = &widget.event {
                    let body = self.action_stmts(&event.actions, scope, 3);
                    out.push_str(&format!(
                        ".on_pressed(say::act(handle, |s: &mut SayState| {{\n{body}    }}))"
                    ));
                }
                out
            }
            WKind::TextField { bound } => {
                used.push(0);
                let mut out = format!("TextField::new({bound}.clone())");
                if let Some(Prop::Placeholder(text)) = widget
                    .props
                    .iter()
                    .find(|p| matches!(p, Prop::Placeholder(_)))
                {
                    out.push_str(&format!(".placeholder({})", rust_str(text)));
                }
                if widget.props.iter().any(|p| matches!(p, Prop::SingleLine)) {
                    out.push_str(".single_line()");
                }
                let write = self.bound_write(bound, scope);
                out.push_str(&format!(
                    ".on_changed(Rc::new(say::on(handle, |s: &mut SayState, v: TextEditingValue| {write})))"
                ));
                if let Some(event) = &widget.event {
                    if event.kind == EventKind::Submitted {
                        let body = self.action_stmts(&event.actions, scope, 3);
                        out.push_str(&format!(
                            ".on_submit(Rc::new(say::on(handle, |s: &mut SayState, text: String| {{\n{body}    }})))"
                        ));
                    }
                }
                out
            }
            WKind::Switch { bound } => {
                used.push(0);
                let write = self.bound_write(bound, scope);
                format!(
                    "Switch::new({bound}).on_changed(Rc::new(say::on(handle, |s: &mut SayState, v: bool| {write})))"
                )
            }
            WKind::Checkbox { bound } => {
                used.push(0);
                let write = self.bound_write(bound, scope);
                format!(
                    "Checkbox::new({bound}).on_changed(Rc::new(say::on(handle, |s: &mut SayState, v: bool| {write})))"
                )
            }
            WKind::Slider { bound, from, to } => {
                used.push(0);
                let mut out_from = *from;
                let mut out_to = *to;
                for (index, prop) in widget.props.iter().enumerate() {
                    if let Prop::From { from: a, to: b } = prop {
                        used.push(index);
                        out_from = *a;
                        out_to = *b;
                    }
                }
                let write = self.bound_write(bound, scope);
                format!(
                    "Slider::new({bound}).range({}, {}).on_changed(Rc::new(say::on(handle, |s: &mut SayState, v: f32| {write})))",
                    rust_f32(out_from),
                    rust_f32(out_to)
                )
            }
            WKind::ListView { list, row } => {
                used.push(0);
                let mut row = *row;
                for (index, prop) in widget.props.iter().enumerate() {
                    if let Prop::EachRow(n) = prop {
                        used.push(index);
                        row = *n;
                    }
                }
                let elem_default = match self.state(list).map(|s| s.kind) {
                    Some(StateKind::ListNumber) => "0.0",
                    _ => "String::new()",
                };
                let elem_kind = match self.state(list).map(|s| s.kind) {
                    Some(StateKind::ListNumber) => StateKind::Number,
                    _ => StateKind::Text,
                };
                let mut inner_scope = scope.clone();
                inner_scope.item = Some(elem_kind);
                let mut closure = String::new();
                closure.push_str("{\n");
                closure.push_str(&format!("    let source_{list} = {list}.clone();\n"));
                closure.push_str(&format!(
                    "    ListView::new(source_{list}.len(), {}, Rc::new(move |_i: usize| {{\n",
                    rust_f32(row)
                ));
                closure.push_str(&format!(
                    "        let item = source_{list}.get(_i).cloned().unwrap_or({elem_default});\n"
                ));
                closure.push_str("        let mut children: Vec<WidgetNode> = Vec::new();\n");
                let before = self.out.len();
                for child in &widget.children {
                    self.emit_node(child, &mut inner_scope, "children", depth + 3);
                }
                // The children were emitted straight into the buffer; splice
                // exactly what they wrote into the closure string.
                let emitted = self.out.split_off(before);
                closure.push_str(&emitted);
                closure.push_str("        if children.len() == 1 {\n");
                closure.push_str("            children.remove(0)\n");
                closure.push_str("        } else {\n");
                closure.push_str("            Flex::column().main_axis_size(MainAxisSize::Min).children(children).into()\n");
                closure.push_str("        }\n");
                closure.push_str("    }))\n");
                closure.push('}');
                // Splice discipline: the emits above wrote into self.out —
                // move them into the expression and clear them.
                let _ = &mut closure;
                used.push(0);
                closure
            }
            WKind::EmptyState { title, icon } => {
                used.push(0);
                let mut out = format!(
                    "EmptyState::new({}, {})",
                    icon.rust(),
                    self.value_rust(title, scope)
                );
                if let Some(Prop::Description(text)) = widget
                    .props
                    .iter()
                    .find(|p| matches!(p, Prop::Description(_)))
                {
                    out.push_str(&format!(".description({})", rust_str(text)));
                }
                if let Some(event) = &widget.event {
                    if event.kind == EventKind::Action {
                        let label = event.label.clone().unwrap_or_default();
                        let body = self.action_stmts(&event.actions, scope, 3);
                        out.push_str(&format!(
                            ".action({}, say::act(handle, |s: &mut SayState| {{\n{body}    }}))",
                            rust_str(&label)
                        ));
                    }
                }
                out
            }
            WKind::IconOnly { icon } => {
                used.push(0);
                format!("Icon::new({})", icon.rust())
            }
        };

        // Remaining props wrap. One total order over every wrapping property
        // (§13.0 as corrected by the review): written order, first written
        // outermost. Collect wrappers in written order, then fold from the
        // last written (innermost) to the first (outermost).
        let mut wrappers: Vec<String> = Vec::new();
        for (index, prop) in widget.props.iter().enumerate() {
            if used.contains(&index) {
                continue;
            }
            match prop {
                Prop::Padded(n) => {
                    wrappers.push(format!(
                        "Container::new().padding(EdgeInsets::all({})).child(§CHILD§)",
                        rust_f32(*n)
                    ));
                }
                Prop::Color(spec) => {
                    wrappers.push(format!(
                        "Container::new().color({}).child(§CHILD§)",
                        self.color_rust(spec)
                    ));
                }
                Prop::Radius(n) => {
                    wrappers.push(format!(
                        "Container::new().radius({}).child(§CHILD§)",
                        rust_f32(*n)
                    ));
                }
                Prop::Aligned(a) => {
                    let alignment = match a {
                        Align::Start => "Alignment::CENTER_LEFT",
                        Align::Center => "Alignment::CENTER",
                        Align::End => "Alignment::CENTER_RIGHT",
                    };
                    wrappers.push(format!(
                        "Container::new().alignment({alignment}).child(§CHILD§)"
                    ));
                }
                Prop::Wide(n) => {
                    wrappers.push(format!("SizedBox::width({}).child(§CHILD§)", rust_f32(*n)));
                }
                Prop::Tall(n) => {
                    wrappers.push(format!("SizedBox::height({}).child(§CHILD§)", rust_f32(*n)));
                }
                Prop::Lines(n) => {
                    // A taller field is a height, not a wrap setting: the
                    // field wraps by design and the box makes room for it.
                    // 24.0 is a line height at the default body size.
                    wrappers.push(format!(
                        "SizedBox::height(({} as f32) * 24.0).child(§CHILD§)",
                        n
                    ));
                }
                Prop::Pin(pin) => {
                    let (axis, value) = match pin {
                        Pin::Top(n) => ("top", rust_f32(*n)),
                        Pin::Bottom(n) => ("bottom", rust_f32(*n)),
                        Pin::Left(n) => ("left", rust_f32(*n)),
                        Pin::Right(n) => ("right", rust_f32(*n)),
                    };
                    wrappers.push(format!("Positioned::new().{axis}({value}).child(§CHILD§)"));
                }
                _ => {}
            }
        }
        for wrapper in wrappers.iter().rev() {
            expr = wrapper.replace("§CHILD§", &expr);
        }
        expr
    }

    /// Container: colour, radius, padding and alignment are its own builders.
    fn container_expr(
        &mut self,
        widget: &Widget,
        scope: &mut Scope,
        depth: usize,
        used: &mut Vec<usize>,
    ) -> String {
        let mut out = String::from("Container::new()");
        for (index, prop) in widget.props.iter().enumerate() {
            match prop {
                Prop::Color(spec) => {
                    used.push(index);
                    out.push_str(&format!(".color({})", self.color_rust(spec)));
                }
                Prop::Radius(n) => {
                    used.push(index);
                    out.push_str(&format!(".radius({})", rust_f32(*n)));
                }
                Prop::Padded(n) => {
                    used.push(index);
                    out.push_str(&format!(".padding(EdgeInsets::all({}))", rust_f32(*n)));
                }
                Prop::Aligned(a) => {
                    used.push(index);
                    let alignment = match a {
                        Align::Start => "Alignment::CENTER_LEFT",
                        Align::Center => "Alignment::CENTER",
                        Align::End => "Alignment::CENTER_RIGHT",
                    };
                    out.push_str(&format!(".alignment({alignment})"));
                }
                _ => {}
            }
        }
        let inner = self.children_vec(widget, scope, depth + 1, false);
        format!("{out}.child({inner})")
    }

    fn flex_expr(
        &mut self,
        widget: &Widget,
        scope: &mut Scope,
        depth: usize,
        column: bool,
        used: &mut Vec<usize>,
    ) -> String {
        used.push(0);
        let axis = if column { "column" } else { "row" };
        let mut out = format!("Flex::{axis}()");
        for (index, prop) in widget.props.iter().enumerate() {
            match prop {
                Prop::Spaced(n) => {
                    used.push(index);
                    out.push_str(&format!(".spacing({})", rust_f32(*n)));
                }
                Prop::ChildrenAligned(a) => {
                    used.push(index);
                    let cross = match a {
                        Cross::Start => "CrossAxisAlignment::Start",
                        Cross::Center => "CrossAxisAlignment::Center",
                        Cross::End => "CrossAxisAlignment::End",
                    };
                    out.push_str(&format!(".cross_axis_alignment({cross})"));
                }
                _ => {}
            }
        }
        let children = self.children_vec(widget, scope, depth + 1, true);
        format!("{out}.children({children})")
    }

    /// A block expression producing `children: Vec<WidgetNode>`.
    fn children_vec(
        &mut self,
        widget: &Widget,
        scope: &mut Scope,
        depth: usize,
        return_vec: bool,
    ) -> String {
        let mut block = String::new();
        block.push_str("{\n");
        block.push_str(&"    ".repeat(depth + 1));
        block.push_str("let mut children: Vec<WidgetNode> = Vec::new();\n");
        let mut inner = scope.clone();
        for child in &widget.children {
            self.emit_node_into(child, &mut inner, "children", depth + 1, &mut block);
        }
        if return_vec {
            block.push_str(&"    ".repeat(depth + 1));
            block.push_str("children\n");
        } else {
            block.push_str(&"    ".repeat(depth + 1));
            block.push_str("if children.len() == 1 {\n");
            block.push_str(&"    ".repeat(depth + 2));
            block.push_str("children.remove(0)\n");
            block.push_str(&"    ".repeat(depth + 1));
            block.push_str("} else {\n");
            block.push_str(&"    ".repeat(depth + 2));
            block.push_str(
                "Flex::column().main_axis_size(MainAxisSize::Min).children(children).into()\n",
            );
            block.push_str(&"    ".repeat(depth + 1));
            block.push_str("}\n");
        }
        block.push_str(&"    ".repeat(depth));
        block.push_str("}\n");
        block
    }

    /// Emit one node into an explicit string buffer (used inside closures and
    /// block expressions, where self.out is the wrong place).
    fn emit_node_into(
        &mut self,
        node: &Node,
        scope: &mut Scope,
        target: &str,
        depth: usize,
        into: &mut String,
    ) {
        let before = self.out.len();
        self.emit_node(node, scope, target, depth);
        let emitted = self.out.split_off(before);
        into.push_str(&emitted);
    }

    fn bound_write(&self, bound: &str, scope: &Scope) -> String {
        if scope.is_dialog && self.modal_fields.iter().any(|f| f == bound) {
            format!("if let Some(m) = s.modal.as_mut() {{ m.{bound} = v; }}")
        } else {
            format!("s.{bound} = v;")
        }
    }

    fn color_rust(&self, spec: &ColorSpec) -> String {
        match spec {
            ColorSpec::Hex(value) => format!("Color::hex(0x{value:06X})"),
            ColorSpec::Named(named) => {
                let name = match named {
                    crate::parse::NamedColor::Red => "RED",
                    crate::parse::NamedColor::Green => "GREEN",
                    crate::parse::NamedColor::Blue => "BLUE",
                    crate::parse::NamedColor::Black => "BLACK",
                    crate::parse::NamedColor::White => "WHITE",
                    crate::parse::NamedColor::Transparent => "TRANSPARENT",
                };
                format!("Color::{name}")
            }
            ColorSpec::Theme(slot) => {
                let field = match slot {
                    ThemeSlot::Primary => "primary",
                    ThemeSlot::OnPrimary => "on_primary",
                    ThemeSlot::Surface => "surface",
                    ThemeSlot::OnSurface => "on_surface",
                    ThemeSlot::SurfaceVariant => "surface_variant",
                    ThemeSlot::OnSurfaceVariant => "on_surface_variant",
                    ThemeSlot::Outline => "outline",
                    ThemeSlot::Error => "error",
                    ThemeSlot::OnError => "on_error",
                };
                format!("theme.colors.{field}")
            }
        }
    }

    // -- values, expressions, conditions ------------------------------------

    /// A display string: literal, or a `format!` over the interpolated parts.
    fn value_rust(&mut self, value: &Value, scope: &Scope) -> String {
        match value {
            Value::Lit(text) => rust_str(text),
            Value::Parts(parts) => {
                let mut fmt = String::from("\"");
                let mut args: Vec<String> = Vec::new();
                for part in parts {
                    match part {
                        Part::Lit(text) => {
                            // Escape as a Rust string first, then double the
                            // braces the format! machinery would read.
                            let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
                            fmt.push_str(&escaped.replace('{', "{{").replace('}', "}}"));
                        }
                        Part::Expr(expr) => {
                            fmt.push_str("{}");
                            args.push(self.display_arg(expr, scope));
                        }
                    }
                }
                fmt.push('"');
                if args.is_empty() {
                    fmt
                } else {
                    format!("format!({fmt}, {})", args.join(", "))
                }
            }
        }
    }

    /// One interpolation argument, in *display* form.
    fn display_arg(&self, expr: &Expr, scope: &Scope) -> String {
        match self.expr_kind(expr, scope) {
            Kind::Flag => format!(
                "if {} {{ \"yes\" }} else {{ \"no\" }}",
                self.expr_rust(expr, scope, Form::Value)
            ),
            _ => self.expr_rust(expr, scope, Form::Display),
        }
    }

    fn expr_kind(&self, expr: &Expr, scope: &Scope) -> Kind {
        match expr {
            Expr::Whole(_) => Kind::Whole,
            Expr::Num(_) => Kind::Number,
            Expr::Str(_) => Kind::Text,
            Expr::Flag(_) => Kind::Flag,
            Expr::Len { .. } => Kind::Whole,
            Expr::Bin { .. } => Kind::Number,
            Expr::Ref { name, .. } => self.name_kind(name, scope),
        }
    }

    fn name_kind(&self, name: &str, scope: &Scope) -> Kind {
        if let Some((_, elem)) = scope.loop_vars.iter().find(|(v, _)| v == name) {
            return match elem {
                StateKind::Number => Kind::Number,
                _ => Kind::Text,
            };
        }
        if name == "item" {
            return match scope.item {
                Some(StateKind::Number) => Kind::Number,
                _ => Kind::Text,
            };
        }
        if let Some(state) = self.state(name) {
            return match state.kind {
                StateKind::Whole => Kind::Whole,
                StateKind::Number => Kind::Number,
                StateKind::Flag => Kind::Flag,
                StateKind::Text => Kind::Text,
                StateKind::ListText | StateKind::ListNumber => Kind::Whole,
            };
        }
        if scope.is_dialog && self.modal_fields.iter().any(|f| f == name) {
            return Kind::Text;
        }
        Kind::Text
    }

    /// A name as it reads in the generated code.
    fn expr_rust(&self, expr: &Expr, scope: &Scope, form: Form) -> String {
        match expr {
            Expr::Whole(v) => format!("{v}"),
            Expr::Num(f) => rust_f32(*f),
            Expr::Str(text) => rust_str(text),
            Expr::Flag(v) => format!("{v}"),
            Expr::Len { list, .. } => format!("{list}.len()"),
            Expr::Bin { op, lhs, rhs, .. } => {
                let op = match op {
                    crate::parse::BinOp::Add => "+",
                    crate::parse::BinOp::Sub => "-",
                };
                format!(
                    "({} {op} {})",
                    self.expr_rust(lhs, scope, form),
                    self.expr_rust(rhs, scope, form)
                )
            }
            Expr::Ref { name, .. } => {
                let is_text = self.name_kind(name, scope) == Kind::Text;
                match is_text {
                    false => name.clone(),
                    true => match form {
                        Form::Display => format!("{name}.text"),
                        Form::Value => format!("{name}.text.clone()"),
                    },
                }
            }
        }
    }

    fn cond_rust(&self, cond: &Cond, scope: &Scope) -> String {
        match cond {
            Cond::Cmp { lhs, op, rhs } => {
                let op = match op {
                    CmpOp::Eq => "==",
                    CmpOp::Ne => "!=",
                    CmpOp::Gt => ">",
                    CmpOp::Lt => "<",
                    CmpOp::Ge => ">=",
                    CmpOp::Le => "<=",
                };
                format!(
                    "({} {op} {})",
                    self.expr_rust(lhs, scope, Form::Value),
                    self.expr_rust(rhs, scope, Form::Value)
                )
            }
            Cond::Flag { name, .. } => name.clone(),
            Cond::Contains { of, what, .. } => match self.name_kind(of, scope) {
                Kind::Text => format!("{}.contains({})", self.of_rust(of, scope), rust_str(what)),
                _ => format!(
                    "{}.iter().any(|entry| entry == {})",
                    self.of_rust(of, scope),
                    rust_str(what)
                ),
            },
            Cond::And(a, b) => format!(
                "({} && {})",
                self.cond_rust(a, scope),
                self.cond_rust(b, scope)
            ),
            Cond::Or(a, b) => format!(
                "({} || {})",
                self.cond_rust(a, scope),
                self.cond_rust(b, scope)
            ),
            Cond::Not(a) => format!("!{}", self.cond_rust(a, scope)),
        }
    }

    /// A condition's left side: text state reads `.text`, lists stay lists.
    fn of_rust(&self, name: &str, scope: &Scope) -> String {
        match self.name_kind(name, scope) {
            Kind::Text => format!("{name}.text"),
            _ => name.to_owned(),
        }
    }

    // -- actions --------------------------------------------------------------

    fn action_stmts(&mut self, actions: &[ActionLine], scope: &Scope, depth: usize) -> String {
        let mut body = String::new();
        for action in actions {
            let stmts = self.action_stmt(action, scope, depth + 1);
            match &action.only_if {
                Some(cond) => {
                    let guard = self.cond_rust(cond, scope);
                    push_into(&mut body, depth, &format!("if {guard} {{"));
                    body.push_str(&stmts);
                    push_into(&mut body, depth, "}");
                }
                None => body.push_str(&stmts),
            }
        }
        body
    }

    fn action_stmt(&mut self, action: &ActionLine, scope: &Scope, depth: usize) -> String {
        let mut body = String::new();
        macro_rules! stmt {
            ($text:expr) => {
                push_into(&mut body, depth, $text)
            };
        }
        macro_rules! block {
            ($open:expr, $inner:expr, $close:expr) => {{
                push_into(&mut body, depth, $open);
                body.push_str($inner);
                push_into(&mut body, depth, $close);
            }};
        }
        match &action.kind {
            AKind::Set { target, value } => {
                let is_modal = scope.is_dialog && self.modal_fields.iter().any(|f| f == target);
                if is_modal {
                    let v = self.expr_rust(value, scope, Form::Value);
                    let inner = {
                        let mut inner = String::new();
                        push_into(
                            &mut inner,
                            depth + 1,
                            &format!("m.{target} = TextEditingValue::new({v});"),
                        );
                        inner
                    };
                    block!(&"if let Some(m) = s.modal.as_mut() {", &inner, "}");
                } else if self.state(target).map(|s| s.kind) == Some(StateKind::Text) {
                    let v = self.expr_rust(value, scope, Form::Value);
                    stmt!(&format!("s.{target} = TextEditingValue::new({v});"));
                } else {
                    let v = self.expr_rust(value, scope, Form::Value);
                    stmt!(&format!("s.{target} = {v};"));
                }
            }
            AKind::Add { target, value } => {
                let v = self.expr_rust(value, scope, Form::Value);
                stmt!(&format!("s.{target} += {v};"));
            }
            AKind::Sub { target, value } => {
                let v = self.expr_rust(value, scope, Form::Value);
                stmt!(&format!("s.{target} -= {v};"));
            }
            AKind::Toggle { target } => {
                stmt!(&format!("s.{target} = !s.{target};"));
            }
            AKind::Append { target, value } => {
                let v = self.expr_rust(value, scope, Form::Value);
                stmt!(&format!("s.{target}.push({v});"));
            }
            AKind::RemoveAt { target, index } => {
                let v = self.expr_rust(index, scope, Form::Value);
                stmt!(&format!(
                    "if {v} >= 0 && ({v} as usize) < s.{target}.len() {{ s.{target}.remove({v} as usize); }}"
                ));
            }
            AKind::Clear { target } => {
                stmt!(&format!("s.{target}.clear();"));
            }
            AKind::Open { screen } => {
                stmt!(&format!("s.open_screen(SayScreens::{});", snake(screen)));
            }
            AKind::GoBack => {
                stmt!("s.go_back();");
            }
            AKind::Snackbar { msg } => {
                let text = match msg {
                    Value::Lit(text) => format!("String::from({})", rust_str(text)),
                    Value::Parts(_) => self.value_rust(msg, scope),
                };
                stmt!(&format!("s.show_toast({text});"));
            }
            AKind::CloseDialog => {
                stmt!("s.close_dialog();");
            }
            AKind::OpenDialog { title, .. } => {
                stmt!(&format!(
                    "s.modal = Some(SayModal::new({}));",
                    rust_str(title)
                ));
            }
        }
        body
    }
}

/// How an expression is wanted at its use site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    /// Borrowed for display: `query.text`.
    Display,
    /// Owned for a value: `query.text.clone()`.
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Whole,
    Number,
    Text,
    Flag,
}

fn push_into(body: &mut String, depth: usize, line: &str) {
    for _ in 0..depth {
        body.push_str("    ");
    }
    body.push_str(line);
    body.push('\n');
}
