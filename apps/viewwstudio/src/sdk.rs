//! What a packaged studio ships alongside itself, and how it proves it.
//!
//! # The decision this implements
//!
//! A preview is a `cdylib` handing a `Box<dyn Widget>` across a library
//! boundary, and that is sound only when both sides came out of **one
//! compilation**. There were three ways to buy that guarantee: publish `vieww`
//! with a stable ABI, relax the guard to a layout hash, or make host and guest
//! the same compilation by construction. This is the third. It is what a
//! mobile toolchain does for its language, what Xcode does with Swift, and what an
//! IDE does with its bundled JDK — because "install the matching compiler and hope the
//! hashes line up" is a contributor's setup, not a product's.
//!
//! Concretely: the studio carries the `rustc` it was built by and the `vieww`
//! rlibs it was linked against, and compiles previews with those. Nothing about
//! `vieww`'s ABI has to be promised, because there is never more than one
//! version of it in the room.
//!
//! # Why a manifest rather than "the files are there"
//!
//! [`install::is_usable`] answers "does this directory hold an rlib and a
//! `deps/`", which was the right question when the studio was scavenging a
//! target directory it did not own. A shipped SDK can answer a better one. The
//! manifest records **what this bundle claims to be**, so a mismatch is a
//! sentence — "built by rustc 1.95.0, but the toolchain here is 1.93.0" —
//! rather than a preview that comes out blank.
//!
//! That matters most for the case nobody tests: a bundle assembled correctly,
//! then partially updated. Copying a newer studio binary over an older SDK
//! leaves every file present and every check based on presence passing.
//!
//! # The format, and why it is not the `json` module
//!
//! Four lines of `key = value`. It is written by a shell script and read here,
//! it will never nest, and a parser small enough to read in one sitting is
//! worth more at this boundary than reuse — this file has to be trustworthy
//! before the studio will run a compiler out of the directory it describes.
//!
//! Unknown keys are ignored rather than rejected, so an older studio can read
//! a newer bundle's manifest far enough to say *which* version it is refusing.
//!
//! [`install::is_usable`]: crate::install::is_usable

use std::path::{Path, PathBuf};

/// The manifest's file name inside the SDK directory.
pub const MANIFEST: &str = "vieww-sdk.toml";

/// The subdirectory holding the bundled compiler, if there is one.
pub const TOOLCHAIN_DIR: &str = "toolchain";

/// The format this studio writes and understands.
///
/// Compared rather than assumed: a bundle from a future studio is refused with
/// its version named, which is a thing a user can act on.
pub const FORMAT: u32 = 1;

/// What a bundle says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The manifest format, so this can change without silent misreads.
    pub format: u32,
    /// The `vieww` version the rlibs were built from.
    pub vieww: String,
    /// `rustc --version`, verbatim, of the compiler that built them.
    pub rustc: String,
    /// The triple the rlibs are for. A bundle is not portable between hosts.
    pub host: String,
    /// The exact `libvieww-<hash>.rlib` the studio binary linked.
    ///
    /// The one field that makes this more than documentation: a target
    /// directory legitimately holds several, and naming the right one is what
    /// removes the guessing.
    pub rlib: String,
}

/// Why a bundle cannot be trusted to compile a preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdkError {
    /// No manifest. Not fatal on its own — a development checkout has none —
    /// but it means nothing here can be verified.
    Missing(PathBuf),
    /// The manifest could not be read from disk.
    Unreadable(String),
    /// A manifest written by a newer studio.
    Format { found: u32, understood: u32 },
    /// A required field was absent or empty.
    Incomplete(&'static str),
    /// The bundle was assembled for a different machine.
    Host { expected: String, found: String },
    /// The bundled compiler is not the one the manifest names — the
    /// partially-updated bundle this module exists for.
    Rustc { expected: String, found: String },
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(path) => write!(
                f,
                "no {MANIFEST} in {} — this is an unpackaged build tree, so \
                 nothing about it can be verified",
                path.display()
            ),
            Self::Unreadable(why) => write!(f, "could not read {MANIFEST}: {why}"),
            Self::Format { found, understood } => write!(
                f,
                "this SDK is format {found} and this studio understands \
                 {understood} — the studio is older than the SDK beside it"
            ),
            Self::Incomplete(field) => {
                write!(
                    f,
                    "{MANIFEST} has no `{field}`, so the bundle is incomplete"
                )
            }
            Self::Host { expected, found } => write!(
                f,
                "this SDK was built for {found} and the studio is running on \
                 {expected}; rlibs are not portable between targets"
            ),
            Self::Rustc { expected, found } => write!(
                f,
                "the SDK says it was built by {expected}, but the compiler in \
                 it reports {found}. The bundle has been partially updated, \
                 and its rlibs cannot be loaded by that compiler."
            ),
        }
    }
}

impl std::error::Error for SdkError {}

impl Manifest {
    /// Parse a manifest's text.
    ///
    /// # Errors
    ///
    /// If the format is newer than [`FORMAT`], or a required field is missing.
    pub fn parse(text: &str) -> Result<Self, SdkError> {
        let mut out = Self {
            format: FORMAT,
            ..Self::default()
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            // Values are written quoted, because `rustc --version` contains
            // spaces and parentheses and a bare value would be ambiguous the
            // first time somebody hand-edits this.
            let value = value.trim().trim_matches('"').trim().to_owned();
            match key.trim() {
                "format" => out.format = value.parse().unwrap_or(0),
                "vieww" => out.vieww = value,
                "rustc" => out.rustc = value,
                "host" => out.host = value,
                "rlib" => out.rlib = value,
                // Ignored, so this studio can still read a newer bundle far
                // enough to report the version it is refusing.
                _ => {}
            }
        }

        if out.format > FORMAT {
            return Err(SdkError::Format {
                found: out.format,
                understood: FORMAT,
            });
        }
        for (field, value) in [
            ("vieww", &out.vieww),
            ("rustc", &out.rustc),
            ("host", &out.host),
            ("rlib", &out.rlib),
        ] {
            if value.is_empty() {
                return Err(SdkError::Incomplete(field));
            }
        }
        Ok(out)
    }

    /// The manifest for the SDK directory `dir`, if it has one.
    ///
    /// # Errors
    ///
    /// [`SdkError::Missing`] when there is no manifest — which is the ordinary
    /// case in a development checkout, and is why the caller treats it as
    /// "unverified" rather than as a failure.
    pub fn read(dir: &Path) -> Result<Self, SdkError> {
        let path = dir.join(MANIFEST);
        if !path.is_file() {
            return Err(SdkError::Missing(dir.to_path_buf()));
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| SdkError::Unreadable(error.to_string()))?;
        Self::parse(&text)
    }

    /// The text form, so the packaging script and the tests agree on it by
    /// construction rather than by both being edited.
    #[must_use]
    pub fn to_text(&self) -> String {
        format!(
            "# Written by packaging/package.sh. Describes the compilation this \
             studio previews against.\nformat = {}\nvieww = \"{}\"\nrustc = \
             \"{}\"\nhost = \"{}\"\nrlib = \"{}\"\n",
            self.format, self.vieww, self.rustc, self.host, self.rlib
        )
    }

    /// Check the bundle against the machine and the compiler actually in it.
    ///
    /// `found_rustc` is the `rustc --version` of `Self::rustc_path`, or
    /// `None` when the bundle ships no compiler — in which case there is
    /// nothing to disagree with and only the host is checked.
    ///
    /// # Errors
    ///
    /// If the bundle is for another target, or its compiler is not the one it
    /// claims.
    pub fn verify(&self, host: &str, found_rustc: Option<&str>) -> Result<(), SdkError> {
        if self.host != host {
            return Err(SdkError::Host {
                expected: host.to_owned(),
                found: self.host.clone(),
            });
        }
        if let Some(found) = found_rustc {
            if found.trim() != self.rustc.trim() {
                return Err(SdkError::Rustc {
                    expected: self.rustc.clone(),
                    found: found.trim().to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// The compiler inside an SDK directory, if it ships one.
///
/// `None` is the development checkout and the "our rlibs, your compiler"
/// bundle: both still work, they are just the configurations where the studio
/// is trusting `PATH` to hold the right compiler rather than knowing it does.
#[must_use]
pub fn rustc_path(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "rustc.exe" } else { "rustc" };
    let path = dir.join(TOOLCHAIN_DIR).join("bin").join(name);
    path.is_file().then_some(path)
}

/// The triple this studio was compiled for.
///
/// From the build script's `TARGET` rather than from `rustc -vV` at runtime.
/// The question is what the **studio binary** is, and the only moment anything
/// can answer that is while it is being built — a `rustc` found on `PATH`
/// afterwards answers a different question, and under cross-compilation
/// answers it wrongly.
#[must_use]
pub const fn host_triple() -> &'static str {
    env!("VIEWWSTUDIO_HOST")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            format: FORMAT,
            vieww: "0.0.1".into(),
            rustc: "rustc 1.95.0 (abc123 2026-01-01)".into(),
            host: "x86_64-unknown-linux-gnu".into(),
            rlib: "libvieww-9f8e7d.rlib".into(),
        }
    }

    #[test]
    fn a_manifest_round_trips_through_its_own_text() {
        // The packaging script writes this format and this module reads it.
        // Having one function produce the text the tests parse is what keeps
        // those two from drifting apart in separate edits.
        let parsed = Manifest::parse(&sample().to_text()).expect("its own output");
        assert_eq!(parsed, sample());
    }

    #[test]
    fn a_value_with_spaces_and_parentheses_survives() {
        // `rustc --version` is the reason values are quoted at all.
        let parsed = Manifest::parse(&sample().to_text()).expect("parse");
        assert_eq!(parsed.rustc, "rustc 1.95.0 (abc123 2026-01-01)");
    }

    #[test]
    fn an_unknown_key_is_ignored_rather_than_fatal() {
        let mut text = sample().to_text();
        text.push_str("channel = \"beta\"\n");
        assert_eq!(Manifest::parse(&text).expect("parse"), sample());
    }

    #[test]
    fn a_newer_format_is_refused_with_its_version_named() {
        // And refused *after* the fields are read, so the error can say which
        // version it is — the whole reason unknown keys are skipped.
        let text = sample().to_text().replace("format = 1", "format = 99");
        assert_eq!(
            Manifest::parse(&text),
            Err(SdkError::Format {
                found: 99,
                understood: FORMAT
            })
        );
    }

    #[test]
    fn a_missing_field_names_itself() {
        let text = sample()
            .to_text()
            .replace("rlib = \"libvieww-9f8e7d.rlib\"", "");
        assert_eq!(Manifest::parse(&text), Err(SdkError::Incomplete("rlib")));
    }

    #[test]
    fn a_bundle_for_another_target_is_refused() {
        let error = sample()
            .verify("aarch64-apple-darwin", None)
            .expect_err("a linux bundle on a mac");
        assert!(
            matches!(error, SdkError::Host { .. }),
            "rlibs are not portable: {error}"
        );
    }

    #[test]
    fn a_partially_updated_bundle_is_caught() {
        // The case this module exists for: every file present, every
        // presence-based check passing, and the compiler is not the one that
        // produced the rlibs beside it.
        let error = sample()
            .verify(
                "x86_64-unknown-linux-gnu",
                Some("rustc 1.93.0 (older 2025-01-01)"),
            )
            .expect_err("a stale toolchain under a new manifest");
        match error {
            SdkError::Rustc { expected, found } => {
                assert!(expected.contains("1.95.0"));
                assert!(found.contains("1.93.0"));
            }
            other => panic!("{other}"),
        }
    }

    #[test]
    fn a_bundle_that_ships_no_compiler_checks_only_what_it_can() {
        // "Our rlibs, your compiler" is a weaker configuration, not a broken
        // one — `compile::Toolchain` still compares PATH's rustc against the
        // one the host was built with.
        assert_eq!(sample().verify("x86_64-unknown-linux-gnu", None), Ok(()));
    }

    #[test]
    fn no_manifest_says_where_it_looked() {
        let dir = std::env::temp_dir().join("vieww-sdk-empty");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let error = Manifest::read(&dir).expect_err("nothing there");
        assert!(matches!(error, SdkError::Missing(_)), "{error}");
        assert!(error.to_string().contains("unpackaged"), "{error}");
    }

    #[test]
    fn a_bundle_with_no_toolchain_directory_reports_no_compiler() {
        let dir = std::env::temp_dir().join("vieww-sdk-no-toolchain");
        std::fs::create_dir_all(&dir).expect("mkdir");
        assert_eq!(rustc_path(&dir), None);
    }
}
