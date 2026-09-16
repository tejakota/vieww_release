# Say: a second way to write a screen

Every buffer so far in this guide has been ordinary Rust. A
buffer named `*.say` is different: it's Say, an
English-facing screen language, and the studio compiles it
by first turning it into the same kind of Rust file you'd
otherwise write by hand.

```text
counter.say ──say-codegen──▶ counter.rs ──studio rustc──▶ cdylib ──dlopen──▶ preview
```

Say changes what's *in* the buffer, not how a buffer becomes
a preview — the generated file is compiled exactly the way
[Build and Run](./05-build.md) already compiles any other
buffer.

## A whole screen

```text
keep a whole number called count starting at 0

screen "Home":
    a column, spaced 16, children aligned to the start:
        a heading "Counter"
        a label "Tapped \(count) times"
        a button "Add one" which when tapped:
            add 1 to count
```

`keep` declares state that survives a recompile — the studio
remounts your screen on every change while you're editing,
and a `keep`ed value is carried across that remount instead
of resetting to its starting value each time.
`\(count)` interpolates a value into text, the same job
`format!` does in a Rust buffer.

## The vocabulary

Say is a fixed set of phrases, not a general-purpose
language — that's deliberate, and it's what lets the studio
report a mistake precisely instead of a Rust compiler error
pointing at generated code you never wrote. Four families:

- **Widgets** — `a card`, `a row`, `a column`, `a text`, `a
  button`, `a text field`, `a list of`, and the rest of the
  studio's widget set (see [Widgets](./02-widgets.md) for
  what each one is).
- **Properties**, after a comma on a widget line —
  `padded N`, `bold`, `with the primary color`,
  `only if …`, and others.
- **Actions**, inside a `which when tapped:` block —
  `set n to E`, `toggle b`, `open the screen "X"`,
  `show a snackbar "M"`.
- **Icons** — ten built-in icons, `the check icon` through
  `the forward chevron`.

## When a phrase isn't understood

Say reports every problem in a file at once, not just the
first, so you see everything wrong in one pass instead of
fixing one line at a time across repeated Renders. Each
message names the exact phrase and where it is; a phrase the
language has reserved for a later version — rather than one
that's simply wrong — says so explicitly instead of guessing
at what you meant.

## Reading the generated Rust

Every line the generator writes carries a `// say:
file:line` comment pointing back at the `.say` line that
produced it, so if you ever need to read the compiled output
— debugging, or curiosity — you can find your way back to
what you actually wrote.

## Why bother, if it's "just" Rust underneath

A `.say` screen has no borrow checker to satisfy and no
Rust syntax to get exactly right — the fixed vocabulary
above is the entire surface you need. It's a narrower tool
than [Ordinary Rust](./03-rust.md) on purpose: reach for a
`.rs` buffer the moment you need a function, a type, or
anything outside that vocabulary, and reach for `.say` when
a screen is squarely inside it.
