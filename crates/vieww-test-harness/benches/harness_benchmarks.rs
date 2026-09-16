//! Real, cheap benchmarks over this crate's own surface — `TestHarness`
//! driving a genuine mounted tree through the same synthetic event and
//! frame-scheduling path every test in this workspace already exercises.
//!
//! # Why these three
//!
//! Each isolates one thing this harness spends real time doing per test run:
//! mounting and settling the first frame (every single test pays this once),
//! injecting and settling a pointer gesture (the majority of interaction
//! tests pay this repeatedly), and a multi-step drag (the most event-heavy
//! single call this crate offers). None is a synthetic microbenchmark
//! disconnected from real usage — each is copied almost verbatim from a
//! pattern this crate's own `src/lib.rs` unit tests already run.
//!
//! # Why sample sizes are small
//!
//! This is infrastructure for catching a regression in *this test harness*,
//! not a tuned performance suite for the render pipeline itself (that is a
//! GPU-backed concern outside this crate's scope). Criterion's own minimum
//! sample size is 10; keeping every group at that floor keeps `cargo bench`
//! fast enough to run routinely rather than being something only CI ever
//! has the patience for.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use vieww_foundation::{Color, Offset, Size};
use vieww_test_harness::TestHarness;
use vieww_widget::{ColoredBox, Flex, SizedBox};

/// A moderately deep tree: a column of colored boxes, each wrapped so the
/// element tree has real depth to build and lay out — deep enough that a
/// regression in tree construction or the first layout pass would show up,
/// shallow enough that the benchmark itself stays fast.
fn moderately_deep_tree() -> Flex {
    let children = (0..40).map(|i| {
        let color = if i % 2 == 0 { Color::RED } else { Color::BLUE };
        ColoredBox::new(color)
            .child(SizedBox::from_size(Size::new(50.0, 10.0)))
            .into()
    });
    Flex::column().children(children)
}

fn bench_mount_and_first_settle(c: &mut Criterion) {
    let mut group = c.benchmark_group("mount_and_first_settle");
    group.sample_size(10);
    group.bench_function("40_child_column", |b| {
        b.iter(|| {
            let mut app = TestHarness::new(Size::new(400.0, 2000.0));
            app.mount(moderately_deep_tree());
            let report = app.tick_and_settle(32);
            std::hint::black_box(report);
        });
    });
    group.finish();
}

fn bench_tap_and_settle(c: &mut Criterion) {
    let mut group = c.benchmark_group("tap_and_settle");
    group.sample_size(10);

    // One harness, mounted once outside the timed loop — this isolates the
    // cost of a single tap-and-settle cycle from the one-time mount cost
    // `bench_mount_and_first_settle` already measures separately.
    let mut app = TestHarness::new(Size::new(400.0, 2000.0));
    app.mount(moderately_deep_tree());
    app.settle(32);

    group.bench_function("repeated_tap", |b| {
        b.iter(|| {
            app.tap(Offset::new(20.0, 20.0));
            let report = app.settle(32);
            std::hint::black_box(report);
        });
    });
    group.finish();
}

fn bench_drag_and_settle(c: &mut Criterion) {
    let mut group = c.benchmark_group("drag_and_settle");
    group.sample_size(10);

    let mut app = TestHarness::new(Size::new(400.0, 2000.0));
    app.mount(moderately_deep_tree());
    app.settle(32);

    group.bench_function("16_step_drag", |b| {
        b.iter(|| {
            let events = app.drag(Offset::new(20.0, 20.0), Offset::new(200.0, 400.0), 16);
            let report = app.settle(64);
            std::hint::black_box((events, report));
        });
    });
    group.finish();
}

criterion_group! {
    name = benches;
    // Short measurement time per group, matching this module's own doc on
    // why the sample sizes are kept at criterion's floor.
    config = Criterion::default().measurement_time(Duration::from_millis(500)).warm_up_time(Duration::from_millis(200));
    targets = bench_mount_and_first_settle, bench_tap_and_settle, bench_drag_and_settle
}
criterion_main!(benches);
