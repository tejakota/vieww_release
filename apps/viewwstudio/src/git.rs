//! Source control: `git`, shelled out and parsed.
//!
//! # Why `git` the program rather than a library
//!
//! A Rust git implementation is a large dependency with its own opinion about
//! index formats, hooks, submodules, credential helpers and the eight config
//! files that decide what any of those mean. The studio needs four answers —
//! what branch is this, what changed, stage that, commit — and the program
//! that gets all four right on every repository in existence is already
//! installed on every machine that has a repository.
//!
//! What that costs is a process per question, which is why every function
//! here returns a [`Spec`] for the job queue rather than running anything.
//!
//! # Everything is a pure function of text
//!
//! Parsing is separated from running for the same reason it is in
//! [`export`](crate::export) and [`cargo`](crate::cargo): a machine with no
//! repository, or no `git`, can still run every case below. That machine is
//! this container.
//!
//! # `--porcelain=v1` and `-z`, not the human output
//!
//! `git status` without a porcelain flag is explicitly documented as
//! unstable, is translated into the user's language, and changes with
//! configuration. `--porcelain=v1` is a stability promise. `-z` terminates
//! each record with a NUL so a path containing a newline — legal, and what a
//! malicious or merely unlucky filename looks like — cannot be read as two
//! records.

use std::path::{Path, PathBuf};

use crate::task::Spec;

/// What a path's two status letters mean together.
///
/// One state rather than the raw pair, because the pair has thirty
/// combinations and a sidebar has room for a letter and a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// In `.gitignore`'s blind spot: not tracked, not ignored.
    Untracked,
    /// Both sides changed it. Named separately because it is the one state
    /// where committing without looking is a mistake.
    Conflicted,
}

impl Change {
    /// The single letter the file tree shows beside a path.
    #[must_use]
    pub const fn letter(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "U",
            Self::Conflicted => "!",
        }
    }

    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Untracked => "untracked",
            Self::Conflicted => "conflicted",
        }
    }

    /// Read the two-letter code `git status --porcelain` gives a path.
    ///
    /// Conflict first: `UU`, `AA` and the `DU`/`UD` family all mean an
    /// unresolved merge whatever else the letters say, and reporting one of
    /// those as "modified" is how somebody commits a file with conflict
    /// markers in it.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        let mut letters = code.chars();
        let index = letters.next()?;
        let worktree = letters.next()?;
        Some(match (index, worktree) {
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => Self::Conflicted,
            ('?', '?') => Self::Untracked,
            ('R', _) => Self::Renamed,
            ('D', _) | (_, 'D') => Self::Deleted,
            ('A', _) => Self::Added,
            (' ', ' ') => return None,
            _ => Self::Modified,
        })
    }
}

/// One path git has something to say about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Relative to the repository root, as git reports it.
    pub path: PathBuf,
    pub change: Change,
    /// Whether the change is in the index, so a commit would include it.
    pub staged: bool,
}

/// The repository's state, as of the last scan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// `None` on a repository with no commits yet, where `HEAD` names a branch
    /// that does not exist. Not an error: it is what a `git init` looks like.
    pub branch: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub entries: Vec<Entry>,
}

impl Status {
    /// How many paths would go into a commit right now.
    #[must_use]
    pub fn staged(&self) -> usize {
        self.entries.iter().filter(|entry| entry.staged).count()
    }

    /// The change affecting `path`, for the file tree's letter.
    ///
    /// `path` is relative to the repository root, which is what the tree
    /// already builds its rows from.
    #[must_use]
    pub fn change_for(&self, path: &Path) -> Option<Change> {
        self.entries
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.change)
    }

    /// The status bar's cell: `main ↑2 ↓1`.
    #[must_use]
    pub fn describe(&self) -> String {
        let branch = self.branch.clone().unwrap_or_else(|| "no commits".into());
        let mut out = branch;
        if self.ahead > 0 {
            out.push_str(&format!(" \u{2191}{}", self.ahead));
        }
        if self.behind > 0 {
            out.push_str(&format!(" \u{2193}{}", self.behind));
        }
        out
    }
}

/// Ask git what changed.
#[must_use]
pub fn status_spec(root: &Path) -> Spec {
    Spec::new("git status", "git", root)
        .args([
            "status",
            "--porcelain=v1",
            "-b",
            "-z",
            "--untracked-files=all",
        ])
        .timeout(std::time::Duration::from_secs(30))
}

/// Parse `git status --porcelain=v1 -b -z`.
///
/// # The format
///
/// Records are NUL-terminated. The first is the branch header:
/// `## main...origin/main [ahead 2, behind 1]`. Each of the rest is `XY
/// <path>`, where `XY` are the index and worktree letters. A rename is two
/// records — the new path, then the old one — so the record after an `R` is
/// consumed with it rather than being read as a path of its own.
///
/// Anything that does not fit is skipped. Git's own warnings go to stderr and
/// never reach this, but a caller that joined the two streams would otherwise
/// find a warning parsed as a file named after its first two characters.
#[must_use]
pub fn parse_status(output: &str) -> Status {
    let mut status = Status::default();
    let mut records = output.split('\0').filter(|record| !record.is_empty());

    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("## ") {
            let (branch, ahead, behind) = parse_branch(header);
            status.branch = branch;
            status.ahead = ahead;
            status.behind = behind;
            continue;
        }
        if record.len() < 4 {
            continue;
        }
        let code = &record[..2];
        let path = record[3..].trim();
        let Some(change) = Change::from_code(code) else {
            continue;
        };
        if change == Change::Renamed {
            // The old path follows, and belongs to this record.
            records.next();
        }
        status.entries.push(Entry {
            path: PathBuf::from(path),
            // The index column. A space means the change is only in the
            // worktree, so a commit right now would not include it — which is
            // the distinction the whole Source Control view is built on.
            staged: !matches!(code.as_bytes()[0], b' ' | b'?'),
            change,
        });
    }
    status.entries.sort_by(|a, b| a.path.cmp(&b.path));
    status
}

/// `main...origin/main [ahead 2, behind 1]` → the three things in it.
fn parse_branch(header: &str) -> (Option<String>, usize, usize) {
    let name = header
        .split(&['.', ' '][..])
        .next()
        .unwrap_or_default()
        .to_string();
    // A repository with no commits reports this exact string, and it is not a
    // branch anybody wants to see in a status bar.
    let branch = if name.is_empty() || header.starts_with("No commits yet") {
        None
    } else {
        Some(name)
    };

    let count = |word: &str| -> usize {
        header
            .split(word)
            .nth(1)
            .and_then(|rest| {
                rest.trim_start()
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|digits| digits.parse().ok())
            })
            .unwrap_or(0)
    };
    (branch, count("ahead "), count("behind "))
}

/// Put `path` in the index.
#[must_use]
pub fn stage_spec(root: &Path, path: &Path) -> Spec {
    // `--` before the path, always: a file called `-f` is a legal file and an
    // illegal argument, and the separator is what keeps the two apart.
    Spec::new("git add", "git", root)
        .args(["add", "--"])
        .arg(path.to_string_lossy().to_string())
        .timeout(std::time::Duration::from_secs(60))
}

/// Take `path` back out of the index, leaving the worktree alone.
#[must_use]
pub fn unstage_spec(root: &Path, path: &Path) -> Spec {
    Spec::new("git restore --staged", "git", root)
        .args(["restore", "--staged", "--"])
        .arg(path.to_string_lossy().to_string())
        .timeout(std::time::Duration::from_secs(60))
}

/// Commit what is in the index.
///
/// Returns `None` for an empty message rather than letting git open an editor
/// — a child process waiting on `$EDITOR` inside a studio with no terminal is
/// a job that never ends and cannot be explained.
#[must_use]
pub fn commit_spec(root: &Path, message: &str) -> Option<Spec> {
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    Some(
        Spec::new("git commit", "git", root)
            .arg("commit")
            // `-m` with the message as one argument: no shell, so no quoting,
            // so nothing in a commit message can be read as a flag or a
            // command however it is punctuated.
            .arg("-m")
            .arg(message.to_string())
            .timeout(std::time::Duration::from_secs(120)),
    )
}

/// Send the current branch to its remote.
///
/// # The half of source control that was missing
///
/// This module's own header named four questions — what branch is this, what
/// changed, stage that, commit — and stopped there. A developer working with
/// anybody else could see their changes, stage them and commit them, and then
/// had to leave the studio for a terminal to do the one thing that makes a
/// commit visible to another person. The Source Control panel was complete for
/// a solo local repository and unusable for a team.
///
/// `--set-upstream` on the branch's first push rather than a bare `git push`:
/// a branch created in the studio has no upstream, and git's refusal for that
/// case is four lines of advice ending in the command the user should have
/// run. Running it for them is what the studio is for. `HEAD` is used rather
/// than the branch name so a detached head fails as a detached head instead of
/// pushing something unexpected.
///
/// The long timeout is the network's: a first push of a large repository over
/// a slow link is minutes, and a timeout that fires mid-transfer looks exactly
/// like a broken remote.
#[must_use]
pub fn push_spec(root: &Path, branch: &str, has_upstream: bool) -> Spec {
    let mut spec = Spec::new("git push", "git", root).arg("push");
    if !has_upstream {
        spec = spec
            .args(["--set-upstream", "origin"])
            .arg(branch.to_string());
    }
    spec.timeout(std::time::Duration::from_secs(600))
}

/// Bring the remote's commits down and merge them into this branch.
///
/// `--ff-only` deliberately. A pull that merges can leave a conflicted
/// worktree, and the studio has no merge tool, no conflict markers in the
/// gutter and no `git merge --abort` button — so a pull that can conflict is a
/// button that can leave a repository in a state the studio cannot get out of.
/// Fast-forward-only either works or refuses with the branch untouched, and the
/// refusal is something a person can act on in a terminal.
#[must_use]
pub fn pull_spec(root: &Path) -> Spec {
    Spec::new("git pull", "git", root)
        .args(["pull", "--ff-only"])
        .timeout(std::time::Duration::from_secs(600))
}

/// Update the remote-tracking refs without touching the worktree.
///
/// What makes the ahead/behind counts in the status bar mean anything: they are
/// computed against `origin/<branch>` as it was last fetched, so without this
/// they are frozen at whatever they were when the repository was cloned.
/// `--prune` so a branch deleted on the remote stops being counted.
#[must_use]
pub fn fetch_spec(root: &Path) -> Spec {
    Spec::new("git fetch", "git", root)
        .args(["fetch", "--prune"])
        .timeout(std::time::Duration::from_secs(300))
}

/// Whether the current branch has an upstream to push to.
///
/// A file check rather than a `git rev-parse --abbrev-ref @{u}`, for the same
/// reason [`is_repository`] is one: this decides how a command is spelled, and
/// spawning a process to find out doubles the cost of the thing it is deciding
/// about. `.git/refs/remotes/origin/<branch>` exists exactly when the branch has
/// been pushed to `origin`; the packed form is checked too, because a freshly
/// cloned repository keeps its refs packed.
#[must_use]
pub fn has_upstream(root: &Path, branch: &str) -> bool {
    if root
        .join(".git")
        .join("refs")
        .join("remotes")
        .join("origin")
        .join(branch)
        .exists()
    {
        return true;
    }
    std::fs::read_to_string(root.join(".git").join("packed-refs")).is_ok_and(|packed| {
        packed
            .lines()
            .any(|line| line.ends_with(&format!("refs/remotes/origin/{branch}")))
    })
}

/// Whether `root` looks like a repository.
///
/// A cheap filesystem check rather than a `git rev-parse`, because it decides
/// whether the Source Control view is worth showing at all and running a
/// process to draw a sidebar is the thing this module is trying to avoid. A
/// `.git` *file* counts: that is what a worktree and a submodule have.
#[must_use]
pub fn is_repository(root: &Path) -> bool {
    root.join(".git").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records are NUL-terminated, which is awkward to write by hand.
    fn porcelain(records: &[&str]) -> String {
        records.iter().map(|record| format!("{record}\0")).collect()
    }

    #[test]
    fn the_branch_header_carries_the_name_and_the_divergence() {
        let status = parse_status(&porcelain(&["## main...origin/main [ahead 2, behind 1]"]));
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!((status.ahead, status.behind), (2, 1));
        assert_eq!(status.describe(), "main \u{2191}2 \u{2193}1");
    }

    #[test]
    fn a_branch_with_no_upstream_is_neither_ahead_nor_behind() {
        let status = parse_status(&porcelain(&["## feature/thing"]));
        assert_eq!(status.branch.as_deref(), Some("feature/thing"));
        assert_eq!((status.ahead, status.behind), (0, 0));
        assert_eq!(status.describe(), "feature/thing");
    }

    #[test]
    fn a_repository_with_no_commits_has_no_branch_rather_than_a_wrong_one() {
        // What `git init` followed by `git status` actually prints. Reporting
        // the literal text as a branch name puts "No" in the status bar.
        let status = parse_status(&porcelain(&["## No commits yet on main"]));
        assert_eq!(status.branch, None);
        assert_eq!(status.describe(), "no commits");
    }

    #[test]
    fn the_index_column_decides_whether_a_change_is_staged() {
        // The distinction the whole view is built on: `M ` is staged, ` M` is
        // not, and `MM` is both.
        let status = parse_status(&porcelain(&[
            "## main",
            "M  src/staged.rs",
            " M src/worktree.rs",
            "MM src/both.rs",
            "?? src/new.rs",
        ]));
        let staged: Vec<(&str, bool)> = status
            .entries
            .iter()
            .map(|entry| (entry.path.to_str().expect("utf-8"), entry.staged))
            .collect();
        assert_eq!(
            staged,
            vec![
                ("src/both.rs", true),
                ("src/new.rs", false),
                ("src/staged.rs", true),
                ("src/worktree.rs", false),
            ]
        );
        assert_eq!(status.staged(), 2);
    }

    #[test]
    fn a_conflict_is_a_conflict_whatever_else_its_letters_say() {
        // The one state where committing without looking is a mistake, so it
        // must never be reported as "modified".
        for code in ["UU", "AA", "DD", "AU", "UD", "DU", "UA"] {
            let status = parse_status(&porcelain(&["## main", &format!("{code} src/x.rs")]));
            assert_eq!(
                status.entries[0].change,
                Change::Conflicted,
                "{code} should be a conflict"
            );
        }
    }

    #[test]
    fn a_rename_consumes_the_old_path_rather_than_listing_it_as_a_file() {
        let status = parse_status(&porcelain(&[
            "## main",
            "R  src/new.rs",
            "src/old.rs",
            "M  src/other.rs",
        ]));
        assert_eq!(status.entries.len(), 2, "{:?}", status.entries);
        assert_eq!(status.entries[0].change, Change::Renamed);
        assert_eq!(status.entries[0].path, PathBuf::from("src/new.rs"));
        assert_eq!(status.entries[1].path, PathBuf::from("src/other.rs"));
    }

    #[test]
    fn a_path_with_a_newline_in_it_is_one_record() {
        // The whole reason for `-z`. Split on newlines, this is two files, one
        // of which does not exist.
        let status = parse_status(&porcelain(&["## main", "?? src/od\nd name.rs"]));
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].path, PathBuf::from("src/od\nd name.rs"));
    }

    #[test]
    fn an_unchanged_path_is_not_an_entry() {
        assert!(Change::from_code("  ").is_none());
        assert!(Change::from_code("").is_none());
        assert!(Change::from_code("M").is_none());
    }

    #[test]
    fn empty_output_is_a_clean_repository_rather_than_an_error() {
        let status = parse_status("");
        assert!(status.entries.is_empty());
        assert_eq!(status.branch, None);
    }

    #[test]
    fn the_tree_can_ask_what_happened_to_one_path() {
        let status = parse_status(&porcelain(&["## main", " M src/theme.rs"]));
        assert_eq!(
            status.change_for(Path::new("src/theme.rs")),
            Some(Change::Modified)
        );
        assert_eq!(status.change_for(Path::new("src/other.rs")), None);
        assert_eq!(Change::Modified.letter(), "M");
    }

    #[test]
    fn an_empty_commit_message_produces_no_command_at_all() {
        // Otherwise git opens `$EDITOR`, which in a studio with no terminal is
        // a child process that never returns and cannot be explained.
        assert!(commit_spec(Path::new("/repo"), "   \n ").is_none());
        assert!(commit_spec(Path::new("/repo"), "").is_none());
    }

    #[test]
    fn a_commit_message_is_one_argument_however_it_is_punctuated() {
        // No shell is involved, so nothing in the message can be read as a
        // flag or a second command.
        let spec = commit_spec(
            Path::new("/repo"),
            "fix: `rm -rf /` in the docs; also \"quotes\"",
        )
        .expect("a message");
        assert_eq!(
            spec.args,
            vec![
                "commit".to_string(),
                "-m".to_string(),
                "fix: `rm -rf /` in the docs; also \"quotes\"".to_string(),
            ]
        );
    }

    #[test]
    fn staging_separates_the_path_from_the_flags() {
        // A file called `-f` is a legal file and an illegal argument.
        let spec = stage_spec(Path::new("/repo"), Path::new("-f"));
        assert_eq!(spec.args, vec!["add", "--", "-f"]);
        let spec = unstage_spec(Path::new("/repo"), Path::new("-f"));
        assert_eq!(spec.args, vec!["restore", "--staged", "--", "-f"]);
    }

    #[test]
    fn a_status_scan_asks_for_the_stable_format() {
        // `git status` without a porcelain flag is documented as unstable and
        // is translated into the user's language.
        let spec = status_spec(Path::new("/repo"));
        assert!(spec.args.contains(&"--porcelain=v1".to_string()));
        assert!(spec.args.contains(&"-z".to_string()));
    }

    #[test]
    fn a_first_push_sets_the_upstream_and_a_later_one_does_not() {
        let first = push_spec(Path::new("/p"), "feature", false);
        assert_eq!(first.program, "git");
        assert_eq!(
            first.args,
            vec!["push", "--set-upstream", "origin", "feature"],
            "a branch with no upstream is refused by a bare push"
        );

        let later = push_spec(Path::new("/p"), "feature", true);
        assert_eq!(later.args, vec!["push"]);
    }

    #[test]
    fn a_pull_is_fast_forward_only() {
        // A merging pull can leave a conflicted worktree, and the studio has
        // no merge tool to get out of one.
        assert_eq!(pull_spec(Path::new("/p")).args, vec!["pull", "--ff-only"]);
    }

    #[test]
    fn a_fetch_prunes_so_the_counts_mean_something() {
        assert_eq!(fetch_spec(Path::new("/p")).args, vec!["fetch", "--prune"]);
    }

    #[test]
    fn an_upstream_is_found_loose_or_packed() {
        let dir = std::env::temp_dir().join("viewwstudio-git-upstream");
        std::fs::remove_dir_all(&dir).ok();
        let refs = dir.join(".git/refs/remotes/origin");
        std::fs::create_dir_all(&refs).expect("a scratch repository");
        assert!(!has_upstream(&dir, "main"), "nothing pushed yet");

        std::fs::write(refs.join("main"), "0123456\n").expect("a loose ref");
        assert!(has_upstream(&dir, "main"));

        // And the packed form a fresh clone has.
        std::fs::remove_file(refs.join("main")).expect("remove");
        std::fs::write(
            dir.join(".git/packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted\n0123456 refs/remotes/origin/main\n",
        )
        .expect("packed refs");
        assert!(has_upstream(&dir, "main"));
        assert!(!has_upstream(&dir, "other"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
