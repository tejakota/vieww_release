//! A stable, versioned, cross-`rustc` plugin ABI.
//!
//! # How this differs from `vieww-reload`
//!
//! `vieww-reload` hot-reloads *your own application* during development: host
//! and guest are compiled from the same dependency graph by the same `rustc`
//! invocation, moments apart, and its own module doc is explicit that this
//! coupling is required, not incidental. A third-party plugin is different in
//! kind: built once, shipped as a binary, and loaded by a host whose `rustc`
//! version, build flags, and even standard library revision it cannot assume
//! match its own. [`abi`] is the answer to that harder problem — a
//! `#[repr(C)]` vtable of plain function pointers, the one shape the C ABI
//! itself guarantees stays laid out the same way across all of that.
//!
//! # The three pieces
//!
//! - [`abi`] — the boundary itself: [`abi::Plugin`], the safe trait a plugin
//!   author implements; [`abi::PluginVTable`]/[`abi::HostVTable`], the
//!   `#[repr(C)]` shapes that actually cross it; and [`abi::leak_vtable`],
//!   the one place `unsafe extern "C"` trampolines bridge the two. Read this
//!   module's own doc for why trait objects cannot make this trip and plain
//!   function pointers can.
//! - [`host`] — a real, ready-to-use [`abi::HostVTable`] for the host side of
//!   the boundary ([`host::stderr_host_vtable`]), plus the reasoning for why
//!   it is a single function rather than a builder.
//! - [`registry`] — the host-side loader: [`registry::PluginRegistry`] opens
//!   a compiled `cdylib`, checks its declared [`abi::AbiVersion`] before
//!   touching anything else, runs its `init`, and hands back a
//!   [`registry::LoadedPlugin`] whose safe methods are the only way to reach
//!   it again.
//!
//! # What a plugin author actually writes
//!
//! Implement [`abi::Plugin`] and annotate it with `#[vieww_plugin::vieww_plugin]`
//! (re-exported from `vieww-plugin-macros`) to generate the `#[no_mangle]
//! extern "C" fn vieww_plugin_entry` every host looks for by name — see that
//! macro's own doc for the exact expansion. A plugin author never calls
//! [`abi::leak_vtable`] or writes an `extern "C" fn` themselves.

pub mod abi;
pub mod host;
pub mod registry;

pub use abi::{AbiVersion, HostVTable, LogLevel, Plugin, PluginCommand, PluginVTable};
pub use host::stderr_host_vtable;
pub use registry::{LoadedPlugin, PluginError, PluginRegistry};
pub use vieww_plugin_macros::vieww_plugin;
