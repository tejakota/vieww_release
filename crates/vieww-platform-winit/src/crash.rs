//! Getting a crash report off the device it happened on.
//!
//! [`vieww_foundation::crash`] has the portable half — what a report *is*, how
//! it is captured, and how it is written to and read back from a directory.
//! What is left is the two things that are only answerable here: where a
//! writable directory actually is on each platform, and how to get a line out
//! on Android, where nothing this process prints to stderr survives.
//!
//! # The whole story, once
//!
//! Shown for a desktop or iOS application, where [`crash_dir`] picks the
//! directory itself. **On Android the first line differs**: the directory lives
//! inside the app's own storage and only the activity knows where that is, so it
//! comes from `android_crash_dir` with the `AndroidApp` in hand. Everything
//! below that line is identical on every platform.
//!
//! ```no_run
//! # // Android has no `crash_dir`, so this example cannot compile for it — and
//! # // the CI job that runs the suite on an Android runtime compiles doctests
//! # // too. Guarding it keeps the code *checked* where it applies rather than
//! # // marking the whole thing `ignore`, which would check it nowhere.
//! # #[cfg(target_os = "android")]
//! # fn main() {}
//! #
//! # #[cfg(not(target_os = "android"))]
//! # fn main() {
//! use vieww_foundation::crash::{Crashes, FileSink};
//! use vieww_platform_winit::{crash, App};
//!
//! # fn send_somewhere(_body: &str) {}
//! let dir = crash::crash_dir();
//!
//! // Before anything else, and before this run can add to the pile: whatever
//! // the *last* run died without being able to tell anyone about.
//! for previous in FileSink::pending(&dir) {
//!     send_somewhere(&previous.body);
//!     previous.remove();
//! }
//!
//! let crashes = Crashes::new()
//!     .with_context("version", env!("CARGO_PKG_VERSION"))
//!     .with_sink(FileSink::new(&dir))
//!     .with_sink(crash::LogSink);
//!
//! // Keep this if the application wants to leave breadcrumbs of its own; the
//! // bridge fills in the platform's.
//! let breadcrumbs = crashes.context_handle();
//!
//! App::new().report_crashes(crashes).run(|_driver| {}).unwrap();
//! # }
//! ```

use std::path::PathBuf;

use vieww_foundation::crash::{CrashReport, CrashSink};

/// Sends every report through the `log` facade at error level.
///
/// **The one sink that matters on Android.** A panic there prints to stderr,
/// and Android throws stderr away — which is why this crate depends on
/// `android_logger` at all, and why a report that only reached the default
/// panic hook has reached nobody. `log::error!` goes to logcat, where `adb` and
/// every crash-collection SDK can see it.
///
/// Cheap where it is redundant: on a desktop with no logger installed the
/// facade discards the call, and the default panic hook has already printed the
/// same message to a terminal somebody is looking at.
#[derive(Debug, Clone, Copy)]
pub struct LogSink;

impl CrashSink for LogSink {
    fn deliver(&self, report: &CrashReport) {
        // The one-line `Display` first, so a truncated logcat still says what
        // happened, then the full text for whatever is scraping it.
        log::error!("vieww crash: {report}");
        log::error!("{}", report.to_text());
    }
}

/// A writable directory for crash files on this platform.
///
/// `std::env::temp_dir()` with a subdirectory. On a desktop that is `/tmp` or
/// the user's temp folder; on iOS it is `NSTemporaryDirectory()`, which is
/// inside the app's sandbox and is the right place for a file that exists only
/// until the next launch reads it.
///
/// # Not on Android
///
/// It answers `/tmp` there, and `/tmp` does not exist on a phone. Android has
/// no writable path that can be derived without the activity, which is why
/// `android_crash_dir` takes one — call that instead, and this function will
/// not silently do the wrong thing because it is not what you called.
///
/// # It is not purge-proof
///
/// The system can empty a temporary directory between launches. That is a real
/// hole for a crash that happens shortly before the OS decides to reclaim
/// space, and the fix is an application-specific directory that the application
/// knows about — which is why [`vieww_foundation::crash::FileSink::new`] takes
/// a path rather than choosing one.
#[must_use]
#[cfg(not(target_os = "android"))]
pub fn crash_dir() -> PathBuf {
    std::env::temp_dir().join("vieww-crashes")
}

/// A writable directory for crash files inside the app's own storage.
///
/// `internal_data_path` is the activity's `filesDir`: private to the app,
/// survives a reboot, and is removed when the app is uninstalled — the three
/// properties a crash file wants. `None` if the activity has not been given one
/// yet, which happens if this is called before the activity is attached.
///
/// Separate from [`crash_dir`] rather than a `cfg` inside it, because a caller
/// on Android has to have the `AndroidApp` in hand and the type system should
/// say so.
#[must_use]
#[cfg(target_os = "android")]
pub fn android_crash_dir(android: &crate::AndroidApp) -> Option<PathBuf> {
    android
        .internal_data_path()
        .map(|files| files.join("crashes"))
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    #[test]
    fn the_crash_dir_is_a_subdirectory_rather_than_the_temp_dir_itself() {
        let dir = crash_dir();
        assert!(
            dir.starts_with(std::env::temp_dir()),
            "it has to be somewhere writable: {dir:?}"
        );
        assert_ne!(
            dir,
            std::env::temp_dir(),
            "pending() lists a whole directory, and that directory must not be \
             one every other process on the machine is also writing to"
        );
    }
}
