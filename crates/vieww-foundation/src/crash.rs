//! What a developer learns after an application has already broken.
//!
//! A panic in a running vieww app currently ends one of two ways, and neither
//! of them reaches the person who has to fix it. Under
//! `ErrorPolicy::Propagate` — the release default — it unwinds out and the
//! process is gone. Under `ErrorPolicy::Placeholder` it is caught, a magenta box
//! is mounted, and the user sees a broken screen that nobody is told about. On a
//! phone both of those are silence: Android throws stdout away, and an iPhone in
//! somebody's pocket has no console attached.
//!
//! This is the third option. A panic is captured into a [`CrashReport`], handed
//! to whatever [`CrashSink`]s an application installed, and — if one of them is
//! a [`FileSink`] — written to disk *before* the process dies, so the next
//! launch can find it and send it somewhere.
//!
//! # Why this lives in the foundation crate
//!
//! It is the one thing here that is not geometry, and the placement is
//! deliberate rather than convenient. A panic hook is global: it fires for a
//! panic in *any* crate, so the type describing one has to be visible to all of
//! them, and this is the only crate every layer already depends on. It also
//! keeps the rule this crate is built on — it pulls in nothing but `std`, and
//! the sinks are a trait rather than a network client precisely so that no
//! opinion about uploading, batching or serialising leaks down here.
//!
//! # The two halves, and why persistence is the one that matters
//!
//! Delivering a report to a log is easy and nearly useless: the process that
//! would do the delivering is the one that just died, and on a user's device
//! nobody is reading logcat. The half that earns its keep is
//! `FileSink::write` followed by [`FileSink::pending`] on the *next* run —
//! that is the only sequence that survives the crash it is reporting on.
//!
//! ```no_run
//! use vieww_foundation::crash::{Crashes, FileSink};
//!
//! # fn upload(_body: &str) {}
//! let dir = std::path::PathBuf::from("/tmp/vieww-crashes");
//!
//! // Anything the last run left behind, before this run can add to it.
//! for previous in FileSink::pending(&dir) {
//!     upload(&previous.body);
//!     previous.remove();
//! }
//!
//! Crashes::new()
//!     .with_context("build", env!("CARGO_PKG_VERSION"))
//!     .with_sink(FileSink::new(&dir))
//!     .install();
//! ```

use std::any::Any;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where in the source a crash came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashLocation {
    /// The file the panic was raised in, as `panic!` recorded it.
    pub file: String,
    /// One-based, as `panic!` records it.
    pub line: u32,
    /// One-based, as `panic!` records it.
    pub column: u32,
}

impl fmt::Display for CrashLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// Whether something actually panicked, or an application chose to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashKind {
    /// Came through the panic hook. Something unwound.
    Panic,
    /// Handed in by [`Crashes::report`]. Nothing unwound — the application
    /// decided this was worth knowing about anyway, which is what a caught
    /// build error is.
    Reported,
}

impl fmt::Display for CrashKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Panic => "panic",
            Self::Reported => "reported",
        })
    }
}

/// One thing that went wrong, with everything known about it at the time.
///
/// Deliberately a plain value with no methods that do I/O: producing it happens
/// on a thread that is usually in the middle of unwinding, and every decision
/// about what to *do* with it belongs to a [`CrashSink`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    /// Panic, or an application's own report.
    pub kind: CrashKind,
    /// The panic message, when the payload was a `&str` or a `String`. A
    /// `panic_any` carrying anything else reports only that it did — there is
    /// nothing useful to say about a payload whose type cannot be named.
    pub message: String,
    /// `None` for a panic raised somewhere without location information, which
    /// in practice means one crossing an FFI boundary.
    pub location: Option<CrashLocation>,
    /// The name of the thread that panicked, or `"<unnamed>"`. Worth having:
    /// "it panicked on the UI thread" and "it panicked on a worker" are
    /// different bugs and the message is often identical.
    pub thread: String,
    /// Captured with `force_capture`, so it is present whether or not
    /// `RUST_BACKTRACE` is set — a user's phone will never have it set, and a
    /// report without a backtrace is usually not actionable.
    ///
    /// Empty when the platform cannot unwind symbols at all.
    pub backtrace: String,
    /// Milliseconds since the Unix epoch, or `0` if the clock is before it.
    ///
    /// A raw number rather than a formatted time: this crate will not take a
    /// date-formatting dependency, and the consumer that eventually renders
    /// this has one.
    pub unix_millis: u128,
    /// Whatever the application attached with [`Crashes::with_context`] plus
    /// anything a [`CrashContext`] held at the moment of the crash.
    ///
    /// This is where a report stops being a stack trace and starts being
    /// diagnosable: which screen was up, which frame number, which device.
    pub context: Vec<(String, String)>,
}

impl CrashReport {
    /// Pull a readable message out of a panic payload.
    ///
    /// The same three cases `vieww_element::BuildError` handles, for the same
    /// reason — and duplicated rather than shared because this crate sits below
    /// that one and a panic hook must not depend on the element tree.
    #[must_use]
    pub fn message_from(payload: &(dyn Any + Send)) -> String {
        if let Some(message) = payload.downcast_ref::<&'static str>() {
            (*message).to_owned()
        } else if let Some(message) = payload.downcast_ref::<String>() {
            message.clone()
        } else {
            String::from("<non-string panic payload>")
        }
    }

    /// A stable, greppable rendering of the whole report.
    ///
    /// Deliberately not JSON. This crate has no serialiser and will not grow
    /// one, the format has to survive being read by a human in a bug report as
    /// often as by a program, and the one thing a crash file must never do is
    /// fail to be written because a value would not encode.
    ///
    /// The shape is `key: value` lines, then a blank line, then the backtrace —
    /// so a parser can stop at the blank line and everything after it is one
    /// opaque block that needs no escaping.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        let mut line = |key: &str, value: &dyn fmt::Display| {
            use fmt::Write as _;
            // Writing to a `String` cannot fail; the result is discarded rather
            // than unwrapped because this runs inside a panic hook, and a hook
            // that panics aborts the process without writing the report it was
            // called to write.
            let _ = writeln!(text, "{key}: {value}");
        };

        line("kind", &self.kind);
        line("at", &self.unix_millis);
        line("thread", &self.thread);
        line("message", &Sanitised(&self.message));
        match &self.location {
            Some(location) => line("location", location),
            None => line("location", &"<unknown>"),
        }
        for (key, value) in &self.context {
            line(&format!("context.{key}"), &Sanitised(value));
        }

        text.push('\n');
        text.push_str(&self.backtrace);
        if !self.backtrace.ends_with('\n') {
            text.push('\n');
        }
        text
    }
}

impl fmt::Display for CrashReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} on {}: {}", self.kind, self.thread, self.message)?;
        if let Some(location) = &self.location {
            write!(f, " (at {location})")?;
        }
        Ok(())
    }
}

/// A value with its newlines flattened, so one field stays one line.
///
/// A panic message can contain anything, including the `\n` that would
/// otherwise let a message forge a second `key: value` line in the output.
struct Sanitised<'a>(&'a str);

impl fmt::Display for Sanitised<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = self.0.split('\n');
        // The first part unprefixed, the rest each behind a separator — so a
        // message with no newline in it comes through completely untouched.
        if let Some(first) = parts.next() {
            f.write_str(first.trim_end_matches('\r'))?;
        }
        for part in parts {
            f.write_str(" ⏎ ")?;
            f.write_str(part.trim_end_matches('\r'))?;
        }
        Ok(())
    }
}

/// Somewhere a [`CrashReport`] goes.
///
/// `Send + Sync` because the hook that calls this can fire on any thread, and
/// `&self` rather than `&mut self` for the same reason: two threads can panic
/// at once, and a sink that needed exclusive access would have to be locked by
/// every caller.
pub trait CrashSink: Send + Sync {
    /// Do something with `report`.
    ///
    /// **This runs while a thread is unwinding, and must not panic.** A panic
    /// in a panic hook aborts the process immediately, which loses the report
    /// and every other sink after this one. Swallow errors; there is nobody
    /// left to hand them to.
    fn deliver(&self, report: &CrashReport);
}

/// Mutable breadcrumbs, readable from the thread that is crashing.
///
/// Handed out by [`Crashes::context_handle`] so an application can keep
/// something current — which screen is up, which frame is being drawn — without
/// reinstalling the hook. Cloning shares the same storage.
#[derive(Debug, Clone, Default)]
pub struct CrashContext {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

impl CrashContext {
    /// An empty set of breadcrumbs.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set `key`, replacing any previous value for it.
    ///
    /// Replacing rather than appending, because these are current facts rather
    /// than a history — a `frame` breadcrumb that appended would be the whole
    /// run's frame numbers by the time anything crashed.
    pub fn set(&self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let mut entries = self.lock();
        match entries.iter_mut().find(|(existing, _)| *existing == key) {
            Some(entry) => entry.1 = value.into(),
            None => entries.push((key, value.into())),
        }
    }

    /// Forget `key`.
    pub fn remove(&self, key: &str) {
        self.lock().retain(|(existing, _)| existing != key);
    }

    /// Everything currently set, in insertion order.
    #[must_use]
    pub fn entries(&self) -> Vec<(String, String)> {
        self.lock().clone()
    }

    /// The lock, with poisoning ignored.
    ///
    /// A poisoned mutex here means a thread panicked while holding it — which
    /// is exactly the situation this whole module exists for. Refusing to read
    /// the breadcrumbs *because* something crashed would throw away the report
    /// at the only moment it is worth anything. The data is a `Vec` of strings
    /// with no invariant a partial write could break.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(String, String)>> {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// The panic hook, its sinks, and the context they are given.
///
/// Built, then [`install`](Self::install)ed once. Installing replaces the
/// previous hook and *chains* to it, so the default hook's message still
/// reaches stderr and a second reporter installed by a host application is not
/// silently disabled.
pub struct Crashes {
    sinks: Vec<Arc<dyn CrashSink>>,
    /// Fixed at install time: build id, platform, anything that cannot change.
    fixed: Vec<(String, String)>,
    context: CrashContext,
}

/// Hand-written because `dyn CrashSink` is not `Debug`, and requiring it would
/// force the bound onto every sink an application writes for the sake of a
/// derive.
impl fmt::Debug for Crashes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Crashes")
            .field("sinks", &self.sinks.len())
            .field("fixed", &self.fixed)
            .field("context", &self.context)
            .finish()
    }
}

impl Default for Crashes {
    fn default() -> Self {
        Self::new()
    }
}

impl Crashes {
    /// A reporter with no sinks, which reports nowhere.
    ///
    /// Useless on purpose: where a report goes is an application's decision and
    /// there is no default that is right for both a desktop binary and a phone.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sinks: Vec::new(),
            fixed: Vec::new(),
            context: CrashContext::new(),
        }
    }

    /// Send every report to `sink` as well.
    #[must_use]
    pub fn with_sink(mut self, sink: impl CrashSink + 'static) -> Self {
        self.sinks.push(Arc::new(sink));
        self
    }

    /// Attach a fact that will not change for the life of the process.
    #[must_use]
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fixed.push((key.into(), value.into()));
        self
    }

    /// A handle for breadcrumbs that *do* change.
    ///
    /// Take this before [`install`](Self::install) — it consumes the reporter —
    /// and keep writing to it for the life of the application.
    #[must_use]
    pub fn context_handle(&self) -> CrashContext {
        self.context.clone()
    }

    /// Build a report for something that did not panic.
    ///
    /// The other half of the gap this module closes. A build error caught by
    /// `ErrorPolicy::Placeholder` never unwinds past the catch, so the panic
    /// hook's report is all there is — and while that report exists, it does
    /// not know which widget was building. An application that wants that in
    /// the report calls this with what it knows.
    pub fn report(&self, message: impl Into<String>) {
        let report = CrashReport {
            kind: CrashKind::Reported,
            message: message.into(),
            location: None,
            thread: thread_name(),
            backtrace: std::backtrace::Backtrace::force_capture().to_string(),
            unix_millis: now_millis(),
            context: self.all_context(),
        };
        self.deliver(&report);
    }

    /// Everything a report should carry, fixed facts first.
    fn all_context(&self) -> Vec<(String, String)> {
        let mut context = self.fixed.clone();
        context.extend(self.context.entries());
        context
    }

    /// Hand `report` to every sink, in the order they were added.
    fn deliver(&self, report: &CrashReport) {
        for sink in &self.sinks {
            sink.deliver(report);
        }
    }

    /// Take over the process's panic hook.
    ///
    /// # What happens to the hook that was already there
    ///
    /// It is kept and called *after* the sinks. Two reasons, in order: the
    /// default hook's job is to print to stderr and that should still happen,
    /// and a report written before the previous hook runs is a report that
    /// survives the previous hook deciding to abort.
    ///
    /// # Reentrancy
    ///
    /// A panic raised *inside* a sink would re-enter this hook and recurse
    /// until the stack ran out. A flag stops the second entry, and the report
    /// that triggered it is dropped rather than chased — one lost report is a
    /// better outcome than a stack overflow standing in for a crash log.
    pub fn install(self) {
        let previous = std::panic::take_hook();
        // Process-wide rather than thread-local: a sink can hand work to
        // another thread, and a panic *there* is still this hook re-entering.
        static REENTERED: AtomicBool = AtomicBool::new(false);

        std::panic::set_hook(Box::new(move |info| {
            if REENTERED.swap(true, Ordering::SeqCst) {
                previous(info);
                return;
            }

            let report = CrashReport {
                kind: CrashKind::Panic,
                message: CrashReport::message_from(info.payload()),
                location: info.location().map(|location| CrashLocation {
                    file: location.file().to_owned(),
                    line: location.line(),
                    column: location.column(),
                }),
                thread: thread_name(),
                backtrace: std::backtrace::Backtrace::force_capture().to_string(),
                unix_millis: now_millis(),
                context: self.all_context(),
            };
            self.deliver(&report);

            REENTERED.store(false, Ordering::SeqCst);
            previous(info);
        }));
    }
}

/// The current thread's name, or a stand-in.
fn thread_name() -> String {
    std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_owned()
}

/// Milliseconds since the Unix epoch; `0` if the clock is set before it.
fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

/// A crash file left behind by a previous run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCrash {
    /// Where it is, so it can be deleted once it has been dealt with.
    pub path: PathBuf,
    /// What [`CrashReport::to_text`] wrote.
    pub body: String,
}

impl PendingCrash {
    /// Delete the file.
    ///
    /// Call it *after* the report has been sent somewhere, not before — a
    /// report deleted on the way to a failed upload is gone for good, and a
    /// report uploaded twice is merely untidy.
    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Writes each report to its own file in a directory.
///
/// The only sink that survives the process it is reporting on, which is the
/// entire point — see the module docs.
#[derive(Debug)]
pub struct FileSink {
    dir: PathBuf,
    /// Distinguishes two crashes inside the same millisecond, which two threads
    /// panicking together genuinely can be.
    sequence: AtomicU64,
}

impl FileSink {
    /// The file extension every crash file gets, and the one
    /// [`pending`](Self::pending) looks for.
    ///
    /// A distinct extension rather than a name prefix, so pointing this at a
    /// directory that holds anything else cannot make [`pending`](Self::pending)
    /// return somebody else's file.
    pub const EXTENSION: &'static str = "viewwcrash";

    /// Write crash files into `dir`, creating it if it does not exist.
    ///
    /// On a phone this wants to be inside the app's own storage — Android's
    /// `filesDir`, iOS's Application Support — because anywhere else either
    /// does not survive an update or cannot be written to at all.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        // Now rather than at crash time. Creating a directory during a panic is
        // one more thing that can fail at the moment nothing may fail, and a
        // sink whose directory is missing is worth discovering at startup.
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            sequence: AtomicU64::new(0),
        }
    }

    /// Every crash file in `dir`, oldest name first.
    ///
    /// Call this at startup, before installing anything: what it returns is
    /// what the *last* run could not tell anyone about. Files that cannot be
    /// read are skipped rather than reported — an unreadable crash file is not
    /// itself worth crashing over.
    ///
    /// Sorted by path, which sorts by time because the filenames lead with a
    /// zero-padded millisecond count.
    #[must_use]
    pub fn pending(dir: impl AsRef<Path>) -> Vec<PendingCrash> {
        let Ok(entries) = std::fs::read_dir(dir.as_ref()) else {
            return Vec::new();
        };

        let mut pending: Vec<PendingCrash> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == Self::EXTENSION)
            })
            .filter_map(|path| {
                std::fs::read_to_string(&path)
                    .ok()
                    .map(|body| PendingCrash { path, body })
            })
            .collect();
        pending.sort_by(|a, b| a.path.cmp(&b.path));
        pending
    }

    /// Where the next report would be written.
    fn next_path(&self, report: &CrashReport) -> PathBuf {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        // Zero-padded so a lexical sort is a chronological one. Sixteen digits
        // covers milliseconds until the year 500,000-odd.
        self.dir.join(format!(
            "{:016}-{sequence:04}.{}",
            report.unix_millis,
            Self::EXTENSION
        ))
    }
}

impl CrashSink for FileSink {
    fn deliver(&self, report: &CrashReport) {
        // Every error swallowed, deliberately. This runs mid-unwind: there is
        // no caller to return to and no way to report a failure to report.
        let _ = std::fs::write(self.next_path(report), report.to_text());
    }
}

/// Passes each report to a closure.
///
/// The escape hatch, and what a test uses. An application that wants reports on
/// stderr, in `log`, or straight into a network client writes three lines here
/// rather than a type.
pub struct FnSink<F>(F);

impl<F> fmt::Debug for FnSink<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnSink").finish_non_exhaustive()
    }
}

impl<F: Fn(&CrashReport) + Send + Sync> FnSink<F> {
    /// Call `deliver` with every report.
    #[must_use]
    pub const fn new(deliver: F) -> Self {
        Self(deliver)
    }
}

impl<F: Fn(&CrashReport) + Send + Sync> CrashSink for FnSink<F> {
    fn deliver(&self, report: &CrashReport) {
        (self.0)(report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(message: &str) -> CrashReport {
        CrashReport {
            kind: CrashKind::Panic,
            message: message.to_owned(),
            location: Some(CrashLocation {
                file: "src/lib.rs".to_owned(),
                line: 12,
                column: 3,
            }),
            thread: "main".to_owned(),
            backtrace: "0: nothing\n".to_owned(),
            unix_millis: 1_700_000_000_000,
            context: vec![("build".to_owned(), "0.0.1".to_owned())],
        }
    }

    /// A scratch directory that cleans itself up, so the file tests do not need
    /// a dev-dependency on `tempfile` in the one crate that has no dependencies.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vieww-crash-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_report_renders_its_fields_before_a_blank_line_and_the_trace_after_it() {
        let text = report("it broke").to_text();
        let (fields, trace) = text
            .split_once("\n\n")
            .expect("a blank line separates them");

        assert!(fields.contains("kind: panic"));
        assert!(fields.contains("message: it broke"));
        assert!(fields.contains("location: src/lib.rs:12:3"));
        assert!(fields.contains("context.build: 0.0.1"));
        assert_eq!(trace, "0: nothing\n");
    }

    #[test]
    fn a_newline_in_a_message_cannot_forge_a_second_field() {
        let text = report("broke\nkind: notapanic").to_text();
        let fields = text.split_once("\n\n").expect("a blank line").0;

        assert_eq!(
            fields
                .lines()
                .filter(|line| line.starts_with("kind:"))
                .count(),
            1,
            "a multi-line message must stay on one line: {fields}"
        );
        assert!(fields.contains("message: broke ⏎ kind: notapanic"));
    }

    #[test]
    fn a_payload_that_is_not_a_string_reports_that_rather_than_nothing() {
        let payload: Box<dyn Any + Send> = Box::new(7_u32);
        assert_eq!(
            CrashReport::message_from(payload.as_ref()),
            "<non-string panic payload>"
        );
    }

    #[test]
    fn a_string_payload_and_a_str_payload_both_come_back_readable() {
        let literal: Box<dyn Any + Send> = Box::new("a literal");
        let formatted: Box<dyn Any + Send> = Box::new(String::from("a String"));
        assert_eq!(CrashReport::message_from(literal.as_ref()), "a literal");
        assert_eq!(CrashReport::message_from(formatted.as_ref()), "a String");
    }

    #[test]
    fn context_replaces_rather_than_accumulates() {
        let context = CrashContext::new();
        context.set("frame", "1");
        context.set("frame", "2");
        context.set("route", "/home");

        assert_eq!(
            context.entries(),
            vec![
                ("frame".to_owned(), "2".to_owned()),
                ("route".to_owned(), "/home".to_owned())
            ],
            "a breadcrumb is a current fact, not a history"
        );

        context.remove("route");
        assert_eq!(
            context.entries(),
            vec![("frame".to_owned(), "2".to_owned())]
        );
    }

    #[test]
    fn a_context_handle_taken_before_install_still_reaches_a_report() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&seen);

        let crashes = Crashes::new()
            .with_context("build", "0.0.1")
            .with_sink(FnSink::new(move |report: &CrashReport| {
                captured.lock().unwrap().push(report.context.clone());
            }));
        let context = crashes.context_handle();
        context.set("frame", "41");

        // Not installed: `report` is the path that does not need the hook, so
        // this test does not disturb the process-wide hook other tests share.
        crashes.report("something worth knowing");
        context.set("frame", "42");
        crashes.report("and again");

        let seen = seen.lock().unwrap();
        assert_eq!(
            seen[0],
            vec![
                ("build".to_owned(), "0.0.1".to_owned()),
                ("frame".to_owned(), "41".to_owned())
            ],
            "fixed context first, then the live breadcrumbs"
        );
        assert_eq!(
            seen[1][1].1, "42",
            "the handle keeps working after the reporter was built"
        );
    }

    #[test]
    fn a_reported_error_is_marked_as_one_and_still_carries_a_backtrace() {
        let seen = Arc::new(Mutex::new(None));
        let captured = Arc::clone(&seen);
        Crashes::new()
            .with_sink(FnSink::new(move |report: &CrashReport| {
                *captured.lock().unwrap() = Some(report.clone());
            }))
            .report("a build error the placeholder swallowed");

        let report = seen.lock().unwrap().clone().expect("a report");
        assert_eq!(report.kind, CrashKind::Reported);
        assert!(
            !report.backtrace.is_empty(),
            "force_capture must not depend on RUST_BACKTRACE"
        );
    }

    #[test]
    fn a_written_report_is_found_by_the_next_run_and_can_be_cleared() {
        let scratch = Scratch::new("roundtrip");
        let sink = FileSink::new(&scratch.0);

        assert!(
            FileSink::pending(&scratch.0).is_empty(),
            "a fresh directory has nothing to send"
        );

        sink.deliver(&report("the first"));
        sink.deliver(&report("the second"));

        let pending = FileSink::pending(&scratch.0);
        assert_eq!(
            pending.len(),
            2,
            "two crashes in one millisecond are two files"
        );
        assert!(pending[0].body.contains("message: the first"));
        assert!(pending[1].body.contains("message: the second"));

        for crash in &pending {
            crash.remove();
        }
        assert!(FileSink::pending(&scratch.0).is_empty());
    }

    #[test]
    fn pending_ignores_anything_that_is_not_a_crash_file() {
        let scratch = Scratch::new("mixed");
        let sink = FileSink::new(&scratch.0);
        sink.deliver(&report("a real one"));
        std::fs::write(scratch.0.join("notes.txt"), "not a crash").unwrap();

        let pending = FileSink::pending(&scratch.0);
        assert_eq!(pending.len(), 1);
        assert!(pending[0].body.contains("a real one"));
    }

    #[test]
    fn a_missing_directory_yields_nothing_rather_than_an_error() {
        assert!(FileSink::pending("/nonexistent/vieww/crashes").is_empty());
    }

    #[test]
    fn files_sort_chronologically_because_the_millisecond_is_zero_padded() {
        let scratch = Scratch::new("ordering");
        let sink = FileSink::new(&scratch.0);

        let mut early = report("earlier");
        early.unix_millis = 900;
        let mut late = report("later");
        late.unix_millis = 1_700_000_000_000;

        // Written late-first, so a sort by name is doing the work rather than
        // the order they happened to land in.
        sink.deliver(&late);
        sink.deliver(&early);

        let pending = FileSink::pending(&scratch.0);
        assert!(
            pending[0].body.contains("earlier"),
            "900 must sort before 1700000000000, which it does not as a bare \
             number: {:?}",
            pending.iter().map(|c| &c.path).collect::<Vec<_>>()
        );
    }
}
