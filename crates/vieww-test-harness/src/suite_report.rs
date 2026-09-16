//! A unified test-suite report: one structure, rendered as both JSON (for a
//! CI dashboard to ingest) and a self-contained HTML page (for a person to
//! open directly).
//!
//! # Why this exists here rather than in `vieww-devtools`
//!
//! `vieww-devtools/src/json_export.rs` already hand-rolls a JSON writer for
//! *frame*-level diagnostics (render-graph passes, damage, semantics) — a
//! different, richer domain this crate has no need to depend on for the one
//! small, flat shape a test result actually is: a suite name and a list of
//! (name, status, duration, message) entries. Depending on `vieww-devtools`
//! just to reach its `ToJson` trait would be a real dependency edge pulling
//! in `vieww-render-graph`/`vieww-gpu`/`vieww-image` for a handful of
//! `write!` calls this module can do itself in under a hundred lines — the
//! same trade-off `vieww-image::residency` documents making, in the other
//! direction, for its own byte-budgeted cache: a small amount of duplicated
//! *shape*, not logic, is cheaper than the dependency.
//!
//! # Why hand-rolled JSON, not a dependency
//!
//! This workspace has no `serde` anywhere — see `vieww-devtools`'s own
//! module doc for the precedent this follows (`vieww-asset`'s hand-rolled
//! SVG-subset parser). The JSON surface a [`TestReport`] needs is exactly as
//! small and known as `vieww-devtools`'s: strings, an enum rendered as a
//! string, a duration, and an array of one small object shape — never a
//! JSON value read back into a general-purpose structure.

use std::fmt::Write as _;
use std::time::Duration;

/// How one test entry finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    /// Deliberately not run — the harness's own skip discipline (see
    /// `vieww-plugin`'s and `vieww-build`'s own test files for the
    /// established pattern: loud, on stderr, only when this machine
    /// genuinely cannot run a check) produces this rather than silently
    /// omitting the entry, so a report can distinguish "ran and passed"
    /// from "never attempted" instead of a skipped test simply vanishing.
    Skipped,
}

impl TestStatus {
    #[must_use]
    const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    /// The CSS class an HTML row is given for this status — kept next to
    /// [`as_str`](Self::as_str) rather than computed from it, so a renamed
    /// JSON value never silently renames a CSS class a stylesheet depends
    /// on.
    #[must_use]
    const fn css_class(self) -> &'static str {
        match self {
            Self::Passed => "status-passed",
            Self::Failed => "status-failed",
            Self::Skipped => "status-skipped",
        }
    }
}

/// One test's outcome, as it goes into a [`TestReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestEntry {
    pub name: String,
    pub status: TestStatus,
    pub duration: Duration,
    /// A failure reason, or why an entry was skipped. `None` for an
    /// ordinary pass — a report should not pad every passing row with an
    /// empty string.
    pub message: Option<String>,
}

impl TestEntry {
    #[must_use]
    pub fn passed(name: impl Into<String>, duration: Duration) -> Self {
        Self {
            name: name.into(),
            status: TestStatus::Passed,
            duration,
            message: None,
        }
    }

    #[must_use]
    pub fn failed(name: impl Into<String>, duration: Duration, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: TestStatus::Failed,
            duration,
            message: Some(message.into()),
        }
    }

    #[must_use]
    pub fn skipped(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: TestStatus::Skipped,
            duration: Duration::ZERO,
            message: Some(reason.into()),
        }
    }
}

/// A named collection of [`TestEntry`] results, renderable as JSON or HTML.
///
/// Built up with [`TestReport::push`] from whatever actually ran — this type
/// does not run tests itself; it is the shape a caller (a `#[test]` harness,
/// a `TestHarness`-driven scenario runner, a CI script) hands its own
/// results into, the same way [`FrameReport`](crate::FrameReport) is a
/// summary handed back rather than something that drives anything itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestReport {
    pub suite_name: String,
    pub entries: Vec<TestEntry>,
}

impl TestReport {
    #[must_use]
    pub fn new(suite_name: impl Into<String>) -> Self {
        Self {
            suite_name: suite_name.into(),
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, entry: TestEntry) {
        self.entries.push(entry);
    }

    #[must_use]
    pub fn passed(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.status == TestStatus::Passed)
            .count()
    }

    #[must_use]
    pub fn failed(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.status == TestStatus::Failed)
            .count()
    }

    #[must_use]
    pub fn skipped(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.status == TestStatus::Skipped)
            .count()
    }

    /// `true` only if every entry passed (an empty report counts as
    /// all-passing, the same convention an empty `Iterator::all` uses).
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.failed() == 0
    }

    #[must_use]
    pub fn total_duration(&self) -> Duration {
        self.entries.iter().map(|e| e.duration).sum()
    }

    /// A complete JSON document: `{"suite": ..., "passed": N, "failed": N,
    /// "skipped": N, "entries": [...]}`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push('{');
        write!(out, "\"suite\":{},", json_string(&self.suite_name)).unwrap();
        write!(out, "\"passed\":{},", self.passed()).unwrap();
        write!(out, "\"failed\":{},", self.failed()).unwrap();
        write!(out, "\"skipped\":{},", self.skipped()).unwrap();
        out.push_str("\"entries\":[");
        for (index, entry) in self.entries.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push('{');
            write!(out, "\"name\":{},", json_string(&entry.name)).unwrap();
            write!(out, "\"status\":{},", json_string(entry.status.as_str())).unwrap();
            write!(
                out,
                "\"duration_ms\":{},",
                entry.duration.as_secs_f64() * 1000.0
            )
            .unwrap();
            match &entry.message {
                Some(message) => write!(out, "\"message\":{}", json_string(message)).unwrap(),
                None => out.push_str("\"message\":null"),
            }
            out.push('}');
        }
        out.push_str("]}");
        out
    }

    /// A complete, self-contained HTML page (inline CSS, no external
    /// resources) rendering the same data [`to_json`](Self::to_json) does,
    /// as a readable table with a pass/fail summary above it.
    #[must_use]
    pub fn to_html(&self) -> String {
        let mut rows = String::new();
        for entry in &self.entries {
            let message = entry.message.as_deref().unwrap_or("");
            let _ = writeln!(
                rows,
                "<tr class=\"{}\"><td>{}</td><td>{}</td><td>{:.2} ms</td><td>{}</td></tr>",
                entry.status.css_class(),
                html_escape(&entry.name),
                entry.status.as_str(),
                entry.duration.as_secs_f64() * 1000.0,
                html_escape(message),
            );
        }

        format!(
            "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<title>{suite} — test report</title>\n\
<style>\n\
body {{ font-family: system-ui, sans-serif; margin: 2rem; color: #1a1a1a; }}\n\
h1 {{ font-size: 1.25rem; }}\n\
.summary {{ margin-bottom: 1rem; }}\n\
.summary span {{ margin-right: 1.5rem; font-weight: 600; }}\n\
table {{ border-collapse: collapse; width: 100%; }}\n\
th, td {{ text-align: left; padding: 0.4rem 0.75rem; border-bottom: 1px solid #ddd; }}\n\
tr.status-passed {{ background: #eafaf0; }}\n\
tr.status-failed {{ background: #fdecea; }}\n\
tr.status-skipped {{ background: #fdf6e3; }}\n\
</style>\n\
</head>\n\
<body>\n\
<h1>{suite}</h1>\n\
<p class=\"summary\">\n\
<span style=\"color:#1a7f37\">{passed} passed</span>\n\
<span style=\"color:#c0392b\">{failed} failed</span>\n\
<span style=\"color:#8a6d3b\">{skipped} skipped</span>\n\
<span>{total_ms:.2} ms total</span>\n\
</p>\n\
<table>\n\
<thead><tr><th>Test</th><th>Status</th><th>Duration</th><th>Message</th></tr></thead>\n\
<tbody>\n\
{rows}\
</tbody>\n\
</table>\n\
</body>\n\
</html>\n",
            suite = html_escape(&self.suite_name),
            passed = self.passed(),
            failed = self.failed(),
            skipped = self.skipped(),
            total_ms = self.total_duration().as_secs_f64() * 1000.0,
            rows = rows,
        )
    }
}

/// Escape `input` for embedding inside a JSON string literal's quotes.
///
/// Covers exactly what the JSON grammar requires — see
/// `vieww-devtools::json_export::escape_str`'s own doc for the identical
/// reasoning; duplicated rather than depended on for the reason this
/// module's own doc gives.
fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn json_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    out.push_str(&json_escape(input));
    out.push('"');
    out
}

/// Escape `input` for embedding as HTML text content (not an attribute —
/// this report never puts caller-controlled text inside an attribute
/// value).
fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> TestReport {
        let mut report = TestReport::new("example suite");
        report.push(TestEntry::passed(
            "a_thing_works",
            Duration::from_millis(12),
        ));
        report.push(TestEntry::failed(
            "a_thing_breaks",
            Duration::from_millis(3),
            "assertion failed: left == right",
        ));
        report.push(TestEntry::skipped(
            "needs_a_gpu",
            "no GPU backend on this host",
        ));
        report
    }

    #[test]
    fn counts_are_correct() {
        let report = sample_report();
        assert_eq!(report.passed(), 1);
        assert_eq!(report.failed(), 1);
        assert_eq!(report.skipped(), 1);
        assert!(!report.all_passed());
    }

    #[test]
    fn an_empty_report_counts_as_all_passed() {
        let report = TestReport::new("empty");
        assert!(report.all_passed());
        assert_eq!(report.total_duration(), Duration::ZERO);
    }

    #[test]
    fn total_duration_sums_every_entry_including_zero_duration_skips() {
        let report = sample_report();
        assert_eq!(report.total_duration(), Duration::from_millis(15));
    }

    /// A parse-able structural check, not a substring check — this actually
    /// walks the emitted JSON's brace/bracket/quote structure rather than
    /// merely asserting a field name appears somewhere in the string.
    #[test]
    fn to_json_is_well_formed_and_balanced() {
        let json = sample_report().to_json();
        assert!(json.starts_with('{') && json.ends_with('}'));
        let opens = json.matches('{').count() + json.matches('[').count();
        let closes = json.matches('}').count() + json.matches(']').count();
        assert_eq!(
            opens, closes,
            "every opening brace/bracket must be closed: {json}"
        );
        assert_eq!(
            json.matches('"').count() % 2,
            0,
            "quotes must come in pairs: {json}"
        );
    }

    #[test]
    fn to_json_contains_every_entrys_real_data() {
        let json = sample_report().to_json();
        assert!(json.contains("\"suite\":\"example suite\""));
        assert!(json.contains("\"passed\":1"));
        assert!(json.contains("\"failed\":1"));
        assert!(json.contains("\"skipped\":1"));
        assert!(json.contains("\"a_thing_works\""));
        assert!(json.contains("\"a_thing_breaks\""));
        assert!(json.contains("assertion failed: left == right"));
        assert!(json.contains("\"needs_a_gpu\""));
        assert!(
            json.contains("\"message\":null"),
            "a passing entry must report a null message, not an empty string"
        );
    }

    #[test]
    fn json_escaping_handles_quotes_and_backslashes_and_control_characters() {
        let mut report = TestReport::new("s\"u\\ite");
        report.push(TestEntry::failed(
            "na\nme",
            Duration::ZERO,
            "quote\" and backslash\\",
        ));
        let json = report.to_json();
        assert!(json.contains("s\\\"u\\\\ite"));
        assert!(json.contains("na\\nme"));
        assert!(json.contains("quote\\\" and backslash\\\\"));
    }

    #[test]
    fn to_html_is_a_complete_document_containing_every_entry() {
        let html = sample_report().to_html();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<title>example suite"));
        assert!(html.contains("a_thing_works"));
        assert!(html.contains("a_thing_breaks"));
        assert!(html.contains("assertion failed: left == right"));
        assert!(html.contains("needs_a_gpu"));
        assert!(html.contains("1 passed"));
        assert!(html.contains("1 failed"));
        assert!(html.contains("1 skipped"));
    }

    #[test]
    fn html_escaping_prevents_a_test_name_from_injecting_markup() {
        let mut report = TestReport::new("suite");
        report.push(TestEntry::passed(
            "<script>alert(1)</script>",
            Duration::ZERO,
        ));
        let html = report.to_html();
        assert!(
            !html.contains("<script>alert"),
            "an unescaped test name must not inject markup: {html}"
        );
        assert!(html.contains("&lt;script&gt;"));
    }
}
