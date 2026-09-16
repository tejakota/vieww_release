//! A per-workspace `AssetBundle` for the previewed screen.
//!
//! # Why this exists
//!
//! The studio's platform services register one `AssetBundle` — the
//! executable's own `assets/` directory — for the whole process. A previewed
//! screen that asks for `images/avatar.png` was handed the studio's assets
//! rather than the project's, which is exactly backwards: a designer iterating
//! on a screen wants to see *that screen's* assets, not the studio's.
//!
//! # The shape
//!
//! [`WorkspaceBundle`] wraps two bundles:
//!
//! 1. The workspace's own `assets/` directory, when there is one. Tried first,
//!    because the project's assets are what the preview is supposed to show.
//! 2. The studio's own bundle — whatever was already registered when the
//!    studio started. Tried second, so anything the studio's chrome needs (and
//!    anything a guest asks for that the project does not have) still resolves.
//!
//! Both halves stay optional. A studio opened without a workspace has no
//! project bundle; a studio whose chrome does not register an `AssetBundle` has
//! no fallback. The wrapper handles both by trying whichever half exists and
//! returning [`AssetError::NotFound`] only when both come up empty.
//!
//! # Why a wrapper rather than a swap
//!
//! Replacing the studio's bundle outright would break the studio's chrome the
//! moment a workspace opened, and replacing it back when the workspace closed
//! would race with any guest asset read in flight. A wrapper that tries the
//! project first and falls back is one registration, made once at startup, and
//! the *project's* path through it changes when the workspace changes — by
//! writing a `RefCell` rather than by re-registering.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use vieww_asset::{AssetBundle, AssetError, DirectoryBundle};
use vieww_foundation::Services;

/// A bundle that tries the workspace's `assets/` first, then the studio's.
///
/// See the module docs for the trade.
pub struct WorkspaceBundle {
    /// The workspace's `assets/` directory, or `None` when there is no
    /// workspace. Swapped through a `RefCell` so the studio can change
    /// workspaces without re-registering the service.
    project: RefCell<Option<DirectoryBundle>>,
    /// The studio's own bundle, captured here so the wrapper has it without
    /// having to look it up in `Services` on every `open`. `None` if the
    /// studio started without an `AssetBundle` registered — which is the case
    /// in tests.
    fallback: Option<Rc<dyn AssetBundle>>,
}

impl std::fmt::Debug for WorkspaceBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceBundle")
            .field("project", &self.project.borrow().is_some())
            .field("fallback", &self.fallback.is_some())
            .finish()
    }
}

impl WorkspaceBundle {
    /// Wrap `fallback` (the studio's existing bundle, if any) in a
    /// `WorkspaceBundle` and register it for `dyn AssetBundle` in `services`,
    /// returning the new bundle so the studio can later set the project path
    /// on it through [`Self::set_project`].
    ///
    /// Call once, at startup. Re-calling replaces the registration, which is
    /// unnecessary and would orphan the previous wrapper if the studio had
    /// handed it out — but is not wrong, because the new wrapper takes the
    /// same fallback.
    #[must_use]
    pub fn install(services: &mut Services) -> Rc<Self> {
        let fallback = services.get::<dyn AssetBundle>();
        let wrapper = Rc::new(Self {
            project: RefCell::new(None),
            fallback,
        });
        services.provide::<dyn AssetBundle>(Rc::clone(&wrapper) as Rc<dyn AssetBundle>);
        wrapper
    }

    /// Point the project half at `root/assets`, or clear it with `None`.
    ///
    /// `root` is the workspace root; the bundle lives at `<root>/assets`. A
    /// workspace without that directory is fine — the wrapper just has no
    /// project half, and reads fall through to the fallback.
    pub fn set_project(&self, root: Option<&std::path::Path>) {
        let next = root.and_then(|root| {
            let assets = root.join("assets");
            if assets.is_dir() {
                Some(DirectoryBundle::at(assets))
            } else {
                None
            }
        });
        *self.project.borrow_mut() = next;
    }

    /// The directory the project half is reading from, for the status bar.
    ///
    /// `None` when there is no workspace or the workspace has no `assets/`
    /// directory.
    #[must_use]
    pub fn project_root(&self) -> Option<PathBuf> {
        self.project
            .borrow()
            .as_ref()
            .map(|bundle| bundle.root().to_path_buf())
    }
}

impl AssetBundle for WorkspaceBundle {
    fn open(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        // Project first. `DirectoryBundle::open` returns `NotFound` if the
        // file is not in the directory, which is the case the fallback is for.
        if let Some(project) = self.project.borrow().as_ref() {
            if let Ok(bytes) = project.open(path) {
                return Ok(bytes);
            }
        }
        // Then the studio's own bundle, if there is one. A studio without a
        // registered bundle returns `NotFound`, which is the answer a guest
        // gets when nothing has the file — and the answer it got before this
        // wrapper existed.
        self.fallback
            .as_ref()
            .ok_or_else(|| AssetError::NotFound(path.to_owned()))?
            .open(path)
    }

    fn contains(&self, path: &str) -> bool {
        if let Some(project) = self.project.borrow().as_ref() {
            if project.contains(path) {
                return true;
            }
        }
        self.fallback
            .as_ref()
            .is_some_and(|fallback| fallback.contains(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_asset::EmbeddedBundle;

    /// A bundle whose `open` reads from a single embedded file, used as the
    /// "studio's own" fallback in the tests below.
    fn studio_bundle() -> Rc<dyn AssetBundle> {
        Rc::new(EmbeddedBundle::new().with("studio/logo.png", b"studio".as_slice()))
    }

    #[test]
    fn a_project_file_beats_the_studio_one() {
        let tmp = tempfile_dir();
        std::fs::write(tmp.join("avatar.png"), b"project").unwrap();
        let project = DirectoryBundle::at(tmp.clone());

        let wrapper = WorkspaceBundle {
            project: RefCell::new(Some(project)),
            fallback: Some(studio_bundle()),
        };
        // The same path exists in both bundles; the project's wins.
        assert_eq!(wrapper.open("avatar.png").unwrap(), b"project");
    }

    #[test]
    fn the_studio_bundle_is_the_fallback() {
        let wrapper = WorkspaceBundle {
            project: RefCell::new(None),
            fallback: Some(studio_bundle()),
        };
        assert_eq!(wrapper.open("studio/logo.png").unwrap(), b"studio");
    }

    #[test]
    fn neither_half_has_the_file_is_not_found() {
        let wrapper = WorkspaceBundle {
            project: RefCell::new(None),
            fallback: Some(studio_bundle()),
        };
        assert!(matches!(
            wrapper.open("missing.png"),
            Err(AssetError::NotFound(_))
        ));
    }

    #[test]
    fn set_project_swaps_the_directory() {
        let wrapper = WorkspaceBundle {
            project: RefCell::new(None),
            fallback: Some(studio_bundle()),
        };
        // No project half yet: a file the studio doesn't have is NotFound.
        assert!(wrapper.open("avatar.png").is_err());

        // Point it at a directory with the file.
        let tmp = tempfile_dir();
        std::fs::create_dir_all(tmp.join("assets")).unwrap();
        std::fs::write(tmp.join("assets").join("avatar.png"), b"project").unwrap();
        wrapper.set_project(Some(&tmp));
        assert_eq!(wrapper.open("avatar.png").unwrap(), b"project");

        // Clear it again: the file goes back to NotFound.
        wrapper.set_project(None);
        assert!(wrapper.open("avatar.png").is_err());
    }

    #[test]
    fn set_project_ignores_a_workspace_without_assets() {
        // A workspace root that does not contain an `assets/` directory is not
        // an error — the wrapper just has no project half.
        let tmp = tempfile_dir();
        let wrapper = WorkspaceBundle {
            project: RefCell::new(None),
            fallback: None,
        };
        wrapper.set_project(Some(&tmp));
        assert!(wrapper.project_root().is_none());
    }

    /// A unique temp directory for one test. `tempfile` is not in the studio's
    /// dependency graph, so this is built from `std::env::temp_dir` and a
    /// process-unique suffix.
    fn tempfile_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "viewwstudio-assets-test-{}-{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
