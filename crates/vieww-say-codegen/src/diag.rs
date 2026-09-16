//! Diagnostics: the codes, the sentences and the line/column each one
//! carries. These run *before* rustc and land in the studio's Problems panel
//! with real positions, so the beginner never meets a rustc error that was
//! really a Say mistake.

use std::rc::Rc;

/// How a diagnostic presents in the Problems panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The file does not compile until this is fixed.
    Error,
    /// The file compiles; this is a note the author would otherwise discover
    /// the hard way (a transition the preview cannot play, a phrase that will
    /// change meaning in v2).
    Warning,
}

/// One problem, pointed at one place in the `.say` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: Rc<String>,
    pub line: u32,
    pub column: u32,
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    pub fn error(
        file: &Rc<String>,
        line: u32,
        column: u32,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            file: Rc::clone(file),
            line,
            column,
            code,
            severity: Severity::Error,
            message: message.into(),
        }
    }

    pub fn warning(
        file: &Rc<String>,
        line: u32,
        column: u32,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            file: Rc::clone(file),
            line,
            column,
            code,
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// The codes this generator emits. Kept as named constants so the tests can
/// assert on them without stringly-typed drift.
pub(crate) mod codes {
    /// The file does not start with the `-- say-language: 1` pragma.
    pub(crate) const E001: &str = "E001";
    /// A name is not usable (bad state name, unknown identifier).
    pub(crate) const E031: &str = "E031";
    /// An expression's type does not fit where it is used.
    pub(crate) const E04X: &str = "E04x";
    /// A reserved phrase: parses, but this version does not generate it.
    pub(crate) const E050: &str = "E050";
    /// A widget phrase this generator does not know.
    pub(crate) const E060: &str = "E060";
    /// A property this widget does not take, or that is misspelt.
    pub(crate) const E070: &str = "E070";
    /// A structural problem: indentation, a missing colon, a stray line.
    pub(crate) const E010: &str = "E010";
    /// Navigation that cannot work: `go back` with nowhere to go back to, or
    /// `open the screen "X"` with no screen "X".
    pub(crate) const E090: &str = "E090";
    /// A linter note: compiles, with a caveat worth saying on the line.
    pub(crate) const W130: &str = "W130";
}
