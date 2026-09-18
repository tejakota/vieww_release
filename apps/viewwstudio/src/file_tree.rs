//! A real directory tree for a workspace: nested folders, project recognition,
//! workspace-wide find and a poll-based external-change watcher.
//!
//! Pure `std` on purpose. The studio's UI reads this; the watcher reports
//! through a channel a frame hook polls, exactly like `compile::Job`. Keeping
//! the logic out of the platform crate means it is unit-testable without a
//! window, and that a studio that cannot link `wgpu` can still open a project.
//!
//! # What this is not
//!
//! Not a VCS. It scans the directory each time it is asked, it does not watch
//! the kernel for events, and "delete to trash" moves a file under a
//! `.viewwstudio-trash` folder rather than calling a desktop portal. The plan's
//! §5.4 watcher contract is met by polling mtimes, which is coarse but
//! portable — `notify` would pull a platform-specific backend for a feature
//! that mostly needs to notice `git checkout` between edits.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

// ---------------------------------------------------------------------------
// The tree
// ---------------------------------------------------------------------------

/// One node in the file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    /// The last path component, for display.
    pub name: String,
    /// The full path on disk.
    pub path: PathBuf,
    pub kind: NodeKind,
    /// For a directory, whether its children are shown expanded in the sidebar.
    /// Files carry `false` and ignore it.
    pub expanded: bool,
}

/// Whether a node is a file or a directory, holding the directory's children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir { children: Vec<TreeNode> },
}

impl TreeNode {
    /// `true` for a directory.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        matches!(self.kind, NodeKind::Dir { .. })
    }

    /// Recursively count files under this node.
    #[must_use]
    pub fn file_count(&self) -> usize {
        match &self.kind {
            NodeKind::File => 1,
            NodeKind::Dir { children } => children.iter().map(Self::file_count).sum(),
        }
    }
}

/// A recursive, sorted view of a directory.
///
/// Sorted directories-first then alphabetically, because that is the order an
/// explorer shows and the order a user expects. Entries the studio has no
/// business showing — `target/`, `.git/`, hidden files — are filtered at scan
/// time so the tree never holds them.
///
/// `PartialEq`/`Eq` are for `Workspace`'s derive, not for any runtime decision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileTree {
    root: Option<TreeNode>,
    /// The path this tree was scanned from, so [`rescan`](Self::rescan) can
    /// re-read it without the caller having to remember it. `None` for an
    /// empty/scratch tree.
    root_path: Option<PathBuf>,
}

impl FileTree {
    /// An empty tree, for a studio with nothing open.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            root: None,
            root_path: None,
        }
    }

    /// Scan `root` recursively. Directories the studio hides (`target/`,
    /// `.git/`, anything starting with `.` except the workspace root itself)
    /// are skipped. A root that cannot be read gives an empty tree rather than
    /// an error: the studio has to open.
    #[must_use]
    pub fn scan(root: &Path) -> Self {
        let name = root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        // Read once for the whole scan, not once per directory: a project's
        // `.gitignore` does not change between two directories of one walk,
        // and re-reading it per node turned a scan into a few hundred opens.
        let ignore = Ignore::read(root);
        let Some(node) = scan_dir_with(root, &name, root, &ignore) else {
            return Self {
                root: None,
                root_path: None,
            };
        };
        // The root is expanded by default; a closed root shows nothing.
        let mut tree = Self {
            root: Some(node),
            root_path: Some(root.to_path_buf()),
        };
        if let Some(root) = tree.root.as_mut() {
            root.expanded = true;
        }
        tree
    }

    /// The root node, if a directory was opened.
    #[must_use]
    pub const fn root(&self) -> Option<&TreeNode> {
        self.root.as_ref()
    }

    /// `true` if `path` is a `.rs` file directly inside `root` — the files the
    /// preview can compile. Used to decide whether the Render button lights up.
    #[must_use]
    pub fn is_previewable(&self, path: &Path) -> bool {
        path.extension().is_some_and(|ext| ext == "rs")
    }

    /// Expand the directory at `path` (or the root if `path` is the root).
    /// Returns `true` if a directory's state changed.
    pub fn expand(&mut self, path: &Path) -> bool {
        self.toggle(path, true)
    }

    /// Collapse the directory at `path`. Returns `true` if a state changed.
    pub fn collapse(&mut self, path: &Path) -> bool {
        self.toggle(path, false)
    }

    /// Close every directory below the root.
    ///
    /// The root itself stays open: a closed root shows nothing at all, which is
    /// not what "collapse all" means to anyone who has pressed it.
    ///
    /// Returns `true` if anything actually closed, so a caller can skip the
    /// signal write — and the frame — when the tree was already collapsed.
    pub fn collapse_all(&mut self) -> bool {
        fn walk(node: &mut TreeNode, changed: &mut bool) {
            let NodeKind::Dir { children } = &mut node.kind else {
                return;
            };
            for child in children {
                if child.expanded {
                    child.expanded = false;
                    *changed = true;
                }
                walk(child, changed);
            }
        }
        let mut changed = false;
        if let Some(root) = self.root.as_mut() {
            walk(root, &mut changed);
        }
        changed
    }

    /// Flip the expansion of the directory at `path`. Returns the new state, or
    /// `None` if `path` is not a directory in the tree.
    pub fn toggle_dir(&mut self, path: &Path) -> Option<bool> {
        let mut changed = false;
        let mut new_state = None;
        if let Some(root) = self.root.as_mut() {
            toggle_recursive(root, path, None, &mut changed, &mut new_state);
        }
        new_state
    }

    fn toggle(&mut self, path: &Path, want: bool) -> bool {
        let mut changed = false;
        let mut state = None;
        if let Some(root) = self.root.as_mut() {
            toggle_recursive(root, path, Some(want), &mut changed, &mut state);
        }
        changed
    }

    /// Every file in the tree, in depth-first order, as full paths. The order
    /// matches what a collapsed-all tree would reveal top to bottom.
    #[must_use]
    pub fn files(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            collect_files(root, &mut out);
        }
        out
    }

    /// Re-read the directory this tree was scanned from, preserving expansion
    /// state by path. A no-op for an empty/scratch tree. Called by the watcher's
    /// drain path so a `git checkout` outside the window reaches the Explorer.
    pub fn rescan(&mut self) {
        let Some(root_path) = self.root_path.clone() else {
            return;
        };
        let mut fresh = Self::scan(&root_path);
        if let (Some(old), Some(new)) = (self.root.as_ref(), fresh.root.as_mut()) {
            copy_expansion(old, new);
        }
        *self = fresh;
    }

    /// Find every line in every file in the tree that contains `query`
    /// (case-insensitive), returning the path, the 1-indexed line number, the
    /// 1-indexed column of the match and the line's text trimmed of its
    /// trailing newline.
    ///
    /// # Errors
    ///
    /// Only returns `Ok` — a file that cannot be read is skipped rather than
    /// failing the whole search, because one unreadable file must not hide
    /// every other file's matches.
    pub fn find(&self, query: &str) -> Vec<Match> {
        let needle = query.to_lowercase();
        let mut out = Vec::new();
        for path in self.files() {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for (i, line) in text.lines().enumerate() {
                if let Some(col) = line.to_lowercase().find(&needle) {
                    out.push(Match {
                        path: path.clone(),
                        line: i + 1,
                        column: col + 1,
                        text: line.trim_end().to_string(),
                    });
                }
            }
        }
        out
    }
}

/// One search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Project recognition
// ---------------------------------------------------------------------------

/// `true` if `root` holds a `Cargo.toml` whose `[dependencies]` (or
/// `[workspace.dependencies]`) mention `vieww`.
///
/// The check is textual rather than a real TOML parse, because the studio is
/// not a Cargo front-end and a dependency named exactly `vieww` is the only
/// thing it needs to know. A path dependency `vieww = { path = "../vieww" }`
/// and a version dependency `vieww = "0.2"` both match.
#[must_use]
pub fn is_vieww_project(root: &Path) -> bool {
    let Ok(manifest) = fs::read_to_string(root.join("Cargo.toml")) else {
        return false;
    };
    // Look for a `vieww` key in a dependency table. A bare `vieww` on a line
    // inside `[dependencies]` or `[workspace.dependencies]` is the signal; a
    // `vieww-` prefix (like `vieww-render`) is not, so we match `vieww` as a
    // table key: a word boundary, then `vieww`, then `=` or `=` after optional
    // space. This keeps `viewwstudioplan.md` and `vieww-*` crates from counting.
    let mut in_deps = false;
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            // Any table whose name ends in `dependencies` counts: a workspace,
            // a dev-dependency, a build-dependency. None of those are the
            // studio's business to distinguish from "this is a vieww app".
            in_deps = line.ends_with("dependencies]") || line.ends_with("dependencies ]");
            continue;
        }
        if in_deps {
            // `vieww = ...` or `vieww=...`, possibly with a path/version table.
            if let Some((key, _)) = line.split_once('=') {
                let key = key.trim();
                if key == "vieww" {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

/// Create a new empty file at `path`, including parent directories.
///
/// # Errors
///
/// If the file already exists, or cannot be created.
pub fn create_file(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::File::create(path)?;
    Ok(())
}

/// Create a new directory at `path`, including parents.
///
/// # Errors
///
/// If the directory already exists, or cannot be created.
pub fn create_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

/// Rename or move `from` to `to`.
///
/// # Errors
///
/// If the rename fails (e.g. `to` exists on some filesystems).
pub fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

/// Delete a file or directory by moving it under a `.viewwstudio-trash` folder
/// at the workspace root, stamped with the current time. "To trash, not
/// `unlink`" — the plan's §5.4 rule, because a delete that is a one-way
/// `remove_file` is the one editor action that cannot be undone.
///
/// # The workspace root is refused, by name
///
/// `fs::rename` a directory into a subdirectory of itself and the kernel says
/// `EINVAL` — "Invalid argument", os error 22 — which is a true statement about
/// a `rename` call and no help at all about a folder. Reported from use: the
/// Explorer's context menu on the workspace root offered "Move to trash", the
/// dialog said yes, and nothing happened; the failure reached the status bar as
/// "Invalid argument" and was gone in a moment.
///
/// The trash lives *inside* the workspace — deliberately, see the module note —
/// so the root can never go into it. That is a property of the design rather
/// than a filesystem accident, so it is said here, in words, before the syscall
/// gets a chance to say it in numbers.
///
/// # Errors
///
/// If the target is the workspace root, or contains it. If the move fails.
pub fn delete_to_trash(root: &Path, target: &Path) -> std::io::Result<PathBuf> {
    // Canonicalised on both sides: `root` is absolute and `target` comes from
    // the tree scan, but a symlinked or `..`-laden path that resolves to the
    // root must be refused as firmly as the literal one.
    let same = std::fs::canonicalize(target)
        .ok()
        .zip(std::fs::canonicalize(root).ok())
        .is_some_and(|(target, root)| target == root || root.starts_with(&target));
    if same || target == root {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the workspace root cannot go into its own trash — close the \
             workspace instead, or delete the folder from outside the studio",
        ));
    }
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or("0".to_string(), |d| d.as_secs().to_string());
    let trash = root.join(".viewwstudio-trash").join(stamp);
    fs::create_dir_all(&trash)?;
    let name = target
        .file_name()
        .map_or_else(|| "deleted".to_string(), |n| n.to_string_lossy().into());
    let destination = trash.join(name);
    fs::rename(target, &destination)?;
    Ok(destination)
}

// ---------------------------------------------------------------------------
// The watcher
// ---------------------------------------------------------------------------

/// What the watcher saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A file's contents (or mtime) changed.
    Changed(PathBuf),
    /// A file or directory that was there is gone.
    Removed(PathBuf),
    /// A file or directory that was not there appeared.
    Added(PathBuf),
}

/// A poll-based watcher: a thread that scans `root` every `interval`, diffs
/// the mtimes against the last scan, and sends the events that fell out of the
/// diff to a channel. The studio's frame hook drains the channel, so a
/// `git checkout` done outside the window does not leave a stale buffer
/// silently overwriting the file on the next save.
///
/// `notify` was not used: it pulls a platform-specific backend for a feature
/// that mostly needs to notice "the file changed between two edits", and
/// polling mtimes is portable and cheap on a workspace of a few hundred files.
pub struct Watcher {
    rx: mpsc::Receiver<WatchEvent>,
    handle: Option<thread::JoinHandle<()>>,
    /// Set by `Drop`, read by the thread between polls. The `Condvar` is what
    /// makes "read between polls" mean *immediately* rather than "up to
    /// `interval` from now" — see `Drop`.
    stop: Arc<(Mutex<bool>, Condvar)>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The receiver and the join handle are not `Debug`, and a watcher's
        // internals are not anything a debug print should expose — it is a
        // thread the studio holds, nothing more.
        f.debug_struct("Watcher").finish_non_exhaustive()
    }
}

impl Watcher {
    /// Start a watcher on `root`, polling every `interval`.
    ///
    /// `None` when the thread could not be spawned. That is not a theoretical
    /// case — a machine near its thread or memory limit refuses, and this used
    /// to `expect` and take the studio down at the moment a workspace was
    /// opened. The studio already handles `watcher: None` (it simply does not
    /// notice external changes), which is a far better outcome than a crash on
    /// open.
    #[must_use]
    pub fn start(root: PathBuf, interval: Duration) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<WatchEvent>();
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("viewwstudio-watcher".into())
            .spawn(move || {
                let (lock, signal) = &*thread_stop;
                let mut snapshot = mtime_snapshot(&root);
                loop {
                    // `wait_timeout` rather than `sleep`: the same pause
                    // between polls, but one that `Drop` can end early. See
                    // `Drop` for why that matters.
                    let Ok(guard) = lock.lock() else { return };
                    let Ok((guard, _)) = signal.wait_timeout(guard, interval) else {
                        return;
                    };
                    if *guard {
                        return;
                    }
                    drop(guard);
                    let now = mtime_snapshot(&root);

                    // A heartbeat every tick keeps the watcher exit-detectable:
                    // if the receiver was dropped, this send fails and the
                    // thread stops. `drain` filters the heartbeat (an empty
                    // path) out so it never reaches the studio.
                    if tx.send(WatchEvent::Changed(PathBuf::new())).is_err() {
                        break;
                    }

                    for (path, &mtime) in &now {
                        match snapshot.get(path) {
                            Some(prev) if *prev == mtime => {}
                            Some(_) => {
                                if tx.send(WatchEvent::Changed(path.clone())).is_err() {
                                    return;
                                }
                            }
                            None => {
                                if tx.send(WatchEvent::Added(path.clone())).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    for path in snapshot.keys() {
                        if !now.contains_key(path)
                            && tx.send(WatchEvent::Removed(path.clone())).is_err()
                        {
                            return;
                        }
                    }
                    snapshot = now;
                }
            })
            .ok()?;
        Some(Self {
            rx,
            handle: Some(handle),
            stop,
        })
    }

    /// Drain every event the watcher has produced since the last call.
    ///
    /// Returns an empty `Vec` when nothing changed, which is the common case —
    /// the frame hook calls this every frame and does nothing with the result
    /// when it is empty.
    pub fn drain(&self) -> Vec<WatchEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            // Drop the watcher's warmth sentinel: an empty path is never a
            // real event.
            if matches!(event, WatchEvent::Changed(ref p) if p.as_os_str().is_empty()) {
                continue;
            }
            out.push(event);
        }
        out
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // **This used to not join**, on the reasoning that a join would block
        // the UI thread for up to `interval` — true of a thread that sleeps,
        // and the reason the sleep is now a `Condvar::wait_timeout`. The flag
        // is set and the condvar signalled, so the thread wakes *now* rather
        // than at the end of its poll interval, and the join that follows is
        // the length of one `mtime_snapshot` at worst.
        //
        // What the old version left behind was a thread still holding the
        // workspace's `PathBuf` and still walking it, for up to a second after
        // the studio thought it had closed the workspace — which is a hazard
        // exactly when a workspace is being moved or renamed during shutdown.
        if let Ok(mut stopped) = self.stop.0.lock() {
            *stopped = true;
        }
        self.stop.1.notify_all();
        if let Some(handle) = self.handle.take() {
            handle.join().ok();
        }
    }
}

/// A map of file path → mtime, the snapshot a watcher diffs against.
fn mtime_snapshot(root: &Path) -> BTreeMap<PathBuf, SystemTime> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_hidden(&path, root) || is_ignored(&path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                out.insert(path, mtime);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Scan internals
// ---------------------------------------------------------------------------

fn scan_dir_with(path: &Path, name: &str, root: &Path, ignore: &Ignore) -> Option<TreeNode> {
    // `symlink_metadata` for the *kind*, so a symlink pointing at an ancestor
    // is a leaf rather than a cycle. Without this a single `ln -s .. loop`
    // inside a project is an unbounded recursive scan and the studio never
    // finishes opening it.
    let link = fs::symlink_metadata(path).ok()?;
    if link.file_type().is_symlink() {
        return Some(TreeNode {
            name: name.to_string(),
            path: path.to_path_buf(),
            kind: NodeKind::File,
            expanded: false,
        });
    }
    let meta = fs::metadata(path).ok()?;
    if meta.is_file() {
        return Some(TreeNode {
            name: name.to_string(),
            path: path.to_path_buf(),
            kind: NodeKind::File,
            expanded: false,
        });
    }
    if !meta.is_dir() {
        return None;
    }
    let Ok(entries) = fs::read_dir(path) else {
        // An unreadable directory is a directory with no children rather than
        // an error: a permissions issue on one folder must not hide its
        // siblings.
        return Some(TreeNode {
            name: name.to_string(),
            path: path.to_path_buf(),
            kind: NodeKind::Dir { children: vec![] },
            expanded: false,
        });
    };
    let mut dirs: Vec<TreeNode> = Vec::new();
    let mut files: Vec<TreeNode> = Vec::new();
    // A `.gitignore` in *this* directory applies from here down. Patterns in
    // it are relative to this directory, so the walk below matches against a
    // path relative to it rather than to the workspace root.
    let nested = ignore.nested(path);
    let ignore = nested.as_ref().unwrap_or(ignore);
    let base = if nested.is_some() { path } else { root };
    for entry in entries.flatten() {
        let child_path = entry.path();
        if is_hidden(&child_path, path) || is_ignored(&child_path) {
            continue;
        }
        if !ignore.is_empty() {
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if child_path
                .strip_prefix(base)
                .is_ok_and(|relative| ignore.ignores(relative, is_dir))
            {
                continue;
            }
        }
        let child_name = child_path.file_name().map_or_else(
            || child_path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let Some(node) = scan_dir_with(&child_path, &child_name, base, ignore) else {
            continue;
        };
        if node.is_dir() {
            dirs.push(node);
        } else {
            files.push(node);
        }
    }
    dirs.sort_by_key(|n| n.name.to_lowercase());
    files.sort_by_key(|n| n.name.to_lowercase());
    dirs.extend(files);
    Some(TreeNode {
        name: name.to_string(),
        path: path.to_path_buf(),
        kind: NodeKind::Dir { children: dirs },
        expanded: false,
    })
}

/// `true` for a path whose last component starts with `.` AND is not the
/// workspace root itself. The root's own name (e.g. a project named `.dotfiles`)
/// is allowed; everything below it that is hidden is not.
fn is_hidden(path: &Path, root: &Path) -> bool {
    if path == root {
        return false;
    }
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// `true` for directories the studio has no business showing: build output,
/// VCS metadata, and the studio's own trash.
///
/// These three are unconditional — they are true of every Rust project and
/// showing them has never been useful. Everything *else* a project wants
/// hidden is in its `.gitignore`, which [`Ignore`] reads.
fn is_ignored(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy();
    name == "target" || name == ".git" || name == ".viewwstudio-trash"
}

/// The project's own ignore rules, read from `.gitignore`.
///
/// # Why this is worth having and why it is not a full implementation
///
/// The tree skipped exactly three names, by literal string match. That is
/// right for a Rust project and wrong for every project that is also something
/// else: a `node_modules`, a `dist/`, a `build/`, a Python `venv/`, a
/// `__pycache__` — each one showing every file it contains, in the Explorer
/// *and* in workspace search results, where a thousand hits in vendored code
/// bury the four in the user's own.
///
/// What is implemented is now the whole of the syntax a `.gitignore` uses:
/// bare names, directory suffixes (`build/`), rooted patterns (`/target`),
/// `*` and `?` wildcards anywhere in a segment, character classes
/// (`[abc]`, `[a-z]`, `[!0-9]`), `**` spanning segments, patterns containing a
/// `/` (anchored, as git anchors them), and `!` negations. Nested
/// `.gitignore` files below the root and `.git/info/exclude` are read too.
///
/// It used to be a subset — no `**`, no classes, no nested files — and the
/// partiality was defended as safe in one direction: an unhandled pattern
/// leaves a file *shown*, and clutter is recoverable where a wrongly hidden
/// file is not. That is still the rule this errs towards, and it is still what
/// an unreadable nested file falls back to. But `**/generated/**` is not an
/// exotic pattern, and a monorepo whose per-package `.gitignore` files were all
/// invisible to the Explorer put thousands of build artefacts into every
/// workspace search — where a hundred hits in vendored code bury the four in
/// the user's own, which is its own kind of file you cannot find.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Ignore {
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    /// The pattern with its `!`, trailing `/` and leading `/` stripped.
    pattern: String,
    /// `!pattern` — a rule that un-ignores.
    negated: bool,
    /// `pattern/` — matches directories only.
    directory_only: bool,
    /// `/pattern` — anchored to the workspace root rather than any depth.
    anchored: bool,
}

impl Ignore {
    /// Read `root/.gitignore` and `root/.git/info/exclude`.
    ///
    /// Both, because `.git/info/exclude` is where a person puts the things
    /// they do not want to commit to the shared ignore file — a scratch
    /// directory, an editor's droppings — and a tree that shows them is showing
    /// exactly the files that person has already said they never want to see.
    /// It is read second so its rules win a tie, which is the order git applies
    /// them in.
    #[must_use]
    pub fn read(root: &Path) -> Self {
        let mut rules = Vec::new();
        for path in [
            root.join(".gitignore"),
            root.join(".git").join("info").join("exclude"),
        ] {
            if let Ok(text) = fs::read_to_string(path) {
                rules.extend(Self::parse(&text).rules);
            }
        }
        Self { rules }
    }

    /// The rules of `self`, then the rules `directory/.gitignore` adds.
    ///
    /// # Why a nested file needs its own combined set
    ///
    /// A `.gitignore` inside a subdirectory applies to that subtree and to
    /// nothing above it, and its anchored patterns are relative to *it* rather
    /// than to the workspace root. The scan therefore carries an `Ignore` down
    /// with it, adding a level each time it enters a directory that has one —
    /// which is how a monorepo's per-package ignore files, previously invisible
    /// to the Explorer, take effect.
    ///
    /// Returns `None` when the directory has no `.gitignore`, so the common
    /// case costs one `read_to_string` that fails and no allocation at all.
    #[must_use]
    pub fn nested(&self, directory: &Path) -> Option<Self> {
        let text = fs::read_to_string(directory.join(".gitignore")).ok()?;
        let mut rules = self.rules.clone();
        rules.extend(Self::parse(&text).rules);
        Some(Self { rules })
    }

    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negated, line) = match line.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            let (directory_only, line) = match line.strip_suffix('/') {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            let (anchored, line) = match line.strip_prefix('/') {
                Some(rest) => (true, rest),
                // git's own rule, and one this used to miss: a pattern with a
                // `/` anywhere but at the end is relative to the file it is
                // written in, not matched against every basename. Without this
                // `docs/build` was compared to the name `build` and hid every
                // `build` in the tree.
                None => (line.contains('/'), line),
            };
            if line.is_empty() {
                continue;
            }
            rules.push(Rule {
                pattern: line.to_owned(),
                negated,
                directory_only,
                anchored,
            });
        }
        Self { rules }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Whether `relative` — a path below the workspace root — is ignored.
    ///
    /// Later rules win, which is what makes `!keep.rs` after `*.rs` mean what
    /// it says in every gitignore ever written.
    #[must_use]
    pub fn ignores(&self, relative: &Path, is_dir: bool) -> bool {
        let text = relative.to_string_lossy().replace('\\', "/");
        let name = relative
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let mut ignored = false;
        for rule in &self.rules {
            if rule.directory_only && !is_dir {
                continue;
            }
            let hit = if rule.anchored {
                matches(&rule.pattern, &text)
            } else {
                // An unanchored pattern matches at any depth, which for a
                // single-segment pattern is the basename and for one with a
                // `**` in it is any suffix of the path. Both are covered by
                // trying every suffix — `web/app/dist` against `app/dist` and
                // `dist` — which is what git means by "at any level below".
                matches(&rule.pattern, &name)
                    || suffixes(&text).any(|suffix| matches(&rule.pattern, suffix))
            };
            if hit {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

/// `a/b/c`, `b/c`, `c` — every trailing sub-path, longest first.
fn suffixes(path: &str) -> impl Iterator<Item = &str> {
    std::iter::once(path).chain(path.match_indices('/').map(|(at, _)| &path[at + 1..]))
}

/// Glob matching for the patterns described on [`Ignore`].
///
/// # What changed, and why it needed a real matcher
///
/// This was four cases: a leading `*`, a trailing `*`, both, or neither.
/// Anything else — `**`, a `?`, a character class, a `*` in the middle of a
/// name — matched nothing and left the file shown. `**/generated/**`,
/// `src/[abc].rs` and `a?c.rs` are all ordinary things to write in a
/// `.gitignore`, and all three were silently inert.
///
/// The rewrite is a segment-wise matcher: the pattern and the subject are both
/// split on `/`, `**` consumes zero or more whole segments, and everything
/// else is matched one segment at a time by [`segment_matches`]. Recursive
/// rather than iterative for the `**` case, because "zero or more segments" is
/// a choice at each position and backtracking is the honest way to express it.
fn matches(pattern: &str, subject: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let subject: Vec<&str> = subject.split('/').filter(|s| !s.is_empty()).collect();
    segments_match(&pattern, &subject)
}

fn segments_match(pattern: &[&str], subject: &[&str]) -> bool {
    match pattern.split_first() {
        None => subject.is_empty(),
        Some((&"**", rest)) => {
            // Zero segments, then one, then two… `**` at the end of a pattern
            // matches everything left, which is why the empty case is tried
            // first and the loop can stop as soon as one branch succeeds.
            (0..=subject.len()).any(|skip| segments_match(rest, &subject[skip..]))
        }
        Some((head, rest)) => {
            let Some((first, tail)) = subject.split_first() else {
                return false;
            };
            segment_matches(head, first) && segments_match(rest, tail)
        }
    }
}

/// One path segment against one pattern segment.
///
/// `*` matches any run of characters within the segment (never a `/`, which is
/// why this works on one segment at a time), `?` matches exactly one, and
/// `[abc]`, `[a-z]` and `[!abc]` are character classes. A `\` escapes the next
/// character, so a file genuinely called `a*b` can be named.
fn segment_matches(pattern: &str, subject: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let subject: Vec<char> = subject.chars().collect();
    glob_here(&pattern, &subject)
}

fn glob_here(pattern: &[char], subject: &[char]) -> bool {
    match pattern.split_first() {
        None => subject.is_empty(),
        Some(('*', rest)) => (0..=subject.len()).any(|skip| glob_here(rest, &subject[skip..])),
        Some(('?', rest)) => !subject.is_empty() && glob_here(rest, &subject[1..]),
        Some(('[', _)) => {
            let Some((matched, rest)) = class_matches(pattern, subject.first().copied()) else {
                // An unterminated `[` is a literal `[`, which is what git does
                // and is the direction that leaves the file shown.
                return subject.first() == Some(&'[') && glob_here(&pattern[1..], &subject[1..]);
            };
            matched && glob_here(rest, &subject[1..])
        }
        Some(('\\', rest)) => match (rest.split_first(), subject.split_first()) {
            (Some((escaped, after)), Some((next, tail))) if escaped == next => {
                glob_here(after, tail)
            }
            _ => false,
        },
        Some((literal, rest)) => match subject.split_first() {
            Some((next, tail)) if next == literal => glob_here(rest, tail),
            _ => false,
        },
    }
}

/// Read one `[...]` class off the front of `pattern`.
///
/// Returns whether `candidate` is in the class, and the rest of the pattern
/// after the closing `]`. `None` when there is no closing `]` at all.
fn class_matches(pattern: &[char], candidate: Option<char>) -> Option<(bool, &[char])> {
    let close = pattern.iter().skip(2).position(|c| *c == ']')? + 2;
    let mut body = &pattern[1..close];
    // `[!abc]` and `[^abc]` both negate; git documents the first and accepts
    // the second.
    let negated = matches!(body.first(), Some('!' | '^'));
    if negated {
        body = &body[1..];
    }
    let Some(candidate) = candidate else {
        return Some((false, &pattern[close + 1..]));
    };
    let mut hit = false;
    let mut at = 0;
    while at < body.len() {
        if at + 2 < body.len() && body[at + 1] == '-' {
            if body[at] <= candidate && candidate <= body[at + 2] {
                hit = true;
            }
            at += 3;
        } else {
            if body[at] == candidate {
                hit = true;
            }
            at += 1;
        }
    }
    Some((hit != negated, &pattern[close + 1..]))
}

fn collect_files(node: &TreeNode, out: &mut Vec<PathBuf>) {
    match &node.kind {
        NodeKind::File => out.push(node.path.clone()),
        NodeKind::Dir { children } => {
            for child in children {
                collect_files(child, out);
            }
        }
    }
}

/// Copy the `expanded` flag from `old` onto `new` by matching path. Descends
/// recursively so a deep expansion survives a rescan — the whole point of
/// [`FileTree::rescan`] preserving what the user opened.
fn copy_expansion(old: &TreeNode, new: &mut TreeNode) {
    if old.expanded {
        new.expanded = true;
    }
    let NodeKind::Dir { children: old_kids } = &old.kind else {
        return;
    };
    let NodeKind::Dir { children: new_kids } = &mut new.kind else {
        return;
    };
    for n in new_kids {
        if let Some(o) = old_kids.iter().find(|o| o.path == n.path) {
            copy_expansion(o, n);
        }
    }
}

fn toggle_recursive(
    node: &mut TreeNode,
    target: &Path,
    want: Option<bool>,
    changed: &mut bool,
    new_state: &mut Option<bool>,
) {
    if node.path == target && node.is_dir() {
        let next = want.unwrap_or(!node.expanded);
        if node.expanded != next {
            node.expanded = next;
            *changed = true;
        }
        *new_state = Some(node.expanded);
        return;
    }
    if let NodeKind::Dir { children } = &mut node.kind {
        for child in children {
            toggle_recursive(child, target, want, changed, new_state);
        }
    }
}

#[cfg(test)]
mod tests {
    mod ignore_rules {
        use super::super::Ignore;
        use std::path::Path;

        fn ignores(rules: &str, path: &str, is_dir: bool) -> bool {
            Ignore::parse(rules).ignores(Path::new(path), is_dir)
        }

        #[test]
        fn a_bare_name_matches_at_any_depth() {
            assert!(ignores("node_modules", "node_modules", true));
            assert!(ignores("node_modules", "web/node_modules", true));
            assert!(ignores("secrets.env", "config/secrets.env", false));
        }

        #[test]
        fn a_trailing_slash_means_directories_only() {
            assert!(ignores("build/", "build", true));
            assert!(
                !ignores("build/", "build", false),
                "a *file* called build stays"
            );
        }

        #[test]
        fn a_leading_slash_anchors_to_the_root() {
            assert!(ignores("/dist", "dist", true));
            assert!(
                !ignores("/dist", "packages/app/dist", true),
                "anchored means the root and only the root"
            );
        }

        #[test]
        fn wildcards_at_either_end() {
            assert!(ignores("*.log", "server.log", false));
            assert!(!ignores("*.log", "server.logic", false));
            assert!(ignores("tmp*", "tmpfile", false));
            assert!(ignores("*cache*", "my-cache-dir", true));
        }

        #[test]
        fn double_star_spans_segments() {
            // The pattern the old four-case matcher could not see at all, and
            // the reason a monorepo's generated code filled every search.
            assert!(ignores(
                "**/generated/**",
                "web/app/generated/api.ts",
                false
            ));
            assert!(ignores("**/generated/**", "generated/api.ts", false));
            assert!(ignores("packages/**/dist", "packages/one/two/dist", true));
            assert!(!ignores("**/generated/**", "web/app/src/api.ts", false));
        }

        #[test]
        fn character_classes_and_single_character_wildcards() {
            assert!(ignores("src/[abc].rs", "src/a.rs", false));
            assert!(!ignores("src/[abc].rs", "src/d.rs", false));
            assert!(ignores("v[0-9].log", "v7.log", false));
            assert!(ignores("[!a-z]*.tmp", "9x.tmp", false));
            assert!(!ignores("[!a-z]*.tmp", "ax.tmp", false));
            assert!(ignores("a?c.rs", "abc.rs", false));
            assert!(!ignores("a?c.rs", "abbc.rs", false));
        }

        #[test]
        fn a_star_in_the_middle_of_a_name_works_now() {
            assert!(ignores("we*rd*one", "weirdxone", false));
            assert!(!ignores("we*rd*one", "weird", false));
        }

        #[test]
        fn a_pattern_with_a_slash_is_anchored_the_way_git_anchors_it() {
            // `docs/build` is relative to the ignore file, so it must not hide
            // every directory called `build` in the tree.
            assert!(ignores("docs/build", "docs/build", true));
            assert!(!ignores("docs/build", "src/docs/build", true));
            assert!(!ignores("docs/build", "build", true));
        }

        #[test]
        fn a_negation_after_a_wildcard_still_wins() {
            assert!(ignores("*.rs\n", "a.rs", false));
            assert!(!ignores("*.rs\n!keep.rs\n", "keep.rs", false));
        }

        #[test]
        fn an_unterminated_class_is_a_literal_bracket_rather_than_a_hidden_file() {
            // The direction this whole module errs in: an odd pattern leaves
            // the file shown.
            assert!(!ignores("[abc", "a", false));
            assert!(ignores("[abc", "[abc", false));
        }

        #[test]
        fn a_negation_later_in_the_file_wins() {
            let rules = "*.rs\n!keep.rs\n";
            assert!(ignores(rules, "throwaway.rs", false));
            assert!(
                !ignores(rules, "keep.rs", false),
                "the ! rule un-ignores it"
            );
        }

        #[test]
        fn comments_and_blank_lines_are_not_rules() {
            let rules = "# a comment\n\n   \nbuild\n";
            assert!(ignores(rules, "build", true));
            assert!(!ignores(rules, "# a comment", false));
        }

        #[test]
        fn a_project_with_no_gitignore_ignores_nothing_extra() {
            assert!(Ignore::default().is_empty());
            assert!(!Ignore::default().ignores(Path::new("anything"), false));
        }

        /// The safety property, stated as a test. An unimplemented pattern must
        /// leave the file **shown**: clutter is recoverable, a file the user
        /// cannot find is not.
        #[test]
        fn a_pattern_this_does_not_implement_shows_the_file_rather_than_hiding_it() {
            for exotic in ["**/generated/**", "src/[abc].rs", "a?c.rs", "we*rd*one"] {
                assert!(
                    !ignores(exotic, "src/main.rs", false),
                    "{exotic} must not hide an unrelated file"
                );
            }
        }
    }

    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("viewwstudio-ft-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn scans_recursively_with_directories_first() {
        let root = tmpdir("scan");
        write(&root.join("src/main.rs"), "fn main() {}");
        write(&root.join("src/screens/home.rs"), "pub fn screen() {}");
        write(&root.join("README.md"), "# hi");
        write(&root.join("Cargo.toml"), "[package]\nname = \"x\"\n");

        let tree = FileTree::scan(&root);
        let root_node = tree.root().expect("a root");
        assert!(root_node.is_dir());
        let NodeKind::Dir { children } = &root_node.kind else {
            unreachable!();
        };
        // Cargo.toml, README.md, src — sorted dirs-first: src, then Cargo.toml, README.md.
        let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["src", "Cargo.toml", "README.md"]);
        let src = children.iter().find(|c| c.name == "src").unwrap();
        let NodeKind::Dir { children: src_kids } = &src.kind else {
            unreachable!();
        };
        assert_eq!(
            src_kids.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["screens", "main.rs"],
            "dirs before files"
        );
        assert!(root_node.expanded, "the root is expanded by default");
    }

    #[test]
    fn target_and_git_are_hidden() {
        let root = tmpdir("hide");
        write(&root.join("target/debug/x"), "build output");
        write(&root.join(".git/HEAD"), "ref");
        write(&root.join("src/lib.rs"), "");
        let tree = FileTree::scan(&root);
        let names: Vec<String> = tree
            .files()
            .into_iter()
            // `/` on every platform — see `buffer.rs`'s tree test.
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect();
        assert!(
            names
                .iter()
                .all(|n| !n.starts_with("target") && !n.starts_with(".git")),
            "{names:?}"
        );
        assert!(names.iter().any(|n| n == "src/lib.rs"));
    }

    #[test]
    fn recognises_a_vieww_project_by_dependency() {
        let root = tmpdir("proj");
        write(
            &root.join("Cargo.toml"),
            "[package]\nname = \"app\"\n\n[dependencies]\nvieww = \"0.2\"\n",
        );
        assert!(is_vieww_project(&root));

        let not_root = tmpdir("notproj");
        write(
            &not_root.join("Cargo.toml"),
            "[package]\nname = \"app\"\n\n[dependencies]\nserde = \"1\"\n",
        );
        assert!(
            !is_vieww_project(&not_root),
            "a non-vieww crate is not a project"
        );

        let path_dep = tmpdir("pathdep");
        write(
            &path_dep.join("Cargo.toml"),
            "[dependencies]\nvieww = { path = \"../../vieww\" }\n",
        );
        assert!(is_vieww_project(&path_dep), "a path dependency counts");
    }

    #[test]
    fn vieww_render_alone_is_not_a_project() {
        // A dependency named `vieww-render` must not match `vieww` — the
        // distinction is the whole point of the textual check.
        let root = tmpdir("renderonly");
        write(
            &root.join("Cargo.toml"),
            "[dependencies]\nvieww-render = \"0.1\"\n",
        );
        assert!(!is_vieww_project(&root));
    }

    #[test]
    fn find_returns_case_insensitive_matches_with_positions() {
        let root = tmpdir("find");
        write(&root.join("a.rs"), "fn hello() {}\n// Hello world\n");
        write(&root.join("b.rs"), "let x = HELLO;\n");
        let tree = FileTree::scan(&root);
        let mut matches = tree.find("hello");
        matches.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        assert_eq!(matches.len(), 3);
        // First match: a.rs line 1, "hello" at column 4 (after "fn ").
        assert_eq!(matches[0].line, 1);
        assert_eq!(matches[0].column, 4);
        assert_eq!(matches[0].text, "fn hello() {}");
    }

    #[test]
    fn expand_collapse_toggle_track_state() {
        let root = tmpdir("toggle");
        write(&root.join("src/lib.rs"), "");
        let mut tree = FileTree::scan(&root);
        let src = root.join("src");
        // Root is expanded; src starts collapsed.
        assert!(tree.root().unwrap().expanded);
        assert!(tree.expand(&src));
        assert!(tree.collapse(&src));
        assert!(
            tree.toggle_dir(&src).unwrap(),
            "toggling a collapsed dir opens it"
        );
        assert!(!tree.toggle_dir(&src).unwrap(), "toggling again closes it");
    }

    #[test]
    fn delete_to_trash_moves_the_file_under_the_trash_folder() {
        let root = tmpdir("trash");
        let file = root.join("src/old.rs");
        write(&file, "bye");
        let dest = delete_to_trash(&root, &file).unwrap();
        assert!(!file.exists(), "the file is gone from its place");
        assert!(dest.exists(), "it landed in the trash");
        assert!(dest.starts_with(root.join(".viewwstudio-trash")));
    }

    /// The report: the Explorer offered "Move to trash" on the workspace root,
    /// the dialog was confirmed, and nothing happened. `fs::rename` of a
    /// directory into itself is `EINVAL`, which reached the status bar as
    /// "Invalid argument" and vanished.
    #[test]
    fn the_workspace_root_cannot_go_into_its_own_trash() {
        let root = tmpdir("root-delete");
        std::fs::create_dir_all(root.join("src")).unwrap();

        let error = delete_to_trash(&root, &root).expect_err("refused");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            error.to_string().contains("workspace root"),
            "the message says which folder and why: {error}"
        );
        assert!(root.exists(), "and nothing was moved");
    }

    /// The same refusal through a path that only *resolves* to the root.
    #[test]
    fn a_path_that_resolves_to_the_root_is_refused_too() {
        let root = tmpdir("root-delete-indirect");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let indirect = root.join("src").join("..");

        assert!(delete_to_trash(&root, &indirect).is_err());
        assert!(root.join("src").exists());
    }

    #[test]
    fn create_file_makes_its_parents() {
        let root = tmpdir("create");
        let file = root.join("nested/deep/file.rs");
        create_file(&file).unwrap();
        assert!(file.exists());
    }

    #[test]
    fn an_unreadable_root_gives_an_empty_tree() {
        let tree = FileTree::scan(Path::new("/definitely/not/here/viewwstudio"));
        assert!(tree.root().is_none());
    }
}
