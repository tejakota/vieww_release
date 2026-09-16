//! Process-unique texture generations.
//!
//! Every atlas exposes a `version` a backend compares against what it last
//! uploaded. A per-atlas counter is not enough for that comparison: two
//! atlases that each changed twice both report `3`, and a renderer that
//! outlives one `Planner` and is handed a second one would skip the upload
//! and draw the new plan against the old texture. That is not hypothetical —
//! it is what the fixture census did, one `Planner` per fixture and one
//! `SceneRenderer` for all of them, and it showed up as gradients in the wrong
//! colours and clips cut by the previous screen's mask.
//!
//! So a version is drawn from one process-wide counter: equal versions mean
//! the same contents of the same atlas, and nothing else.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A generation no other atlas state in this process has had.
#[must_use]
pub fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}
