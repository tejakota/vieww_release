//! Loading a real, compiled plugin `cdylib` off disk and calling into it.
//!
//! Everything in [`crate::abi`] is the *shape* of the boundary; this module
//! is the one place that actually crosses it — `dlopen`ing a shared library,
//! resolving [`abi::ENTRY_SYMBOL`], checking [`AbiVersion::is_compatible_with`]
//! before touching anything else, and wrapping the raw [`PluginVTable`] in a
//! [`LoadedPlugin`] whose safe methods are the only way callers outside this
//! module ever reach it again.
//!
//! # Keeping the library alive as long as the vtable is used
//!
//! Every function pointer in a loaded [`PluginVTable`] is code mapped into
//! this process by the `dlopen` call that produced it — once the
//! [`libloading::Library`] that opened it is dropped, those pointers point
//! into unmapped memory. [`LoadedPlugin`] therefore holds the `Library` and
//! the vtable together, and its `Drop` impl runs the plugin's own shutdown
//! sequence (`shutdown` then `drop`, in that order — see [`PluginVTable`]'s
//! own doc for why) *before* the automatic field drop closes the library, so
//! nothing ever calls through a pointer into an already-unmapped module.

use std::fmt;
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};

use crate::abi::{
    self, AbiStr, AbiVersion, HostVTable, PluginCommand, PluginVTable, STATUS_OK,
    STATUS_UNKNOWN_COMMAND,
};

/// Why loading or invoking a plugin failed.
#[derive(Debug)]
pub enum PluginError {
    /// The shared library itself could not be opened (missing file, wrong
    /// architecture, unresolved symbols other than the entry point).
    Open(libloading::Error),
    /// The library opened, but does not export [`abi::ENTRY_SYMBOL`] — it is
    /// not a `vieww` plugin at all, or was built against an incompatible
    /// macro version that named its entry point differently.
    MissingEntrySymbol(libloading::Error),
    /// [`abi::EntryFn`] returned a null pointer instead of a real vtable.
    NullVTable,
    /// The plugin declares an [`AbiVersion`] this host cannot safely call
    /// into. Nothing beyond reading `abi_version` itself — always the first
    /// field of [`PluginVTable`], by invariant, in every version — was
    /// touched before this was detected.
    IncompatibleVersion {
        plugin: AbiVersion,
        host: AbiVersion,
    },
    /// The plugin's own [`crate::abi::Plugin::init`] returned `Err`. The
    /// plugin has already been sent `shutdown` and unloaded by the time this
    /// is returned — see [`crate::abi::Plugin::init`]'s own doc for why
    /// `shutdown` is still owed after a failed `init`.
    InitFailed,
    /// [`LoadedPlugin::invoke`] was asked for a command id this plugin does
    /// not have.
    UnknownCommand(String),
    /// The plugin's own [`crate::abi::Plugin::invoke`] returned `Err`. This
    /// ABI version does not carry error text back across the boundary — see
    /// [`crate::abi::Plugin::invoke`]'s own doc for why.
    InvokeFailed(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(err) => write!(f, "could not open plugin library: {err}"),
            Self::MissingEntrySymbol(err) => write!(
                f,
                "not a vieww plugin (no {} export): {err}",
                String::from_utf8_lossy(abi::ENTRY_SYMBOL)
            ),
            Self::NullVTable => write!(f, "plugin entry point returned a null vtable"),
            Self::IncompatibleVersion { plugin, host } => write!(
                f,
                "plugin ABI v{}.{} is incompatible with this host's v{}.{}",
                plugin.major, plugin.minor, host.major, host.minor
            ),
            Self::InitFailed => write!(f, "plugin's init() returned an error"),
            Self::UnknownCommand(id) => write!(f, "plugin has no command {id:?}"),
            Self::InvokeFailed(id) => write!(f, "plugin's invoke({id:?}) returned an error"),
        }
    }
}

impl std::error::Error for PluginError {}

/// A single plugin `cdylib`, loaded and initialized, ready to be invoked.
///
/// Constructed only by [`PluginRegistry::load`] — there is no public
/// constructor that skips the version check or the `init` call.
pub struct LoadedPlugin {
    /// Kept alive for as long as `vtable`'s function pointers are called
    /// through — see this module's own doc for why dropping it early would
    /// be unsound. Never read directly after construction; its only job is
    /// outliving `vtable`.
    _library: Library,
    vtable: *mut PluginVTable,
    path: PathBuf,
    name: String,
    version: String,
    commands: Vec<PluginCommand>,
}

// SAFETY: a `LoadedPlugin` is never accessed from two threads at once by
// anything in this crate (every method takes `&mut self` or is read-only over
// already-owned, copied-out data), and the vtable's own contract already
// requires calls into one instance to be sequential, never re-entered — see
// `crate::abi::instance_mut`'s own doc. `Library` and a raw pointer are
// otherwise `!Send`/`!Sync` only because their auto traits can't see that.
unsafe impl Send for LoadedPlugin {}

impl fmt::Debug for LoadedPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("path", &self.path)
            .field("name", &self.name)
            .field("version", &self.version)
            .field("commands", &self.commands)
            .finish_non_exhaustive()
    }
}

impl LoadedPlugin {
    /// The path this plugin was loaded from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The plugin's own reported name, copied out at load time.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The plugin's own reported version string, copied out at load time.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The commands this plugin offers — a snapshot taken once, immediately
    /// after a successful `init`. See [`PluginVTable`]'s own doc for why a
    /// plugin cannot change its command list after that point in this ABI
    /// version.
    #[must_use]
    pub fn commands(&self) -> &[PluginCommand] {
        &self.commands
    }

    /// Run the command named `id`.
    ///
    /// # Errors
    /// [`PluginError::UnknownCommand`] if `id` matches none of
    /// [`Self::commands`] — checked against the snapshot here as well as by
    /// the trampoline on the plugin's own side, so a caller gets the same
    /// answer [`Self::commands`] already promised without a round trip
    /// across the ABI first. [`PluginError::InvokeFailed`] if the plugin's
    /// own handler ran and returned an error.
    pub fn invoke(&mut self, id: &str) -> Result<(), PluginError> {
        if !self.commands.iter().any(|command| command.id == id) {
            return Err(PluginError::UnknownCommand(id.to_owned()));
        }
        // SAFETY: `self.vtable` is a live pointer produced by a successful
        // `load`, and `_library` (kept alive by `self`) still maps the code
        // its function pointers point into.
        let status =
            unsafe { ((*self.vtable).invoke)((*self.vtable).instance, AbiStr::borrowed(id)) };
        match status {
            STATUS_OK => Ok(()),
            STATUS_UNKNOWN_COMMAND => Err(PluginError::UnknownCommand(id.to_owned())),
            _ => Err(PluginError::InvokeFailed(id.to_owned())),
        }
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        // SAFETY: `self.vtable` is live and `_library` — dropped only after
        // this fn returns, per Rust's field-drop order following a custom
        // `Drop::drop` — still maps the code both calls run.
        unsafe {
            ((*self.vtable).shutdown)((*self.vtable).instance);
            ((*self.vtable).drop)((*self.vtable).instance);
        }
        // `self.vtable` itself (the small, fixed `PluginVTable` allocation)
        // is never freed — see `leak_vtable`'s own doc for why that leak is
        // permanent and bounded to one allocation per load, not per call.
    }
}

/// A collection of loaded plugins, keeping each one's [`Library`] alive for
/// exactly as long as its [`LoadedPlugin`] is kept.
///
/// Deliberately thin: this is a `Vec` with a `load` that does the unsafe
/// dance once, not a plugin *manager* with discovery, dependency ordering, or
/// hot-swapping — those are all real, separate concerns a host built on top
/// of this can add without this crate guessing their shape.
#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
}

impl PluginRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the `cdylib` at `path`, check its ABI version, run its `init`
    /// with `host`, and — if all three succeed — keep it loaded and return a
    /// reference to it.
    ///
    /// # Errors
    /// See [`PluginError`]'s variants. On any error the library is unloaded
    /// before returning (a version mismatch skips straight to unloading,
    /// touching nothing else on the vtable; a failed `init` first sends the
    /// plugin its owed `shutdown` — see [`crate::abi::Plugin::init`]'s own
    /// doc — then unloads).
    ///
    /// # Safety
    /// `path` must name a genuine `vieww` plugin `cdylib` built against a
    /// compatible `vieww-plugin` version. Loading and running arbitrary
    /// native code is inherently unsafe in the way every `dlopen` is: this
    /// function cannot verify the library does not do something else
    /// entirely with its `vieww_plugin_entry` export before the version
    /// check ever gets a chance to refuse it.
    pub unsafe fn load(
        &mut self,
        path: impl AsRef<Path>,
        host: &HostVTable,
    ) -> Result<&LoadedPlugin, PluginError> {
        let path = path.as_ref().to_path_buf();
        let library = unsafe { Library::new(&path) }.map_err(PluginError::Open)?;

        // SAFETY: `abi::ENTRY_SYMBOL` is the fixed, documented export name
        // every plugin built against this crate provides as an
        // `abi::EntryFn`; a mismatched signature here is the "not actually a
        // vieww plugin" case `MissingEntrySymbol` exists for.
        let entry: Symbol<abi::EntryFn> =
            unsafe { library.get(abi::ENTRY_SYMBOL) }.map_err(PluginError::MissingEntrySymbol)?;

        // SAFETY: calling a plugin's declared entry point, which by contract
        // returns either null or a pointer to a `PluginVTable` it leaks for
        // the life of the process (see `leak_vtable`'s own doc).
        let vtable = unsafe { entry() };
        if vtable.is_null() {
            return Err(PluginError::NullVTable);
        }

        // Reading only `abi_version` — always the first field, by this
        // crate's own versioning invariant — before anything else is
        // touched, exactly as this module's doc promises.
        // SAFETY: `vtable` is non-null and `PluginVTable`'s first field is
        // `abi_version` in every version this crate has ever shipped.
        let plugin_version = unsafe { (*vtable).abi_version };
        if !plugin_version.is_compatible_with(AbiVersion::CURRENT) {
            // Deliberately not calling `shutdown` or `drop`: an incompatible
            // major version may not lay out those very function pointers at
            // the offsets this host's `PluginVTable` definition expects, so
            // calling through them here would be exactly the unchecked,
            // wrong-offset call the version gate exists to prevent. The
            // plugin's own instance leaks in this one path; `library` is
            // still unloaded below.
            drop(library);
            return Err(PluginError::IncompatibleVersion {
                plugin: plugin_version,
                host: AbiVersion::CURRENT,
            });
        }

        // SAFETY: version-checked above; every other field is now known to
        // be at the offset this host's `PluginVTable` definition expects.
        let init_status = unsafe { ((*vtable).init)((*vtable).instance, host) };
        if init_status != STATUS_OK {
            // Owed regardless of init's outcome — see `Plugin::init`'s doc.
            unsafe {
                ((*vtable).shutdown)((*vtable).instance);
                ((*vtable).drop)((*vtable).instance);
            }
            drop(library);
            return Err(PluginError::InitFailed);
        }

        // SAFETY: `init` succeeded, so the commands snapshot behind
        // `command_count`/`command_at` is now populated and each `AbiStr`
        // returned is valid until the *next* call into this instance — read
        // and copied into owned `String`s immediately, before any other
        // call, exactly as `PluginVTable`'s own doc requires of a caller
        // that needs one to outlive that window.
        let (name, version, commands) = unsafe {
            let name = ((*vtable).name)((*vtable).instance).as_str().to_owned();
            let version = ((*vtable).version)((*vtable).instance).as_str().to_owned();
            let count = ((*vtable).command_count)((*vtable).instance);
            let mut commands = Vec::with_capacity(count);
            for index in 0..count {
                let command = ((*vtable).command_at)((*vtable).instance, index);
                commands.push(PluginCommand::new(
                    command.id.as_str().to_owned(),
                    command.title.as_str().to_owned(),
                ));
            }
            (name, version, commands)
        };

        self.plugins.push(LoadedPlugin {
            _library: library,
            vtable,
            path,
            name,
            version,
            commands,
        });
        Ok(self.plugins.last().expect("just pushed"))
    }

    /// The plugins currently loaded, in load order.
    #[must_use]
    pub fn plugins(&self) -> &[LoadedPlugin] {
        &self.plugins
    }

    /// The plugins currently loaded, mutable — needed to call
    /// [`LoadedPlugin::invoke`], which takes `&mut self`.
    pub fn plugins_mut(&mut self) -> &mut [LoadedPlugin] {
        &mut self.plugins
    }

    /// Unload the plugin at `index`, running its `shutdown`/`drop` sequence
    /// via [`LoadedPlugin`]'s own `Drop` impl.
    pub fn unload(&mut self, index: usize) -> Option<LoadedPlugin> {
        if index < self.plugins.len() {
            Some(self.plugins.remove(index))
        } else {
            None
        }
    }

    /// How many plugins are currently loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::stderr_host_vtable;

    #[test]
    fn loading_a_nonexistent_file_reports_an_open_error() {
        let mut registry = PluginRegistry::new();
        let host = stderr_host_vtable();
        let result = unsafe { registry.load("/nonexistent/path/to/plugin.so", &host) };
        assert!(matches!(result, Err(PluginError::Open(_))));
        assert!(registry.is_empty());
    }

    #[test]
    fn loading_a_file_that_is_not_a_shared_library_reports_an_open_error() {
        let dir = std::env::temp_dir();
        let path = dir.join("vieww_plugin_registry_test_not_a_library.txt");
        std::fs::write(&path, b"not a shared library").expect("write temp file");

        let mut registry = PluginRegistry::new();
        let host = stderr_host_vtable();
        let result = unsafe { registry.load(&path, &host) };
        assert!(matches!(result, Err(PluginError::Open(_))));

        let _ = std::fs::remove_file(&path);
    }

    /// The registry's own accounting (empty/len/unload) needs no real
    /// `cdylib` to exercise; the `dlopen`-and-call path against a genuine
    /// compiled plugin is covered end to end in `tests/example_plugin.rs`,
    /// which builds one with a real `cargo build` subprocess.
    #[test]
    fn a_fresh_registry_is_empty() {
        let registry = PluginRegistry::new();
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
        assert!(registry.plugins().is_empty());
    }

    #[test]
    fn unloading_out_of_range_returns_none_and_touches_nothing() {
        let mut registry = PluginRegistry::new();
        assert!(registry.unload(0).is_none());
    }
}
