# Onto a device

The preview is for judging a screen quickly. **Build and
Run** is for the real thing: your whole project, compiled
as an actual binary, launched or packaged for a target you
choose.

## The targets

```text
Desktop
Windows
Android
iOS
```

`Desktop` builds for the machine the studio is running on.
`Windows` cross-compiles a `.exe` from any host, which is a
separate row because it needs its own Rust target and its
own linker rather than reusing the desktop's. `Android` and
`iOS` need their platform's own SDK — the studio checks for
each tool before starting a build, and if one is missing it
names the tool and the command that installs it, rather
than letting the build fail partway through with a linker
error about something that was never there.

## What each one produces

Desktop and Windows builds are copied straight out of
`target/release` into an export directory beside your
project — an ordinary optimised binary, ready to hand
someone.

Android produces a **debug-signed** `.apk`. Debug-signed,
and named as such: a release-signed APK needs a keystore, a
key alias, and two passwords, none of which belong in a
text field that logs to an Output panel. A debug-signed APK
installs on any phone with developer mode switched on,
which covers "give this to a colleague to try" without the
studio ever holding a signing secret.

## Onto a phone

With a device connected and developer mode on, **Build and
Run** installs and launches the app directly rather than
only producing a file — the same command either way, with
the target already chosen by what is plugged in.

## The one rule that is not about a missing tool

iOS builds require a Mac. That is a rule of Apple's
toolchain, not a gap this studio could close with an
installer, and it is reported as exactly that on any other
host — a plain statement of the requirement, not a missing
dependency to go looking for.
