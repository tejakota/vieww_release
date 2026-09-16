//! Where an application's own files come from.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use vieww_foundation::ServiceError;

/// Why an asset could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    /// No asset by that name.
    NotFound(String),
    /// It exists and could not be read.
    Unreadable(String),
    /// It was read and is not what it claimed to be.
    Decode(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "no asset at {path}"),
            Self::Unreadable(message) => write!(f, "reading an asset: {message}"),
            Self::Decode(message) => write!(f, "decoding an asset: {message}"),
        }
    }
}

impl std::error::Error for AssetError {}

impl From<AssetError> for ServiceError {
    /// So a bundle can sit behind the same error type as every other service.
    ///
    /// A missing asset maps to `Failed` rather than `Unsupported`: the
    /// capability is present and working, and one file is not there. Reporting
    /// it as unsupported would tell a caller to stop asking, which is exactly
    /// wrong for a typo in a path.
    fn from(error: AssetError) -> Self {
        Self::failed(error.to_string())
    }
}

/// A source of the application's own files.
///
/// A [service](vieww_foundation::service), so the platform registers one and an
/// application can replace it: a downloaded content pack, a test double serving
/// fixtures from memory, or a directory on disk during development all satisfy
/// this and none of them needs a change here.
///
/// # Paths are slash-separated and relative
///
/// `images/avatar.png`, never `/images/avatar.png` and never a backslash. The
/// backing store is a zip inside an APK on one platform and a directory on
/// another, and the only path syntax both agree on is the simple one.
pub trait AssetBundle: 'static {
    /// The bytes of one asset.
    ///
    /// # Errors
    ///
    /// [`AssetError::NotFound`] if there is no such asset, and
    /// [`AssetError::Unreadable`] if there is and it could not be read.
    fn open(&self, path: &str) -> Result<Vec<u8>, AssetError>;

    /// `true` if [`open`](Self::open) would find something.
    ///
    /// Defaults to attempting the read, which is correct and wasteful. A bundle
    /// with an index — a zip, which is what an APK is — should override it.
    fn contains(&self, path: &str) -> bool {
        self.open(path).is_ok()
    }
}

/// Assets compiled into the binary.
///
/// The only bundle that works identically on every platform, because there is no
/// filesystem involved at all — which is what makes it the right default for the
/// handful of files a framework-level application genuinely cannot start
/// without.
///
/// ```
/// use vieww_asset::{AssetBundle, EmbeddedBundle};
///
/// // In an application this is `include_bytes!("../assets/logo.png")`, which is
/// // where the `&'static [u8]` comes from and why the bytes need no lifetime.
/// let bundle = EmbeddedBundle::new().with("icons/logo.png", b"\x89PNG...".as_slice());
///
/// assert!(bundle.contains("icons/logo.png"));
/// assert!(!bundle.contains("icons/missing.png"));
/// ```
#[derive(Debug, Default)]
pub struct EmbeddedBundle {
    files: HashMap<String, &'static [u8]>,
}

impl EmbeddedBundle {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file. Adding the same path twice replaces.
    #[must_use]
    pub fn with(mut self, path: impl Into<String>, bytes: &'static [u8]) -> Self {
        self.files.insert(path.into(), bytes);
        self
    }

    /// How many files are embedded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl AssetBundle for EmbeddedBundle {
    fn open(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        self.files
            .get(path)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| AssetError::NotFound(path.to_owned()))
    }

    fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
}

/// Assets in a directory on disk.
///
/// For development, and for iOS — where an app bundle *is* a directory and the
/// resources sit inside it. Not for Android, where assets live compressed inside
/// the APK and only the asset manager can reach them.
///
/// # Escaping the directory is refused rather than sandboxed
///
/// A path containing `..` is rejected outright. This is not a security boundary
/// — the application wrote the path — but a `..` in an asset path is always a
/// mistake, and one that works on a developer's machine and fails inside a
/// bundle is the worst kind.
#[derive(Debug, Clone)]
pub struct DirectoryBundle {
    root: PathBuf,
}

impl DirectoryBundle {
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The directory being served.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, AssetError> {
        if path.split('/').any(|part| part == "..") {
            return Err(AssetError::Unreadable(format!(
                "{path} climbs out of the bundle, which works in development \
                 and fails everywhere else"
            )));
        }
        Ok(self.root.join(path))
    }
}

impl AssetBundle for DirectoryBundle {
    fn open(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        let full = self.resolve(path)?;
        std::fs::read(&full).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => AssetError::NotFound(path.to_owned()),
            _ => AssetError::Unreadable(format!("{}: {error}", full.display())),
        })
    }

    fn contains(&self, path: &str) -> bool {
        self.resolve(path).is_ok_and(|full| full.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_embedded_file_comes_back_byte_for_byte() {
        let bundle = EmbeddedBundle::new().with("a.txt", b"hello".as_slice());
        assert_eq!(bundle.open("a.txt").expect("open"), b"hello");
    }

    #[test]
    fn a_missing_asset_is_not_found_rather_than_unreadable() {
        // The distinction a caller acts on: a typo versus a broken install.
        let bundle = EmbeddedBundle::new();
        assert!(matches!(
            bundle.open("nope.png"),
            Err(AssetError::NotFound(_))
        ));
    }

    #[test]
    fn a_path_that_climbs_out_of_the_bundle_is_refused() {
        let bundle = DirectoryBundle::at("/tmp/vieww-assets");
        let error = bundle.open("../../etc/passwd").unwrap_err();
        assert!(matches!(error, AssetError::Unreadable(_)), "{error}");
    }

    #[test]
    fn a_directory_bundle_reads_a_real_file() {
        let dir = std::env::temp_dir().join(format!("vieww-assets-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("images")).expect("mkdir");
        std::fs::write(dir.join("images/a.bin"), b"bytes").expect("write");

        let bundle = DirectoryBundle::at(&dir);
        assert!(bundle.contains("images/a.bin"));
        assert_eq!(bundle.open("images/a.bin").expect("open"), b"bytes");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundle_error_becomes_a_service_failure_rather_than_unsupported() {
        // Unsupported tells a caller to stop asking, which is wrong for a typo.
        let error: ServiceError = AssetError::NotFound("x.png".to_owned()).into();
        assert!(!error.is_permanent());
    }
}
