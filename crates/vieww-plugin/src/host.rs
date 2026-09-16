//! A real [`HostVTable`] to hand a plugin, matching the shape any actual
//! embedding application would build.
//!
//! # Why this is a free function, not a builder
//!
//! [`LogFn`](crate::abi::LogFn) is a plain `extern "C" fn` — a C ABI vtable
//! slot cannot close over host state, the same limitation every C plugin
//! system has for its callback slots. A host that wants log output routed
//! somewhere other than stderr (a file, an in-app console, a `tracing`
//! subscriber) writes its own `extern "C" fn` of the same signature and its
//! own [`HostVTable`] literal directly — there is no builder this module
//! could offer around that call that would be anything but a more indirect
//! way to write the same few lines. What this module gives instead is one
//! real, ready-to-use default: a host with no reason to want anything else
//! should not have to write an `extern "C" fn` just to get started.

use crate::abi::{AbiStr, AbiVersion, HostVTable, LogLevel};

/// Prints `message` to stderr, prefixed with `level`, in the shape a plugin
/// author would actually see while developing against a real host.
///
/// # Why unsafe reading `message` here is sound
///
/// [`AbiStr::as_str`] requires the pointed-to memory to still be valid at the
/// time of the call. This function is only ever reachable as the `log` field
/// of a [`HostVTable`] a plugin calls synchronously, during one of its own
/// vtable calls, with a string it owns for the duration of that call — never
/// stored and read back later. That is exactly the validity window the
/// caller (the plugin) is responsible for honouring, and every trampoline in
/// [`crate::abi`] that could produce a host-bound `AbiStr` does so from a
/// value still alive on its own stack.
extern "C" fn log_to_stderr(level: LogLevel, message: AbiStr) {
    // SAFETY: see this function's own doc.
    let text = unsafe { message.as_str() };
    let label = match level {
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    };
    eprintln!("[plugin:{label}] {text}");
}

/// A [`HostVTable`] that logs to stderr and stamps [`AbiVersion::CURRENT`] —
/// a real, usable default for a host with no reason to route plugin log
/// output anywhere else.
#[must_use]
pub fn stderr_host_vtable() -> HostVTable {
    HostVTable {
        abi_version: AbiVersion::CURRENT,
        log: log_to_stderr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_host_vtable_stamps_the_current_abi_version() {
        let host = stderr_host_vtable();
        assert_eq!(host.abi_version, AbiVersion::CURRENT);
    }

    /// Not a check on stderr's actual bytes — capturing a child process's
    /// file descriptor is more machinery than this is worth — but a real
    /// call through the exact function pointer a plugin would hold, for
    /// every level, confirming none of them panics or mishandles an
    /// `AbiStr` (including the boundary case of an empty one).
    #[test]
    fn logging_at_every_level_does_not_panic() {
        let host = stderr_host_vtable();
        for level in [
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ] {
            (host.log)(level, AbiStr::borrowed("a message from a test"));
        }
        (host.log)(LogLevel::Info, AbiStr::EMPTY);
    }
}
