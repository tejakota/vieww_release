//! Counting allocations, so a claim about allocation is a measurement.
//!
//! Checklist item 12 asks for performance to be **measurable permanently**, and
//! `docs/AIMS.md` puts the reason in one line:
//!
//! > **A count of what is tested is not a reading of what is fast.** Where a
//! > claim is about cost, the test has to count.
//!
//! The paint layer already counts the work it chooses to do — `paint_count`,
//! `scene_rebuilds`, `FlattenStats::built`. None of those sees an *allocation*,
//! which is the cost `docs/ARCHITECTURE-AUDIT.md`'s finding 2 is entirely
//! about: a command carrying a `Path` and a rounded `Clip` allocates twice
//! every time it is lifted into another coordinate space, and nothing in the
//! suite could say so.
//!
//! # How
//!
//! A `GlobalAlloc` that forwards to the system allocator and counts. It is
//! `#[cfg(test)]`, so it exists only in this crate's test binary — a shipped
//! build has the system allocator with no wrapper, no counter and no atomics.
//!
//! # The counter is per thread, and it has to be
//!
//! A process-wide counter was the first version, and it measured whatever else
//! the test harness happened to be doing: `cargo test` runs tests in parallel,
//! so `the_cost_of_a_lift_does_not_grow_with_the_command_count` passed on its
//! own and failed in the full suite, having counted another test's allocations.
//! A mutex around `measure` does not fix that — it serialises measurements
//! against each other, not against the ninety tests that are not measuring.
//!
//! So the count is a `thread_local!`, and a measurement sees only the
//! allocations made on the thread that asked for it. Initialised with `const`
//! so that reading it cannot itself allocate, and read through `try_with` so
//! that an allocation during thread-local teardown is ignored rather than
//! panicking inside the allocator.
//!
//! # What a number still does not mean
//!
//! Every allocation on that thread during the closure, including any made by
//! something the closure calls into. A **difference between two runs of the
//! same shape** is the meaningful reading, which is why the tests below assert
//! relationships rather than exact totals.
//!
//! # When the instrument is not installed at all
//!
//! `#[global_allocator]` is an override of the `__rust_alloc` shim, and that
//! override is **ignored when `std` is linked dynamically**: the shim is
//! already resolved inside `libstd-<hash>.so`, and this crate's static never
//! gets a call. The workspace links that way — `.cargo/config.toml` sets
//! `-C prefer-dynamic` on the desktop triples, because vieww Studio's panic
//! boundary needs the host and the previews it `dlopen`s to share one `std`,
//! and without it a panic in a user's widget aborts the editor.
//!
//! So on those triples this counter reads **zero for everything**, and a
//! measurement of zero is indistinguishable from "nothing allocated" — which
//! is precisely the reading these tests exist to make. An instrument that has
//! been switched off must not answer as though it were on: `installed` asks
//! whether the allocator was ever called, and every test below refuses to
//! assert a relationship it cannot measure, saying so out loud instead.
//!
//! To take a real reading, run this crate's tests with the flag off:
//!
//! ```console
//! $ CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS= cargo test -p vieww-paint --lib counting
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    /// Allocations made on this thread. `const` init so that touching it
    /// cannot allocate, which inside a `GlobalAlloc` would recurse.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Add one to this thread's count, if the thread still has one.
fn record() {
    // `try_with`, not `with`: during thread-local destruction the value is
    // gone, and an allocation then would panic inside the allocator.
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

fn count() -> usize {
    ALLOCATIONS.try_with(Cell::get).unwrap_or(0)
}

/// The system allocator, plus a counter.
pub(crate) struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, and this
        // forwards it unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // A realloc that has to move is an allocation; one that grows in place
        // is not. `System` does not say which happened, so this counts it as
        // one either way — the conservative direction, since undercounting is
        // what would let a regression through.
        record();
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// How many allocations `work` performed **on this thread**.
///
/// No lock: the count is thread-local, so a measurement is already isolated
/// from every other test running beside it.
pub(crate) fn measure<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let before = count();
    let value = work();
    (value, count().saturating_sub(before))
}

/// Whether this allocator is the one the process is actually using.
///
/// A `Vec` is allocated and the counter is read: if the count did not move,
/// the `#[global_allocator]` override was not honoured — see this module's
/// docs for the `-C prefer-dynamic` reason — and no number from `measure` on
/// this thread means anything.
///
/// Cheap enough to call per test, and deliberately not cached: the answer is a
/// property of the linked binary, and a `OnceLock` here would itself allocate
/// inside the thing being asked about.
pub(crate) fn installed() -> bool {
    let (value, count) = measure(|| vec![0_u8; 32]);
    // Kept alive across the read so the optimiser cannot delete the allocation
    // this is asking about.
    std::hint::black_box(&value);
    count > 0
}

/// Skip a measurement, with the reason, when the counter is not wired in.
///
/// A `return` rather than a failure: the workspace's own configuration is what
/// switches the instrument off, so a red test here would be the build telling
/// the truth and being marked wrong for it. A printed line is what a person
/// reading `cargo test -- --nocapture` needs, and the module docs carry the
/// command that takes the reading properly.
macro_rules! needs_counter {
    () => {
        if !$crate::counting::installed() {
            println!(
                "skipped: the global allocator override is not active — `std` is \
                 linked dynamically here (see .cargo/config.toml). Re-run with \
                 CARGO_TARGET_<TRIPLE>_RUSTFLAGS= to measure."
            );
            return;
        }
    };
}
// Only this module uses it; `macro_rules!` is scoped to the rest of the file
// from its definition onward, so no re-export is needed.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counter_counts() {
        needs_counter!();
        let (_, none) = measure(|| 1 + 1);
        assert_eq!(none, 0, "arithmetic does not allocate");

        let (v, one) = measure(|| vec![0_u8; 64]);
        assert_eq!(v.len(), 64);
        assert!(one >= 1, "a Vec allocates at least once");

        let (_, many) = measure(|| {
            let mut total = 0;
            for _ in 0..10 {
                total += vec![0_u8; 8].len();
            }
            total
        });
        assert!(
            many >= 10,
            "ten allocations should count at least ten, counted {many}"
        );
    }

    /// A cloned `Vec` allocates; that is the baseline the path tests measure
    /// against, so it is worth pinning that the instrument sees it.
    #[test]
    fn a_clone_of_a_heap_value_is_visible() {
        needs_counter!();
        let source = vec![1_u8, 2, 3];
        let (_, count) = measure(|| source.clone());
        assert!(count >= 1, "cloning a Vec allocates, counted {count}");
    }

    // ---- checklist 7: what lifting a command into another space costs -------

    use crate::{Canvas, Command, Paint, Scene};
    use vieww_foundation::{Color, Offset, Path, Transform};

    /// A path with `verbs` segments.
    fn path(verbs: usize) -> Path {
        let mut path = Path::new();
        path.move_to(Offset::ZERO);
        for i in 1..verbs {
            #[allow(clippy::cast_precision_loss)]
            path.line_to(Offset::new(i as f32, i as f32));
        }
        path.close();
        path
    }

    /// A scene of `count` filled paths.
    fn scene_of_paths(count: usize, verbs: usize) -> Scene {
        let mut scene = Scene::new();
        let shape = path(verbs);
        for _ in 0..count {
            scene.fill_path(&shape, Paint::from(Color::BLACK));
        }
        scene
    }

    /// Checklist item 7: lifting a command into another coordinate space no
    /// longer allocates per command.
    ///
    /// # The number this replaced
    ///
    /// `docs/ARCHITECTURE-AUDIT.md`'s finding 2 asked for a measurement before
    /// anything changed, and added: *do not pre-emptively `Rc` the path; the
    /// transform has to produce new geometry regardless.*
    ///
    /// The measurement said otherwise. `Command::transformed` composes
    /// `transform.then(by)` and rewrites the *clip*; it never touches the
    /// path's geometry, because the transform rides on the command rather than
    /// being baked into the verbs — which
    /// `lifting_does_not_rewrite_the_geometry` below asserts directly. So the
    /// `Vec<PathVerb>` inside every `FillPath` was allocated, copied and
    /// dropped unchanged, once per command, per lift. This test read **84
    /// allocations to lift 64 filled paths**.
    ///
    /// `Path` shares its verbs behind an `Arc` now, and the same measurement
    /// reads **1** — the `Vec` the results are collected into, and nothing per
    /// path at all.
    ///
    /// The bound below is deliberately loose. What is being pinned is that the
    /// cost stopped being proportional to the number of commands; asserting the
    /// exact figure would fail on a change to `Vec`'s growth policy, which is
    /// not what this is about.
    #[test]
    fn lifting_a_command_no_longer_allocates_per_path() {
        needs_counter!();
        let scene = scene_of_paths(64, 32);
        let by = Transform::translate(Offset::new(10.0, 5.0));

        let (lifted, count) = measure(|| {
            scene
                .commands()
                .iter()
                .map(|command| command.transformed(by))
                .collect::<Vec<Command>>()
        });

        assert_eq!(lifted.len(), 64);
        assert!(
            count <= 16,
            "64 paths lifted in {count} allocations; it was 84 before the verbs \
             were shared and 1 after, so anything near 64 means a `Path` is \
             being deep-copied again"
        );
        println!("lifting 64 filled paths of 32 verbs: {count} allocations");
    }

    /// Doubling the commands must not double the allocations. The test above
    /// could pass by luck on one size; this one is about the *shape* of the
    /// cost, which is what finding 2 was really about.
    #[test]
    fn the_cost_of_a_lift_does_not_grow_with_the_command_count() {
        needs_counter!();
        let by = Transform::translate(Offset::new(1.0, 1.0));
        let lift = |scene: &Scene| {
            scene
                .commands()
                .iter()
                .map(|command| command.transformed(by))
                .collect::<Vec<Command>>()
        };

        let small = scene_of_paths(32, 32);
        let large = scene_of_paths(256, 32);
        let (_, few) = measure(|| lift(&small));
        let (_, many) = measure(|| lift(&large));

        assert!(
            many <= few + 8,
            "32 commands cost {few} allocations and 256 cost {many}; the lift is \
             still paying per command"
        );
    }

    /// And the geometry is genuinely untouched by the lift, which is the fact
    /// the note above rests on.
    #[test]
    fn lifting_does_not_rewrite_the_geometry() {
        let scene = scene_of_paths(1, 8);
        let by = Transform::translate(Offset::new(10.0, 5.0));
        let original = &scene.commands()[0];
        let lifted = original.transformed(by);

        let (Command::FillPath { path: before, .. }, Command::FillPath { path: after, .. }) =
            (original, &lifted)
        else {
            panic!("expected two filled paths");
        };
        assert_eq!(
            before.verbs(),
            after.verbs(),
            "the lift rewrote the verbs, so sharing them would not be free"
        );
    }
}
