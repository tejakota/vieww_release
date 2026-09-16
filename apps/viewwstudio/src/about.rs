//! What version this is, what it was built from, and where its files live.
//!
//! # Why a module for four strings
//!
//! Because they are the four strings every support conversation starts with,
//! and before this they did not exist anywhere a user could reach. The version
//! appeared in one place — a sixty-pixel status-bar cell — and there was no
//! About box, no build identifier, no licence, and nothing that said where the
//! settings file the studio now writes actually is.
//!
//! Everything here is resolved at **compile time** from Cargo's own
//! environment, except the paths, which are resolved at run time because that
//! is what they are. Nothing is typed twice: a version bumped in `Cargo.toml`
//! is a version this reports, with no second place to forget.
//!
//! `VIEWWSTUDIO_BUILD` is optional and is set by the release workflow to the
//! commit it built. A development build has none, and says so rather than
//! printing an empty field — "unknown" in a bug report is information, and a
//! blank line is not.

/// The crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The licence, from the workspace manifest.
pub const LICENSE: &str = env!("CARGO_PKG_LICENSE");

/// The commit this was built from, when the build system said so.
#[must_use]
pub fn build() -> &'static str {
    option_env!("VIEWWSTUDIO_BUILD").unwrap_or("development build")
}

/// The host triple this binary targets.
#[must_use]
pub fn target() -> String {
    // Not `env!("TARGET")` — Cargo does not set that for an ordinary build, and
    // a build script existing only to record it is a build script. Assembled
    // from `std::env::consts` instead, which is the same information and needs
    // no build step. Not `const`: these are `&'static str` constants but
    // `format!` is not const, and a hand-rolled const concatenation of three of
    // them is not worth the trick.
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// Where the settings and session files are written.
#[must_use]
pub fn data_dir() -> std::path::PathBuf {
    vieww_platform_winit::storage::data_dir()
}

/// The About dialog's list, one field per line.
#[must_use]
pub fn lines() -> Vec<String> {
    vec![
        format!("Version    {VERSION}"),
        format!("Build      {}", build()),
        format!("Target     {}", target()),
        format!("Licence    {LICENSE}"),
        format!("Data       {}", data_dir().display()),
    ]
}

/// The same, as one block somebody can paste into a bug report.
#[must_use]
pub fn report() -> String {
    let mut out = String::from("vieww Studio\n");
    for line in lines() {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_the_crates_own() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert!(!VERSION.is_empty());
    }

    /// A blank field in a bug report is worse than a stated unknown.
    #[test]
    fn a_development_build_says_so_rather_than_leaving_a_blank() {
        assert!(!build().is_empty());
    }

    #[test]
    fn every_line_has_a_label_and_a_value() {
        for line in lines() {
            let (label, value) = line.split_at(11);
            assert!(!label.trim().is_empty(), "{line}");
            assert!(!value.trim().is_empty(), "{line}");
        }
    }

    #[test]
    fn the_report_is_pasteable() {
        let report = report();
        assert!(report.starts_with("vieww Studio"));
        assert!(report.contains(VERSION));
        assert!(report.ends_with('\n'));
    }

    /// The one thing a user cannot otherwise find out: where the studio put
    /// the settings file it now writes.
    #[test]
    fn the_data_directory_is_reported() {
        assert!(report().contains("Data"));
        assert!(!data_dir().as_os_str().is_empty());
    }
}
