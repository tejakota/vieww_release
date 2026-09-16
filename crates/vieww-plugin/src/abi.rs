//! The stable ABI boundary: `#[repr(C)]` types and function-pointer vtables
//! only, so a plugin's binary compatibility with the host does not depend on
//! either side using the same rustc version — unlike `vieww-reload`'s guest
//! ABI, which is same-compilation-unit-coupled by design (its own module doc
//! is explicit: host and guest must be "one dependency graph", the same
//! `rustc` invocation) and exists for a different job entirely — hot-reloading
//! *your own application* during development, where that coupling is a
//! reasonable price for speed. A third-party plugin, built once and shipped,
//! cannot make that assumption about who eventually loads it or with which
//! toolchain.
//!
//! # Why function-pointer vtables, not trait objects, cross this boundary
//!
//! A `Box<dyn Trait>`'s vtable layout is a rustc implementation detail with no
//! stability guarantee across compiler versions or even compiler flags — two
//! binaries built by different rustc releases may lay a trait object's vtable
//! out differently, so passing one across a `dlopen` boundary is exactly the
//! "looks like it compiles, corrupts memory at runtime" failure mode this
//! project's own conventions warn hardest against (see `vieww-foundation`'s
//! and `vieww-paint`'s module docs on honest stubs versus wrong-looking code).
//! A `#[repr(C)]` struct of plain function pointers has a layout the C ABI
//! itself guarantees platform-stable, which is why every FFI plugin system —
//! each OS's own driver model, the `abi_stable` crate — is built the same way:
//! never a Rust trait object at the boundary, always a C-shaped vtable.
//!
//! [`Plugin`] is the safe, ergonomic trait a plugin author actually
//! implements; [`leak_vtable`] is the one place `unsafe` trampolines adapt it
//! onto [`PluginVTable`]. Nothing about the boundary's safety depends on a
//! plugin author getting `unsafe` code right themselves — that is the whole
//! point of the split, and precisely what `#[vieww_plugin]`
//! (`vieww-plugin-macros`) exists to make the *only* thing wired by hand.

use std::os::raw::c_void;

/// This crate's own ABI version. Bump [`MAJOR`] for a breaking change to any
/// [`PluginVTable`] or [`HostVTable`] field's meaning, order, or removal;
/// bump [`MINOR`] for an additive, backward-compatible change (new fields
/// appended after the existing ones). See [`AbiVersion::is_compatible_with`]
/// for exactly what a version mismatch does.
pub const MAJOR: u32 = 1;
pub const MINOR: u32 = 0;

/// The one thing checked before any other call crosses the boundary.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiVersion {
    pub major: u32,
    pub minor: u32,
}

impl AbiVersion {
    pub const CURRENT: Self = Self {
        major: MAJOR,
        minor: MINOR,
    };

    /// `true` if a plugin declaring `self` may be loaded by a host whose own
    /// version is `host`.
    ///
    /// Majors must match exactly — a breaking change refuses to load rather
    /// than call a function pointer at the wrong offset into a
    /// differently-shaped struct, which is corruption, not a crash you can
    /// debug from the message it prints. A plugin's minor may not exceed the
    /// host's: a plugin built against a newer, additive minor version may
    /// assume a vtable field the host's own (older but compatible) struct
    /// definition does not have room for.
    #[must_use]
    pub const fn is_compatible_with(self, host: Self) -> bool {
        self.major == host.major && self.minor <= host.minor
    }
}

/// A borrowed UTF-8 string crossing the ABI boundary as a pointer and a
/// length — never `&str` or `String` directly, whose representations (a fat
/// pointer; an allocation, a length and a capacity) are not part of any
/// stable C ABI, unlike `(*const u8, usize)`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiStr {
    ptr: *const u8,
    len: usize,
}

impl AbiStr {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };

    #[must_use]
    pub fn borrowed(s: &str) -> Self {
        Self {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    /// Read this back as a `&str`.
    ///
    /// Defensive rather than panicking or erroring on a null pointer or
    /// invalid UTF-8 — both read as `""`. A plugin boundary is a trust
    /// boundary; the host should not be crashable by a plugin that sends
    /// malformed bytes for a display string, and `""` is a display string
    /// too, just an unhelpful one.
    ///
    /// # Safety
    /// The memory this points at must still be valid for `len` bytes at the
    /// time of the call. Every [`AbiStr`] this crate produces documents
    /// exactly how long that is (see [`PluginVTable`]'s field docs); an
    /// `AbiStr` a caller has held onto past its documented validity window
    /// is a caller bug, not this function's to detect.
    #[must_use]
    pub unsafe fn as_str(&self) -> &str {
        if self.ptr.is_null() || self.len == 0 {
            return "";
        }
        let bytes = std::slice::from_raw_parts(self.ptr, self.len);
        std::str::from_utf8(bytes).unwrap_or("")
    }
}

/// One command a plugin offers, as it crosses the ABI.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiCommand {
    pub id: AbiStr,
    pub title: AbiStr,
}

/// A log severity, carried across the boundary as a single byte rather than
/// `#[repr(C)]` (which, for a field-less enum, is platform-defined —
/// commonly `c_int` but not guaranteed — where `#[repr(u8)]` is exact and
/// small).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

/// `0`: succeeded.
pub const STATUS_OK: i32 = 0;
/// The plugin reported a failure with no more specific code.
pub const STATUS_ERROR: i32 = -1;
/// [`PluginVTable::invoke`] was asked for a command id this plugin does not
/// have.
pub const STATUS_UNKNOWN_COMMAND: i32 = -2;

pub type LogFn = extern "C" fn(level: LogLevel, message: AbiStr);

/// What a host exposes back to a plugin. Passed to
/// [`PluginVTable::init`] as a borrowed pointer, valid for the duration of
/// that one call — a plugin that wants to log later keeps [`Self::log`]
/// itself (a plain function pointer, trivially `Copy`), not the pointer to
/// this struct.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostVTable {
    pub abi_version: AbiVersion,
    pub log: LogFn,
}

pub type NameFn = extern "C" fn(instance: *mut c_void) -> AbiStr;
pub type VersionFn = extern "C" fn(instance: *mut c_void) -> AbiStr;
pub type InitFn = extern "C" fn(instance: *mut c_void, host: *const HostVTable) -> i32;
pub type ShutdownFn = extern "C" fn(instance: *mut c_void);
pub type CommandCountFn = extern "C" fn(instance: *mut c_void) -> usize;
pub type CommandAtFn = extern "C" fn(instance: *mut c_void, index: usize) -> AbiCommand;
pub type InvokeFn = extern "C" fn(instance: *mut c_void, id: AbiStr) -> i32;
pub type DropFn = extern "C" fn(instance: *mut c_void);

/// What a plugin exports, once, from its `vieww_plugin_entry` symbol.
///
/// # Lifetimes at the boundary, stated exactly
///
/// - [`Self::instance`] is a boxed, plugin-owned value. Every function
///   pointer here except [`Self::drop`] takes it borrowed; [`Self::drop`]
///   consumes it and must be the very last call made into this vtable.
/// - An [`AbiStr`] returned by [`Self::name`], [`Self::version`], or
///   embedded in an [`AbiCommand`] from [`Self::command_at`] is valid only
///   until the *next* call into this same `instance` — it borrows memory
///   [`Self::instance`] itself owns, the same convention `readdir`'s reused
///   buffer or `strerror`'s static one use in C. A host that needs one to
///   outlive the next call must copy it into an owned `String` immediately.
/// - [`Self::command_count`] and [`Self::command_at`] read a snapshot of
///   this plugin's commands taken once, right after [`Self::init`]
///   succeeds — not recomputed on every call. A plugin whose available
///   commands can change at runtime is out of scope for this ABI version;
///   see this module's own doc for why a first version of any ABI draws a
///   line rather than trying to anticipate every future shape.
#[repr(C)]
#[derive(Debug)]
pub struct PluginVTable {
    pub abi_version: AbiVersion,
    pub instance: *mut c_void,
    pub name: NameFn,
    pub version: VersionFn,
    pub init: InitFn,
    pub shutdown: ShutdownFn,
    pub command_count: CommandCountFn,
    pub command_at: CommandAtFn,
    pub invoke: InvokeFn,
    pub drop: DropFn,
}

/// The fixed symbol name every plugin `cdylib` exports, and every host looks
/// up by exactly this name. A `\0`-terminated byte string because
/// `libloading::Library::get` (like `dlsym`) wants one.
pub const ENTRY_SYMBOL: &[u8] = b"vieww_plugin_entry\0";

pub type EntryFn = unsafe extern "C" fn() -> *mut PluginVTable;

/// One command a plugin offers, in the safe, owned shape [`Plugin::commands`]
/// returns — turned into an [`AbiCommand`] only at the ABI boundary itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCommand {
    pub id: String,
    pub title: String,
}

impl PluginCommand {
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

/// The safe trait a plugin author implements. Everything here is ordinary,
/// safe Rust; [`leak_vtable`] is the only place it meets `unsafe extern "C"`.
pub trait Plugin: 'static {
    fn name(&self) -> &str;
    fn version(&self) -> &str;

    /// Called once, immediately after this plugin is loaded, with the
    /// [`HostVTable`] this run of the host provides.
    ///
    /// The default does nothing and succeeds — most plugins have no
    /// meaningful setup to fail. Returning `Err` refuses the load: the host
    /// calls neither [`Self::commands`] nor [`Self::invoke`] on a plugin
    /// whose `init` failed, and calls [`Self::shutdown`] regardless so a
    /// plugin that partially set up resources before failing still gets to
    /// release them.
    fn init(&mut self, host: &HostVTable) -> Result<(), String> {
        let _ = host;
        Ok(())
    }

    /// Called once, when the host is done with this plugin. Not called if
    /// this plugin was never successfully loaded at all (a version mismatch
    /// refuses before `init` ever runs) — but *is* called after a failed
    /// `init`, per that method's own doc.
    fn shutdown(&mut self) {}

    /// The commands this plugin offers, read once right after a successful
    /// [`Self::init`] — see [`PluginVTable`]'s own doc for exactly when.
    fn commands(&self) -> Vec<PluginCommand> {
        Vec::new()
    }

    /// Run the command named `id`.
    ///
    /// # Errors
    /// Any `Err` reaches the host as [`STATUS_ERROR`] — this ABI version
    /// does not carry error *text* back across the boundary (a real, scoped
    /// limitation: doing so needs an out-parameter buffer and an
    /// author-supplied capacity, which is exactly the kind of interface a
    /// first version should not guess the shape of — see this module's
    /// "Lifetimes at the boundary" doc for the same reasoning applied to
    /// `commands`). [`STATUS_UNKNOWN_COMMAND`] is produced by the trampoline
    /// itself when `id` matches none of [`Self::commands`], never by this
    /// method.
    fn invoke(&mut self, id: &str) -> Result<(), String>;
}

/// The boxed value behind [`PluginVTable::instance`]: the plugin itself, plus
/// the one-shot commands snapshot [`PluginVTable`]'s doc describes.
struct PluginBox<P: Plugin> {
    plugin: P,
    commands: Vec<PluginCommand>,
}

/// Build a [`PluginVTable`] for `plugin`, leaking both the boxed instance and
/// the vtable itself for the life of the process.
///
/// # Why this leaks, on purpose, twice over
///
/// The vtable has to remain valid for as long as the host might call through
/// it, and a `dlopen`ed plugin has no natural moment before process exit at
/// which a host is required to have finished with it (unlike an in-process
/// value with an owner) — the same trade-off `vieww-reload`'s own module doc
/// makes explicit for its guest handle, which "leaks a library... on
/// purpose" every reload. The plugin *instance* is the one thing this leak
/// does not have to cover forever: [`PluginVTable::drop`] frees it for real,
/// so a host that calls `drop` when it unloads a plugin does not leak the
/// plugin's own resources — only the small, fixed vtable allocation and its
/// entry symbol's leaked `Box` are permanent, and both are one allocation
/// per plugin load, not per call.
///
/// This is what `#[vieww_plugin]` (`vieww-plugin-macros`) calls from the
/// generated `vieww_plugin_entry` — a plugin author never calls it directly.
#[must_use]
pub fn leak_vtable<P: Plugin>(plugin: P) -> *mut PluginVTable {
    let boxed = Box::new(PluginBox {
        plugin,
        commands: Vec::new(),
    });
    let instance = Box::into_raw(boxed).cast::<c_void>();

    let vtable = Box::new(PluginVTable {
        abi_version: AbiVersion::CURRENT,
        instance,
        name: trampoline_name::<P>,
        version: trampoline_version::<P>,
        init: trampoline_init::<P>,
        shutdown: trampoline_shutdown::<P>,
        command_count: trampoline_command_count::<P>,
        command_at: trampoline_command_at::<P>,
        invoke: trampoline_invoke::<P>,
        drop: trampoline_drop::<P>,
    });
    Box::into_raw(vtable)
}

/// # Safety
/// `instance` must be a live `*mut PluginBox<P>` produced by
/// [`leak_vtable::<P>`] for this same `P`, not yet passed to
/// [`trampoline_drop`].
unsafe fn instance_ref<'a, P: Plugin>(instance: *mut c_void) -> &'a PluginBox<P> {
    &*(instance.cast::<PluginBox<P>>())
}

/// # Safety
/// Same contract as [`instance_ref`], plus: no other reference to this same
/// instance may be alive concurrently — true for every call in this module,
/// since a plugin's vtable functions are only ever called one at a time by a
/// single host thread, never re-entered.
unsafe fn instance_mut<'a, P: Plugin>(instance: *mut c_void) -> &'a mut PluginBox<P> {
    &mut *(instance.cast::<PluginBox<P>>())
}

extern "C" fn trampoline_name<P: Plugin>(instance: *mut c_void) -> AbiStr {
    let boxed = unsafe { instance_ref::<P>(instance) };
    AbiStr::borrowed(boxed.plugin.name())
}

extern "C" fn trampoline_version<P: Plugin>(instance: *mut c_void) -> AbiStr {
    let boxed = unsafe { instance_ref::<P>(instance) };
    AbiStr::borrowed(boxed.plugin.version())
}

extern "C" fn trampoline_init<P: Plugin>(instance: *mut c_void, host: *const HostVTable) -> i32 {
    let boxed = unsafe { instance_mut::<P>(instance) };
    let host_ref = unsafe { &*host };
    match boxed.plugin.init(host_ref) {
        Ok(()) => {
            // The one-shot commands snapshot — see `PluginVTable`'s doc for
            // exactly why this is read here and not on every
            // `command_count`/`command_at` call.
            boxed.commands = boxed.plugin.commands();
            STATUS_OK
        }
        Err(_) => STATUS_ERROR,
    }
}

extern "C" fn trampoline_shutdown<P: Plugin>(instance: *mut c_void) {
    let boxed = unsafe { instance_mut::<P>(instance) };
    boxed.plugin.shutdown();
}

extern "C" fn trampoline_command_count<P: Plugin>(instance: *mut c_void) -> usize {
    let boxed = unsafe { instance_ref::<P>(instance) };
    boxed.commands.len()
}

extern "C" fn trampoline_command_at<P: Plugin>(instance: *mut c_void, index: usize) -> AbiCommand {
    let boxed = unsafe { instance_ref::<P>(instance) };
    match boxed.commands.get(index) {
        Some(command) => AbiCommand {
            id: AbiStr::borrowed(&command.id),
            title: AbiStr::borrowed(&command.title),
        },
        None => AbiCommand {
            id: AbiStr::EMPTY,
            title: AbiStr::EMPTY,
        },
    }
}

extern "C" fn trampoline_invoke<P: Plugin>(instance: *mut c_void, id: AbiStr) -> i32 {
    let boxed = unsafe { instance_mut::<P>(instance) };
    let id = unsafe { id.as_str() };
    if !boxed.commands.iter().any(|command| command.id == id) {
        return STATUS_UNKNOWN_COMMAND;
    }
    match boxed.plugin.invoke(id) {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_ERROR,
    }
}

extern "C" fn trampoline_drop<P: Plugin>(instance: *mut c_void) {
    // Reclaims and drops the box `leak_vtable` created — the one part of a
    // plugin load that is not permanently leaked. See `leak_vtable`'s own
    // doc for exactly what still is.
    drop(unsafe { Box::from_raw(instance.cast::<PluginBox<P>>()) });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        Init,
        Shutdown,
        Invoked(String),
    }

    struct TestPlugin {
        calls: std::rc::Rc<RefCell<Vec<Call>>>,
        fail_init: bool,
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            "Test Plugin"
        }
        fn version(&self) -> &str {
            "9.9.9"
        }
        fn init(&mut self, _host: &HostVTable) -> Result<(), String> {
            self.calls.borrow_mut().push(Call::Init);
            if self.fail_init {
                Err("nope".into())
            } else {
                Ok(())
            }
        }
        fn shutdown(&mut self) {
            self.calls.borrow_mut().push(Call::Shutdown);
        }
        fn commands(&self) -> Vec<PluginCommand> {
            vec![
                PluginCommand::new("greet", "Say hello"),
                PluginCommand::new("count", "Count something"),
            ]
        }
        fn invoke(&mut self, id: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(Call::Invoked(id.to_owned()));
            if id == "count" {
                Err("count is not implemented in this test".into())
            } else {
                Ok(())
            }
        }
    }

    extern "C" fn noop_log(_level: LogLevel, _message: AbiStr) {}

    fn host_vtable() -> HostVTable {
        HostVTable {
            abi_version: AbiVersion::CURRENT,
            log: noop_log,
        }
    }

    fn plugin(calls: std::rc::Rc<RefCell<Vec<Call>>>, fail_init: bool) -> TestPlugin {
        TestPlugin { calls, fail_init }
    }

    #[test]
    fn version_compatibility_requires_the_same_major() {
        let v1_0 = AbiVersion { major: 1, minor: 0 };
        let v2_0 = AbiVersion { major: 2, minor: 0 };
        assert!(!v2_0.is_compatible_with(v1_0));
        assert!(!v1_0.is_compatible_with(v2_0));
    }

    #[test]
    fn a_plugin_may_require_no_more_than_the_hosts_minor() {
        let host = AbiVersion { major: 1, minor: 2 };
        assert!(AbiVersion { major: 1, minor: 0 }.is_compatible_with(host));
        assert!(AbiVersion { major: 1, minor: 2 }.is_compatible_with(host));
        assert!(
            !AbiVersion { major: 1, minor: 3 }.is_compatible_with(host),
            "a plugin built against a newer minor may assume a field this host does not have"
        );
    }

    #[test]
    fn abi_str_round_trips_a_rust_string() {
        let text = "hello, plugin";
        let abi = AbiStr::borrowed(text);
        assert_eq!(unsafe { abi.as_str() }, text);
    }

    #[test]
    fn abi_str_empty_reads_back_as_an_empty_string() {
        assert_eq!(unsafe { AbiStr::EMPTY.as_str() }, "");
    }

    /// The whole point of the vtable: every call goes through real
    /// `extern "C"` function pointers built by `leak_vtable`, exercised here
    /// exactly as a host would use them — no shortcut back to safe Rust.
    #[test]
    fn the_full_vtable_lifecycle_round_trips_through_the_c_abi() {
        let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
        let vtable = leak_vtable(plugin(std::rc::Rc::clone(&calls), false));
        let vtable = unsafe { &*vtable };

        assert_eq!(vtable.abi_version, AbiVersion::CURRENT);
        assert_eq!(
            unsafe { (vtable.name)(vtable.instance).as_str() },
            "Test Plugin"
        );
        assert_eq!(
            unsafe { (vtable.version)(vtable.instance).as_str() },
            "9.9.9"
        );

        // Before `init`, the commands snapshot is empty — see `PluginVTable`'s doc.
        assert_eq!((vtable.command_count)(vtable.instance), 0);

        let host = host_vtable();
        let status = (vtable.init)(vtable.instance, &host);
        assert_eq!(status, STATUS_OK);
        assert_eq!(*calls.borrow(), vec![Call::Init]);

        assert_eq!((vtable.command_count)(vtable.instance), 2);
        let first = (vtable.command_at)(vtable.instance, 0);
        assert_eq!(unsafe { first.id.as_str() }, "greet");
        assert_eq!(unsafe { first.title.as_str() }, "Say hello");

        let invoked = (vtable.invoke)(vtable.instance, AbiStr::borrowed("greet"));
        assert_eq!(invoked, STATUS_OK);
        assert_eq!(
            *calls.borrow(),
            vec![Call::Init, Call::Invoked("greet".into())]
        );

        let unknown = (vtable.invoke)(vtable.instance, AbiStr::borrowed("does-not-exist"));
        assert_eq!(unknown, STATUS_UNKNOWN_COMMAND);

        let failing = (vtable.invoke)(vtable.instance, AbiStr::borrowed("count"));
        assert_eq!(failing, STATUS_ERROR);

        (vtable.shutdown)(vtable.instance);
        assert_eq!(
            *calls.borrow(),
            vec![
                Call::Init,
                Call::Invoked("greet".into()),
                Call::Invoked("count".into()),
                Call::Shutdown
            ]
        );

        (vtable.drop)(vtable.instance);
        // `vtable.instance` is dangling from here on; nothing after this
        // line may touch it. `vtable` (the leaked `PluginVTable` allocation
        // itself) is intentionally never freed — see `leak_vtable`'s doc.
    }

    #[test]
    fn a_failed_init_reports_an_error_and_leaves_the_commands_snapshot_empty() {
        let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
        let vtable = leak_vtable(plugin(std::rc::Rc::clone(&calls), true));
        let vtable = unsafe { &*vtable };

        let host = host_vtable();
        let status = (vtable.init)(vtable.instance, &host);
        assert_eq!(status, STATUS_ERROR);
        assert_eq!(
            (vtable.command_count)(vtable.instance),
            0,
            "a failed init must not populate commands"
        );

        // `shutdown` is still owed, per `Plugin::shutdown`'s own doc.
        (vtable.shutdown)(vtable.instance);
        assert_eq!(*calls.borrow(), vec![Call::Init, Call::Shutdown]);

        (vtable.drop)(vtable.instance);
    }

    #[test]
    fn invoking_an_unknown_command_never_reaches_the_plugins_own_invoke() {
        let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
        let vtable = leak_vtable(plugin(std::rc::Rc::clone(&calls), false));
        let vtable = unsafe { &*vtable };
        let host = host_vtable();
        (vtable.init)(vtable.instance, &host);
        calls.borrow_mut().clear();

        (vtable.invoke)(vtable.instance, AbiStr::borrowed("nonexistent"));
        assert!(
            calls.borrow().is_empty(),
            "the trampoline must reject it before Plugin::invoke ever runs"
        );

        (vtable.drop)(vtable.instance);
    }
}
