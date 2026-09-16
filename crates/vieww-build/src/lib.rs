//! Packaging pipelines and build-environment diagnostics for a vieww project.
//!
//! Three modules, each answering one question a `vieww` CLI user asks before
//! or after writing code:
//!
//! * [`doctor`] — "can a build even be attempted on this machine". Real
//!   probes: `rustc --version`, `cargo --version`, the `RUSTUP_TOOLCHAIN`
//!   environment variable, and [`vieww_hardware::Capability::Display`] reused
//!   rather than re-probed.
//! * [`scaffold`] — `vieww new`'s template: a minimal desktop app, real
//!   enough that `tests/scaffold_build.rs` compiles it with an actual `cargo
//!   build`.
//! * [`package`] — turning a built project into a platform artefact, for the
//!   five targets its own `Cargo.toml` description names, with an honest
//!   error — never a panic, never a silent no-op — everywhere a required tool
//!   or host is missing.
//!
//! See each module's own docs for what is real and fully exercised on the
//! machine building this workspace versus real-but-gated behind a host or
//! toolchain this environment does not have.

pub mod doctor;
pub mod package;
pub mod scaffold;

mod util;
