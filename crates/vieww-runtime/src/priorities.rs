//! Work priority classes.
//!
//! `docs/RENDERER-V2-NOTES.md`'s "GPU-driven execution" and the broader
//! architecture review both point at the same missing piece: a scheduler
//! with real priority classes, so urgent work (input, accessibility) is
//! never stuck behind cheap-but-unimportant work (a background prefetch).
//! Classic concurrent-scheduler designs are the direct precedent this
//! mirrors — several in-flight priorities so urgent updates are never
//! blocked by lower ones.

/// Ordered lowest-value-first-out: [`Scheduler`](crate::scheduler::Scheduler)
/// always runs the *lowest* [`Priority`] value it has queued next, so
/// declaring `Input` before `Background` in this enum is exactly the
/// ordering that matters. Do not reorder these variants without also
/// re-reading `scheduler.rs`'s `Ord` note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Pointer/keyboard input and accessibility events. Never delayed by
    /// anything else queued.
    Input,
    /// A currently-visible, currently-running animation frame.
    VisibleAnimation,
    /// Visible UI state that isn't mid-animation (a rebuild triggered by a
    /// state change the user just caused).
    VisibleState,
    /// Prefetch and layout work for content not yet on screen (the next
    /// page in a pager, the next chunk of a virtualized list).
    Prefetch,
    /// Everything else: telemetry, cache warming, idle-time cleanup.
    Background,
}

impl Priority {
    pub const ALL: [Self; 5] = [
        Self::Input,
        Self::VisibleAnimation,
        Self::VisibleState,
        Self::Prefetch,
        Self::Background,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_outranks_everything() {
        for p in Priority::ALL {
            assert!(Priority::Input <= p);
        }
    }

    #[test]
    fn background_is_last() {
        for p in Priority::ALL {
            assert!(p <= Priority::Background);
        }
    }
}
