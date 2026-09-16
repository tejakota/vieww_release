//! What to drop when the operating system says memory is short.
//!
//! # Why this is a level and not a notification
//!
//! Every cache in this framework is a bet that keeping something costs less
//! than recomputing it. That bet is correct right up until the OS decides the
//! process is the reason the machine is struggling, at which point it inverts:
//! Android kills the largest background process, and iOS kills one that ignores
//! `didReceiveMemoryWarning`. So the caches need a way to be told, and the thing
//! they need to be told is *how much to give up* — which is a level, not a fact.
//!
//! Before this existed the only eviction anywhere in the framework was
//! `IMAGE_RETAIN_TICKS`, which is a **clock**: an image is released two seconds
//! after its last use, whether the machine has sixteen gigabytes free or is
//! forty milliseconds from being killed. A clock cannot answer a pressure
//! signal, because the pressure signal's entire content is "sooner than that".
//!
//! # Why it is a call and not a published value
//!
//! [`Capture`](crate::Capture) is published above the tree because the *picture*
//! depends on it: a masked frame has to already be masked before anything is
//! captured, so no application code may run at the deciding moment. Memory
//! pressure is the opposite shape. It changes nothing about what the next frame
//! looks like — a trimmed cache draws exactly what a warm one draws, only
//! slower — so there is nothing for a widget to rebuild for, and publishing it
//! would mark the whole tree pending at the precise moment the machine is least
//! able to afford a full rebuild.
//!
//! So it is a call that travels down through the things that hold memory, and
//! the levels below are what each of them reads to decide.
//!
//! # Mapping from the platforms
//!
//! The three levels are the portable intersection, chosen so that no platform
//! has to invent a signal it does not receive:
//!
//! | platform | signal | level |
//! |---|---|---|
//! | Android | `onTrimMemory(TRIM_MEMORY_RUNNING_MODERATE)` | [`Moderate`](Self::Moderate) |
//! | Android | `onTrimMemory(TRIM_MEMORY_RUNNING_LOW \| RUNNING_CRITICAL)` | [`Critical`](Self::Critical) |
//! | Android | `onTrimMemory(TRIM_MEMORY_UI_HIDDEN \| BACKGROUND \| COMPLETE)` | [`Backgrounded`](Self::Backgrounded) |
//! | iOS | `didReceiveMemoryWarning` | [`Critical`](Self::Critical) |
//! | iOS | `applicationDidEnterBackground` | [`Backgrounded`](Self::Backgrounded) |
//! | macOS/Windows/Linux | (none delivered today) | — |
//!
//! **iOS has one signal and it maps to `Critical`, not `Moderate`.** UIKit does
//! not warn twice; by the time the warning arrives the process is already a
//! candidate. Treating it as a gentle hint is how an application gets killed
//! while holding a cache it was about to release.

use core::fmt;

/// How much memory the process is being asked to give back.
///
/// Ordered by severity, so `level >= MemoryPressure::Critical` is a legitimate
/// way to ask "is this serious" without matching every variant — and adding a
/// level later cannot silently flip an existing comparison, because a new one
/// would have to be placed in the ordering deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum MemoryPressure {
    /// Nothing is wrong. The default, and never delivered by a platform — it
    /// exists so a cache can describe its ordinary behaviour in the same
    /// vocabulary as its trimming behaviour, rather than having "no pressure"
    /// be the absence of a value.
    #[default]
    None,
    /// The machine would like some memory back, and there is time.
    ///
    /// Drop what is cheap to rebuild and unlikely to be needed immediately:
    /// entries nothing on screen is currently using. Keep anything a visible
    /// frame would have to recompute, because the next frame is still expected
    /// to be smooth.
    Moderate,
    /// The process is a candidate for being killed.
    ///
    /// Drop everything that can be rebuilt at all, including work the next
    /// frame will immediately have to redo. A frame that takes 40ms is
    /// survivable; being killed is not. This is where iOS's single warning
    /// lands.
    Critical,
    /// The application is no longer on screen.
    ///
    /// The most aggressive level, and the *least* urgent — nothing is being
    /// drawn, so there is no frame to make slow, and the process is now
    /// exactly the kind of thing Android's low-memory killer looks for first.
    /// Everything derived goes.
    Backgrounded,
}

impl MemoryPressure {
    /// `true` when a cache should drop entries nothing is currently using.
    #[must_use]
    pub const fn trims_unused(self) -> bool {
        !matches!(self, Self::None)
    }

    /// `true` when a cache should drop **everything**, including entries the
    /// next frame will have to rebuild.
    #[must_use]
    pub const fn trims_everything(self) -> bool {
        matches!(self, Self::Critical | Self::Backgrounded)
    }
}

impl fmt::Display for MemoryPressure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "none",
            Self::Moderate => "moderate",
            Self::Critical => "critical",
            Self::Backgrounded => "backgrounded",
        })
    }
}

/// Something that holds derived data it can rebuild.
///
/// Implemented by every cache in the framework. It is a trait rather than an
/// inherent method on each so that the platform layer can hold a
/// `&mut dyn Trim` list and fan one signal out without knowing what is in it —
/// and, more importantly, so that **an application's own caches can join**. An
/// image cache in application code is exactly as much of a liability under
/// pressure as one in the framework, and there is otherwise no way for it to
/// hear the signal.
pub trait Trim {
    /// Release what `pressure` says to release.
    ///
    /// Must be safe to call at any time, including mid-frame and repeatedly:
    /// an implementation drops *derived* data only, never anything that would
    /// change what the next frame draws. A trimmed cache and a warm one must
    /// produce identical pixels — that is the property that makes it correct
    /// to call this from a signal handler nobody scheduled.
    fn trim(&mut self, pressure: MemoryPressure);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_is_ordered_so_a_comparison_means_what_it_reads() {
        assert!(MemoryPressure::None < MemoryPressure::Moderate);
        assert!(MemoryPressure::Moderate < MemoryPressure::Critical);
        assert!(MemoryPressure::Critical < MemoryPressure::Backgrounded);
    }

    #[test]
    fn none_trims_nothing_at_all() {
        assert!(!MemoryPressure::None.trims_unused());
        assert!(!MemoryPressure::None.trims_everything());
    }

    /// The distinction the whole enum exists for: `Moderate` gives back what is
    /// idle and keeps what the next frame needs; the two serious levels do not
    /// make that distinction because they cannot afford to.
    #[test]
    fn moderate_keeps_what_a_visible_frame_would_need_and_the_rest_do_not() {
        assert!(MemoryPressure::Moderate.trims_unused());
        assert!(
            !MemoryPressure::Moderate.trims_everything(),
            "a moderate warning must not make the next frame slow"
        );

        for level in [MemoryPressure::Critical, MemoryPressure::Backgrounded] {
            assert!(level.trims_unused(), "{level}");
            assert!(level.trims_everything(), "{level}");
        }
    }

    /// iOS delivers one warning and it is not a gentle one. Pinned as a test
    /// rather than left in the module docs, because the mapping is the kind of
    /// thing a later reader "tidies" into the middle level.
    #[test]
    fn the_level_ios_maps_to_drops_everything() {
        assert!(MemoryPressure::Critical.trims_everything());
    }
}
