//! Opening a rebuilt library and asking it for a widget tree.
//!
//! **This is the part of hot reload that no test reaches.** Everything else in
//! this crate is arithmetic over timestamps and hashes; this is `dlopen`,
//! `dlsym`, and Rust values crossing a library boundary. It is kept as small as
//! it can be for exactly that reason.
//!
//! # The library is never closed
//!
//! `docs/HOT-RELOAD.md` §3, and it is not negotiable:
//!
//! > `Box<dyn ElementState>` and `dyn Widget` objects hold vtable pointers into
//! > the library that created them. `dlclose` makes every one of those dangling,
//! > and the tree is full of them.
//!
//! So every [`Guest`] leaks its library, deliberately, by
//! [`Library::into_raw`](libloading::Library::into_raw). A reload costs a library's worth of address space and
//! that is the price of the tree surviving. **This is why hot reload is
//! development-only** — not because it is unstable, but because the leak is
//! unbounded by design.
//!
//! It also rescues something subtler. `Widget::debug_name` returns
//! `&'static str`, and after a reload those point into the old library. They
//! stay valid *because* nothing is ever unloaded. Anyone who decides to be tidy
//! and call `dlclose` breaks every one of them at once.
//!
//! # Why both sides must be built together
//!
//! A `WidgetNode` crosses this boundary as a Rust value. Rust has no stable ABI,
//! so host and guest agree on its layout only if they were compiled by the same
//! compiler against the same `vieww-widget`. In practice that means **one cargo
//! workspace, one `cargo build`** — which is what a development loop is anyway.
//! A guest built separately, or against a different version, is undefined
//! behaviour rather than an error, and there is no check that can catch it here.

use std::fmt;
use std::path::{Path, PathBuf};

use vieww_widget::WidgetNode;

use crate::Fingerprint;

/// The symbol a guest exports to hand over its widget tree.
///
/// Returns an owning pointer the host takes back with `Box::from_raw`. A raw
/// pointer rather than the value, because `extern "C"` says nothing useful
/// about how a `WidgetNode` is returned — the pointer is the only part of this
/// signature the C ABI actually pins down.
pub const ROOT_SYMBOL: &[u8] = b"__vieww_guest_root\0";

/// The symbol a guest exports to declare the shape of its state.
///
/// A plain `u64` for the same reason: it is the widest thing that means the
/// same on both sides without agreement about layout.
pub const FINGERPRINT_SYMBOL: &[u8] = b"__vieww_guest_fingerprint\0";

type RootFn = unsafe extern "C" fn() -> *mut WidgetNode;
type FingerprintFn = unsafe extern "C" fn() -> u64;

/// Why a guest could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The library would not open.
    Open(PathBuf, libloading::Error),
    /// It opened but does not export what a guest must.
    ///
    /// Nearly always a missing `vieww_reload::guest!` in the library, or a
    /// crate that is not `crate-type = ["cdylib"]`.
    Symbol(&'static str, libloading::Error),
    /// It could not be copied to the directory it has to be loaded from.
    ///
    /// Only reachable where those are two different directories, which on a
    /// desktop they are not — see [`Watch::stage_in`](crate::Watch::stage_in).
    /// Distinguished from [`Open`](Self::Open) because "the library never
    /// arrived" and "the linker refused it" are opposite diagnoses, and on
    /// Android the first is much the likelier.
    Stage(PathBuf, std::io::Error),
    /// The library loaded, and **panicked while building its widget tree**.
    ///
    /// The one failure mode that is the *application's* rather than the
    /// toolchain's, and the reason it is reported rather than fatal: a developer
    /// mid-edit is exactly the person whose `unwrap` is about to fail, and
    /// taking the window down for it discards the running application, the
    /// session, and the reason they opened the reloader in the first place.
    ///
    /// The previous guest keeps running. See [`Guest::root`].
    Panicked(PathBuf),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(path, error) => write!(f, "could not open {}: {error}", path.display()),
            Self::Symbol(name, error) => write!(
                f,
                "{name} is missing: {error}. Does the library invoke \
                 `vieww_reload::guest!` and build as a cdylib?"
            ),
            Self::Stage(path, error) => {
                write!(f, "could not copy the guest to {}: {error}", path.display())
            }
            Self::Panicked(path) => write!(
                f,
                "{} panicked while building its root; the previous build is \
                 still running",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// A loaded application library.
///
/// Holds no `Library` handle, because it never closes one — see the module
/// docs. What it holds are two function pointers into a mapping that outlives
/// the process.
pub struct Guest {
    root: RootFn,
    fingerprint: Fingerprint,
    path: PathBuf,
}

impl fmt::Debug for Guest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Guest")
            .field("path", &self.path)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl Guest {
    /// Open a guest library and read what it exports.
    ///
    /// # Safety of what this does, stated rather than hidden behind `unsafe`
    ///
    /// This is a safe function that does an unsafe thing, and the reason it is
    /// not marked `unsafe` is that marking it would not help: no caller can
    /// discharge the obligation. Loading *any* native library runs its
    /// initialisers and trusts its contents, and the requirement here — that it
    /// was built from this workspace, by this compiler, in this `cargo build` —
    /// is a property of the build system rather than of the call site.
    ///
    /// It is development tooling, gated behind the application choosing to call
    /// it, and it is documented at every level rather than made to look
    /// dangerous at one.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref().to_path_buf();

        // SAFETY: the caller's build system guarantees this is our own guest,
        // built alongside this host. Nothing checkable at this line.
        let library = unsafe { libloading::Library::new(&path) }
            .map_err(|error| LoadError::Open(path.clone(), error))?;

        // SAFETY: the symbols are the ones `guest!` exports, with the signatures
        // it gives them.
        let (root, fingerprint) = unsafe {
            let root = *library
                .get::<RootFn>(ROOT_SYMBOL)
                .map_err(|e| LoadError::Symbol("__vieww_guest_root", e))?;
            let fingerprint = *library
                .get::<FingerprintFn>(FINGERPRINT_SYMBOL)
                .map_err(|e| LoadError::Symbol("__vieww_guest_fingerprint", e))?;
            (root, fingerprint())
        };

        // **Leaked on purpose.** Closing this would dangle every vtable pointer
        // in the element tree. See the module docs; this line is the whole of
        // that decision.
        std::mem::forget(library);

        Ok(Self {
            root,
            fingerprint: Fingerprint::from_bits(fingerprint),
            path,
        })
    }

    /// The shape of state this guest declared.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Where it was loaded from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Ask the guest to build its widget tree.
    ///
    /// Called once per reload rather than once per frame: the result is a
    /// description the host then owns and rebuilds from, exactly as if the
    /// application had called `set_root` itself.
    ///
    /// # Errors
    ///
    /// [`LoadError::Panicked`] if the guest panicked while building. Nothing
    /// else can fail here — the symbol was resolved at load time.
    ///
    /// # The abort this replaced
    ///
    /// `__vieww_guest_root` is `extern "C"`, and a panic that reaches an
    /// `extern "C"` boundary **aborts the process**. So a guest with a failing
    /// `unwrap` in its build did not leave the previous screen standing, as the
    /// documentation here promised and as every other failure path in this crate
    /// genuinely does — it took the whole window down, along with the session
    /// the developer had built up to reach the screen they were editing.
    ///
    /// The guard is in the `guest!` macro, on the guest's side of the boundary,
    /// which is the only side it can be on: unwinding has to be stopped *before*
    /// it crosses. A panicking build returns null, which is what this reads.
    pub fn root(&self) -> Result<WidgetNode, LoadError> {
        // SAFETY: `guest!` returns either `Box::into_raw(Box::new(node))` — an
        // owning pointer to a live `WidgetNode` that nothing else holds — or
        // null, when its `catch_unwind` caught a panic. Freed by the host's
        // allocator, which is the guest's too: one process, one allocator.
        let raw = unsafe { (self.root)() };
        if raw.is_null() {
            return Err(LoadError::Panicked(self.path.clone()));
        }
        Ok(unsafe { *Box::from_raw(raw) })
    }
}

/// Export the two symbols a guest library must provide.
///
/// ```ignore
/// // in the application's `cdylib`
/// vieww_reload::guest! {
///     root: build_screen,
///     state: [CounterState, ScrollState],
/// }
/// ```
///
/// `root` names a `fn() -> impl Into<WidgetNode>`. `state` names every type the
/// application keeps in an `ElementState` or a `Signal` — the list a reload is
/// only safe across if none of them changed shape. Leaving a type out does not
/// fail to compile; it fails to *notice*, which is why the list sits next to the
/// root function rather than anywhere else.
#[macro_export]
macro_rules! guest {
    (root: $root:path $(, state: [$($state:ty),* $(,)?])? $(,)?) => {
        /// Hand the host a freshly built widget tree.
        ///
        /// # Safety
        ///
        /// Called only by `vieww_reload::Guest::root`, which takes ownership of
        /// the returned pointer.
        #[no_mangle]
        pub unsafe extern "C" fn __vieww_guest_root() -> *mut $crate::WidgetNode {
            // **The unwind guard, and it has to be on this side.**
            //
            // This function is `extern "C"`, and a panic that reaches an
            // `extern "C"` boundary aborts the process. A host cannot catch it —
            // by the time control would return there, the runtime has already
            // decided. So the catch is here, inside the guest, before the
            // boundary.
            //
            // Without it a failing `unwrap` in a reloaded build took the whole
            // window down, which is the opposite of what a hot reloader is for:
            // the developer whose build panics is precisely the one mid-edit,
            // and the previous screen surviving is the entire value of the
            // feature.
            //
            // `AssertUnwindSafe` because building a widget tree reads the
            // application's own statics and signals, and there is nothing here
            // that could observe a torn value afterwards — the panicking build
            // is discarded whole and the host keeps the guest it already had.
            //
            // Null on panic; `Guest::root` turns that into
            // `LoadError::Panicked`. The payload is deliberately dropped inside
            // the guest rather than carried across: a `Box<dyn Any>` allocated
            // by one dylib and dropped by another is exactly the kind of thing
            // that works until a build changes an allocator.
            let built = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                let node: $crate::WidgetNode = ::core::convert::Into::into($root());
                ::std::boxed::Box::into_raw(::std::boxed::Box::new(node))
            }));
            match built {
                ::core::result::Result::Ok(raw) => raw,
                ::core::result::Result::Err(_) => ::core::ptr::null_mut(),
            }
        }

        /// Declare the shape of this build's state.
        ///
        /// # Safety
        ///
        /// Called only by `vieww_reload::Guest::load`.
        #[no_mangle]
        pub unsafe extern "C" fn __vieww_guest_fingerprint() -> u64 {
            $crate::Fingerprint::of(&[
                $($((
                    ::core::any::type_name::<$state>(),
                    ::core::mem::size_of::<$state>(),
                    ::core::mem::align_of::<$state>(),
                )),*)?
            ])
            .to_bits()
        }
    };
}
