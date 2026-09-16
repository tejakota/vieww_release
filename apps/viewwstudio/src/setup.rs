//! Turning the toolchain checklist from a report into something you can act on.
//!
//! # The finding
//!
//! The Toolchain view listed every requirement with a tick or a cross and, next
//! to each cross, the command that would fix it. That is a good report and it is
//! still a report: the user's next move was to select the text of a command out
//! of a sidebar, find a terminal, paste it, and come back — for each of five
//! missing things, on the day they were trying to build an APK for the first
//! time. The information was all there and none of it was reachable.
//!
//! # Two ways to run it, because they are genuinely different
//!
//! **In the studio.** `Studio::install_in_panel` runs the command through the
//! same job queue every build uses: output streams into the Output panel, it is
//! cancellable from Tasks, and the exit status is reported. This is right for
//! everything that runs unattended — `rustup target add`, `cargo install
//! cargo-ndk`, `brew install`.
//!
//! **In a terminal.** `Studio::install_in_terminal` opens the platform's
//! terminal with the command typed into it. This is not a nicety: Android's
//! `sdkmanager` stops and asks you to read and accept several licence
//! agreements, `xcode-select --install` opens a system dialog, and a `sudo` step
//! wants a password. A command that asks a question and is run somewhere with no
//! keyboard attached to it does not fail — it hangs, which is worse.
//!
//! The view offers both and defaults to neither: the row says which one each
//! command wants, because the studio knows and the user should not have to find
//! out by waiting.
//!
//! # What this deliberately does not do
//!
//! It does not download an SDK itself, and it does not `sudo`. Every command
//! here is one the platform's own tooling documents — `sdkmanager`, `rustup`,
//! `cargo install`, `xcode-select` — run as the user, in the open, with its
//! output visible. A studio that shipped its own Android downloader would be a
//! second, worse package manager that nobody could audit and everybody would
//! have to trust.

use std::path::PathBuf;

use crate::task::Spec;

/// How a command wants to be run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runs {
    /// Unattended: no prompts, so the Output panel is the better home for it.
    Quietly,
    /// Asks something — a licence, a password, a system dialog — so it needs a
    /// terminal with a keyboard attached.
    Interactively,
}

/// Whether `command` is going to ask the user something.
///
/// A short list of known-interactive tools rather than a guess, because being
/// wrong in one direction hangs a job in a panel that cannot answer it, and
/// being wrong in the other opens a terminal window for nothing.
#[must_use]
pub fn how_it_runs(command: &str) -> Runs {
    const ASKS: [&str; 5] = [
        "sdkmanager",
        "xcode-select",
        "sudo",
        "apt install",
        "keytool",
    ];
    if ASKS.iter().any(|tool| command.contains(tool)) || needs_root(command) {
        Runs::Interactively
    } else {
        Runs::Quietly
    }
}

/// Whether `command` is a system package manager that will want to be root.
///
/// # The report this comes from
///
/// Clicking **Install** beside Gradle produced, in the Output panel:
///
/// ```text
/// ! Error: Could not open lock file /var/lib/dpkg/lock-frontend - open (13: Permission denied)
/// ! Error: Unable to acquire the dpkg frontend lock, are you root?
/// [Install Gradle] failed with exit code 100
/// ```
///
/// Which is apt saying the only thing it can say to a button that ran it as the
/// desktop user. `how_it_runs` already knew about `"apt install"` and marks such
/// a line for the terminal — but "marks" only *tinted* the Terminal button, and
/// the Install button beside it stayed live and did what it said.
///
/// This is the list of program names that need privileges. `brew`, `sdkmanager`,
/// `rustup` and `cargo install` are absent on purpose: they install into a
/// user-owned prefix and run fine in the panel, which is what makes the panel
/// worth having.
///
/// Matched on the *first word*, so a package literally named `apt-something`
/// being installed by brew does not trip it.
#[must_use]
pub fn needs_root(command: &str) -> bool {
    // Every system package manager the install lines offer, across the three
    // hosts the studio runs on — not just the Linux ones. `choco` wants an
    // elevated shell on Windows and says so by failing; `port` wants `sudo` on
    // macOS. `winget` and `scoop` install per-user by default and `brew` owns
    // its own prefix, so those three stay out and keep working in the panel.
    const ROOT: [&str; 9] = [
        "apt", "apt-get", "dnf", "yum", "pacman", "zypper", "snap", "port", "choco",
    ];
    let mut words = command.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    if first == "sudo" || first == "doas" || first == "pkexec" {
        return true;
    }
    // `apt list` and `dnf search` read; only the subcommands that write need
    // root, and those are the ones an install line carries.
    const WRITES: [&str; 6] = ["install", "add", "-S", "-Sy", "-Syu", "reinstall"];
    ROOT.contains(&first) && words.any(|word| WRITES.contains(&word))
}

/// The shell a command is handed to, and how to say "run this".
///
/// `sh -c` everywhere but Windows, where it is `cmd /C`: the install lines in
/// `toolchains.rs` are shell one-liners — they contain `&&`, quotes and
/// arguments with semicolons in them (`ndk;27.0.12077973`) — so handing them to
/// a shell is not laziness, it is the only thing that runs them as written.
#[must_use]
pub fn shell() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("/bin/sh", "-c")
    }
}

/// A task that runs `command` in a shell, for the Output panel.
#[must_use]
pub fn quiet_spec(label: impl Into<String>, command: &str, dir: PathBuf) -> Spec {
    let (program, flag) = shell();
    let mut spec = Spec::new(label, program, dir);
    spec.args = vec![flag.to_owned(), command.to_owned()];
    // Same reason a build sets it: escape sequences would be drawn literally in
    // a panel that is text, not a terminal emulator.
    spec.env
        .push(("CARGO_TERM_COLOR".to_owned(), "never".to_owned()));
    spec
}

/// A task that opens the platform's terminal with `command` in it.
///
/// # Why there is a list per platform and not one command
///
/// There is no such thing as "the terminal" on Linux: a machine has whichever
/// its desktop shipped, and the only portable answer is to try the ones that
/// exist in the order a distribution would. `x-terminal-emulator` is Debian's
/// own alternatives symlink and is first for that reason; the rest are the four
/// that cover most of everything else.
///
/// macOS drives Terminal.app through AppleScript, because `open -a Terminal`
/// cannot pass a command — it can only open a file.
///
/// Windows uses `start`, which is a `cmd` builtin rather than a program, hence
/// the `cmd /C` around it; `/K` keeps the window open after the command
/// finishes, which is the whole point when the command has just printed
/// something the user needs to read.
///
/// Returns `None` when nothing suitable is on the machine, and the caller says
/// so rather than pretending a window opened.
#[must_use]
pub fn terminal_spec(command: &str, dir: PathBuf) -> Option<Spec> {
    // The command, then a shell that stays: a terminal that closes the instant
    // an install fails takes the error message with it.
    let stay = format!(
        "{command}; echo; echo '— done. Close this window when you are finished.'; exec $SHELL"
    );

    if cfg!(target_os = "macos") {
        let script = format!(
            "tell application \"Terminal\"\n activate\n do script \"cd {} && {}\"\nend tell",
            shell_quote(&dir.display().to_string()),
            command.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let mut spec = Spec::new("Open Terminal", "osascript", dir);
        spec.args = vec!["-e".to_owned(), script];
        return Some(spec);
    }

    if cfg!(windows) {
        let mut spec = Spec::new("Open Terminal", "cmd", dir);
        spec.args = vec![
            "/C".to_owned(),
            "start".to_owned(),
            // An empty title, or `start` reads the next quoted argument as one.
            String::new(),
            "cmd".to_owned(),
            "/K".to_owned(),
            command.to_owned(),
        ];
        return Some(spec);
    }

    const TERMINALS: [(&str, &[&str]); 6] = [
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-e"]),
        ("kitty", &[]),
        ("xterm", &["-e"]),
    ];

    let found = TERMINALS
        .iter()
        .find(|(program, _)| which(program).is_some())?;

    let mut spec = Spec::new("Open Terminal", found.0, dir);
    spec.args = found.1.iter().map(|flag| (*flag).to_owned()).collect();
    spec.args.push("/bin/sh".to_owned());
    spec.args.push("-c".to_owned());
    spec.args.push(stay);
    Some(spec)
}

/// `'` -quoted for a shell.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The first directory on `PATH` holding `program`.
///
/// A `PATH` walk rather than spawning `which`, for the reason `toolchains.rs`
/// gives about the rest of its discovery: spawning a process to ask whether a
/// process can be spawned is a stall on the frame that asks.
#[must_use]
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(program);
        candidate.is_file().then_some(candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_system_package_manager_needs_root() {
        // The reported failure: Install beside Gradle ran `apt install gradle`
        // in the Output panel and got "are you root?" back.
        assert!(needs_root("apt install gradle"));
        assert!(needs_root("apt-get install default-jdk"));
        assert!(needs_root("dnf install mingw64-gcc"));
        assert!(needs_root("pacman -S gradle"));
        assert!(needs_root("choco install mingw"));
        assert!(needs_root("sudo anything at all"));
    }

    /// And the ones that install into a prefix the user owns do not — which is
    /// what keeps the Output panel worth having.
    #[test]
    fn a_user_prefix_installer_does_not() {
        assert!(!needs_root("brew install gradle"));
        assert!(!needs_root("cargo install cargo-ndk"));
        assert!(!needs_root("rustup target add aarch64-linux-android"));
        assert!(!needs_root("winget install Gradle.Gradle"));
        assert!(!needs_root("scoop install gradle"));
        // Reading, not writing.
        assert!(!needs_root("apt list --installed"));
    }

    #[test]
    fn anything_needing_root_wants_a_terminal() {
        assert_eq!(how_it_runs("dnf install mingw64-gcc"), Runs::Interactively);
        assert_eq!(how_it_runs("cargo install cargo-ndk"), Runs::Quietly);
    }

    #[test]
    fn a_licence_prompt_wants_a_terminal() {
        // The case this exists for: `sdkmanager` stops and asks you to type
        // `y` several times. Run in a panel it looks like a hang.
        assert_eq!(
            how_it_runs("`sdkmanager --install \"ndk;27.0.12077973\"`"),
            Runs::Interactively
        );
        assert_eq!(how_it_runs("xcode-select --install"), Runs::Interactively);
    }

    #[test]
    fn an_unattended_install_stays_in_the_panel() {
        assert_eq!(
            how_it_runs("rustup target add aarch64-linux-android"),
            Runs::Quietly
        );
        assert_eq!(how_it_runs("cargo install cargo-ndk"), Runs::Quietly);
    }

    #[test]
    fn a_command_is_handed_to_a_shell_whole() {
        // Not split on spaces: the install lines carry `&&`, quotes and
        // `ndk;27.0.12077973`, and a naive split turns each of those into a
        // separate argument that means something else.
        let spec = quiet_spec(
            "Install",
            "cargo install cargo-ndk && echo done",
            std::env::temp_dir(),
        );
        assert_eq!(spec.args.len(), 2);
        assert!(spec.args[1].contains("&&"));
    }

    #[test]
    fn a_terminal_is_asked_for_by_name_and_may_not_be_there() {
        // Nothing is asserted about *which* terminal: the machine running the
        // tests has whichever it has. What is asserted is the contract — a spec
        // or an honest `None`, never a spec naming a program that is not there.
        if let Some(spec) = terminal_spec("echo hello", std::env::temp_dir()) {
            assert!(!spec.program.is_empty());
            assert!(spec.args.iter().any(|arg| arg.contains("echo hello")));
        }
    }
}
