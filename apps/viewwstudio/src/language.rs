//! What kind of file this is, and what the editor may claim about it.
//!
//! # The finding
//!
//! The studio had exactly one grammar and applied it to everything. The file
//! tree listed every file and `open_path` opened any of them, so `Cargo.toml`,
//! `README.md`, JSON and shell scripts all arrived in an editor that ran
//! tree-sitter's **Rust** parser over them — and the status bar announced
//! "Rust" underneath, because that cell was the string literal `"Rust"`.
//! Beside it sat `"UTF-8"`, also a literal, which would have said UTF-8 over a
//! Latin-1 file.
//!
//! A status bar that reports a constant is worse than one that reports nothing,
//! because the user believes it.
//!
//! # What this module does and does not do
//!
//! It **names** languages and says which of them the studio can highlight. It
//! is not a second highlighter: adding TOML or JSON grammars is a dependency
//! decision, not a naming one, and the honest state today is "Rust is
//! highlighted, everything else is legible plain text with a correct label".
//! [`Language::highlighted`] is the single place that answers that, so adding a
//! grammar later is a change here and in `highlight.rs` and nowhere else.
//!
//! Detection is by extension, with a short table of well-known bare filenames
//! for the files that have no extension and are still obviously something —
//! `Cargo.lock`, `Makefile`, `Dockerfile`. Content sniffing is deliberately not
//! done: it is slow on the open path, wrong on short files, and the cost of
//! being wrong here is a mislabelled status cell rather than a broken buffer.

use std::path::Path;

/// A language the editor can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    Rust,
    /// Say — the English-facing screen language the studio compiles to Rust
    /// before the preview sees it. The variant is small on purpose: the
    /// studio never parses Say itself, it only names the language, comments
    /// it, and hands the buffer to the codegen step ahead of Render.
    Say,
    Toml,
    Markdown,
    Json,
    Yaml,
    Html,
    Css,
    JavaScript,
    TypeScript,
    Python,
    Shell,
    Sql,
    C,
    Cpp,
    Go,
    Java,
    Ruby,
    Swift,
    Kotlin,
    Xml,
    Ini,
    Make,
    Docker,
    Diff,
    Log,
    /// Anything else. Named "Plain Text" rather than "Unknown", because from
    /// the user's side that is what it is.
    #[default]
    Text,
}

impl Language {
    /// What the status bar shows.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Say => "Say",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Python => "Python",
            Self::Shell => "Shell",
            Self::Sql => "SQL",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::Ruby => "Ruby",
            Self::Swift => "Swift",
            Self::Kotlin => "Kotlin",
            Self::Xml => "XML",
            Self::Ini => "INI",
            Self::Make => "Makefile",
            Self::Docker => "Dockerfile",
            Self::Diff => "Diff",
            Self::Log => "Log",
            Self::Text => "Plain Text",
        }
    }

    /// Whether the studio has a grammar for this.
    ///
    /// One `true` today. It is a method rather than `== Language::Rust`
    /// scattered through the editor so that the day a second grammar arrives is
    /// a one-line change here.
    #[must_use]
    pub const fn highlighted(self) -> bool {
        // **Five, not one.** This answered `Rust` alone, and the module doc
        // above was honest about the consequence: everything else was "legible
        // plain text with a correct label". The four added here are the files
        // a Rust project contains besides its `.rs` — its manifest, its JSON
        // configs, its CI workflow, its README — which is to say the files
        // somebody actually opens and expects to see coloured.
        //
        // Still the single switch point the doc promises: a grammar is a line
        // here, a line in `highlight::grammar_for`, and an arm in
        // `highlight::style_for`.
        matches!(
            self,
            Self::Rust | Self::Toml | Self::Json | Self::Yaml | Self::Markdown
        )
    }

    /// Whether the preview pipeline can compile this file.
    ///
    /// Separate from [`Self::highlighted`] even though both are Rust-only
    /// today, because they answer different questions and will diverge: a
    /// highlighted TOML file is still not a screen the studio can render.
    #[must_use]
    pub const fn previewable(self) -> bool {
        matches!(self, Self::Rust | Self::Say)
    }

    /// The comment prefix used by [`crate::edit_ops`]'s comment toggle.
    ///
    /// `None` for languages with no line comment, where toggling must do
    /// nothing rather than insert `//` into a JSON file and break it.
    #[must_use]
    pub const fn line_comment(self) -> Option<&'static str> {
        match self {
            Self::Rust
            | Self::Cpp
            | Self::C
            | Self::Go
            | Self::Java
            | Self::JavaScript
            | Self::TypeScript
            | Self::Css
            | Self::Swift
            | Self::Kotlin => Some("//"),
            Self::Toml
            | Self::Yaml
            | Self::Python
            | Self::Shell
            | Self::Ruby
            | Self::Ini
            | Self::Make
            | Self::Docker => Some("#"),
            Self::Sql | Self::Say => Some("--"),
            _ => None,
        }
    }

    /// The language of the file at `path`.
    #[must_use]
    pub fn of_path(path: &Path) -> Self {
        // A bare name first: `Makefile` and `Dockerfile` have no extension, and
        // `Cargo.lock`'s extension would say "lock", which is not a language.
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            if let Some(language) = Self::of_filename(name) {
                return language;
            }
        }
        path.extension()
            .and_then(|ext| ext.to_str())
            .map_or(Self::Text, Self::of_extension)
    }

    /// The language of a file called exactly `name`, if that name is enough.
    #[must_use]
    pub fn of_filename(name: &str) -> Option<Self> {
        // Case-insensitive, because `makefile` and `Makefile` are both used and
        // Windows would not distinguish them anyway.
        let lowered = name.to_ascii_lowercase();
        Some(match lowered.as_str() {
            "cargo.lock" | "cargo.toml" => Self::Toml,
            "makefile" | "gnumakefile" => Self::Make,
            "dockerfile" | "containerfile" => Self::Docker,
            ".gitignore" | ".dockerignore" | ".ignore" => Self::Text,
            "rust-toolchain" => Self::Toml,
            _ => return None,
        })
    }

    /// The language for a file extension, without the dot.
    #[must_use]
    pub fn of_extension(extension: &str) -> Self {
        let lowered = extension.to_ascii_lowercase();
        match lowered.as_str() {
            "rs" => Self::Rust,
            "say" => Self::Say,
            "toml" => Self::Toml,
            "md" | "markdown" => Self::Markdown,
            "json" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "html" | "htm" => Self::Html,
            "css" => Self::Css,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "ts" | "tsx" => Self::TypeScript,
            "py" | "pyi" => Self::Python,
            "sh" | "bash" | "zsh" | "fish" => Self::Shell,
            "sql" => Self::Sql,
            "c" | "h" => Self::C,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" => Self::Cpp,
            "go" => Self::Go,
            "java" => Self::Java,
            "rb" => Self::Ruby,
            "swift" => Self::Swift,
            "kt" | "kts" => Self::Kotlin,
            "xml" | "svg" => Self::Xml,
            "ini" | "cfg" | "conf" => Self::Ini,
            "diff" | "patch" => Self::Diff,
            "log" => Self::Log,
            _ => Self::Text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn of(name: &str) -> Language {
        Language::of_path(&PathBuf::from(name))
    }

    #[test]
    fn the_obvious_cases() {
        assert_eq!(of("src/main.rs"), Language::Rust);
        assert_eq!(of("src/screens/home.say"), Language::Say);
        assert_eq!(of("Cargo.toml"), Language::Toml);
        assert_eq!(of("README.md"), Language::Markdown);
        assert_eq!(of("data.json"), Language::Json);
    }

    /// The finding, as an assertion: these used to be labelled "Rust".
    #[test]
    fn a_project_is_not_all_one_language() {
        for (name, expected) in [
            ("Cargo.toml", Language::Toml),
            ("Cargo.lock", Language::Toml),
            ("README.md", Language::Markdown),
            ("build.sh", Language::Shell),
            ("Makefile", Language::Make),
            ("Dockerfile", Language::Docker),
            (".gitignore", Language::Text),
        ] {
            assert_eq!(of(name), expected, "{name}");
            assert_ne!(of(name), Language::Rust, "{name} is not Rust");
        }
    }

    #[test]
    fn a_bare_name_beats_its_extension() {
        // "lock" is not a language; the file is TOML.
        assert_eq!(of("Cargo.lock"), Language::Toml);
    }

    #[test]
    fn case_does_not_matter() {
        assert_eq!(of("MAIN.RS"), Language::Rust);
        assert_eq!(of("makefile"), Language::Make);
        assert_eq!(of("DOCKERFILE"), Language::Docker);
    }

    #[test]
    fn an_unknown_extension_is_plain_text_and_says_so() {
        assert_eq!(of("notes.wibble"), Language::Text);
        assert_eq!(Language::Text.name(), "Plain Text");
    }

    #[test]
    fn a_file_with_no_name_at_all_does_not_panic() {
        assert_eq!(Language::of_path(Path::new("")), Language::Text);
        assert_eq!(Language::of_path(Path::new("/")), Language::Text);
    }

    /// Previewing is Rust-only; highlighting is five languages, and everything
    /// else has to be told so rather than finding out by looking wrong.
    ///
    /// This used to read `only_rust_is_highlighted_and_previewable_today` and
    /// pin both halves to Rust. The two answers have now diverged, which is
    /// what [`Language::previewable`]'s own doc said they would do: a grammar
    /// makes a file *readable*, and only the vieww compile pipeline makes one
    /// *renderable*.
    #[test]
    fn five_languages_are_highlighted_and_only_rust_is_previewable() {
        assert!(Language::Rust.highlighted());
        assert!(Language::Rust.previewable());
        for coloured in [
            Language::Toml,
            Language::Json,
            Language::Yaml,
            Language::Markdown,
        ] {
            assert!(coloured.highlighted(), "{}", coloured.name());
            assert!(
                !coloured.previewable(),
                "{} has a grammar, which is not the same as compiling to a widget",
                coloured.name()
            );
        }
        for plain in [Language::Text, Language::Sql, Language::Python] {
            assert!(!plain.highlighted(), "{}", plain.name());
            assert!(!plain.previewable(), "{}", plain.name());
        }
    }

    /// Toggling a comment in a language with no line comment must do nothing
    /// rather than insert `//` and break the file.
    #[test]
    fn a_language_with_no_line_comment_says_none() {
        assert_eq!(Language::Rust.line_comment(), Some("//"));
        assert_eq!(Language::Toml.line_comment(), Some("#"));
        assert_eq!(Language::Sql.line_comment(), Some("--"));
        assert_eq!(Language::Json.line_comment(), None);
        assert_eq!(Language::Markdown.line_comment(), None);
    }

    #[test]
    fn every_language_has_a_name_that_is_not_empty() {
        for language in [
            Language::Rust,
            Language::Say,
            Language::Toml,
            Language::Markdown,
            Language::Json,
            Language::Yaml,
            Language::Html,
            Language::Css,
            Language::JavaScript,
            Language::TypeScript,
            Language::Python,
            Language::Shell,
            Language::Sql,
            Language::C,
            Language::Cpp,
            Language::Go,
            Language::Java,
            Language::Ruby,
            Language::Swift,
            Language::Kotlin,
            Language::Xml,
            Language::Ini,
            Language::Make,
            Language::Docker,
            Language::Diff,
            Language::Log,
            Language::Text,
        ] {
            assert!(!language.name().is_empty(), "{language:?}");
        }
    }
}
