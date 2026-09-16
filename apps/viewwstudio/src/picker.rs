//! The in-app directory picker, and why there is one.
//!
//! # There is no file dialog to call
//!
//! `HTMLPROTOTYPEMAPPING.md` §7 lists it plainly: *native file picker dialog —
//! absent, no `FileDialog` service*. vieww publishes `Storage` and `Clipboard`
//! and nothing that opens a folder, so an application that wants one builds it.
//!
//! Until this module existed, the studio's workspace root was `argv[1]` and
//! only `argv[1]`. Launched from a desktop icon it opened on a scratch buffer
//! and stayed there: no file tree, no workspace search, no git, no cargo, no
//! build, no export — every one of those needs a root, and nothing in the
//! application could give it one. Seven of the nine Build menu items greyed
//! themselves out correctly and none of them could say why.
//!
//! # The shape
//!
//! One immutable listing per directory, re-read on every navigation. A picker
//! that cached would be a picker that shows a folder somebody deleted, and the
//! whole interaction is three clicks long — there is nothing here worth
//! keeping warm.
//!
//! Directories only. A folder picker that lists files is a folder picker that
//! makes the user scroll past forty `.rs` files to find `src`, and none of them
//! is a thing they can choose.
//!
//! `std` only, deliberately, like `json`, `task`, `toolchains`, `scaffold`,
//! `edit_ops` and `folding` — so it runs under `ci/standalone.sh` with no
//! network, no GPU and no font stack.
//!
//! # Volumes, not just parents
//!
//! `Picker::parent` walks up one component at a time, which works inside one
//! filesystem but stops at its root: `Path::parent("/").is_none()` on Unix and
//! `Path::parent("C:\\").is_none()` on Windows. A picker that started in
//! `~/projects` could not reach `/mnt/external` on Linux, `/Volumes/USB` on
//! macOS or `D:\\code` on Windows — which is the entire point of an "Open
//! Folder" dialog on a desktop, and the thing every native picker does with a
//! sidebar of drives.
//!
//! [`Picker::volumes`] is that sidebar. It is `std` only too: `/proc/mounts`
//! on Linux, `/Volumes` on macOS, drive letters on Windows, and the root plus
//! `$HOME` everywhere else. Each volume is one row in the picker's header, and
//! a click navigates rather than confirms — same as a directory row, just at
//! the other end of the address.

use std::path::{Path, PathBuf};

/// Directories that are never worth showing in a picker.
///
/// `target` is the big one: it is the largest directory in every Rust checkout,
/// it is never a workspace root, and a picker that lists it invites somebody to
/// open forty thousand build artefacts as a project.
const SKIPPED: [&str; 5] = ["target", "node_modules", ".git", ".cargo", ".rustup"];

/// One row in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// Whether this directory holds a `Cargo.toml` that mentions vieww.
    ///
    /// Shown as a mark on the row, so the folder somebody is looking for is
    /// visibly the folder somebody is looking for. It is *not* a filter: a
    /// user who wants to open something else is not wrong, and a picker that
    /// hides what it disapproves of is a picker people fight.
    pub is_vieww: bool,
}

/// A filesystem volume the picker can jump to.
///
/// A row in the picker's header rather than its body, because a volume is a
/// *peer* of the directory the picker is in rather than a child of it: `D:\`
/// is not inside `C:\Users`, and `/mnt/usb` is not inside `/home`. Listing
/// volumes among the entries would invite a click that looks right and goes
/// nowhere, which is the same reason `parent()` is shown as `..` and not as
/// a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// What the chip shows. The mountpoint's last component when there is one,
    /// the drive letter on Windows, `/` on Unix.
    pub name: String,
    /// Where the chip goes.
    pub path: PathBuf,
}

/// A directory, listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    /// The volumes this machine has mounted, in the order they should be
    /// offered — root first, then `$HOME` if it differs, then everything else
    /// discovered by [`volumes`]. Read at construction because a chip that
    /// changed mid-session would be a chip the user clicked that was no longer
    /// there, and a freshly-plugged USB between two clicks is rare enough that
    /// closing and reopening the picker is the correct refresh.
    pub volumes: Vec<Volume>,
    /// Set when `cwd` could not be read — a permission error, or a path that
    /// stopped existing between the click and the listing. Shown in place of
    /// the list rather than as an empty folder, which is the same picture for
    /// two very different situations.
    pub error: Option<String>,
}

impl Picker {
    /// List `at`.
    ///
    /// Never fails: an unreadable directory comes back as an empty listing
    /// carrying `error`, because the picker still has to draw something and
    /// "up" still has to work from wherever the user got stuck.
    #[must_use]
    pub fn at(at: impl AsRef<Path>) -> Self {
        let cwd = at.as_ref().to_path_buf();
        let read = match std::fs::read_dir(&cwd) {
            Ok(read) => read,
            Err(error) => {
                return Self {
                    cwd,
                    entries: Vec::new(),
                    volumes: volumes(),
                    error: Some(format!("Cannot open this folder — {error}")),
                }
            }
        };

        let mut entries: Vec<Entry> = read
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                // Hidden directories are hidden because their owner said so.
                // `.git` is in `SKIPPED` as well, for the day somebody turns
                // this rule off and wonders why the picker got slow.
                if name.starts_with('.') || SKIPPED.contains(&name.as_str()) {
                    return None;
                }
                let path = entry.path();
                Some(Entry {
                    is_vieww: is_vieww_workspace(&path),
                    name,
                    path,
                })
            })
            .collect();

        // Case-insensitive, so `Src` and `assets` sort where a reader expects
        // rather than where ASCII puts them. Ties broken by the raw name so the
        // order is total and the listing does not shuffle between reads.
        entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });

        Self {
            cwd,
            entries,
            volumes: volumes(),
            error: None,
        }
    }

    /// Where the picker opens when nothing better is known.
    ///
    /// The current directory, then `$HOME`, then the root. Three fallbacks
    /// because all three can fail — a process whose working directory was
    /// deleted, a daemon with no `HOME` — and a picker that panics on any of
    /// them is worse than one that opens somewhere unhelpful.
    #[must_use]
    pub fn start() -> Self {
        let at = std::env::current_dir()
            .ok()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/"));
        Self::at(at)
    }

    /// The directory above this one, if there is one.
    #[must_use]
    pub fn parent(&self) -> Option<PathBuf> {
        self.cwd.parent().map(Path::to_path_buf)
    }

    /// Whether `cwd` itself is a vieww workspace — which is what the picker's
    /// "Open this folder" button is confirming.
    #[must_use]
    pub fn cwd_is_vieww(&self) -> bool {
        is_vieww_workspace(&self.cwd)
    }

    /// What the header shows: the last two components, or the whole path when
    /// it is short.
    ///
    /// A picker sitting eight directories deep on a 248-point sidebar shows the
    /// *middle* of a path if it shows all of it, which is the least useful part.
    #[must_use]
    pub fn crumb(&self) -> String {
        let components: Vec<String> = self
            .cwd
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        match components.len() {
            0 => "/".to_owned(),
            1..=3 => self.cwd.display().to_string(),
            n => format!("…/{}/{}", components[n - 2], components[n - 1]),
        }
    }
}

/// Whether `path` holds a `Cargo.toml` that mentions vieww.
///
/// A read of one file, not a `cargo metadata`: the picker draws a mark beside a
/// row, and spawning cargo per row would make opening a folder with thirty
/// subdirectories take a second. It is a hint, and it is allowed to be wrong
/// about a workspace that renames its dependency.
#[must_use]
pub fn is_vieww_workspace(path: &Path) -> bool {
    let manifest = path.join("Cargo.toml");
    std::fs::read_to_string(manifest).is_ok_and(|text| text.contains("vieww"))
}

/// Every volume this machine has mounted.
///
/// `std` only, on every platform — see the module docs for why a platform
/// service was the wrong answer here. The list is recomputed each time a
/// `Picker` is constructed, which is each navigation, so a USB plugged in
/// mid-session appears on the next click rather than at the next launch.
///
/// # What is and is not a volume
///
/// A volume is a *root* the picker can jump to in one click. Internal
/// mountpoints on Linux (`/proc`, `/sys`, `/dev`, `/run/...`) are filtered out
/// because they are not directories a Rust project lives in; the same is true
/// of macOS's `/System/Volumes/*` synthetic links, which would otherwise
/// duplicate the root volume under a different name. Everything else —
/// removable media, network mounts, additional internal partitions — is kept.
#[must_use]
pub fn volumes() -> Vec<Volume> {
    let mut out: Vec<Volume> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // The root, on every platform. Always present so there is always a way
    // out of a deep tree even when no other volume is enumerable.
    push(
        &mut out,
        &mut seen,
        Volume {
            name: "/".to_owned(),
            path: PathBuf::from("/"),
        },
    );

    // `$HOME` is its own chip on every platform, because a user opening a
    // folder is usually opening one inside their home and the volume it lives
    // on is the wrong address for it. Listed before the platform enumeration
    // so the home chip sits next to `/` rather than between two USB drives.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        push(
            &mut out,
            &mut seen,
            Volume {
                name: "Home".to_owned(),
                path: home,
            },
        );
    }

    append_platform_volumes(&mut out, &mut seen);
    out
}

/// Push `volume` onto `out` unless `path` was already pushed.
///
/// The whole deduplication logic, named because it appears three times — for
/// `/`, for `$HOME`, and for each platform volume — and writing it once is the
/// difference between "obviously correct" and "correct after you read all
/// three".
fn push(out: &mut Vec<Volume>, seen: &mut std::collections::HashSet<PathBuf>, volume: Volume) {
    if volume.path.as_os_str().is_empty() {
        return;
    }
    // Canonicalise when possible, so `/home/./me` and `/home/me` do not both
    // appear. A failure to canonicalise — the path no longer exists, the
    // platform does not support it — falls back to the raw path, which is
    // still a useful row and never the wrong destination.
    let key = volume
        .path
        .canonicalize()
        .unwrap_or_else(|_| volume.path.clone());
    if seen.insert(key) {
        out.push(volume);
    }
}

#[cfg(target_os = "linux")]
fn append_platform_volumes(out: &mut Vec<Volume>, seen: &mut std::collections::HashSet<PathBuf>) {
    // `/proc/mounts` is the kernel's own list of mounted filesystems, in a
    // stable text format. It is the source of truth on Linux and the cheapest
    // thing to read: no `statvfs` per candidate, no `getmntent`, no library.
    let Ok(text) = std::fs::read_to_string("/proc/mounts") else {
        return;
    };
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(_device) = parts.next() else {
            continue;
        };
        let Some(raw) = parts.next() else { continue };
        let Some(fstype) = parts.next() else { continue };

        // Pseudo-filesystems are not places a project lives, and listing them
        // invites a click that opens an empty `sysfs` and looks like a bug.
        if !is_real_filesystem(fstype) {
            continue;
        }
        // `/proc/mounts` escapes spaces as `\040`, tabs as `\011`, etc. The
        // kernel's own format — reverse the four it actually uses.
        let mountpoint = unescape_mount(raw);

        // Skip the kernel's own pseudo-mounts that survived the fstype filter
        // (`tmpfs` on `/run/user/...`, `devtmpfs` on `/dev`, etc.). They are
        // real directories with real entries and would otherwise appear as
        // volumes, which they are not.
        if mountpoint == "/"
            || mountpoint.starts_with("/proc/")
            || mountpoint.starts_with("/sys/")
            || mountpoint.starts_with("/dev/")
            || mountpoint.starts_with("/run/")
        {
            continue;
        }

        let path = PathBuf::from(&mountpoint);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| mountpoint.clone());
        push(out, seen, Volume { name, path });
    }
}

#[cfg(target_os = "linux")]
fn is_real_filesystem(fstype: &str) -> bool {
    // The set of fstype strings the kernel reports for filesystems a Rust
    // project could plausibly live on. Anything else (`proc`, `sysfs`,
    // `devtmpfs`, `cgroup`, `tmpfs` on `/tmp`, `fuse.gvfsd-fuse`) is a kernel
    // or desktop abstraction, not a volume.
    matches!(
        fstype,
        "ext4"
            | "ext3"
            | "ext2"
            | "btrfs"
            | "xfs"
            | "zfs"
            | "f2fs"
            | "ntfs3"
            | "ntfs"
            | "vfat"
            | "exfat"
            | "exfat-fuse"
            | "fuseblk"
            | "fuse.sshfs"
            | "cifs"
            | "9p"
            | "ecryptfs"
            | "apfs"
            | "hfsplus"
    )
}

#[cfg(target_os = "linux")]
fn unescape_mount(raw: &str) -> String {
    // The kernel escapes octal sequences `\040` etc. in mount points. Only
    // space (`\040`), tab (`\011`), newline (`\012`) and backslash (`\134`)
    // are ever produced, so this is the entire table — and a hand-rolled scan
    // is the cheapest thing that handles `\040\040` correctly, which a
    // `replace` chain does too but at four allocations rather than one.
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut digits = String::new();
        for _ in 0..3 {
            match chars.peek() {
                Some(d) if d.is_ascii_digit() => digits.push(chars.next().unwrap()),
                _ => break,
            }
        }
        match u32::from_str_radix(digits.as_str(), 8) {
            Ok(0o040) => out.push(' '),
            Ok(0o011) => out.push('\t'),
            Ok(0o012) => out.push('\n'),
            Ok(0o134) => out.push('\\'),
            _ => {
                // An escape we do not recognise: keep it verbatim so a name
                // is mangled visibly rather than silently. This branch is
                // unreachable in practice; the comment is for the next reader.
                out.push('\\');
                out.push_str(&digits);
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn append_platform_volumes(out: &mut Vec<Volume>, seen: &mut std::collections::HashSet<PathBuf>) {
    // macOS mounts every external volume — USB drives, disk images, network
    // shares — under `/Volumes`. The root volume itself does not always appear
    // here, and when it does it is a synthetic link back to `/`, so it is
    // skipped by the deduplication against `/` already in `out`.
    let Ok(read) = std::fs::read_dir("/Volumes") else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        push(out, seen, Volume { name, path });
    }
}

#[cfg(target_os = "windows")]
fn append_platform_volumes(out: &mut Vec<Volume>, seen: &mut std::collections::HashSet<PathBuf>) {
    // Windows exposes each volume as a drive letter root. `C:` through `Z:`
    // is the range worth probing: `A:` and `B:` are reserved for floppies and
    // probing them stalls on hardware that still has the controller, which is
    // rare and exactly the case that hangs an "Open Folder" dialog on first
    // use. The 24 `is_dir` calls together cost a millisecond or two and never
    // touch the disk for letters with no volume assigned.
    for letter in b'C'..=b'Z' {
        // `u8 as char` is sound for every `u8` — and every byte in `C..=Z` is
        // ASCII, so the cast is the entire table.
        let letter_char = char::from_u32(u32::from(letter)).unwrap_or('?');
        let drive = format!("{letter_char}:\\");
        let path = PathBuf::from(drive);
        if !path.is_dir() {
            continue;
        }
        push(
            out,
            seen,
            Volume {
                name: format!("{letter_char}:"),
                path,
            },
        );
    }
    // UNC paths (`\\server\share`) are not enumerable by walking a directory,
    // and probing them would need a `WNet` call. They are rare enough in a
    // folder-picker context that an explicit "type a path" affordance would
    // serve them better than another row of chips — and that affordance does
    // not exist yet, so neither does this row.
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn append_platform_volumes(_out: &mut Vec<Volume>, _seen: &mut std::collections::HashSet<PathBuf>) {
    // No platform enumeration on FreeBSD, OpenBSD, illumos, Fuchsia, etc.
    // `/` and `$HOME` — already in `out` — are the two paths every Unix-like
    // agrees on, and a third platform that needs more deserves its own arm
    // rather than a guess.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary tree, removed when the test ends.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("vieww-picker-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("temp root");
            Self(root)
        }

        fn dir(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(&path).expect("temp dir");
            path
        }

        fn file(&self, name: &str, body: &str) {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("temp parent");
            }
            std::fs::write(path, body).expect("temp file");
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_directories_are_listed_and_they_are_sorted() {
        let tree = Tree::new("sorted");
        tree.dir("src");
        tree.dir("Assets");
        tree.dir("zebra");
        tree.file("Cargo.toml", "[package]\nname = \"x\"\n");
        tree.file("main.rs", "fn main() {}");

        let picker = Picker::at(&tree.0);
        let names: Vec<&str> = picker.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Assets", "src", "zebra"]);
    }

    #[test]
    fn hidden_and_heavy_directories_are_skipped() {
        let tree = Tree::new("skipped");
        tree.dir("src");
        tree.dir("target");
        tree.dir("node_modules");
        tree.dir(".git");
        tree.dir(".hidden");

        let picker = Picker::at(&tree.0);
        let names: Vec<&str> = picker.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src"]);
    }

    #[test]
    fn a_vieww_workspace_is_marked() {
        let tree = Tree::new("marked");
        let project = tree.dir("project");
        std::fs::write(
            project.join("Cargo.toml"),
            "[dependencies]\nvieww = { path = \"../vieww\" }\n",
        )
        .expect("manifest");
        tree.dir("other");
        std::fs::write(tree.0.join("other/Cargo.toml"), "[package]\nname = \"o\"\n")
            .expect("manifest");

        let picker = Picker::at(&tree.0);
        let marked: Vec<(&str, bool)> = picker
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_vieww))
            .collect();
        assert_eq!(marked, [("other", false), ("project", true)]);
    }

    /// The failure that would otherwise be an empty folder, which looks like a
    /// folder with nothing in it.
    #[test]
    fn an_unreadable_directory_says_so_rather_than_looking_empty() {
        let picker = Picker::at("/definitely/not/a/directory/anywhere");
        assert!(picker.entries.is_empty());
        assert!(picker.error.is_some(), "no error was reported");
        // And up still works from there, which is the only way out.
        assert!(picker.parent().is_some());
    }

    /// The same unreadable directory still offers volumes — a picker that
    /// loses its volume row when its directory cannot be read is one that
    /// traps the user in a folder the kernel would not let it list.
    #[test]
    fn an_unreadable_directory_still_offers_volumes() {
        let picker = Picker::at("/definitely/not/a/directory/anywhere");
        assert!(picker.error.is_some());
        assert!(!picker.volumes.is_empty(), "volumes must always be offered");
        assert!(
            picker.volumes.iter().any(|v| v.path == Path::new("/")),
            "the root is always reachable"
        );
    }

    #[test]
    fn the_root_has_no_parent_and_does_not_panic() {
        let picker = Picker::at("/");
        assert_eq!(picker.parent(), None);
        assert_eq!(picker.crumb(), "/");
    }

    #[test]
    fn a_deep_path_shows_its_last_two_components() {
        let picker = Picker {
            cwd: PathBuf::from("/home/someone/code/photo-studio"),
            entries: Vec::new(),
            volumes: Vec::new(),
            error: None,
        };
        assert_eq!(picker.crumb(), "…/code/photo-studio");
    }

    #[test]
    fn start_always_lists_something_readable() {
        let picker = Picker::start();
        assert!(picker.cwd.is_absolute(), "{}", picker.cwd.display());
    }

    // ------------------------------------------------------------- volumes

    /// The whole point of the change that added `volumes`: there is always at
    /// least one row, because a picker that can lose its way out of a deep
    /// tree is one a user closes and starts over.
    #[test]
    fn volumes_never_returns_empty() {
        let v = volumes();
        assert!(
            !v.is_empty(),
            "volumes() must always offer at least the root"
        );
        assert!(
            v.iter().any(|volume| volume.path == Path::new("/")),
            "the root is always present, on every platform"
        );
    }

    /// `push` is the entire deduplication contract: the same path under two
    /// names appears once, with the first name winning. This is what keeps
    /// `/` and `/Volumes/Macintosh HD` (which is `/`) from both appearing.
    ///
    /// Built on a real tempdir rather than `/` because `canonicalize` on `/`
    /// behaves differently across platforms, and the test is about `push`, not
    /// about the platform's canonical form of the root.
    #[test]
    fn push_deduplicates_by_canonical_path() {
        let tree = Tree::new("dedupe");
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        push(
            &mut out,
            &mut seen,
            Volume {
                name: "first".to_owned(),
                path: tree.0.clone(),
            },
        );
        push(
            &mut out,
            &mut seen,
            Volume {
                name: "second".to_owned(),
                path: tree.0.clone(),
            },
        );
        assert_eq!(out.len(), 1, "the same path appears once");
        assert_eq!(out[0].name, "first", "the first name wins");
    }

    /// A picker at `/` still offers every other volume the machine has —
    /// which is the behaviour the picker's "Open Folder" sidebar needs to
    /// have to be worth having at all.
    ///
    /// Not asserted as `picker.volumes == volumes()` because the two calls run
    /// at different instants and a mount event between them would make the
    /// test flake — the *contract* is "the picker carries the volume list",
    /// not "the picker carries one specific snapshot of it".
    #[test]
    fn a_picker_at_the_root_offers_every_volume() {
        let picker = Picker::at("/");
        assert!(!picker.volumes.is_empty());
        // The row that matches `cwd` is intentionally kept — hiding it would
        // make the chip list jump as the user navigates, which is the motion
        // a header should never make.
        assert!(
            picker.volumes.iter().any(|v| v.path == Path::new("/")),
            "the picker at / still lists / as a volume"
        );
    }

    /// A picker at a normal directory still carries every volume, because the
    /// directory it is in does not change what is mounted.
    #[test]
    fn a_picker_in_a_normal_directory_carries_every_volume() {
        let tree = Tree::new("volumes");
        let picker = Picker::at(&tree.0);
        assert!(!picker.volumes.is_empty());
        assert!(
            picker.volumes.iter().any(|v| v.path == Path::new("/")),
            "the root is always reachable from anywhere"
        );
    }

    /// Volumes navigate rather than confirm — they are entries in the same
    /// contract `Picker::at` honours, so a chip click is `picker_to(path)`,
    /// which is what the existing `Studio::picker_to` already calls. This is
    /// the test that keeps that contract honest.
    #[test]
    fn a_volume_path_is_a_valid_picker_at_target() {
        for volume in volumes() {
            // `Picker::at` never panics, even on paths it cannot read — that
            // is the contract `open_picker` relies on, and a volume that
            // violated it would crash the studio on a chip click.
            let _ = Picker::at(&volume.path);
        }
    }
}
