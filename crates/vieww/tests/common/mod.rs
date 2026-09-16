//! Shared test harnesses.
//!
//! `mod common;` in an integration test brings this in. Each submodule is
//! `#![allow(dead_code)]` because a test binary that uses half the harness would
//! otherwise warn about the other half — and the warning would be about the
//! harness rather than about anything wrong.

#![allow(unreachable_pub)]

pub mod idle;
