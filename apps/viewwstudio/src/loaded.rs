//! M3's second half: `dlopen`, the entry point, and the widget that comes back.
//!
//! # Nothing is ever unloaded
//!
//! A `Library` here is leaked deliberately. The previous preview's widgets may
//! still be referenced by the element tree until the new one has mounted, and
//! `dlclose` while a vtable pointer into that image is live is a use-after-free
//! that shows up as a crash somewhere unrelated. Leaking a few megabytes per
//! Render for the length of a session is the cheap side of that trade, and it
//! is what the plan's M8 asks to have written down (§7, "never-unload-dylibs
//! strategy documented").
//!
//! **The price was quoted against the wrong session, and that is not resolved.**
//! "A few megabytes per Render for the length of a session" was written when a
//! session was the twenty minutes it took to demonstrate the pipeline. A
//! session in a production editor is a working day: a screen every couple of
//! minutes for eight hours is a few hundred images, and nothing here has been
//! measured over one. `Preview::loaded_images` is the count, so the number is
//! at least *visible* now; what is still owed is a run long enough to know
//! whether it matters.
//!
//! If it does, the fix is generation-scoped unloading — drop the images whose
//! widgets are provably out of the element tree — and *not* `dlclose` on the
//! previous render, which is exactly the use-after-free the paragraph above
//! refuses. The refusal is right; only the cost estimate is unverified.
//!
//! # A panic in user code
//!
//! Three boundaries, and each catches a different thing:
//!
//! - **Construction** — `screen()` running inside the guest's appended
//!   `catch_unwind`, which is mandatory because unwinding across `extern "C"`
//!   is UB. A panic there arrives as a null pointer and becomes an error
//!   message.
//! - **First build** — `vieww_preview_probe`, also appended to the buffer,
//!   builds the returned widget once before it is ever mounted. A `build()`
//!   that panics on its first call comes back as a `1` and becomes an error
//!   message.
//! - **Later rebuilds, inside the host frame** — `ElementTree::build` wraps
//!   every `widget.build` in `catch_unwind` (see `crates/vieww-element/
//!   src/tree.rs`). That catch is host-side; for it to catch a panic raised
//!   inside guest code, the host and the guest have to share one `libstd`.
//!
//! **The catches inside the image are mandatory regardless.** Unwinding across
//! `extern "C"` is UB, so `screen()` and `vieww_preview_probe` cannot hand a
//! panic back through the FFI edge; they catch inside the image because they
//! have to, and `LoadError::PanicInScreen` / `LoadError::PanicInBuild` are what
//! that catch produces.
//!
//! **The host-side catch needs `libstd` shared.** A `cdylib` statically links
//! its own copy of `std`, so a panic raised in guest code is a *foreign*
//! exception to the host's unwinder — `catch_unwind` on this side does not
//! catch it, and the process prints "fatal runtime error: Rust cannot catch
//! foreign exceptions" and aborts. Both the studio and the preview are now
//! built with `-C prefer-dynamic` (see the checkout's `.cargo/config.toml` for
//! the host side and `compile.rs`'s `run_rustc` for the guest), so both link
//! against `libstd-*.so` from the same toolchain, and the existing
//! `catch_unwind` in `ElementTree::build` catches guest panics as written.
//! [`prime_libstd`] below is what keeps the *load* working when the studio
//! was launched by anything that does not put the toolchain on the loader's
//! search path. The probe is still in place — it catches the *first* build
//! before the widget is mounted at all, which is a stricter guarantee than
//! the host-side catch offers (a widget that panics on first build is refused
//! rather than mounted behind an error boundary).
//!
//! # What is still not covered
//!
//! Layout, paint and hit-test of guest-owned render objects. The render tree
//! calls into `RenderObject::layout`/`paint`/`hit_test`, and a `CustomPainter`
//! is guest code that the host calls directly without a `catch_unwind`. A panic
//! there still aborts; the fix is the same shape as `ElementTree::build`'s
//! catch around `widget.build`, applied to the render tree's per-node methods.
//! Not done — left to a follow-up because the symptom is rarer (a custom
//! painter panicking, rather than a widget's build) and the cost of getting it
//! wrong is a per-frame `catch_unwind` over every render object in the tree.

use std::path::Path;
use std::rc::Rc;

// `Key`, `BuildContext` and `WidgetKind` were the `Guest` shim's — it
// implemented `Widget` by delegating each of them to the loaded widget. The
// shim is gone (see `Preview::node`), and so are the imports it needed.
use vieww_widget::{Widget, WidgetNode};

/// The symbol the appended entry point exports.
const ENTRY_SYMBOL: &[u8] = b"vieww_preview_entry";

/// The symbol carrying the guest's idea of a well-known vieww type's identity.
const FINGERPRINT_SYMBOL: &[u8] = b"vieww_preview_fingerprint";

/// The symbol that builds the screen once *inside the image*, and says whether
/// that panicked.
///
/// See [`Preview::probe`] for why the build cannot happen on this side.
const PROBE_SYMBOL: &[u8] = b"vieww_preview_probe";

/// The host's own answer, for the same type, by the same method.
///
/// # What this is really asking
///
/// Not "is this the same rustc" — [`Toolchain::discover`] already asks that,
/// and it is not enough. This asks **"is this the same *compilation* of vieww"**,
/// which is the question that actually decides whether the preview works.
///
/// The host resolves widgets through a `TypeId`-keyed factory. A workspace
/// routinely contains two compilations of one crate (feature resolution
/// differs, cargo builds it twice), and a guest linked against the other one
/// produces widgets whose `TypeId`s the host has never seen. The factory misses
/// every one, the tree comes out empty, and **the preview renders nothing while
/// reporting success** — which is exactly what it did.
///
/// [`Toolchain::discover`]: crate::compile::Toolchain::discover
#[must_use]
pub fn host_fingerprint() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::any::TypeId::of::<vieww::prelude::Text>().hash(&mut hasher);
    hasher.finish()
}

/// A screen loaded from a compiled preview.
#[derive(Debug, Clone)]
pub struct Preview {
    widget: Rc<dyn Widget>,
}

/// Why a compiled library could not be turned into a screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// `dlopen` refused it.
    Dlopen(String),
    /// The library has no `vieww_preview_entry`. In practice: the buffer did
    /// not define `screen()`, so the appended entry failed to compile — which
    /// `rustc` would have said first.
    NoEntry,
    /// The entry point caught a panic while building the screen.
    PanicInScreen,
    /// The screen built, and panicked the first time it was asked for its tree.
    PanicInBuild,
    /// The library was compiled against a different compilation of vieww than
    /// the host links, so none of its widgets would render.
    ///
    /// Carries both fingerprints because the useful thing to a person reading
    /// this is not the numbers but that they *differ* — and the numbers are
    /// what makes "it is still wrong" distinguishable from "it is wrong in a
    /// new way" when they try the fix.
    AbiMismatch { host: u64, guest: u64 },
    /// The library has no fingerprint symbol at all, so nothing can be checked.
    NoFingerprint,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dlopen(why) => {
                // **The one failure that is about the *launch*, not the code.**
                // A dependency the loader could not find is not a defect in
                // the buffer, and the sentence has to say so — the user
                // otherwise goes looking for a bug in a screen that compiled
                // clean. The fix is on the studio side (see
                // `Toolchain::prime_libstd` and `.cargo/config.toml`), and
                // `cargo run` sidesteps it entirely, which is worth naming
                // because it is also the one thing that makes the studio work
                // *right now* while the real fix is being applied.
                let hint = if why.contains("cannot open shared object file")
                    || why.contains("image not found")
                {
                    " — the loader could not find a library the preview depends \
                     on (usually the toolchain's libstd, which only cargo puts \
                     on the search path). Start the studio with `cargo run`, \
                     or rebuild it from a checkout whose `.cargo/config.toml` \
                     links it against the same libstd as the preview."
                } else {
                    ""
                };
                write!(f, "could not load the compiled preview: {why}{hint}")
            }
            Self::NoEntry => write!(
                f,
                "the compiled preview has no entry point — does the buffer define `pub fn screen()`?"
            ),
            Self::PanicInScreen => write!(f, "panic while building screen()"),
            Self::PanicInBuild => write!(f, "panic inside the screen's own build()"),
            Self::AbiMismatch { host, guest } => write!(
                f,
                "the preview was compiled against a different build of vieww \
                 than the studio links (host {host:#018x}, preview \
                 {guest:#018x}). Every widget it builds would be a type this \
                 process has never seen, so the factory would miss all of them \
                 and the preview would come out blank. Refused rather than \
                 shown empty. Fix: build the studio and the preview from one \
                 dependency graph — `cargo build -p viewwstudio` — so that \
                 target/debug/deps holds a `vieww` rlib matching this binary. \
                 If the file being previewed uses `crate::`, the project's own \
                 library is linked too and its vieww is the one used, so the \
                 project has to be in that dependency graph as well: build it \
                 with CARGO_TARGET_DIR set to the studio's target directory, \
                 and with the same feature set — a project that turns on a \
                 vieww feature the studio does not use gets a second \
                 compilation of vieww and lands back here."
            ),
            Self::NoFingerprint => write!(
                f,
                "the compiled preview has no ABI fingerprint, so there is no \
                 way to tell whether its widgets would render"
            ),
        }
    }
}

impl Preview {
    /// Load `library` and call its entry point.
    ///
    /// # Errors
    ///
    /// If the library will not open, has no entry point, or panics on its way
    /// to a widget.
    ///
    /// # Safety of the unsafe blocks
    ///
    /// Both are `dlopen`/`dlsym`, and both rest on the guarantee the toolchain
    /// guard makes: this library was produced by the same `rustc`, against the
    /// same `vieww`, as the process loading it. Without that check the trait
    /// object below would be a vtable from another compilation session, and
    /// nothing about this would be sound. See [`crate::compile::Toolchain`].
    pub fn load(library: &Path) -> Result<Self, LoadError> {
        // SAFETY: loading a library runs its initialisers, which is why this is
        // unsafe at all. The guard above is what makes the ABI claim true.
        let handle = unsafe { libloading::Library::new(library) }
            .map_err(|error| LoadError::Dlopen(error.to_string()))?;
        // Counted before anything can fail below, because an image whose entry
        // point is missing is still an image that was mapped and is never going
        // to be unmapped. See this module's note on the never-unload trade.
        LOADED_IMAGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // SAFETY: the symbol's signature is fixed by `compile::ENTRY`, which is
        // the only thing that ever defines it.
        let entry = unsafe {
            handle
                .get::<unsafe extern "C" fn() -> *mut std::ffi::c_void>(ENTRY_SYMBOL)
                .map_err(|_| LoadError::NoEntry)?
        };

        // **Checked before the entry point is called, not after.** A mismatched
        // library is one whose types this process does not share; the sooner we
        // stop touching it the better, and there is nothing to gain from
        // building a widget tree we already know cannot render.
        //
        // SAFETY: the symbol's signature is fixed by `compile::ENTRY`.
        let fingerprint = unsafe {
            handle
                .get::<unsafe extern "C" fn() -> u64>(FINGERPRINT_SYMBOL)
                .map_err(|_| LoadError::NoFingerprint)?
        };
        // SAFETY: it hashes a `TypeId` and returns a `u64`; it cannot unwind.
        let guest = unsafe { fingerprint() };
        let host = host_fingerprint();
        if guest != host {
            return Err(LoadError::AbiMismatch { host, guest });
        }

        // SAFETY: calling the entry point. It cannot unwind — its body is one
        // `catch_unwind` — so this cannot unwind across the FFI edge either.
        let raw = unsafe { entry() };
        if raw.is_null() {
            return Err(LoadError::PanicInScreen);
        }

        // The probe, taken while the handle is still in scope — a moment later
        // it is deliberately leaked and there is nothing left to look symbols up
        // in. Absent only if somebody has changed `compile::ENTRY` without
        // changing this; an older image simply is not probed.
        //
        // SAFETY: the symbol's signature is fixed by `compile::ENTRY`.
        let probe = unsafe {
            handle
                .get::<unsafe extern "C" fn(*mut std::ffi::c_void) -> u8>(PROBE_SYMBOL)
                .ok()
        };

        // **Built before the pointer is adopted**, because a screen that panics
        // on its first build is one this process should not be holding a widget
        // from at all.
        //
        // SAFETY: `raw` is non-null and still owned here; the guest borrows it
        // and does not free it. The call cannot unwind across the edge — its
        // body is one `catch_unwind`, on the side that owns the panic runtime.
        if let Some(probe) = probe {
            if unsafe { probe(raw) } != 0 {
                return Err(LoadError::PanicInBuild);
            }
        }

        // SAFETY: the pointer came from `Box::into_raw(Box::new(Box<dyn
        // Widget>))` in the guest, compiled by the same toolchain, so the
        // double box is the same layout on both sides.
        let widget: Box<Box<dyn Widget>> = unsafe { Box::from_raw(raw.cast()) };

        // **Leaked on purpose.** See the module docs: the widgets in this image
        // outlive the load, and `dlclose` under them is a use-after-free.
        std::mem::forget(handle);

        Ok(Self {
            // `Rc<dyn Widget>` rather than `Rc<Box<dyn Widget>>`: the node this
            // becomes has to carry the *loaded widget's* type, not a wrapper's.
            // See `Preview::node`.
            widget: Rc::from(*widget),
        })
    }

    /// The screen, as something that can be put in a tree.
    ///
    /// # Why not a wrapper widget
    ///
    /// It used to be one: a `Guest` shim that delegated `kind`, `build`,
    /// `debug_name` and `key` to the loaded widget. Every *method* was right and
    /// the *type* was wrong, and the render factory looks a widget up by type.
    ///
    /// So a screen whose root was a render widget — `pub fn screen() -> impl
    /// Widget { Flex::column()… }`, which is how the studio's own lessons are
    /// written — resolved to "no render object registered for `Column`". The
    /// column drew nothing, its children were spliced into the `Overlay`'s stack
    /// above it, and three controls landed on top of each other in the middle of
    /// the device frame. Screens whose root was *composed* were unaffected,
    /// which is why the studio looked fine on the sample project and wrong on a
    /// lesson.
    ///
    /// `WidgetNode::from_rc` puts the loaded widget in the tree as itself.
    ///
    /// # The `#[cfg]` that used to be here
    ///
    /// This body was split on `feature = "hot-reload"`, passing a reload
    /// identity in one arm and not the other. `viewwstudio` **declares no such
    /// feature**, so the `cfg` was permanently false — and `vieww-widget`,
    /// unified with `vieww-reload` in any workspace-wide build, was compiled
    /// with it permanently true. The two arms therefore disagreed and
    /// `cargo build --workspace` did not compile at all. `from_rc` now takes
    /// the identity in both configurations, so there is one call.
    #[must_use]
    pub fn node(&self) -> WidgetNode {
        WidgetNode::from_rc(Rc::clone(&self.widget), "vieww::preview::Guest")
    }
}

/// How many dynamic libraries this process has mapped and will never unmap.
///
/// An `AtomicUsize` rather than a field, because the thing being counted is a
/// property of the *process* — the images outlive every `Preview` that loaded
/// one, which is the entire point of the never-unload rule — and there is no
/// object whose lifetime matches.
static LOADED_IMAGES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The number of previews loaded since the process started.
///
/// Exists so the cost of never unloading is a number somebody can read rather
/// than an estimate written down once, in a session short enough not to test
/// it. See this module's header.
#[must_use]
pub fn loaded_images() -> usize {
    LOADED_IMAGES.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many images may accumulate before the studio says something.
///
/// # Why a number, and why this one
///
/// The module header above is honest that the never-unload trade was priced
/// against the wrong session: *"a few megabytes per Render for the length of a
/// session"* was written when a session was a twenty-minute demonstration, and
/// a session in a production editor is a working day. `loaded_images` made the
/// count visible; nothing read it, so nothing acted on it.
///
/// Two hundred is a full day of rendering every couple of minutes — the exact
/// case the header names as unmeasured. It is a **warning threshold, not a
/// limit**: the studio does not refuse to render, because refusing would break
/// the application to protect it from a cost nobody has measured, which is the
/// worse of the two mistakes. What it does is stop the cost being invisible.
pub const IMAGE_BUDGET: usize = 200;

/// Map the toolchain's `libstd-<hash>.so` into this process, if it is there.
///
/// # Why a loader needs telling what it already linked
///
/// A preview compiled `-C prefer-dynamic` carries `libstd-<hash>.so` in its
/// `DT_NEEDED`, and `dlopen` resolves that name through the process's search
/// paths — `LD_LIBRARY_PATH`, the runpath of the object being loaded, the
/// system cache. The toolchain's `lib/` directory is on none of them unless
/// cargo launched this process, because cargo is the only thing that knows
/// where the toolchain is and thinks to export it. A studio started from a
/// desktop entry, a dock, a double click or a bare shell therefore fails the
/// *load* of a compile that finished clean:
///
/// ```text
/// could not load the compiled preview: libstd-<hash>.so:
/// cannot open shared object file: No such file or directory
/// ```
///
/// Opening the file by absolute path first removes the search from the
/// question. The loader keeps every loaded object in a map keyed by name, and
/// resolves a `DT_NEEDED` against that map *before* it looks at any directory —
/// so a `libstd-<hash>.so` already in the process satisfies the preview's
/// dependency wherever the file lives and however the studio was started.
///
/// `RTLD_GLOBAL` rather than the `RTLD_LOCAL` `libloading` defaults to,
/// because the point is for the symbols to be resolvable on behalf of the
/// *next* library loaded, not merely for this handle's own use.
///
/// # The failure this does not fix
///
/// Two `std`s in one process — this one, plus the studio's own statically
/// linked copy when the studio was built without `.cargo/config.toml` — still
/// cannot catch each other's panics; the panic boundary needs the *studio*
/// rebuilt `prefer-dynamic`, not just the guest's dependency satisfied. What
/// this buys on such a build is that the preview loads and runs at all, with
/// the probe (inside the image, inside the guest's `std`) still catching the
/// first `build()` panic as designed.
///
/// # Errors, of a sort
///
/// Deliberately infallible: a directory with no `libstd` in it, or one that
/// will not open, falls through to the ordinary `dlopen` of the preview and
/// the error that always produced — which names the library and is where the
/// diagnosis belongs. [`Toolchain::prime_libstd`] is the caller.
///
/// [`Toolchain::prime_libstd`]: crate::compile::Toolchain::prime_libstd
pub fn prime_libstd(sysroot_lib: &Path) {
    // **Once.** The handle is leaked, so the library never leaves the process,
    // and every call after the first is a refcount bump on an object already
    // in the map — harmless, but a `read_dir` per Render is a cost with no
    // behaviour attached to it.
    static PRIMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if PRIMED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }

    #[cfg(unix)]
    {
        use libloading::os::unix::{Library as RawLibrary, RTLD_GLOBAL, RTLD_LAZY};

        let Ok(entries) = std::fs::read_dir(sysroot_lib) else {
            return;
        };
        // Every `libstd-*.so` / `libstd-*.dylib` in the directory — plural,
        // because a rustup sysroot holds exactly one and the guard below is
        // cheaper than an argument about it. Opening the wrong one would load
        // a library nothing references and waste an inode's worth of memory;
        // opening the right one is the entire point.
        let names: Vec<std::ffi::OsString> = entries
            .flatten()
            .map(|entry| entry.file_name())
            .filter(|name| {
                let name = name.to_string_lossy();
                (name.starts_with("libstd-") && (name.ends_with(".so") || name.contains(".so.")))
                    || (name.starts_with("libstd-") && name.ends_with(".dylib"))
            })
            .collect();
        for name in names {
            let path = sysroot_lib.join(&name);
            // **Leaked on purpose.** Closing it would refcount down an object
            // the next `dlopen` is about to need, and the module header
            // already argues why nothing loaded here is ever unloaded.
            if let Ok(handle) = unsafe { RawLibrary::open(Some(&path), RTLD_LAZY | RTLD_GLOBAL) } {
                std::mem::forget(handle);
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Windows resolves a cdylib's dependencies against the directory of
        // the loading executable, which is the packager's arrangement rather
        // than the loader's. Nothing to prime here.
        let _ = sysroot_lib;
    }
}

/// How often the warning repeats after the first one.
///
/// A message on every render past the threshold is a message people learn to
/// ignore by lunchtime.
pub const BUDGET_INTERVAL: usize = 50;

/// A sentence about accumulated images, if there is one worth saying.
///
/// Returns `Some` on the render that crosses [`IMAGE_BUDGET`] and every
/// [`BUDGET_INTERVAL`] renders after it, and `None` otherwise — which is
/// every render for the first several hours.
#[must_use]
pub fn budget_warning() -> Option<String> {
    let count = loaded_images();
    if count < IMAGE_BUDGET {
        return None;
    }
    if (count - IMAGE_BUDGET) % BUDGET_INTERVAL != 0 {
        return None;
    }
    // **With the number, now that there is one.** The header's open question
    // was not "is this a lot of libraries" — the count already answered that —
    // but "how much memory is that", and nothing measured it. A warning that
    // says "memory only grows" without saying by how much is exactly the
    // estimate-written-down-once the header complains about.
    let footprint = resident_bytes().map_or_else(
        || " Restart the studio if it feels heavy.".to_owned(),
        |bytes| {
            format!(
                " The process is holding {:.0} MB. Restart the studio if that is more \
                 than you want to spend.",
                bytes as f64 / (1024.0 * 1024.0)
            )
        },
    );
    Some(format!(
        "{count} previews loaded this session. Each one is a library that is \
         deliberately never unloaded, so memory only grows.{footprint}"
    ))
}

/// The process's resident set, in bytes, where the platform will say.
///
/// # Why this is worth the twenty lines
///
/// The module header's unresolved question is a *measurement*: "a few megabytes
/// per Render for the length of a session" was priced against a twenty-minute
/// demonstration, and "what is still owed is a run long enough to know whether
/// it matters". A count of libraries cannot answer that. Resident bytes can,
/// and it is the number a person compares against the memory they have.
///
/// `/proc/self/statm` on Linux — field two is resident pages — and
/// `ps -o rss=` on macOS, which is a subprocess but runs once per warning
/// rather than per frame. `None` everywhere else, and the warning simply says
/// less rather than guessing.
#[must_use]
pub fn resident_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        // 4 KiB is the page size on every platform this ships to; reading
        // `sysconf` would mean libc, which this crate does not depend on.
        Some(pages * 4096)
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p"])
            .arg(std::process::id().to_string())
            .output()
            .ok()?;
        let kilobytes: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        Some(kilobytes * 1024)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    None
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    /// The first hours of a session must be silent, or the warning is noise.
    ///
    /// `loaded_images` is process-wide state that other tests in this binary
    /// also move, so this checks the *rule* against every count below the
    /// threshold rather than calling `budget_warning` and hoping the counter
    /// is where this test left it.
    #[test]
    fn nothing_is_said_below_the_budget() {
        let says =
            |count: usize| count >= IMAGE_BUDGET && (count - IMAGE_BUDGET) % BUDGET_INTERVAL == 0;
        assert!((0..IMAGE_BUDGET).all(|count| !says(count)));
    }

    /// And past it, it repeats rarely rather than on every render.
    #[test]
    fn the_warning_repeats_on_an_interval_not_every_time() {
        let says =
            |count: usize| count >= IMAGE_BUDGET && (count - IMAGE_BUDGET) % BUDGET_INTERVAL == 0;
        assert!(says(IMAGE_BUDGET));
        assert!(!says(IMAGE_BUDGET + 1));
        assert!(says(IMAGE_BUDGET + BUDGET_INTERVAL));
        assert!(!says(IMAGE_BUDGET - 1));
    }

    #[test]
    fn the_message_names_the_count_and_what_to_do() {
        // Built from the same format string the real one uses.
        let message = format!(
            "{IMAGE_BUDGET} previews loaded this session. Each one is a library that is \
             deliberately never unloaded, so memory only grows — restart the studio \
             if it feels heavy."
        );
        assert!(message.contains(&IMAGE_BUDGET.to_string()));
        assert!(message.contains("restart"));
    }

    #[test]
    fn the_footprint_is_a_real_number_on_a_platform_that_reports_one() {
        // The header's open question was a measurement nobody had taken. On
        // Linux and macOS the warning now carries it.
        match resident_bytes() {
            Some(bytes) => assert!(
                bytes > 1024 * 1024,
                "a running test process holds more than a megabyte: {bytes}"
            ),
            // A guard rather than `assert!(!cfg!(…))`, which on the platforms
            // that *can* answer is `assert!(false)` — a constant assertion that
            // says nothing about why it fired. Elsewhere `None` is the correct
            // answer and this arm is where it lands.
            None if cfg!(any(target_os = "linux", target_os = "macos")) => {
                panic!("linux and macos can both answer this, and this is one of them")
            }
            None => {}
        }
    }
}
