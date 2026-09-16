# Ordinary Rust

A buffer here is not a template or a restricted subset —
it is normal Rust, compiled by the real compiler, with the
real borrow checker and the real type errors. Anything you
could write in a `.rs` file in any other project, you can
write here: functions, structs, enums, traits, generics,
closures, iterators, `match`.

## Your own functions

`screen()` is just where the preview starts reading. Split
the work into ordinary functions the way you would in any
crate:

```rust
fn greeting(hour: u32) -> &'static str {
    match hour {
        0..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

pub fn screen() -> impl Widget {
    Text::new(greeting(9))
}
```

## Tests

`#[test]` works exactly as it does anywhere else. Write one
beside the widget it checks, and the Problems panel reports
a failure the same way it reports a compile error.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morning_before_noon() {
        assert_eq!(
            greeting(9),
            "Good morning",
        );
    }
}
```

## The one limit on a preview

The preview compiles your buffer as a small dynamic
library and loads it into the running studio, so it can
rebuild in well under a second instead of relinking the
whole application. That is fast because it is *not* a full
program: there is no `main`, no argument parsing, no
network access initiated from inside the sandboxed build,
and no writing to disk from the widget you are previewing.
None of that is a rule about the language — it is normal
Rust the whole way down — it is a property of what a
preview is allowed to do while it is only being looked at,
not shipped.

That limit stops applying the moment you leave the preview:
**Build and Run** compiles your project as the real,
complete binary — with `main`, with the platform's own
services, with everything a shipped app can do — and either
launches it or produces the artefact you asked for. The next
two pages cover what that step sees and what it can produce.
