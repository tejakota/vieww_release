//! Say — the English-facing screen language for vieww.
//!
//! A `.say` file is the *only* source. This crate turns it into one byte-for-
//! byte deterministic Rust file, and that file is what the studio compiles
//! exactly as it compiles an edited Rust buffer today: same `rustc`
//! invocation, same `pub fn screen()` contract, same `dlopen`. Say changes
//! *what the buffer contains*, not how a buffer becomes a preview.
//!
//! # The shape of the pipeline
//!
//! ```text
//! counter.say ──say-codegen──▶ counter.rs ──studio rustc──▶ cdylib ──dlopen──▶ preview
//! ```
//!
//! Everything the generated file needs is emitted into it — including a small
//! `mod say` of state helpers — so the compile pipeline stays untouched: the
//! guest is linked with `--extern vieww` alone, exactly as any hand-written
//! screen is.
//!
//! # The state model (and why)
//!
//! Generated state lives in one `ElementState` on the generated root widget,
//! created at mount by `create_state`, written by handlers through
//! `ctx.state_handle()`, and polled once a frame by
//! `ElementTree::poll_states` via `take_pending`. `snapshot`/`restore` are
//! generated from the `keep` list, which is what lets a screen's state
//! survive the studio's recompile-and-remount around every Render — the host
//! half of that (`snapshot_states` / `restore_states` around the guest root)
//! already works. A previewed guest cannot reach a `Runtime` (there is no
//! `Inherited<Runtime>` across the preview boundary), so Say does not try:
//! this is the same pattern the studio's own `scaffold.rs` template ships.
//!
//! # What v1 of this generator covers
//!
//! State (`keep`), screens and navigation as a route stack, the core layout
//! and text widgets, buttons, bound controls (`a text field bound to s`),
//! lists (`a list of`, `for each`), `only if` rows, interpolation and `how
//! many`, conditional actions, snackbars and modal dialogs — every phrase
//! with an exact Rust counterpart, every generated statement preceded by a
//! `// say: file:line` marker. Phrases the vocabulary reserves but this
//! generator does not emit yet answer `E050` with what to do today.

use std::fmt;

mod diag;
mod gen;
mod lex;
mod parse;
mod parse_line;

pub use diag::{Diagnostic, Severity};

/// The result of compiling one `.say` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    /// The complete Rust source, ready for the studio's compile pipeline.
    pub rust: String,
    /// Warnings the file still carries: it compiles, and the author would
    /// still want to know (a transition the preview cannot play, a block
    /// that lost its indentation). The studio merges these into Problems.
    pub warnings: Vec<Diagnostic>,
}

/// Compile `.say` source into Rust.
///
/// `file_name` is the name the diagnostics and the generated `// say:`
/// markers carry — usually the buffer name, `home.say`.
///
/// # Errors
///
/// Every problem found, not the first one: the parser collects diagnostics
/// and keeps going where it can, so a beginner sees everything wrong with
/// their file in one pass rather than one error per Render.
pub fn compile(file_name: &str, source: &str) -> Result<Generated, Vec<Diagnostic>> {
    let file = std::rc::Rc::new(file_name.to_owned());
    let lines = lex::lex(source);
    let program = parse::parse(&file, &lines);
    match program {
        Ok(program) => Ok(Generated {
            rust: gen::generate(&file, &program),
            warnings: program.warnings,
        }),
        Err(diagnostics) => Err(diagnostics),
    }
}

/// The word-level registry the editor tools read: the canonical phrases, the
/// properties, the icons and the theme colour slots. One vocabulary, many
/// readers — the generator here, and the studio's completion and docs.
pub mod registry {
    /// The widget phrases, canonical form first.
    pub const WIDGET_PHRASES: &[&str] = &[
        "a card",
        "a row",
        "a column",
        "a stack",
        "a center",
        "a fixed area of W by H",
        "a text",
        "a heading",
        "a label",
        "a button",
        "a text field",
        "a switch",
        "a checkbox",
        "a slider",
        "a list of",
        "an empty state",
        "an icon",
        "a floating action button",
    ];

    /// The properties one widget line can carry, after a comma.
    pub const PROPERTIES: &[&str] = &[
        "padded N",
        "spaced N",
        "children aligned to the start",
        "size N",
        "bold",
        "with the primary color",
        "color #RRGGBB",
        "radius N",
        "N wide",
        "N tall",
        "with placeholder \"…\"",
        "single line",
        "in N lines",
        "each row N",
        "described as \"…\"",
        "labelled \"…\"",
        "from A to B",
        "pinned top N",
        "only if …",
    ];

    /// The action lines an event block accepts.
    pub const ACTIONS: &[&str] = &[
        "set n to E",
        "add E to n",
        "subtract E from n",
        "toggle b",
        "append E to L",
        "remove the item at E from L",
        "clear L",
        "open the screen \"X\"",
        "go back",
        "show a snackbar \"M\"",
        "close this dialog",
    ];

    /// The built-in icons, exactly the ten `vieww_widget::icons` constructors.
    pub const ICONS: &[&str] = &[
        "the check icon",
        "the close icon",
        "the plus icon",
        "the minus icon",
        "the left chevron",
        "the right chevron",
        "the up chevron",
        "the down chevron",
        "the back chevron",
        "the forward chevron",
    ];
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {} {}",
            self.file, self.line, self.column, self.code, self.message
        )
    }
}

#[cfg(test)]
mod tests {
    /// The fixture is the contract: `counter_gen.rs` is `saygen`'s output for
    /// `counter.say`, committed so a generator change that alters the output
    /// shows up as a reviewable diff. If this test fails, either the
    /// generator changed (re-run saygen and commit the new fixture
    /// deliberately) or the fixture drifted (fix the fixture).
    ///
    /// **Line endings are normalised before comparing.** The generator emits
    /// `\n`, and a Windows checkout can hand this `\r\n` — `.gitattributes`
    /// asks for LF, but a clone made before it existed, or a `core.autocrlf`
    /// left on, still delivers CRLF, and this test then failed on the Windows
    /// runner with two identical-looking 8KB strings in the message. What is
    /// under test is the generated *code*, not which bytes git chose to write
    /// the fixture with.
    #[test]
    fn the_counter_fixture_is_current() {
        let source = include_str!("../tests/fixtures/counter.say");
        let expected = include_str!("../tests/fixtures/counter_gen.rs");
        let generated = crate::compile("counter.say", source).expect("the fixture compiles");
        assert_eq!(
            generated.rust.replace("\r\n", "\n"),
            expected.replace("\r\n", "\n"),
            "the committed fixture is stale"
        );
    }

    /// Byte-identical output on every run is the codegen contract (spec
    /// §15.6): a rebuild cache keyed on content hash only works if the
    /// same input never produces two outputs.
    #[test]
    fn generation_is_deterministic() {
        let source = include_str!("../tests/fixtures/counter.say");
        let first = crate::compile("counter.say", source).unwrap().rust;
        let second = crate::compile("counter.say", source).unwrap().rust;
        assert_eq!(first, second);
    }

    /// The review's C1, pinned: the action rows compile as written, which is
    /// what the include! test proves end to end — this one keeps the
    /// failure readable when only the actions changed.
    #[test]
    fn the_counter_parses_without_diagnostics() {
        let source = include_str!("../tests/fixtures/counter.say");
        assert!(crate::compile("counter.say", source).is_ok());
    }
}
