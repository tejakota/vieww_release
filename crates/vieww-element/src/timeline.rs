//! Timeline orchestration: coordinated animation sequences.
//!
//! # Why this lives in vieww-element and not vieww-animation
//!
//! A timeline drives [`Signal`]s — it writes to them every tick, and the
//! widget tree rebuilds in response. `Signal` is defined here;
//! `vieww-animation` sits below us and cannot name it. The timeline
//! *uses* springs from `vieww-animation` but its output mechanism is
//! the reactive layer, so here it lives.
//!
//! # The shape
//!
//! A timeline is a tree of steps:
//!
//! - An [`Action`] — animate one signal from here to there
//! - A [`Callback`] — run code at a point in the sequence
//! - A group that runs its children **in parallel**, **in sequence**,
//!   or **staggered** `interval` apart
//!
//! The tree is data, not closures, because data can be inspected and
//! cancelled. The builder API (`TimelineBuilder`) is the ergonomic way
//! to produce the tree.
//!
//! # What "duration" means for a sequence
//!
//! A spring's settle time depends on stiffness, damping, and initial
//! velocity — it cannot be computed in advance. Sequences therefore
//! advance to the next child when the previous *starts*, not when it
//! finishes. For staggered list reveals this is exactly what you want:
//! the reveal cascades rather than marching in lock-step.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use vieww_animation::{SpringAnimation as Spring, SpringPreset, Ticker, Tickers};

use crate::Signal;

/// One thing to animate: a signal, a from, a to.
#[derive(Clone)]
pub struct Action {
    /// The signal to drive. The timeline writes to it every tick;
    /// subscribers rebuild as usual.
    pub target: Signal<f32>,
    /// Where the value starts.
    ///
    /// The *current* value at play time is ignored — a timeline whose
    /// result depends on when you pressed play is a timeline nobody
    /// can reason about.
    pub from: f32,
    /// Where the value ends.
    pub to: f32,
    /// How far into the timeline this action begins.
    pub delay: Duration,
    /// The feel of the motion.
    pub preset: SpringPreset,
}

impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Action")
            .field("from", &self.from)
            .field("to", &self.to)
            .field("delay", &self.delay)
            .finish()
    }
}

/// A closure that can be cloned but run only once.
///
/// Shared so a step tree can be cloned; `Option` so the first tick to reach it
/// can `take` the closure and call it.
pub type CallbackSlot = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

/// Code to run at a point in the timeline.
///
/// For the 5% of sequences that need side effects a spring cannot
/// express: focus a node, play a sound, log an event.
pub struct Callback {
    pub at: Duration,
    /// The closure, in a slot that can be cloned but run only once.
    ///
    /// A bare `Box<dyn FnOnce()>` cannot be cloned, and `Timeline::play`
    /// clones the step tree so one timeline can be played more than once.
    /// `Rc<RefCell<Option<_>>>` keeps both properties: clones share the slot,
    /// and whichever tick reaches it first `take`s the closure and runs it.
    pub run: CallbackSlot,
}

impl Callback {
    /// A callback that fires `at` into the timeline.
    #[must_use]
    pub fn new<F: FnOnce() + 'static>(at: Duration, run: F) -> Self {
        Self {
            at,
            run: Rc::new(RefCell::new(Some(Box::new(run) as Box<dyn FnOnce()>))),
        }
    }
}

impl Clone for Callback {
    fn clone(&self) -> Self {
        Self {
            at: self.at,
            run: Rc::clone(&self.run),
        }
    }
}

impl std::fmt::Debug for Callback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Callback").field("at", &self.at).finish()
    }
}

/// One node in a timeline tree.
#[derive(Debug, Clone)]
pub enum Step {
    /// Animate one value.
    Action(Action),
    /// Run code.
    Callback(Callback),
    /// Run children simultaneously. The group finishes when the last child does.
    Parallel(Vec<Step>),
    /// Run children one after another.
    Sequence(Vec<Step>),
    /// Run children `interval` apart, in order.
    ///
    /// The workhorse of list reveals: fifteen rows, 30ms apart, each
    /// springing in — the whole thing takes 450ms plus one spring's
    /// settle time, and looks hand-crafted.
    Stagger {
        interval: Duration,
        children: Vec<Step>,
    },
}

impl Step {
    /// The total *nominal* duration of this subtree: how long until
    /// every action has been given its target.
    ///
    /// This is not the settle time — that depends on the spring — but
    /// it is what sequences use to decide when the next child begins.
    fn total_delay(&self) -> Duration {
        match self {
            Step::Action(a) => a.delay,
            Step::Callback(c) => c.at,
            Step::Parallel(children) => children
                .iter()
                .map(Step::total_delay)
                .max()
                .unwrap_or_default(),
            Step::Sequence(children) => children.iter().map(Step::total_delay).sum(),
            Step::Stagger { interval, children } => {
                let count = children.len().saturating_sub(1);
                children
                    .iter()
                    .map(Step::total_delay)
                    .max()
                    .unwrap_or_default()
                    + *interval * count as u32
            }
        }
    }
}

/// A built, ready-to-run animation sequence.
///
/// # Examples
///
/// A staggered list reveal — the single most common use:
///
/// ```ignore
/// let rows: Vec<Signal<f32>> = (0..5).map(|_| runtime.signal(0.0)).collect();
///
/// let timeline = TimelineBuilder::new()
///     .stagger(Duration::from_millis(30), |s| {
///         for row in &rows {
///             s.action(row, 0.0, 1.0);
///         }
///     })
///     .build();
///
/// let handle = timeline.play(&mut tickers);
/// ```
#[derive(Debug)]
pub struct Timeline {
    root: Step,
}

impl Timeline {
    /// The root step, for inspection.
    #[must_use]
    pub fn step(&self) -> &Step {
        &self.root
    }

    /// Run this timeline, registering it with `tickers`.
    ///
    /// Returns a [`TimelineHandle`] that can cancel. The timeline runs
    /// to completion on its own — the handle is for the rare caller
    /// that needs to stop it.
    ///
    /// Takes no `Runtime`: every [`Action`] holds a [`Signal`], and a signal
    /// already knows the runtime it belongs to. The parameter that used to be
    /// here was never read.
    pub fn play(&self, tickers: &mut Tickers) -> TimelineHandle {
        let player = Rc::new(RefCell::new(Player::new(self.root.clone())));
        tickers.add(&player);
        TimelineHandle { player }
    }

    /// Register this timeline with `tickers` **without starting it**, and
    /// return a handle that can start it later.
    ///
    /// # Why this exists
    ///
    /// [`play`](Self::play) needs `&mut Tickers` at the moment the animation
    /// begins, and a widget callback has no route to the ticker collection —
    /// `App::run` hands the driver to a setup closure, and `before_frame`
    /// yields a `&FrameLog`. So a timeline could choreograph an entrance and
    /// could not respond to a tap, which is the one thing every other framework
    /// treats as hello-world.
    ///
    /// The fix is the shape [`Animation`](crate::Animation),
    /// [`ScrollController`](crate::ScrollController) and
    /// [`NavigatorController`](crate::NavigatorController) already use: take
    /// the collection **once**, at setup, where a `&mut` is available; expose
    /// `&self` methods afterwards. [`TimelinePlayer`] is `Clone` and holds only
    /// an `Rc`, so it moves into a handler like any other captured state.
    ///
    /// The registration is permanent for the handle's lifetime — an idle player
    /// ticks to `false` and costs nothing, and `Tickers` holds a `Weak`, so
    /// dropping the last handle unregisters it on the next frame.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use vieww_animation::{SpringPreset, Tickers};
    /// use vieww_element::{Runtime, TimelineBuilder};
    ///
    /// let runtime = Runtime::new();
    /// let mut tickers = Tickers::new();
    /// let opacity = runtime.signal(0.0f32);
    ///
    /// // Setup: the one place with a `&mut Tickers`.
    /// let reveal = TimelineBuilder::new()
    ///     .animate(&opacity, 0.0, 1.0, SpringPreset::Standard)
    ///     .build()
    ///     .attach(&mut tickers);
    ///
    /// // A tap handler. No `&mut` anywhere.
    /// let on_pressed = move || reveal.play();
    /// on_pressed();
    ///
    /// tickers.advance(Duration::from_millis(16));
    /// assert!(opacity.peek() >= 0.0);
    /// ```
    pub fn attach(&self, tickers: &mut Tickers) -> TimelinePlayer {
        let player = Rc::new(RefCell::new(Player::idle()));
        tickers.add(&player);
        TimelinePlayer {
            root: self.root.clone(),
            player,
        }
    }
}

/// A timeline registered with a ticker collection, playable from anywhere.
///
/// Returned by [`Timeline::attach`]. Cloning gives another handle to the *same*
/// registration, so a handler and a `before_frame` hook can hold one each.
///
/// Every method takes `&self`, which is the whole point — see
/// [`Timeline::attach`] for why.
#[derive(Clone, Debug)]
pub struct TimelinePlayer {
    /// Kept so a replay can build a fresh [`Player`] from the same description.
    /// A [`Step`] is `Clone` and holds `Signal`s, which are shared cells — so a
    /// replay writes the same signals the first run did, rather than a copy of
    /// them.
    root: Step,
    player: Rc<RefCell<Player>>,
}

impl TimelinePlayer {
    /// Start the timeline, or restart it from the beginning if it is running.
    ///
    /// **Restart rather than ignore**, because the caller is a button: tapping
    /// "Reveal" twice should replay the reveal, not silently do nothing the
    /// second time. A caller that wants the other behaviour has
    /// [`is_running`](Self::is_running) to check first.
    ///
    /// The replacement is a whole new `Player`, so `start` is `None` again
    /// and the next tick anchors the run to that frame's timestamp — the same
    /// vsync-relative timing a fresh [`Timeline::play`] would get.
    pub fn play(&self) {
        *self.player.borrow_mut() = Player::new(self.root.clone());
    }

    /// Stop the timeline and leave every value where it is.
    pub fn cancel(&self) {
        self.player.borrow_mut().cancelled = true;
    }

    /// `true` while the timeline is still running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        let player = self.player.borrow();
        !player.cancelled && player.running
    }
}

/// Control over a playing timeline.
#[derive(Clone, Debug)]
pub struct TimelineHandle {
    player: Rc<RefCell<Player>>,
}

impl TimelineHandle {
    /// Stop the timeline and leave values where they are.
    pub fn cancel(&self) {
        self.player.borrow_mut().cancelled = true;
    }

    /// `true` while the timeline is still running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        !self.player.borrow().cancelled && self.player.borrow().running
    }
}

/// The running state of one timeline.
///
/// Implements `Ticker` so the existing frame pipeline advances it.
struct Player {
    /// The springs that have been started, with their target signals.
    springs: Vec<SpringEntry>,
    /// Callbacks not yet fired, with their absolute fire time.
    callbacks: Vec<(Duration, CallbackSlot)>,
    /// Steps waiting for their turn (used by Sequence and Stagger).
    pending: Vec<(Duration, Step)>,
    /// Actions not yet started, with their absolute start time.
    actions: Vec<(Duration, Action)>,
    /// Whether the timeline has finished or been cancelled.
    running: bool,
    cancelled: bool,
    /// When the timeline started. `None` until the first tick.
    start: Option<Duration>,
}

/// One active spring and the signal it writes to.
struct SpringEntry {
    spring: Spring,
    signal: Signal<f32>,
    target: f32,
}

impl Player {
    fn new(root: Step) -> Self {
        let mut player = Self {
            springs: Vec::new(),
            callbacks: Vec::new(),
            pending: Vec::new(),
            actions: Vec::new(),
            running: true,
            cancelled: false,
            start: None,
        };
        player.enqueue(Duration::ZERO, root);
        player
    }

    /// A player that is registered but has nothing to run.
    ///
    /// `running: false` from the start, so [`Ticker::is_animating`] is `false`
    /// and an attached-but-unplayed timeline does not keep the frame loop
    /// awake. [`TimelinePlayer::play`] replaces the whole value, which is why
    /// this needs no reset path of its own.
    fn idle() -> Self {
        Self {
            springs: Vec::new(),
            callbacks: Vec::new(),
            pending: Vec::new(),
            actions: Vec::new(),
            running: false,
            cancelled: false,
            start: None,
        }
    }

    /// Flatten a step into pending actions/callbacks/sub-steps.
    ///
    /// Sequences enqueue only their first child; the rest chain when
    /// that one's delay elapses. Parallels enqueue everything at once.
    /// Staggers enqueue with cumulative offsets.
    fn enqueue(&mut self, base: Duration, step: Step) {
        match step {
            Step::Action(action) => {
                self.actions.push((base + action.delay, action));
            }
            Step::Callback(callback) => {
                // `callback.run` is already the shared once-slot; keep the
                // handle rather than wrapping it again.
                self.callbacks.push((base + callback.at, callback.run));
            }
            Step::Parallel(children) => {
                for child in children {
                    self.enqueue(base, child);
                }
            }
            Step::Sequence(children) => {
                // Only the first child starts now; the rest chain.
                let mut children = children.into_iter().peekable();
                if let Some(first) = children.next() {
                    let first_duration = first.total_delay();
                    self.enqueue(base, first);
                    let mut offset = base + first_duration;
                    for child in children {
                        let d = child.total_delay();
                        self.pending.push((offset, child));
                        offset += d;
                    }
                }
            }
            Step::Stagger { interval, children } => {
                for (i, child) in children.into_iter().enumerate() {
                    let at = base + interval * i as u32;
                    self.enqueue(at, child);
                }
            }
        }
    }
}

impl Ticker for Player {
    /// Run the whole timeline to its end on this frame: see
    /// [`Ticker::settle`](vieww_animation::Ticker::settle).
    ///
    /// A timeline is a *schedule* of motion, so settling it is not one jump but
    /// the whole programme executed at once, in order: every remaining action
    /// lands on its target and every remaining callback fires. Firing the
    /// callbacks is the part that is easy to leave out and expensive to have
    /// left out — a stagger's completion handler is what dismisses the sheet or
    /// enables the button, and a reduced-motion user who never receives it is
    /// left looking at an interface that has stopped responding.
    fn settle(&mut self, _now: Duration) -> bool {
        if self.cancelled {
            self.running = false;
            return false;
        }

        // Springs already in flight land on the target they were retargeted to.
        for entry in &mut self.springs {
            entry.signal.set(entry.target);
        }
        self.springs.clear();

        // Actions that had not started yet skip straight to their end, in
        // schedule order — an action written later in the timeline is allowed to
        // overwrite the same signal, and running them out of order would leave
        // the earlier one's value standing.
        self.actions.sort_by_key(|(at, _)| *at);
        for (_, action) in std::mem::take(&mut self.actions) {
            action.target.set(action.to);
        }

        // Then the callbacks, also in schedule order, and also with the borrow
        // released before the call — a callback that touches this timeline would
        // otherwise panic on re-entry, which is the same reason `tick` takes
        // them out in their own statement.
        self.callbacks.sort_by_key(|(at, _)| *at);
        for (_, run) in std::mem::take(&mut self.callbacks) {
            let taken = run.borrow_mut().take();
            if let Some(f) = taken {
                f();
            }
        }

        self.pending.clear();
        self.running = false;
        true
    }

    fn tick(&mut self, now: Duration) -> bool {
        if self.cancelled {
            self.running = false;
            return false;
        }

        let start = *self.start.get_or_insert(now);
        let elapsed = now.saturating_sub(start);

        // Start any actions whose time has come.
        let mut i = 0;
        while i < self.actions.len() {
            let (at, _) = &self.actions[i];
            if *at <= elapsed {
                let (_, action) = self.actions.remove(i);
                // Set the from value immediately, so the first frame
                // the subscriber rebuilds already shows the start of
                // the motion.
                action.target.set(action.from);
                let mut spring = Spring::new(action.from, action.preset);
                spring.retarget(action.to);
                self.springs.push(SpringEntry {
                    spring,
                    signal: action.target,
                    target: action.to,
                });
            } else {
                i += 1;
            }
        }

        // Fire callbacks whose time has come.
        let mut i = 0;
        while i < self.callbacks.len() {
            let (at, _) = &self.callbacks[i];
            if *at <= elapsed {
                let (_, run) = self.callbacks.remove(i);
                // Take the closure out in its own statement, so the `RefMut`
                // guard is dropped before `f()` runs. Calling it inside the
                // `if let` holds the borrow across the call — and a callback
                // that touched this timeline would then panic on re-entry.
                let taken = run.borrow_mut().take();
                if let Some(f) = taken {
                    f();
                }
            } else {
                i += 1;
            }
        }

        // Advance springs, writing to their signals.
        let mut any_spring_alive = false;
        for entry in &mut self.springs {
            if entry.spring.tick(now) {
                entry.signal.set(entry.spring.value());
                any_spring_alive = true;
            } else {
                // The spring has settled; ensure the signal holds the
                // final value.
                entry.signal.set(entry.target);
            }
        }
        // Drop settled springs.
        self.springs.retain(|e| e.spring.is_animating());

        // Enqueue any sequenced children whose turn has arrived.
        let mut i = 0;
        while i < self.pending.len() {
            let (at, _) = &self.pending[i];
            if *at <= elapsed {
                let (at, step) = self.pending.remove(i);
                self.enqueue(at, step);
            } else {
                i += 1;
            }
        }

        // Done when nothing is pending, animating, or waiting.
        let done = self.actions.is_empty()
            && self.callbacks.is_empty()
            && self.pending.is_empty()
            && !any_spring_alive;

        if done {
            self.running = false;
        }

        !done
    }

    fn is_animating(&self) -> bool {
        self.running && !self.cancelled
    }

    /// The earliest moment a frame would change anything here.
    ///
    /// # The defect this fixes
    ///
    /// A timeline is mostly **waiting**. A stagger of eight rows at eighty
    /// milliseconds apart spends the first six hundred milliseconds with one or
    /// two springs alive and six actions sitting on the clock; a sequence with a
    /// half-second pause in it spends that half-second doing nothing at all.
    /// With no deadline to offer, the default answer — `None`, meaning *as soon
    /// as possible* — was the whole of the event loop's information, so the loop
    /// ran a full frame every vsync for the entire duration of the wait,
    /// producing an identical picture each time.
    ///
    /// That is the same defect `Ticker::next_deadline` was introduced for, in
    /// the same shape: `viewwstudio`'s caret pinned an idle window at one core
    /// at 100%. This module is the other ticker in the framework that changes on
    /// a *schedule* rather than continuously, and it was never taught to say so.
    ///
    /// # Why an animating spring still answers `None`
    ///
    /// Because a spring genuinely does produce a different value on every frame,
    /// and the only honest deadline for it is the next one. So the answer is
    /// `None` whenever a spring is alive, and the soonest scheduled moment
    /// otherwise — which is exactly the interval a timeline spends idle, and the
    /// only interval a loop could have slept through anyway.
    fn next_deadline(&self, now: Duration) -> Option<Duration> {
        if !self.is_animating() {
            // Nothing is owed at all. The registry pairs this with
            // `is_animating`, which is what separates "no deadline" from "no
            // work"; answering a time here would ask for a frame to do nothing.
            return None;
        }
        if self.springs.iter().any(|entry| entry.spring.is_animating()) {
            return None;
        }

        // Everything left is scheduled against `start`, which is the first tick's
        // timestamp. Before that first tick there is no clock to measure from and
        // the next frame is the one that establishes it.
        let start = self.start?;

        let soonest = self
            .actions
            .iter()
            .map(|(at, _)| *at)
            .chain(self.callbacks.iter().map(|(at, _)| *at))
            .chain(self.pending.iter().map(|(at, _)| *at))
            .min()?;

        // Saturating, because a deadline already passed is "now" rather than a
        // negative duration — a frame that ran late has work owed immediately.
        Some(start.saturating_add(soonest).max(now))
    }
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("actions_pending", &self.actions.len())
            .field("springs_active", &self.springs.len())
            .field("running", &self.running)
            .finish()
    }
}

// No `impl Ticker for Rc<RefCell<Player>>`: `Tickers::add` is generic over
// `T: Ticker` and takes `&Rc<RefCell<T>>` itself, so `Player: Ticker` is all
// that is needed — and implementing a foreign trait for a foreign type would
// not be allowed anyway.

// ── The builder ────────────────────────────────────────────────────────────

/// Build a [`Timeline`] step by step.
///
/// # Examples
///
/// A staggered list reveal:
///
/// ```ignore
/// let timeline = TimelineBuilder::new()
///     .stagger(Duration::from_millis(30), |s| {
///         for row in &rows {
///             s.action(row, 0.0, 1.0);
///         }
///     })
///     .build();
/// ```
///
/// A sequence — fade in, then slide:
///
/// ```ignore
/// let timeline = TimelineBuilder::new()
///     .sequence(|seq| {
///         seq.action(&opacity, 0.0, 1.0)
///            .action(&offset, 20.0, 0.0)
///     })
///     .build();
/// ```
#[derive(Debug)]
pub struct TimelineBuilder {
    root: Vec<Step>,
}

impl Default for TimelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineBuilder {
    /// An empty timeline.
    #[must_use]
    pub fn new() -> Self {
        Self { root: Vec::new() }
    }

    /// Add a step at the top level (runs in parallel with other top-level steps).
    ///
    /// Named `push_step` rather than `add`: a method called `add` taking one
    /// argument and returning `Self` reads as `std::ops::Add`, which this is
    /// not.
    #[must_use]
    pub fn push_step(mut self, step: Step) -> Self {
        self.root.push(step);
        self
    }

    /// Animate `signal` from `from` to `to`.
    #[must_use]
    pub fn animate(self, signal: &Signal<f32>, from: f32, to: f32, preset: SpringPreset) -> Self {
        self.push_step(Step::Action(Action {
            target: signal.clone(),
            from,
            to,
            delay: Duration::ZERO,
            preset,
        }))
    }

    /// Run children simultaneously.
    #[must_use]
    pub fn parallel<F>(self, f: F) -> Self
    where
        F: FnOnce(ParallelBuilder) -> ParallelBuilder,
    {
        let inner = f(ParallelBuilder::new());
        self.push_step(Step::Parallel(inner.steps))
    }

    /// Run children one after another.
    #[must_use]
    pub fn sequence<F>(self, f: F) -> Self
    where
        F: FnOnce(SequenceBuilder) -> SequenceBuilder,
    {
        let inner = f(SequenceBuilder::new());
        self.push_step(Step::Sequence(inner.steps))
    }

    /// Run children `interval` apart.
    #[must_use]
    pub fn stagger<F>(self, interval: Duration, f: F) -> Self
    where
        F: FnOnce(StaggerBuilder) -> StaggerBuilder,
    {
        let inner = f(StaggerBuilder::new());
        self.push_step(Step::Stagger {
            interval,
            children: inner.steps,
        })
    }

    /// Finish building.
    ///
    /// A single top-level step is the step itself; several become an
    /// implicit parallel — that is what "I added several things to a
    /// timeline" means to a caller who did not group them.
    #[must_use]
    pub fn build(self) -> Timeline {
        let root = match self.root.len() {
            0 => Step::Parallel(Vec::new()),
            1 => self.root.into_iter().next().unwrap(),
            _ => Step::Parallel(self.root),
        };
        Timeline { root }
    }
}

/// Collects steps for a parallel group.
#[derive(Debug)]
pub struct ParallelBuilder {
    steps: Vec<Step>,
}

impl ParallelBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Animate a signal.
    #[must_use]
    pub fn action(mut self, signal: &Signal<f32>, from: f32, to: f32) -> Self {
        self.steps.push(Step::Action(Action {
            target: signal.clone(),
            from,
            to,
            delay: Duration::ZERO,
            preset: SpringPreset::Standard,
        }));
        self
    }

    /// Animate a signal with an explicit preset.
    #[must_use]
    pub fn action_with(
        mut self,
        signal: &Signal<f32>,
        from: f32,
        to: f32,
        preset: SpringPreset,
    ) -> Self {
        self.steps.push(Step::Action(Action {
            target: signal.clone(),
            from,
            to,
            delay: Duration::ZERO,
            preset,
        }));
        self
    }

    /// Add a nested step.
    #[must_use]
    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }
}

impl Default for ParallelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Collects steps for a sequential group.
#[derive(Debug)]
pub struct SequenceBuilder {
    steps: Vec<Step>,
}

impl SequenceBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Animate a signal, after the previous step completes.
    #[must_use]
    pub fn action(mut self, signal: &Signal<f32>, from: f32, to: f32) -> Self {
        self.steps.push(Step::Action(Action {
            target: signal.clone(),
            from,
            to,
            delay: Duration::ZERO,
            preset: SpringPreset::Standard,
        }));
        self
    }

    /// Animate a signal with an explicit preset.
    #[must_use]
    pub fn action_with(
        mut self,
        signal: &Signal<f32>,
        from: f32,
        to: f32,
        preset: SpringPreset,
    ) -> Self {
        self.steps.push(Step::Action(Action {
            target: signal.clone(),
            from,
            to,
            delay: Duration::ZERO,
            preset,
        }));
        self
    }

    /// Run code at this point.
    #[must_use]
    pub fn then<F: FnOnce() + 'static>(mut self, run: F) -> Self {
        self.steps
            .push(Step::Callback(Callback::new(Duration::ZERO, run)));
        self
    }
}

impl Default for SequenceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Collects steps for a staggered group.
#[derive(Debug)]
pub struct StaggerBuilder {
    steps: Vec<Step>,
}

impl StaggerBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Animate a signal. Starts `interval` after the previous child.
    #[must_use]
    pub fn action(mut self, signal: &Signal<f32>, from: f32, to: f32) -> Self {
        self.steps.push(Step::Action(Action {
            target: signal.clone(),
            from,
            to,
            delay: Duration::ZERO,
            preset: SpringPreset::Expressive,
        }));
        self
    }

    /// Animate a signal with an explicit preset.
    #[must_use]
    pub fn action_with(
        mut self,
        signal: &Signal<f32>,
        from: f32,
        to: f32,
        preset: SpringPreset,
    ) -> Self {
        self.steps.push(Step::Action(Action {
            target: signal.clone(),
            from,
            to,
            delay: Duration::ZERO,
            preset,
        }));
        self
    }
}

impl Default for StaggerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(16);

    /// **The regression test for the loop this used to pin awake.**
    ///
    /// A timeline is mostly waiting: a stagger of eight rows eighty milliseconds
    /// apart spends most of its life with the springs settled and the next
    /// action sitting on the clock. With no deadline to offer, the registry's
    /// answer was "as soon as possible", so the event loop ran a full frame
    /// every vsync throughout — producing an identical picture each time.
    ///
    /// This is the same defect and the same fix as `viewwstudio`'s caret, which
    /// held an idle window at one core at 100%. The measurement is the point:
    /// the codebase's rule is that an optimisation needs a test where the work
    /// is countable, so this counts frames rather than asserting the deadline
    /// exists.
    #[test]
    fn a_waiting_timeline_lets_the_loop_sleep_instead_of_polling() {
        let runtime = crate::Runtime::new();
        let (a, b) = (runtime.signal(0.0f32), runtime.signal(0.0f32));

        // Two actions a full second apart: after the first spring settles there
        // is nothing at all to do until the second starts.
        let timeline = TimelineBuilder::new()
            .stagger(Duration::from_secs(1), |s| {
                s.action(&a, 0.0, 1.0).action(&b, 0.0, 1.0)
            })
            .build();

        let mut tickers = Tickers::new();
        let _handle = timeline.play(&mut tickers);

        // Drive it the way an event loop does: advance to the deadline the
        // registry names, or by one frame when it names none.
        let mut now = Duration::ZERO;
        let mut frames = 0u32;
        while tickers.is_animating() && now < Duration::from_millis(2500) {
            let next = tickers
                .next_deadline(now)
                .unwrap_or_else(|| now.saturating_add(FRAME))
                .max(now.saturating_add(Duration::from_millis(1)));
            now = next;
            tickers.advance(now);
            frames += 1;
        }

        assert!(
            (a.peek() - 1.0).abs() < 0.1 && (b.peek() - 1.0).abs() < 0.1,
            "both actions still have to run to completion: a={} b={}",
            a.peek(),
            b.peek()
        );
        // At 60Hz, two and a half seconds of polling is ~150 frames. The
        // springs themselves legitimately need every frame while they move; the
        // second between them needs none.
        assert!(
            frames < 90,
            "the loop woke {frames} times; the second of dead air between the \
             two actions is being spent redrawing an unchanged picture"
        );
    }

    /// The deadline must not let the loop sleep through a *moving* spring: a
    /// spring produces a different value every frame, and the only honest
    /// answer for one is "the next frame".
    #[test]
    fn a_moving_spring_asks_for_every_frame() {
        let runtime = crate::Runtime::new();
        let value = runtime.signal(0.0f32);
        let timeline = TimelineBuilder::new()
            .animate(&value, 0.0, 1.0, SpringPreset::Standard)
            .build();

        let mut tickers = Tickers::new();
        let _handle = timeline.play(&mut tickers);
        tickers.advance(FRAME);

        assert_eq!(
            tickers.next_deadline(FRAME),
            None,
            "a spring in flight has no deadline but the next vsync"
        );
    }

    #[test]
    fn a_timeline_animates_a_signal() {
        let runtime = crate::Runtime::new();
        let opacity = runtime.signal(0.0f32);

        let timeline = TimelineBuilder::new()
            .animate(&opacity, 0.0, 1.0, SpringPreset::Standard)
            .build();

        let mut tickers = Tickers::new();
        let _handle = timeline.play(&mut tickers);

        // Run frames until the timeline finishes.
        let mut t = Duration::ZERO;
        for _ in 0..120 {
            t += FRAME;
            tickers.advance(t);
        }

        assert!(
            (opacity.peek() - 1.0).abs() < 0.1,
            "arrived: {}",
            opacity.peek()
        );
    }

    #[test]
    fn staggered_children_start_at_different_times() {
        let runtime = crate::Runtime::new();
        let a = runtime.signal(0.0f32);
        let b = runtime.signal(0.0f32);

        let timeline = TimelineBuilder::new()
            .stagger(Duration::from_millis(100), |s| {
                s.action(&a, 0.0, 1.0).action(&b, 0.0, 1.0)
            })
            .build();

        let mut tickers = Tickers::new();
        let _handle = timeline.play(&mut tickers);

        // After ~50ms, `a` should be animating but `b` should not have started.
        let mut t = Duration::ZERO;
        for _ in 0..4 {
            t += FRAME;
            tickers.advance(t);
        }

        assert!(a.peek() > 0.0, "a started: {}", a.peek());
        assert_eq!(b.peek(), 0.0, "b has not started yet");
    }

    #[test]
    fn cancelling_stops_the_animation() {
        let runtime = crate::Runtime::new();
        let value = runtime.signal(0.0f32);

        let timeline = TimelineBuilder::new()
            .animate(&value, 0.0, 100.0, SpringPreset::Standard)
            .build();

        let mut tickers = Tickers::new();
        let handle = timeline.play(&mut tickers);

        // Run a few frames, then cancel.
        let mut t = Duration::ZERO;
        for _ in 0..5 {
            t += FRAME;
            tickers.advance(t);
        }
        let mid = value.peek();
        handle.cancel();

        for _ in 0..50 {
            t += FRAME;
            tickers.advance(t);
        }

        assert_eq!(value.peek(), mid, "stopped where it was cancelled");
        assert!(!handle.is_running());
    }

    #[test]
    fn sequence_runs_children_in_order() {
        let runtime = crate::Runtime::new();
        let first = runtime.signal(0.0f32);
        let second = runtime.signal(0.0f32);

        let timeline = TimelineBuilder::new()
            .sequence(|seq| seq.action(&first, 0.0, 1.0).action(&second, 0.0, 1.0))
            .build();

        // The sequence advances when the first action *starts* — meaning
        // the nominal delay of the first action. Both actions have
        // delay=0, so they both start immediately.
        let mut tickers = Tickers::new();
        let _handle = timeline.play(&mut tickers);

        let mut t = Duration::ZERO;
        for _ in 0..60 {
            t += FRAME;
            tickers.advance(t);
        }

        // Both should have arrived.
        assert!((first.peek() - 1.0).abs() < 0.1);
        assert!((second.peek() - 1.0).abs() < 0.1);
    }

    // ── Timeline::attach — playing from a handler ─────────────────────────

    /// The gap this closes: `play` needs `&mut Tickers`, and a widget callback
    /// has none. `attach` takes the collection once and hands back something a
    /// handler can own.
    #[test]
    fn an_attached_timeline_plays_from_a_closure_that_holds_no_ticker() {
        let runtime = crate::Runtime::new();
        let opacity = runtime.signal(0.0f32);

        let mut tickers = Tickers::new();
        let player = TimelineBuilder::new()
            .animate(&opacity, 0.0, 1.0, SpringPreset::Standard)
            .build()
            .attach(&mut tickers);

        // Exactly what a `Button::on_pressed` closure gets: a `'static` value
        // captured by move, with no borrow of anything on the frame driver.
        let on_pressed: Box<dyn Fn()> = Box::new({
            let player = player.clone();
            move || player.play()
        });

        // Frames before the tap must not move the signal — an attached
        // timeline is registered, not running.
        let mut t = Duration::ZERO;
        for _ in 0..10 {
            t += FRAME;
            tickers.advance(t);
        }
        assert_eq!(opacity.peek(), 0.0, "attaching must not start the timeline");
        assert!(!player.is_running());

        on_pressed();

        for _ in 0..120 {
            t += FRAME;
            tickers.advance(t);
        }
        assert!(
            (opacity.peek() - 1.0).abs() < 0.01,
            "the handler's timeline must have run: {}",
            opacity.peek()
        );
    }

    /// An idle attached timeline must not keep the frame loop awake, or every
    /// screen with a tappable reveal on it would render at 60fps forever.
    #[test]
    fn an_attached_timeline_is_not_animating_until_it_is_played() {
        let runtime = crate::Runtime::new();
        let value = runtime.signal(0.0f32);

        let mut tickers = Tickers::new();
        let player = TimelineBuilder::new()
            .animate(&value, 0.0, 1.0, SpringPreset::Standard)
            .build()
            .attach(&mut tickers);

        assert!(!tickers.is_animating(), "idle before play");
        assert!(!tickers.advance(FRAME), "an idle tick reports no change");

        player.play();
        assert!(tickers.is_animating(), "animating after play");
    }

    /// Tapping the button twice replays, rather than doing nothing the second
    /// time. The restart has to re-anchor the clock too — a run that kept the
    /// first run's `start` would think it was already finished.
    #[test]
    fn playing_again_restarts_from_the_beginning() {
        let runtime = crate::Runtime::new();
        let value = runtime.signal(0.0f32);

        let mut tickers = Tickers::new();
        let player = TimelineBuilder::new()
            .animate(&value, 0.0, 1.0, SpringPreset::Standard)
            .build()
            .attach(&mut tickers);

        let mut t = Duration::ZERO;
        player.play();
        for _ in 0..120 {
            t += FRAME;
            tickers.advance(t);
        }
        assert!((value.peek() - 1.0).abs() < 0.01, "first run arrived");

        // Seconds later — a real second tap is never at t = 0.
        player.play();
        t += FRAME;
        tickers.advance(t);
        assert!(
            value.peek() < 0.5,
            "the replay must snap back to the start, not stay at the end: {}",
            value.peek()
        );

        for _ in 0..120 {
            t += FRAME;
            tickers.advance(t);
        }
        assert!((value.peek() - 1.0).abs() < 0.01, "second run arrived");
    }

    /// Dropping the last handle has to unregister the player, or a screen that
    /// is navigated away from leaks a ticker per visit.
    #[test]
    fn dropping_the_last_handle_unregisters_the_player() {
        let runtime = crate::Runtime::new();
        let value = runtime.signal(0.0f32);

        let mut tickers = Tickers::new();
        {
            let player = TimelineBuilder::new()
                .animate(&value, 0.0, 1.0, SpringPreset::Standard)
                .build()
                .attach(&mut tickers);
            player.play();
            assert_eq!(tickers.len(), 1);
        }

        tickers.advance(FRAME);
        assert_eq!(tickers.len(), 0, "the weak entry must be dropped");
    }

    #[test]
    fn cancelling_an_attached_timeline_leaves_the_value_where_it_is() {
        let runtime = crate::Runtime::new();
        let value = runtime.signal(0.0f32);

        let mut tickers = Tickers::new();
        let player = TimelineBuilder::new()
            .animate(&value, 0.0, 1.0, SpringPreset::Standard)
            .build()
            .attach(&mut tickers);

        player.play();
        let mut t = Duration::ZERO;
        for _ in 0..6 {
            t += FRAME;
            tickers.advance(t);
        }
        let midway = value.peek();
        assert!(midway > 0.0 && midway < 1.0, "midway: {midway}");

        player.cancel();
        assert!(!player.is_running());
        for _ in 0..60 {
            t += FRAME;
            tickers.advance(t);
        }
        assert_eq!(value.peek(), midway, "a cancelled timeline writes nothing");
    }
}
