//! N3: reading what `cargo build --message-format=json` says.
//!
//! `compile.rs` runs one `rustc` over one file and reads `--error-format=json`.
//! This reads the *other* build path — `cargo` over a whole project — and the
//! two are different enough to deserve their own module:
//!
//! | | `compile.rs` | here |
//! |---|---|---|
//! | records | flat `rustc` diagnostics | a wrapper per record, several kinds |
//! | files | one, known before the compile | any file in the project |
//! | outcome | an `.so` to load | a binary to run, and a success flag |
//!
//! The **file** row is the one that matters. The preview compiles a buffer
//! whose name the studio chose, so `parse_diagnostics` is handed that name and
//! attributes every diagnostic to it. A project build has no such answer: an
//! error belongs to whichever file `rustc` names, which may be a file the
//! editor has never opened. So every diagnostic here carries the path cargo
//! gave it, and the Problems panel's existing open-the-file-first jump (M5)
//! does the rest.
//!
//! # Paths
//!
//! `span.file_name` is relative to the **workspace root** — the directory
//! holding the `Cargo.toml` cargo was run against — not to the current
//! directory and not to the file. [`Message::Diagnostic`] carries it exactly as
//! cargo wrote it; resolving it is the caller's job because only the caller
//! knows which directory it ran the build in.
//!
//! # Records with no span at all
//!
//! A linker failure, or `can't find crate`, has no source location. Dropping
//! those would hide the single most common way a real build fails, so they are
//! kept with an empty `file` and line 1, and the panel shows them without a
//! jump. That is a deliberate asymmetry: **a diagnostic you cannot click is
//! still a diagnostic you must read.**

use std::path::PathBuf;

use crate::json::Json;
use crate::state::{Diagnostic, Severity};

/// One line of `cargo --message-format=json`, understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// A `rustc` diagnostic about some file in the project.
    Diagnostic(Diagnostic),
    /// A build artefact. `executable` is `Some` only for a binary target, which
    /// is exactly what Build-and-Run needs and what a library build never has.
    Artifact { executable: Option<PathBuf> },
    /// The build ended. `success` is cargo's own verdict, which is not the same
    /// as "no errors were printed" — a build can fail after the last diagnostic.
    Finished { success: bool },
}

/// Read one line. `None` for a line that is not a message this cares about —
/// blank lines, `build-script-executed`, human-readable output cargo mixed in,
/// and anything that is not JSON at all.
///
/// Not an error, because a `Task` streams stdout as it arrives and a partial
/// line is normal. Refusing to parse is the common case, not the failure case.
#[must_use]
pub fn parse_line(line: &str) -> Option<Message> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let value = Json::parse(line).ok()?;

    match value.str_field("reason")? {
        "compiler-message" => diagnostic(value.get("message")?).map(Message::Diagnostic),
        "compiler-artifact" => Some(Message::Artifact {
            // `executable` is present and `null` for a library, absent on older
            // cargo. Both mean "nothing to run", and `as_str` answers `None` for
            // each without the two needing to be told apart.
            executable: value
                .get("executable")
                .and_then(Json::as_str)
                .map(PathBuf::from),
        }),
        "build-finished" => Some(Message::Finished {
            success: value
                .get("success")
                .and_then(Json::as_bool)
                .unwrap_or(false),
        }),
        _ => None,
    }
}

/// Turn one `rustc` diagnostic object into the studio's own.
///
/// `None` for the records that are not worth showing: the `note`/`help` levels,
/// which arrive again as children of the diagnostic they belong to, and the
/// two summary lines rustc ends a failed compile with.
fn diagnostic(message: &Json) -> Option<Diagnostic> {
    let severity = match message.str_field("level")? {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => return None,
    };

    let text = message.str_field("message")?;
    if text.starts_with("aborting due to") || text.starts_with("For more information") {
        return None;
    }

    // `code` is an object — `{"code":"E0425","explanation":"…"}` — or `null` for
    // a diagnostic rustc gave no code, where the level is the honest label.
    let code = message
        .path(["code", "code"])
        .and_then(Json::as_str)
        .unwrap_or(match severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        });

    let span = location(message);

    Some(Diagnostic {
        file: span.file,
        severity,
        code: code.to_string(),
        message: text.to_string(),
        help: help(message),
        line: span.line,
        column: span.column,
        end_line: span.end_line,
        end_column: span.end_column,
    })
}

/// Where the diagnostic points.
///
/// The **primary** span, not the first one. A borrow error names three or four
/// spans and only one of them is where the caret should land; taking `spans[0]`
/// puts the user at the borrow rather than at the error, which reads as the
/// panel being wrong about its own diagnostic.
fn location(message: &Json) -> Span {
    let spans = message.get("spans").and_then(Json::as_array).unwrap_or(&[]);

    let primary = spans
        .iter()
        .find(|span| span.get("is_primary").and_then(Json::as_bool) == Some(true))
        .or_else(|| spans.first());

    primary.map_or_else(Span::unknown, |span| {
        let line = span.get("line_start").and_then(Json::as_u32).unwrap_or(1);
        let column = span.get("column_start").and_then(Json::as_u32).unwrap_or(1);
        Span {
            file: span.str_field("file_name").unwrap_or_default().to_string(),
            line,
            column,
            // Defaulting the end to the start, not to the end of the line: a
            // span that could not be read is a point, and a mark running from
            // a point to the line's end is a confident claim about text the
            // compiler said nothing about.
            end_line: span.get("line_end").and_then(Json::as_u32).unwrap_or(line),
            end_column: span
                .get("column_end")
                .and_then(Json::as_u32)
                .unwrap_or(column),
        }
    })
}

/// Where a diagnostic points, both ends of it.
struct Span {
    file: String,
    line: u32,
    column: u32,
    end_line: u32,
    end_column: u32,
}

impl Span {
    fn unknown() -> Self {
        Self {
            file: String::new(),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        }
    }
}

/// rustc's suggestion, if it made one.
///
/// A machine-applicable fix lives on a **child** diagnostic's span as
/// `suggested_replacement`, and the child's own `message` is the sentence a
/// person reads ("consider borrowing here"). Preferring the replacement and
/// falling back to the child's text gives the panel something useful in both
/// shapes, and gives nothing rather than something misleading when rustc had no
/// advice.
fn help(message: &Json) -> Option<String> {
    let children = message.get("children").and_then(Json::as_array)?;

    for child in children {
        if !matches!(child.str_field("level"), Some("help" | "note")) {
            continue;
        }
        let replacement = child
            .get("spans")
            .and_then(Json::as_array)
            .unwrap_or(&[])
            .iter()
            .find_map(|span| span.str_field("suggested_replacement"));

        if let Some(replacement) = replacement {
            // A multi-line replacement in a one-line cell is noise; the panel
            // has the full text of the diagnostic either way.
            let flat = replacement.replace('\n', " ");
            let flat = flat.trim();
            if !flat.is_empty() {
                return Some(format!("try `{flat}`"));
            }
        }
        if let Some(text) = child.str_field("message") {
            return Some(text.to_string());
        }
    }
    None
}

/// Every diagnostic in a whole stream, in the order cargo emitted them.
///
/// For a build that has already finished. A live build reads [`parse_line`] as
/// each line arrives instead, so the panel fills while the build runs.
#[must_use]
pub fn diagnostics(stream: &str) -> Vec<Diagnostic> {
    stream
        .lines()
        .filter_map(parse_line)
        .filter_map(|message| match message {
            Message::Diagnostic(d) => Some(d),
            _ => None,
        })
        .collect()
}

/// The binary a finished build produced, if it produced one.
///
/// The **last** executable artefact, not the first: cargo emits one per binary
/// target it built, and the one the project is named for comes last. A project
/// with several `[[bin]]`s needs a choice the UI has to make, and this is the
/// documented default rather than a silent one.
#[must_use]
pub fn executable(stream: &str) -> Option<PathBuf> {
    stream
        .lines()
        .filter_map(parse_line)
        .filter_map(|message| match message {
            Message::Artifact { executable } => executable,
            _ => None,
        })
        .next_back()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real record, copied from `cargo build --message-format=json` output
    /// rather than written by hand — the point being that hand-written fixtures
    /// agree with whatever the parser already does.
    const ERROR: &str = r#"{"reason":"compiler-message","package_id":"path+file:///w/app#0.1.0","manifest_path":"/w/app/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"app","src_path":"/w/app/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"message":{"rendered":"error[E0425]: cannot find value `x` in this scope\n --> src/main.rs:2:13\n  |\n2 |     let y = x;\n  |             ^ not found in this scope\n\n","$message_type":"diagnostic","children":[],"code":{"code":"E0425","explanation":"An unresolved name was used."},"level":"error","message":"cannot find value `x` in this scope","spans":[{"byte_end":30,"byte_start":29,"column_end":14,"column_start":13,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":"not found in this scope","line_end":2,"line_start":2,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

    const WARNING: &str = r#"{"reason":"compiler-message","package_id":"path+file:///w/app#0.1.0","manifest_path":"/w/app/Cargo.toml","target":{"kind":["bin"],"name":"app","src_path":"/w/app/src/main.rs"},"message":{"rendered":"warning: unused variable: `y`","$message_type":"diagnostic","children":[{"children":[],"code":null,"level":"help","message":"if this is intentional, prefix it with an underscore","rendered":null,"spans":[{"byte_end":26,"byte_start":25,"column_end":10,"column_start":9,"file_name":"src/main.rs","is_primary":true,"line_end":2,"line_start":2,"suggested_replacement":"_y","suggestion_applicability":"MachineApplicable","text":[]}]}],"code":{"code":"unused_variables","explanation":null},"level":"warning","message":"unused variable: `y`","spans":[{"byte_end":26,"byte_start":25,"column_end":10,"column_start":9,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":null,"line_end":2,"line_start":2,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

    const ARTIFACT: &str = r#"{"reason":"compiler-artifact","package_id":"path+file:///w/app#0.1.0","manifest_path":"/w/app/Cargo.toml","target":{"kind":["bin"],"name":"app","src_path":"/w/app/src/main.rs"},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/w/app/target/debug/app"],"executable":"/w/app/target/debug/app","fresh":false}"#;

    const LIB_ARTIFACT: &str = r#"{"reason":"compiler-artifact","package_id":"path+file:///w/lib#0.1.0","manifest_path":"/w/lib/Cargo.toml","target":{"kind":["lib"],"name":"lib","src_path":"/w/lib/src/lib.rs"},"filenames":["/w/lib/target/debug/liblib.rlib"],"executable":null,"fresh":false}"#;

    fn one(line: &str) -> Diagnostic {
        match parse_line(line).expect("a message") {
            Message::Diagnostic(d) => d,
            other => panic!("expected a diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn an_error_keeps_its_file_code_and_place() {
        let d = one(ERROR);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code, "E0425");
        assert_eq!(d.message, "cannot find value `x` in this scope");
        assert_eq!(d.file, "src/main.rs");
        assert_eq!((d.line, d.column), (2, 13));
        assert_eq!(d.help, None);
    }

    /// The `rendered` field of `ERROR` contains the literal text `error[E0425]`
    /// and a `|` table. A substring parser reading `"level":"` or `"message":"`
    /// finds fragments of that rendering. This is the case `json.rs` exists for,
    /// asserted at the level that actually ships.
    #[test]
    fn the_rendered_field_does_not_leak_into_the_parse() {
        let d = one(ERROR);
        assert!(
            !d.message.contains("-->"),
            "read `rendered`, not `message`: {d:?}"
        );
        assert!(
            !d.message.contains('|'),
            "read `rendered`, not `message`: {d:?}"
        );
    }

    #[test]
    fn a_warning_takes_its_help_from_the_child_suggestion() {
        let d = one(WARNING);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, "unused_variables");
        assert_eq!(d.help.as_deref(), Some("try `_y`"));
    }

    /// The child carries `"level":"help"`. Reading the *first* level in the
    /// line — which is what a substring search does, since `children` precedes
    /// `level` in cargo's own key order — reports this warning as a help.
    #[test]
    fn a_childs_level_is_not_the_parents_level() {
        assert_eq!(one(WARNING).severity, Severity::Warning);
    }

    #[test]
    fn the_primary_span_wins_over_the_first_span() {
        let line = r#"{"reason":"compiler-message","message":{"level":"error","message":"borrow","code":null,"children":[],"spans":[{"file_name":"src/other.rs","line_start":9,"column_start":5,"is_primary":false},{"file_name":"src/main.rs","line_start":42,"column_start":7,"is_primary":true}]}}"#;
        let d = one(line);
        assert_eq!(d.file, "src/main.rs");
        assert_eq!((d.line, d.column), (42, 7));
        // No code, so the level is the label rather than an empty cell.
        assert_eq!(d.code, "error");
    }

    #[test]
    fn a_diagnostic_with_no_span_survives_without_a_place() {
        let line = r#"{"reason":"compiler-message","message":{"level":"error","message":"linking with `cc` failed: exit status: 1","code":null,"children":[],"spans":[]}}"#;
        let d = one(line);
        assert_eq!(d.message, "linking with `cc` failed: exit status: 1");
        assert_eq!(d.file, "");
        assert_eq!((d.line, d.column), (1, 1));
    }

    #[test]
    fn notes_and_summaries_are_dropped() {
        for line in [
            r#"{"reason":"compiler-message","message":{"level":"note","message":"required by this bound","code":null,"children":[],"spans":[]}}"#,
            r#"{"reason":"compiler-message","message":{"level":"error","message":"aborting due to 2 previous errors","code":null,"children":[],"spans":[]}}"#,
            r#"{"reason":"compiler-message","message":{"level":"failure-note","message":"some errors have detailed explanations","code":null,"children":[],"spans":[]}}"#,
        ] {
            assert_eq!(parse_line(line), None, "should be dropped: {line}");
        }
    }

    #[test]
    fn artifacts_and_the_finish_are_told_apart() {
        assert_eq!(
            parse_line(ARTIFACT),
            Some(Message::Artifact {
                executable: Some(PathBuf::from("/w/app/target/debug/app"))
            })
        );
        assert_eq!(
            parse_line(LIB_ARTIFACT),
            Some(Message::Artifact { executable: None })
        );
        assert_eq!(
            parse_line(r#"{"reason":"build-finished","success":true}"#),
            Some(Message::Finished { success: true })
        );
        assert_eq!(
            parse_line(r#"{"reason":"build-finished","success":false}"#),
            Some(Message::Finished { success: false })
        );
    }

    /// A build that fails **after** the last diagnostic — a link step, a
    /// missing target — prints no error record at all. Treating "no errors" as
    /// success is how a red build shows a green tick.
    #[test]
    fn success_comes_from_cargo_not_from_counting_errors() {
        let stream = format!("{ARTIFACT}\n{{\"reason\":\"build-finished\",\"success\":false}}");
        assert!(diagnostics(&stream).is_empty());
        assert_eq!(
            parse_line(stream.lines().next_back().unwrap()),
            Some(Message::Finished { success: false })
        );
    }

    #[test]
    fn non_json_lines_are_not_an_error() {
        for line in [
            "",
            "   ",
            "   Compiling app v0.1.0 (/w/app)",
            "error: could not compile `app` (bin \"app\") due to 1 previous error",
            "{not json",
            r#"{"reason":"build-script-executed","package_id":"x"}"#,
        ] {
            assert_eq!(parse_line(line), None, "should be ignored: {line:?}");
        }
    }

    #[test]
    fn a_whole_stream_reads_in_order() {
        let stream = format!(
            "   Compiling app v0.1.0\n{ERROR}\n{WARNING}\n{ARTIFACT}\n{{\"reason\":\"build-finished\",\"success\":true}}\n"
        );
        let found = diagnostics(&stream);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].severity, Severity::Error);
        assert_eq!(found[1].severity, Severity::Warning);
        assert_eq!(
            executable(&stream),
            Some(PathBuf::from("/w/app/target/debug/app"))
        );
    }

    #[test]
    fn the_last_executable_is_the_one_offered() {
        let first = ARTIFACT.replace("/w/app/target/debug/app", "/w/app/target/debug/helper");
        let stream = format!("{first}\n{LIB_ARTIFACT}\n{ARTIFACT}");
        assert_eq!(
            executable(&stream),
            Some(PathBuf::from("/w/app/target/debug/app"))
        );
    }
}
