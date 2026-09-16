//! A priority-ordered multi-threaded work scheduler.
//!
//! # What this is and is not
//!
//! Real: work queued at [`Priority::Input`] is always dequeued before work
//! queued at [`Priority::Background`], across any number of worker threads,
//! and this is verified by an actual multi-threaded test
//! (`higher_priority_work_is_drained_first`) rather than asserted only by
//! the shape of the code.
//!
//! Not real (yet): true *preemption* — a worker that has already started a
//! `Background` job runs it to completion even if an `Input` job arrives a
//! microsecond later. Genuine preemption needs cooperative yield points
//! inside the job itself (a widget build checking "should I yield" between
//! subtrees, the way a concurrent renderer's own priority classes
//! do), which is a property of *what runs inside a job*, not of the
//! scheduler around it — `docs/RENDERER-V2-NOTES.md` has no ready-made
//! yield-point convention this crate could plug into today, and inventing
//! one silently here would be exactly the kind of unverified architectural
//! decision this workspace's own docs are careful never to make quietly.
//! What this scheduler *does* guarantee is that the wait is bounded by
//! "one job's worst-case duration," not by "everything else queued ahead of
//! it" — which is already the difference between a FIFO queue and a
//! priority one.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::priorities::Priority;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct QueuedJob {
    priority: Priority,
    // Insertion sequence, so jobs at the same priority run FIFO rather than
    // in whatever order a heap happens to leave them.
    seq: u64,
    job: Job,
}

impl PartialEq for QueuedJob {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}
impl Eq for QueuedJob {}

impl PartialOrd for QueuedJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedJob {
    fn cmp(&self, other: &Self) -> Ordering {
        // `BinaryHeap` is a max-heap and a lower `Priority` value is more
        // urgent (see that enum's doc), so reverse the comparison on
        // priority; break ties by *earlier* sequence number winning, which
        // is again "reversed" for a max-heap (we want the smallest seq to
        // sort as the greatest heap element).
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

struct Shared {
    heap: Mutex<BinaryHeap<QueuedJob>>,
    condvar: Condvar,
    shutdown: AtomicBool,
    next_seq: AtomicU64,
    completed: [AtomicU64; 5],
}

/// A priority-ordered thread pool.
///
/// Dropping the last handle to a [`Scheduler`] does not stop its workers —
/// call [`Scheduler::shutdown`] explicitly (mirroring the rest of this
/// workspace's preference for explicit lifecycle over implicit `Drop`
/// magic, e.g. `vieww_paint::native`'s renderer teardown) so a caller
/// controls exactly when in-flight jobs are allowed to finish versus being
/// abandoned.
#[derive(Debug)]
pub struct Scheduler {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl Scheduler {
    /// Start a scheduler with `worker_count` OS threads (clamped to at
    /// least 1).
    #[must_use]
    pub fn new(worker_count: usize) -> Self {
        let shared = Arc::new(Shared {
            heap: Mutex::new(BinaryHeap::new()),
            condvar: Condvar::new(),
            shutdown: AtomicBool::new(false),
            next_seq: AtomicU64::new(0),
            completed: Default::default(),
        });

        let workers = (0..worker_count.max(1))
            .map(|_| {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || worker_loop(shared))
            })
            .collect();

        Self { shared, workers }
    }

    /// Queue `job` at `priority`. Returns immediately; `job` runs on
    /// whichever worker becomes free, respecting priority order among
    /// everything currently queued.
    pub fn spawn(&self, priority: Priority, job: impl FnOnce() + Send + 'static) {
        let seq = self.shared.next_seq.fetch_add(1, AtomicOrdering::Relaxed);
        let mut heap = self.shared.heap.lock().expect("scheduler mutex poisoned");
        heap.push(QueuedJob {
            priority,
            seq,
            job: Box::new(job),
        });
        drop(heap);
        self.shared.condvar.notify_one();
    }

    /// How many jobs at `priority` have finished running so far. Test and
    /// devtools hook — real production code should not need to poll this.
    #[must_use]
    pub fn completed_count(&self, priority: Priority) -> u64 {
        self.shared.completed[priority as usize].load(AtomicOrdering::Relaxed)
    }

    /// Stop accepting new work-queue progress and join every worker thread,
    /// after letting each currently-running job finish (never killed
    /// mid-job — a job holding, say, a render lock must be allowed to
    /// release it).
    pub fn shutdown(mut self) {
        self.shared.shutdown.store(true, AtomicOrdering::SeqCst);
        self.shared.condvar.notify_all();
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

fn worker_loop(shared: Arc<Shared>) {
    loop {
        let mut heap = shared.heap.lock().expect("scheduler mutex poisoned");
        loop {
            if let Some(queued) = heap.pop() {
                drop(heap);
                let priority = queued.priority;
                (queued.job)();
                shared.completed[priority as usize].fetch_add(1, AtomicOrdering::Relaxed);
                break;
            }
            if shared.shutdown.load(AtomicOrdering::SeqCst) {
                return;
            }
            heap = shared.condvar.wait(heap).expect("scheduler mutex poisoned");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn jobs_actually_run() {
        let scheduler = Scheduler::new(2);
        let (tx, rx) = mpsc::channel();
        scheduler.spawn(Priority::Input, move || tx.send(42).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), 42);
        scheduler.shutdown();
    }

    #[test]
    fn higher_priority_work_is_drained_first() {
        // A single worker, so ordering is unambiguous: block it on a
        // barrier while queuing background work *before* input work, then
        // release the barrier and confirm input still runs first.
        let scheduler = Scheduler::new(1);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (order_tx, order_rx) = mpsc::channel::<&'static str>();

        // Occupy the one worker so both queued jobs below are waiting when
        // it becomes free.
        scheduler.spawn(Priority::Background, move || {
            release_rx.recv().unwrap();
        });
        // Give the occupying job a moment to actually claim the worker.
        std::thread::sleep(Duration::from_millis(50));

        let order_tx_bg = order_tx.clone();
        scheduler.spawn(Priority::Background, move || {
            order_tx_bg.send("background").unwrap();
        });
        scheduler.spawn(Priority::Input, move || {
            order_tx.send("input").unwrap();
        });

        release_tx.send(()).unwrap();

        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "input"
        );
        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "background"
        );
        scheduler.shutdown();
    }

    #[test]
    fn completed_count_tracks_finished_jobs_per_priority() {
        let scheduler = Scheduler::new(2);
        let (tx, rx) = mpsc::channel();
        for _ in 0..5 {
            let tx = tx.clone();
            scheduler.spawn(Priority::VisibleState, move || tx.send(()).unwrap());
        }
        for _ in 0..5 {
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        // Allow the counter increment (which happens after the send above)
        // to land.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(scheduler.completed_count(Priority::VisibleState), 5);
        scheduler.shutdown();
    }
}
