# State

Three mechanisms, and a rule for choosing between them.

| You want | Use |
|---|---|
| A value several widgets read, that changes | A `Signal<T>` |
| Something one widget owns and must *tear down* | `create_state` |
| A value published down a subtree | `Inherited<T>` |

The rule: **start with a signal**. Reach for element state only when
something needs disposing — a timer, a subscription, an animation controller
— and for `Inherited` only when the value is genuinely ambient, like a theme.

## Signals

A signal is a value that lives outside the tree. Reading one during `build`
subscribes that widget; writing one marks exactly the widgets that read it.

```rust
let count = driver.elements().runtime().signal(0i32);

// Reading — inside build(), this is the subscription.
let value = count.get();

// Writing — from anywhere, including an event handler.
count.set(5);
count.update(|n| *n += 1);

// Reading without subscribing, for a handler that only wants the value.
let current = count.peek();
```

### Why there is no dependency array

Because the read *is* the registration. There is no list of dependencies to
keep in sync with the body of a function, so the class of bug where they
disagree does not exist. This is also why `peek` is a separate method: it is
how you say "I want the value and I do not want to be woken when it changes",
which in a callback-registration API is the thing people forget.

### What actually rebuilds

Only the elements that read the signal. Not their parents, not their
children, not the tree. You can watch it:

```sh
cargo run -p vieww --example counter
```

which prints `builds=` per node before and after a write.

### `Memo` for derived values

```rust
let total = runtime.memo(move || items.get().iter().map(|i| i.price).sum::<u32>());
```

A memo recomputes when what it read changed, and — importantly — only
notifies its own readers if the *result* changed. A derived value that
happens to come out the same does not rebuild anything.

## Element state

For anything with a lifecycle. `create_state` runs once when the element
mounts; the state is dropped when it unmounts.

```rust
#[derive(Debug)]
struct Ticker;

#[derive(Debug, Default)]
struct TickerState {
    elapsed: Duration,
}

impl ElementState for TickerState {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    // Called every frame while `is_animating` is true.
    fn tick(&mut self, now: Duration) -> bool {
        self.elapsed = now;
        true // "I changed; rebuild me"
    }

    fn is_animating(&self) -> bool { true }

    // The reason to use element state at all: somewhere to hang the teardown.
    fn dispose(&mut self) {
        // close the subscription, cancel the timer, stop the controller
    }
}

#[widget]
impl Ticker {
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(TickerState::default()))
    }

    fn build(&self, ctx: &BuildContext) -> impl Into<WidgetNode> {
        let elapsed = ctx.state(|s: &TickerState| s.elapsed).unwrap_or_default();
        Text::new(format!("{:.1}s", elapsed.as_secs_f32()))
    }
}
```

Note `ctx.state(...)` returns an `Option` — a widget's `build` can be called
in contexts where no element backs it (a tree dump, for instance), and the
honest answer there is "there is no state", not a panic.

## Inherited values

`Inherited<T>` publishes a value to a whole subtree; anything below reads it
by type, from its own position.

```rust
Theme::new(ThemeData::dark()).child(app)
```

and anywhere below:

```rust
let theme = ThemeData::of(ctx);
```

This is how `Theme`, `Directionality`, `Localizations` and `WindowSizeClass`
all work — there is one mechanism, not four. Reading an inherited value
subscribes that widget to changes in it, the same way a signal does.

Publish your own with `Inherited::new(value, child)`, and read it with
`ctx.inherit::<T>()`.

Use it for genuinely ambient things. A value only two widgets need is a
parameter, and passing it is clearer than making it ambient.

## Loading something

vieww has **no executor and no `async fn`** — deliberately. An application
doing anything slow has usually chosen a runtime already, and being made to
run a second one is worse than having none. The seam is one trait:

```rust
pub trait Spawn: Send + Sync + 'static {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>);
}
```

tokio, smol, rayon and a bare `std::thread` all satisfy that in one line. If
you have no runtime, use the one that ships:

```rust
use vieww::foundation::task::Pool;

// One worker per core, bounded. Create it once and keep it.
let pool = Arc::new(Pool::new());
```

`Pool` is a fixed set of threads sharing a queue — not a runtime, no futures,
no I/O reactor, and no dependency. Prefer it to `Threads`, which starts one
OS thread per task and will happily start two hundred.

Then `AsyncBuilder` does the rest:

```rust
AsyncBuilder::new(
    pool.clone(),
    waker.clone(),
    || fetch_the_thing(),                 // runs off the UI thread
    |value| match value {                 // runs on the UI thread, in build
        AsyncValue::Pending => Skeleton::new().into(),
        AsyncValue::Ready(v) => Text::new(v.title.clone()).into(),
        AsyncValue::Failed(e) => InlineError::new(e.to_string()).into(),
        AsyncValue::Lost => InlineError::new("the task did not finish").into(),
    },
)
```

### The waker, and why it is not optional

A task's result is noticed during a frame. An application waiting on a
network response is not drawing frames — nothing is animating and nobody is
touching the screen — so without a waker the response sits in a channel until
the user happens to prod the window. "It updates when you touch it" is not a
loading state.

`vieww-platform-winit`'s `App::waker()` gives you one wired to its event
loop. Use it.

### Why a signal cannot be written from a worker

`Signal<T>` holds `Rc<RefCell<T>>` and is `!Send` by construction. The whole
tree is single-threaded, which is what makes rebuilds cheap and reference
counting free of atomics. So a result crosses back through a channel and is
applied on the UI thread during a frame, where writing is allowed. That is
what `AsyncBuilder` is doing for you.

## `AsyncValue::Lost`

The fourth variant is the one worth knowing about: it means the worker
panicked or vanished without producing a result. It exists because the
alternative — a task that stays `Pending` forever — is a spinner that never
stops, which is the hardest kind of bug to notice in a screenshot and the
easiest to notice in a support ticket. Handle it.
