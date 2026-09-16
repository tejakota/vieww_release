//! Every action the studio can perform, named once.
//!
//! # Why a registry rather than closures on the buttons
//!
//! Before this, "Render" existed three times over: as a click handler on the
//! Render button, as a word in a menu strip that did nothing, and as a
//! keystroke that did not exist at all. Three copies of one action is three
//! chances for them to disagree about what it does or whether it is available —
//! and the failure mode is the quiet one, where the menu item works and the
//! shortcut silently does something slightly different.
//!
//! So there is one list. [`Command::ALL`] is the whole of what the studio can
//! be asked to do; the menus are that list filtered by [`Command::menu`], the
//! palette is that list filtered by a search string, the shortcut layer is that
//! list matched against a key, and every one of them ends up in
//! [`Studio::run`](crate::state::Studio::run). A command that is not in the
//! list is not reachable from anywhere, which is the property that keeps the
//! three in step.
//!
//! # Chords are described, not drawn
//!
//! A [`Chord`] carries the *meaning* — "the shortcut modifier and S" — rather
//! than a platform's rendering of it, and [`Chord::describe`] turns it into
//! `⌘S` or `Ctrl+S` at the point it is shown. The alternative is a `&'static
//! str` per binding, which is a string that has to be kept honest by hand and
//! is wrong on half the machines that read it.

use vieww_foundation::{KeyEvent, LogicalKey, Modifiers, NamedKey, TargetPlatform};

/// Which menu a command appears under, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    File,
    Edit,
    /// Everything that answers "take me to…". Split out of `View` on
    /// 2026-08-25, when that menu had reached thirty-two items and 876 logical
    /// pixels — taller than the window on any laptop, and past the point where
    /// a menu is something you read rather than something you scan.
    ///
    /// The cut is not arbitrary: `View` had two unrelated jobs in it, *change
    /// what is on screen* and *move the caret somewhere else*, and the second
    /// one is what every editor people arrive from calls Go.
    Go,
    View,
    Render,
    /// `cargo build` and what it produces. Separate from `Render` on purpose:
    /// plan 2 §4.3 — *preview renders a widget, build produces an application*
    /// — and two menus is the plainest way for the UI never to blur them.
    Build,
    Help,
    /// Reachable from the palette and a keystroke, but not listed in a menu —
    /// the platform switches, which live on the preview's own picker.
    None,
}

impl Menu {
    /// The menus that get a title in the strip, in order.
    pub const BAR: [Self; 7] = [
        Self::File,
        Self::Edit,
        Self::Go,
        Self::View,
        Self::Render,
        Self::Build,
        Self::Help,
    ];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::Go => "Go",
            Self::View => "View",
            Self::Render => "Render",
            Self::Build => "Build",
            Self::Help => "Help",
            Self::None => "",
        }
    }
}

/// A keystroke, described by what it means rather than by how it prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    /// Command on Apple, Control elsewhere. See [`Modifiers::shortcut_for`].
    pub shortcut: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: LogicalKey,
}

impl Chord {
    /// The shortcut modifier and a character key.
    #[must_use]
    pub fn cmd(character: &str) -> Self {
        Self {
            shortcut: true,
            shift: false,
            alt: false,
            key: LogicalKey::Character(character.to_string()),
        }
    }

    /// The shortcut modifier and a named key.
    #[must_use]
    pub const fn cmd_named(key: NamedKey) -> Self {
        Self {
            shortcut: true,
            shift: false,
            alt: false,
            key: LogicalKey::Named(key),
        }
    }

    /// A bare named key — Escape, which closes the find bar and the palette.
    #[must_use]
    pub const fn bare(key: NamedKey) -> Self {
        Self {
            shortcut: false,
            shift: false,
            alt: false,
            key: LogicalKey::Named(key),
        }
    }

    #[must_use]
    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    #[must_use]
    pub const fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// Whether `event` is this chord on `platform`.
    ///
    /// Key releases and auto-repeats are both rejected: a shortcut fires once,
    /// on the way down, and holding it must not fire it forty times.
    /// Comparison of a character key is case-insensitive because Shift+S
    /// produces `"S"` on every layout, and a chord that asked for `"s"` would
    /// otherwise never match its own shifted form.
    #[must_use]
    pub fn matches(&self, event: &KeyEvent, platform: TargetPlatform) -> bool {
        if !event.is_down() || event.repeat {
            return false;
        }

        let shortcut = Modifiers::shortcut_for(platform);
        let held = event.modifiers;
        if held.contains(shortcut) != self.shortcut {
            return false;
        }
        if held.shift() != self.shift || held.alt() != self.alt {
            return false;
        }
        // Nothing beyond what the chord asked for. Without this, Ctrl+Alt+S
        // would also fire Ctrl+S — the case `Modifiers`' own docs call out.
        let extra = held
            .without(if self.shortcut {
                shortcut
            } else {
                Modifiers::NONE
            })
            .without(if self.shift {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            })
            .without(if self.alt {
                Modifiers::ALT
            } else {
                Modifiers::NONE
            });
        if !extra.is_empty() {
            return false;
        }

        match (&self.key, &event.key) {
            (LogicalKey::Character(want), LogicalKey::Character(got)) => {
                want.eq_ignore_ascii_case(got)
            }
            (want, got) => want == got,
        }
    }

    /// How this chord is written on `platform`.
    #[must_use]
    pub fn describe(&self, platform: TargetPlatform) -> String {
        let apple = platform.is_apple();
        let mut out = String::new();
        if self.shortcut {
            out.push_str(if apple { "⌘" } else { "Ctrl+" });
        }
        if self.shift {
            out.push_str(if apple { "⇧" } else { "Shift+" });
        }
        if self.alt {
            out.push_str(if apple { "⌥" } else { "Alt+" });
        }
        match &self.key {
            LogicalKey::Character(character) => out.push_str(&character.to_uppercase()),
            LogicalKey::Named(NamedKey::Enter) => out.push_str(if apple { "⏎" } else { "Enter" }),
            LogicalKey::Named(NamedKey::Escape) => out.push_str("Esc"),
            LogicalKey::Named(named) => out.push_str(&format!("{named:?}")),
            LogicalKey::Unidentified => out.push('?'),
        }
        out
    }
}

/// Everything the studio can be asked to do.
///
/// Ordered as the palette lists them when nothing has been typed: the things
/// somebody reaches for most, first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    // Render
    Render,
    CancelRender,
    PlatformIos,
    PlatformAndroid,
    PlatformDesktop,
    ToggleSafeArea,
    /// Toggle the previewed screen between light and dark themes.
    ///
    /// `ThemeData::adaptive(platform, dark)`'s `dark` argument used to be
    /// hardcoded, which meant the preview could not answer "does this look
    /// right in dark mode". This writes the signal `device()` reads.
    TogglePreviewDark,
    /// Open the live-preview caution dialog.
    ///
    /// The live preview mounts a built-in demo app inside the device frame
    /// without compiling. The caution explains that it is a visual
    /// representation, not the user's code, and that Build and Run is needed
    /// for the real app. "Continue" sets `live_preview = true`.
    LivePreview,
    /// Put the sample `flow.rs` content in the active buffer, ready to click
    /// Render and trigger the live preview.
    RestoreFlow,

    // File
    /// The welcome card's first row: an untitled Say counter that renders
    /// straight away. No folder, no name, no Cargo.toml — the fastest path
    /// from "what is this?" to a screen in the device frame.
    TrySayScreen,
    NewProject,
    /// Open a folder as the workspace, through the studio's own picker.
    ///
    /// There is no platform file dialog to call — see [`crate::picker`]. Before
    /// this existed the root was `argv[1]` and nothing else, so a studio
    /// launched from a desktop icon could never reach a project at all, and
    /// every command that needs a root greyed itself out with no way to fix it.
    OpenFolder,
    /// Put the sample screen back in the active buffer.
    ///
    /// The buffer a fresh studio opens with is the one thing in the application
    /// guaranteed to compile and render, and emptying it used to be
    /// irreversible past the end of the undo stack.
    RestoreSample,
    NewFile,
    /// Send the current branch to `origin`.
    Push,
    /// Fast-forward the current branch from `origin`.
    Pull,
    /// Update the remote-tracking refs, so ahead/behind means something.
    Fetch,
    /// Close every expanded folder in the Explorer.
    ///
    /// The sidebar header has drawn a collapse-all icon since M0 with an empty
    /// handler behind it — an affordance that promised an action and delivered
    /// nothing, which `panel.rs` already names as a bug worth fixing when it
    /// found the same shape on the Output panel's trash icon.
    CollapseFolders,
    /// N2: a screen or a widget from a template, into an open project.
    NewScreen,
    NewWidget,
    Save,
    SaveAll,
    RevertFile,
    CloseTab,

    // Edit
    Undo,
    Redo,
    /// The three the pasteboard carries out.
    ///
    /// `RenderEditableText` implements all three in full and reads its
    /// `Clipboard` from the inherited services, so ⌘X/⌘C/⌘V have always worked
    /// *inside* a focused field. What did not exist was any way to find that
    /// out: no menu row, no palette entry, nothing in the shortcut sheet. A
    /// feature nobody can discover is a feature only its author has.
    Cut,
    Copy,
    Paste,
    ToggleComment,
    Indent,
    Outdent,
    /// N6: another caret, one line down or up.
    AddCursorBelow,
    AddCursorAbove,
    /// Select the word at the caret, then the next occurrence of it.
    AddCursorAtNextOccurrence,
    /// Back to one caret.
    ClearCursors,
    /// Fold or unfold the innermost region at the caret.
    ToggleFold,
    FoldAll,
    UnfoldAll,
    /// Run `rustfmt` over the active buffer.
    Format,
    /// Whether saving runs the formatter first.
    ToggleFormatOnSave,
    Find,
    Replace,
    FindNext,
    FindPrevious,
    CloseFind,

    // View
    CommandPalette,
    /// Open the palette on files. The same palette; a different mode.
    GoToFile,
    /// Open the palette on the active buffer's definitions.
    GoToSymbol,
    /// Open the palette on a line number.
    GoToLine,
    TogglePanel,
    ToggleSidebar,
    /// Show or hide the far-left column of view switches — the VS Code
    /// "activity bar". The sidebar has had Ctrl+B since M0; this is the
    /// narrower column beside it, and Ctrl+Alt+U is its chord.
    ToggleActivityBar,
    /// Show or hide the preview pane.
    ToggleRightPane,
    /// Everything away but the code.
    ZenMode,
    ToggleTheme,
    /// Show the previewed screen's render tree.
    ShowInspector,
    /// Preview zoom: in, out, and back to fitting the pane.
    ZoomIn,
    ZoomOut,
    ZoomFit,
    /// The three developer overlays. See `ui/overlays.rs`.
    ToggleDamageOverlay,
    ToggleSemanticsOverlay,
    PickWidget,
    NextTab,
    PreviousTab,
    ShowProblems,
    ShowExplorer,
    ShowSearch,
    ShowToolchain,
    /// N8: the theme's tokens.
    ShowTokens,
    /// N7: start or stop `rust-analyzer`.
    ToggleAnalyzer,
    /// Open the Run box in the bottom panel.
    ShowRun,
    /// Ask rust-analyzer what could go at the caret.
    Complete,
    /// Ask rust-analyzer what is at the caret.
    Hover,
    /// Jump to where the symbol under the caret is defined.
    ///
    /// The ⌘-click of every other editor, and the thing a developer reading an
    /// unfamiliar codebase reaches for most.
    GotoDefinition,
    /// List every use of the symbol under the caret.
    FindReferences,
    /// Write theme, snippet and keymap starting points, and say where.
    WriteTemplates,
    /// Replace across every file in the workspace, after a preview.
    ReplaceInFiles,
    /// Close every tab but the active one.
    CloseOtherTabs,
    /// Close every tab to the right of the active one.
    CloseTabsToTheRight,
    /// Close every tab whose file is saved.
    CloseSavedTabs,
    /// Bring a dead `rust-analyzer` back. See `Studio::restart_analyzer`.
    RestartAnalyzer,
    /// Wrap long lines at the pane edge instead of scrolling sideways.
    ToggleWordWrap,
    /// Version, build and licence, in a dialog.
    About,
    /// The welcome view: recent workspaces and the things a new user needs.
    ShowWelcome,
    /// N10: render when the buffer settles, without pressing anything.
    ToggleAutoRender,
    ShowSettings,
    ToggleIndentGuides,
    ToggleInlineDiagnostics,
    ShowTasks,
    /// Source control: what changed, and a box to commit it with.
    ShowSource,
    RefreshGit,
    Commit,

    // Build
    Build,
    BuildRelease,
    BuildAndRun,
    CancelBuild,
    /// N5: open the Export view.
    Export,
    /// Run the export the view has selected.
    ExportSelected,
    CancelExport,
    /// Ask `adb` what is plugged in.
    ScanDevices,

    // Panel
    ClearOutput,
    /// Forget every job that has ended.
    ClearFinishedTasks,
    /// Stop everything the studio has running.
    CancelAllTasks,

    // Help
    ShowShortcuts,
    /// Show or hide the note under the preview's device frame.
    TogglePreviewNote,
    /// Write the starter `live.rs` into the workspace and open it.
    CreateLiveFile,
}

impl Command {
    /// Every command, in palette order.
    pub const ALL: [Self; 107] = [
        Self::Render,
        Self::CancelRender,
        Self::Build,
        Self::BuildRelease,
        Self::BuildAndRun,
        Self::CancelBuild,
        // Above `Save`, because a studio with no folder open can do almost
        // nothing else and this is the command that fixes that.
        Self::TrySayScreen,
        Self::OpenFolder,
        Self::Save,
        Self::SaveAll,
        Self::CommandPalette,
        Self::GoToFile,
        Self::GoToSymbol,
        Self::GoToLine,
        Self::Find,
        Self::Replace,
        Self::FindNext,
        Self::FindPrevious,
        Self::CloseFind,
        Self::Undo,
        Self::Redo,
        // Three commands that were declared, given titles, given chords and
        // handled — and left out of this array, which is the one list the
        // menus, the palette and the shortcut layer are all built from. This
        // module's own header states the invariant they violated: *a command
        // that is not in the list is not reachable from anywhere*. They are the
        // three most-used editing commands in any editor and they were dead in
        // all three places.
        Self::ToggleComment,
        Self::Indent,
        Self::Outdent,
        Self::NewProject,
        Self::NewFile,
        Self::CollapseFolders,
        Self::Push,
        Self::Pull,
        Self::Fetch,
        Self::NewScreen,
        Self::NewWidget,
        Self::RestoreSample,
        Self::RevertFile,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::TogglePanel,
        Self::ToggleSidebar,
        Self::ToggleActivityBar,
        Self::ToggleTheme,
        Self::ToggleSafeArea,
        Self::TogglePreviewDark,
        Self::LivePreview,
        Self::RestoreFlow,
        Self::PlatformIos,
        Self::PlatformAndroid,
        Self::PlatformDesktop,
        Self::ShowExplorer,
        Self::ShowSearch,
        Self::ShowProblems,
        Self::ShowToolchain,
        Self::ShowTokens,
        Self::ShowRun,
        Self::Complete,
        Self::Hover,
        Self::GotoDefinition,
        Self::FindReferences,
        Self::ReplaceInFiles,
        Self::WriteTemplates,
        Self::CloseOtherTabs,
        Self::CloseTabsToTheRight,
        Self::CloseSavedTabs,
        Self::ToggleAnalyzer,
        Self::RestartAnalyzer,
        Self::ToggleWordWrap,
        Self::About,
        Self::ShowWelcome,
        Self::ToggleAutoRender,
        Self::ShowSettings,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::ToggleRightPane,
        Self::ShowInspector,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ZoomFit,
        Self::PickWidget,
        Self::ToggleDamageOverlay,
        Self::ToggleSemanticsOverlay,
        Self::ZenMode,
        Self::ToggleIndentGuides,
        Self::ToggleInlineDiagnostics,
        Self::ShowTasks,
        Self::ShowSource,
        Self::RefreshGit,
        Self::Commit,
        Self::AddCursorBelow,
        Self::AddCursorAbove,
        Self::AddCursorAtNextOccurrence,
        Self::ClearCursors,
        Self::ToggleFold,
        Self::FoldAll,
        Self::UnfoldAll,
        Self::Format,
        Self::ToggleFormatOnSave,
        Self::ClearFinishedTasks,
        Self::CancelAllTasks,
        Self::ClearOutput,
        // Below the everyday commands on purpose: `ALL` is palette order, and
        // an export is a thing somebody does at the end of a week rather than
        // between two keystrokes.
        Self::Export,
        Self::ExportSelected,
        Self::CancelExport,
        Self::ScanDevices,
        Self::ShowShortcuts,
        Self::TogglePreviewNote,
        Self::CreateLiveFile,
    ];

    /// What the menu item and the palette row say.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Render => "Render",
            Self::CancelRender => "Cancel Render",
            Self::PlatformIos => "Preview as iOS",
            Self::PlatformAndroid => "Preview as Android",
            Self::PlatformDesktop => "Preview as Desktop",
            Self::ToggleSafeArea => "Toggle Safe-Area Overlay",
            Self::TogglePreviewDark => "Toggle Preview Dark Mode",
            Self::LivePreview => "Live Preview (No Build)",
            Self::RestoreFlow => "Load Flow File",
            Self::TrySayScreen => "Try a Say Screen Right Now",
            Self::NewProject => "New Project\u{2026}",
            Self::OpenFolder => "Open Folder…",
            Self::RestoreSample => "Restore Sample Buffer",
            Self::NewFile => "New File",
            Self::CollapseFolders => "Collapse All Folders",
            Self::Push => "Push",
            Self::Pull => "Pull",
            Self::Fetch => "Fetch",
            Self::NewScreen => "New Screen from Template",
            Self::NewWidget => "New Widget from Template",
            Self::Save => "Save",
            Self::SaveAll => "Save All",
            Self::RevertFile => "Revert File",
            Self::CloseTab => "Close Tab",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::ToggleRightPane => "Toggle Preview Pane",
            Self::ZenMode => "Zen Mode",
            Self::ShowInspector => "Widget Inspector",
            Self::ZoomIn => "Preview: Zoom In",
            Self::ZoomOut => "Preview: Zoom Out",
            Self::ZoomFit => "Preview: Fit to Pane",
            Self::ToggleDamageOverlay => "Show Repainted Regions",
            Self::ToggleSemanticsOverlay => "Show Semantics Tree",
            Self::PickWidget => "Pick a Widget to Inspect",
            Self::ToggleComment => "Toggle Comment",
            Self::Indent => "Indent",
            Self::Outdent => "Outdent",
            Self::Find => "Find",
            Self::Replace => "Find and Replace",
            Self::FindNext => "Find Next",
            Self::FindPrevious => "Find Previous",
            Self::CloseFind => "Close Find",
            Self::CommandPalette => "Command Palette",
            Self::GoToFile => "Go to File",
            Self::GoToSymbol => "Go to Symbol in File",
            Self::GoToLine => "Go to Line",
            Self::TogglePanel => "Toggle Panel",
            Self::ToggleSidebar => "Toggle Sidebar",
            Self::ToggleActivityBar => "Toggle Activity Bar",
            Self::ToggleTheme => "Toggle Dark Theme",
            Self::NextTab => "Next Tab",
            Self::PreviousTab => "Previous Tab",
            Self::ShowProblems => "Show Problems",
            Self::ShowExplorer => "Show Explorer",
            Self::ShowSearch => "Show Search",
            Self::ShowToolchain => "Show Toolchain",
            Self::ShowSettings => "Show Settings",
            Self::Build => "Build",
            Self::BuildRelease => "Build (Release)",
            Self::BuildAndRun => "Build and Run",
            Self::CancelBuild => "Cancel Build",
            Self::ClearOutput => "Clear Output",
            Self::Export => "Export\u{2026}",
            Self::ExportSelected => "Export (run the selected format)",
            Self::CancelExport => "Cancel Export",
            Self::ScanDevices => "Scan for Devices",
            Self::AddCursorBelow => "Add Cursor Below",
            Self::AddCursorAbove => "Add Cursor Above",
            Self::AddCursorAtNextOccurrence => "Add Cursor at Next Occurrence",
            Self::ClearCursors => "Clear Extra Cursors",
            Self::ToggleFold => "Fold / Unfold at Cursor",
            Self::FoldAll => "Fold All Regions",
            Self::UnfoldAll => "Unfold All Regions",
            Self::Format => "Format Document (rustfmt)",
            Self::ToggleFormatOnSave => "Toggle Format on Save",
            Self::ShowTasks => "Show Tasks",
            Self::ShowSource => "Source Control",
            Self::RefreshGit => "Refresh Source Control",
            Self::Commit => "Commit Staged Changes",
            Self::ClearFinishedTasks => "Clear Finished Tasks",
            Self::CancelAllTasks => "Cancel All Tasks",
            Self::ToggleIndentGuides => "Toggle Indent Guides",
            Self::ToggleInlineDiagnostics => "Toggle Inline Diagnostic Messages",
            Self::ShowTokens => "Theme Tokens",
            Self::ShowRun => "Run a Command\u{2026}",
            Self::Complete => "Complete at Caret",
            Self::Hover => "What Is This?",
            Self::GotoDefinition => "Go to Definition",
            Self::FindReferences => "Find References",
            Self::ReplaceInFiles => "Replace in Files\u{2026}",
            Self::WriteTemplates => "Write Customisation Templates",
            Self::CloseOtherTabs => "Close Other Tabs",
            Self::CloseTabsToTheRight => "Close Tabs to the Right",
            Self::CloseSavedTabs => "Close Saved Tabs",
            Self::ToggleAnalyzer => "rust-analyzer: Start / Stop",
            Self::RestartAnalyzer => "rust-analyzer: Restart",
            Self::ToggleWordWrap => "Toggle Word Wrap",
            Self::About => "About vieww Studio",
            Self::ShowWelcome => "Show Welcome",
            Self::ToggleAutoRender => "Auto Render on Change",
            Self::ShowShortcuts => "Keyboard Shortcuts",
            Self::TogglePreviewNote => "Show or Hide the Preview Note",
            Self::CreateLiveFile => "Create live.rs for the Live Preview",
        }
    }

    /// Which menu lists it.
    #[must_use]
    pub const fn menu(self) -> Menu {
        match self {
            Self::TrySayScreen
            | Self::NewProject
            | Self::OpenFolder
            | Self::RestoreSample
            | Self::NewScreen
            | Self::NewWidget
            | Self::NewFile
            | Self::Save
            | Self::SaveAll
            | Self::RevertFile
            | Self::CloseTab
            | Self::CloseOtherTabs
            | Self::CloseTabsToTheRight
            | Self::CloseSavedTabs => Menu::File,
            Self::Undo
            | Self::Redo
            | Self::Cut
            | Self::Copy
            | Self::Paste
            | Self::ToggleComment
            | Self::Indent
            | Self::Outdent
            | Self::Find
            | Self::ReplaceInFiles
            | Self::Replace
            | Self::FindNext
            | Self::FindPrevious
            | Self::Format
            | Self::AddCursorBelow
            | Self::AddCursorAbove
            | Self::AddCursorAtNextOccurrence
            | Self::ClearCursors
            | Self::ToggleFold
            | Self::FoldAll
            | Self::UnfoldAll
            | Self::ToggleFormatOnSave => Menu::Edit,
            Self::GoToFile
            | Self::GoToSymbol
            | Self::GoToLine
            | Self::NextTab
            | Self::PreviousTab
            | Self::GotoDefinition
            | Self::FindReferences => Menu::Go,
            Self::CommandPalette
            | Self::TogglePanel
            | Self::CollapseFolders
            | Self::ToggleSidebar
            | Self::ToggleActivityBar
            | Self::ToggleRightPane
            | Self::ZenMode
            | Self::ShowInspector
            | Self::ToggleTheme
            | Self::ShowProblems
            | Self::ShowExplorer
            | Self::ShowSearch
            | Self::ShowSettings
            | Self::ToggleIndentGuides
            | Self::ToggleInlineDiagnostics
            | Self::ToggleAnalyzer
            | Self::RestartAnalyzer
            | Self::ShowRun
            | Self::Complete
            | Self::Hover
            | Self::ToggleWordWrap
            | Self::ShowWelcome
            | Self::WriteTemplates
            | Self::ShowTasks
            | Self::ShowTokens
            | Self::ShowSource => Menu::View,
            Self::About => Menu::Help,
            Self::ZoomIn
            | Self::ZoomOut
            | Self::ZoomFit
            | Self::ToggleDamageOverlay
            | Self::ToggleSemanticsOverlay
            | Self::PickWidget
            | Self::ToggleAutoRender => Menu::Render,
            Self::Render
            | Self::CancelRender
            | Self::ToggleSafeArea
            | Self::TogglePreviewDark
            | Self::LivePreview
            | Self::RestoreFlow
            | Self::PlatformIos
            | Self::PlatformAndroid
            | Self::PlatformDesktop => Menu::Render,
            Self::Build | Self::BuildRelease | Self::BuildAndRun | Self::CancelBuild => Menu::Build,
            Self::Export
            | Self::ExportSelected
            | Self::CancelExport
            | Self::ScanDevices
            | Self::CancelAllTasks => Menu::Build,
            Self::ShowToolchain | Self::ShowShortcuts => Menu::Help,
            Self::TogglePreviewNote => Menu::View,
            Self::CreateLiveFile => Menu::Render,
            Self::CloseFind
            | Self::ClearOutput
            | Self::ClearFinishedTasks
            | Self::RefreshGit
            | Self::Push
            | Self::Pull
            | Self::Fetch
            | Self::Commit => Menu::None,
        }
    }

    /// The keystroke that runs it, if it has one.
    ///
    /// The choices follow the platform conventions people already have:
    /// ⌘S saves, ⌘Z undoes, ⌘F finds, ⌘P opens the palette. ⌘⏎ renders,
    /// which is the "run the thing" chord every notebook and query console
    /// uses and the only one here that had to be picked rather than inherited.
    #[must_use]
    pub fn chord(self) -> Option<Chord> {
        Some(match self {
            Self::Render => Chord::cmd_named(NamedKey::Enter),
            Self::CancelRender => Chord::cmd_named(NamedKey::Enter).with_shift(),
            Self::Save => Chord::cmd("s"),
            Self::SaveAll => Chord::cmd("s").with_shift(),
            Self::RevertFile => Chord::cmd("r").with_shift(),
            Self::NewFile => Chord::cmd("n"),
            Self::NewProject => Chord::cmd("n").with_shift(),
            // ⌘O, which is Open on every desktop there has ever been.
            Self::OpenFolder => Chord::cmd("o"),
            Self::CloseTab => Chord::cmd("w"),
            Self::Undo => Chord::cmd("z"),
            Self::Redo => Chord::cmd("z").with_shift(),
            // The chords the field already answers to. Listing them here does
            // not intercept anything — `Studio::run` hands each one straight to
            // the focused object — it makes them *visible*, in the menu, the
            // palette and the shortcut sheet.
            Self::Cut => Chord::cmd("x"),
            Self::Copy => Chord::cmd("c"),
            Self::Paste => Chord::cmd("v"),
            // **Not** ⌘⌥B, which the prototype uses for this and which
            // `BuildRelease` has held here since N3 — `every_chord_is_unique`
            // is what caught it. P for preview, beside ⌘P and ⌘⇧P.
            Self::ToggleRightPane => Chord::cmd("p").with_alt(),
            Self::ShowInspector => Chord::cmd("i").with_shift(),
            Self::ZoomIn => Chord::cmd("="),
            Self::ZoomOut => Chord::cmd("-"),
            Self::ZoomFit => Chord::cmd("0"),
            // The chord every editor uses for this, and the reason `Indent`
            // and `Outdent` have none: Tab and shift-Tab belong to the focused
            // field, and stealing them at the shortcut layer would stop Tab
            // working in the find bar and every other input in the window.
            Self::ToggleComment => Chord::cmd("/"),
            Self::Find => Chord::cmd("f"),
            Self::Replace => Chord::cmd("f").with_alt(),
            Self::FindNext => Chord::cmd("g"),
            Self::FindPrevious => Chord::cmd("g").with_shift(),
            Self::CloseFind => Chord::bare(NamedKey::Escape),
            Self::CommandPalette => Chord::cmd("p"),
            // ⌘P stays the palette, because it has been that here since M0 and
            // moving a binding people have in their fingers to match another
            // editor's is a change with a cost and no benefit. The three modes
            // get their own keys beside it rather than instead of it.
            Self::GoToFile => Chord::cmd("p").with_shift(),
            Self::GoToSymbol => Chord::cmd("o").with_shift(),
            Self::GoToLine => Chord::cmd("l"),
            Self::TogglePanel => Chord::cmd("j"),
            Self::ToggleSidebar => Chord::cmd("b"),
            // Beside ⌘B rather than on it: ⌘B is the sidebar and ⌘⇧B and
            // ⌘⌥B are the builds. U is the one letter near B this table had
            // free at both modifier depths, and a hide-the-column chord that
            // sits a hand's width from the hide-the-panel chord it resembles
            // is the one people will find by feel.
            Self::ToggleActivityBar => Chord::cmd("u").with_alt(),
            Self::ToggleTheme => Chord::cmd("k"),
            Self::NextTab => Chord::cmd("]"),
            Self::PreviousTab => Chord::cmd("["),
            Self::ShowProblems => Chord::cmd("m").with_shift(),
            // **Not** the ⇧G every editor with a source control panel uses:
            // here that is Find Previous and has been since M0, and
            // `every_chord_is_unique` is what caught the collision. The
            // binding people already have in their fingers for stepping back
            // through matches keeps it — source control is opened once and
            // searched through many times.
            Self::ShowSource => Chord::cmd("g").with_alt(),
            Self::PlatformIos => Chord::cmd("1"),
            Self::PlatformAndroid => Chord::cmd("2"),
            Self::PlatformDesktop => Chord::cmd("3"),
            Self::Build => Chord::cmd("b").with_shift(),
            Self::BuildRelease => Chord::cmd("b").with_alt(),
            Self::BuildAndRun => Chord::cmd("r"),
            Self::CancelBuild => Chord::cmd("."),
            Self::Format => Chord::cmd("f").with_shift(),
            Self::Export => Chord::cmd("e").with_shift(),
            // The chord every editor uses for this, and the one people already
            // have in their fingers.
            Self::ToggleFold => Chord::cmd("l").with_shift(),
            Self::AddCursorBelow => Chord::cmd_named(NamedKey::ArrowDown).with_alt(),
            Self::AddCursorAbove => Chord::cmd_named(NamedKey::ArrowUp).with_alt(),
            Self::AddCursorAtNextOccurrence => Chord::cmd("d"),
            Self::TrySayScreen
            | Self::Indent
            | Self::Outdent
            | Self::ClearOutput
            | Self::ToggleSafeArea
            | Self::TogglePreviewDark
            | Self::LivePreview
            | Self::RestoreFlow
            | Self::ShowExplorer
            | Self::ShowSearch
            | Self::ShowToolchain
            | Self::ShowSettings
            | Self::ToggleIndentGuides
            | Self::ToggleInlineDiagnostics
            | Self::ToggleFormatOnSave
            | Self::ExportSelected
            | Self::CancelExport
            | Self::ScanDevices
            | Self::FoldAll
            | Self::UnfoldAll
            | Self::ClearCursors
            | Self::ShowTasks
            | Self::RefreshGit
            | Self::Commit
            | Self::ClearFinishedTasks
            | Self::CollapseFolders
            | Self::Push
            | Self::Pull
            | Self::Fetch
            | Self::CancelAllTasks
            // No chord on purpose: it is a recovery, not a habit, and every
            // spare File-menu letter is one keystroke away from a command that
            // rewrites the buffer.
            | Self::RestoreSample
            // Named, so they go through the palette where the name is typed —
            // a template with no name is a file called `untitled`.
            | Self::NewScreen
            | Self::NewWidget
            // Two chords deep (⌘K Z) in the prototype, and chords here are one
            // key. Reachable from the View menu and the palette, which is where
            // somebody looks for it the first time anyway.
            | Self::ZenMode
            | Self::ToggleDamageOverlay
            | Self::ToggleSemanticsOverlay
            | Self::PickWidget
            | Self::ShowTokens
            | Self::ToggleAnalyzer
            | Self::RestartAnalyzer
            | Self::About
            | Self::ShowWelcome
            | Self::CloseOtherTabs
            | Self::CloseTabsToTheRight
            | Self::CloseSavedTabs
            | Self::ReplaceInFiles
            | Self::WriteTemplates
            | Self::ShowRun
            | Self::ToggleAutoRender
            | Self::ShowShortcuts
            | Self::TogglePreviewNote
            | Self::CreateLiveFile => return None,
            // The chord every editor uses for completion, and the one people
            // press without thinking.
            Self::Complete => Chord::cmd_named(NamedKey::Space),
            Self::Hover => Chord::cmd("i"),
            // Not F12 — `NamedKey` has no function keys. Not ⌘B either, the
            // other convention, which is already the sidebar; nor ⌘D, which is
            // Add Cursor at Next Occurrence. ⌥⌘D and ⇧⌥⌘D are free, and the
            // letter still says "definition".
            Self::GotoDefinition => Chord::cmd("d").with_alt(),
            Self::FindReferences => Chord::cmd("d").with_alt().with_shift(),
            // Alt so it does not take ⌘W, which closes the tab.
            Self::ToggleWordWrap => Chord::cmd("w").with_alt(),
        })
    }

    /// The command `event` asks for, if any.
    ///
    /// Searched over [`ALL`](Self::ALL) rather than matched key by key, so a
    /// binding that exists in the list is reachable and one that does not is
    /// not — there is no second place a chord could be handled.
    #[must_use]
    pub fn for_key(event: &KeyEvent, platform: TargetPlatform) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|command| command.chord().is_some_and(|c| c.matches(event, platform)))
    }

    /// The commands whose titles match `query`, for the palette.
    ///
    /// A subsequence match rather than a substring one: `sv` finds "Save" and
    /// "Save All", which is what the muscle memory of every palette expects.
    #[must_use]
    pub fn matching(query: &str) -> Vec<Self> {
        let query = query.trim().to_lowercase();
        Self::ALL
            .iter()
            .copied()
            .filter(|command| subsequence(&query, &command.title().to_lowercase()))
            .collect()
    }
}

/// Whether every character of `needle` appears in `haystack`, in order.
fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut characters = haystack.chars();
    needle
        .chars()
        .all(|wanted| characters.any(|character| character == wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const LINUX: TargetPlatform = TargetPlatform::Linux;
    const MAC: TargetPlatform = TargetPlatform::MacOS;

    fn key(character: &str, modifiers: Modifiers) -> KeyEvent {
        KeyEvent::character(character, Duration::ZERO).with_modifiers(modifiers)
    }

    #[test]
    fn the_shortcut_modifier_follows_the_platform() {
        let control = key("s", Modifiers::CONTROL);
        let meta = key("s", Modifiers::META);

        assert_eq!(Command::for_key(&control, LINUX), Some(Command::Save));
        assert_eq!(
            Command::for_key(&control, MAC),
            None,
            "Ctrl+S is not Save on a Mac"
        );
        assert_eq!(Command::for_key(&meta, MAC), Some(Command::Save));
    }

    #[test]
    fn shift_selects_the_other_command_rather_than_being_ignored() {
        let plain = key("s", Modifiers::CONTROL);
        let shifted = key("S", Modifiers::CONTROL.union(Modifiers::SHIFT));
        assert_eq!(Command::for_key(&plain, LINUX), Some(Command::Save));
        assert_eq!(Command::for_key(&shifted, LINUX), Some(Command::SaveAll));
    }

    #[test]
    fn an_extra_modifier_does_not_fire_the_simpler_chord() {
        let event = key("s", Modifiers::CONTROL.union(Modifiers::ALT));
        assert_eq!(
            Command::for_key(&event, LINUX),
            None,
            "Ctrl+Alt+S is not Ctrl+S — the case Modifiers' own docs warn about"
        );
    }

    #[test]
    fn a_release_and_an_auto_repeat_are_not_a_shortcut() {
        let mut up = key("s", Modifiers::CONTROL);
        up.state = vieww_foundation::KeyState::Up;
        assert_eq!(Command::for_key(&up, LINUX), None);

        let repeat = key("s", Modifiers::CONTROL).repeated();
        assert_eq!(
            Command::for_key(&repeat, LINUX),
            None,
            "holding ⌘S must save once, not once per repeat"
        );
    }

    #[test]
    fn every_chord_is_unique() {
        // Two commands on one chord means one of them is unreachable, and which
        // one depends on the order of a list nobody thinks of as ordered.
        let mut seen: Vec<(Chord, Command)> = Vec::new();
        for command in Command::ALL {
            let Some(chord) = command.chord() else {
                continue;
            };
            if let Some((_, other)) = seen.iter().find(|(existing, _)| *existing == chord) {
                panic!(
                    "{:?} and {other:?} both claim {}",
                    command,
                    chord.describe(LINUX)
                );
            }
            seen.push((chord, command));
        }
    }

    #[test]
    fn every_command_in_the_list_is_reachable_from_a_menu_or_the_palette() {
        for command in Command::ALL {
            assert!(
                !command.title().is_empty(),
                "{command:?} has nothing to show in the palette"
            );
        }
    }

    #[test]
    fn a_chord_prints_the_way_the_platform_writes_it() {
        let save = Command::Save.chord().expect("Save has a chord");
        assert_eq!(save.describe(MAC), "⌘S");
        assert_eq!(save.describe(LINUX), "Ctrl+S");

        let render = Command::Render.chord().expect("Render has a chord");
        assert_eq!(render.describe(MAC), "⌘⏎");
        assert_eq!(render.describe(LINUX), "Ctrl+Enter");
    }

    #[test]
    fn the_palette_matches_on_a_subsequence() {
        assert!(Command::matching("sv").contains(&Command::Save));
        assert!(Command::matching("rndr").contains(&Command::Render));
        assert!(
            Command::matching("").len() == Command::ALL.len(),
            "an empty query lists everything"
        );
        assert!(Command::matching("zzzz").is_empty());
    }

    #[test]
    fn escape_is_a_chord_with_no_modifiers() {
        let escape = KeyEvent::named(NamedKey::Escape, Duration::ZERO);
        assert_eq!(Command::for_key(&escape, LINUX), Some(Command::CloseFind));
        assert_eq!(
            Command::for_key(&escape, MAC),
            Some(Command::CloseFind),
            "the same on both, because Escape is not a shortcut-modifier chord"
        );
    }

    /// No two commands answer to the same keystroke.
    ///
    /// `customise::parse_keymap` refuses a user override that would collide
    /// with another binding; nothing checked the built-in table against itself,
    /// and a collision there is the same defect with no file to blame — one of
    /// the two commands is simply unreachable, and which one depends on the
    /// order `Shortcuts` happens to walk.
    #[test]
    fn no_two_commands_share_a_chord() {
        let mut seen: Vec<(Chord, Command)> = Vec::new();
        for command in Command::ALL {
            let Some(chord) = command.chord() else {
                continue;
            };
            if let Some((_, other)) = seen.iter().find(|(held, _)| *held == chord) {
                panic!(
                    "{:?} and {:?} are both bound to {}",
                    other,
                    command,
                    chord.describe(TargetPlatform::MacOS)
                );
            }
            seen.push((chord, command));
        }
        assert!(seen.len() > 40, "the table still has chords in it");
    }

    /// Every character a chord can put on screen has to be one the studio's
    /// embedded font actually carries.
    ///
    /// # The bug this guards
    ///
    /// `describe` writes `⌘⇧P` on Apple and `Ctrl+Shift+P` everywhere else.
    /// The embedded font is a **subset** of DejaVu — `ci/tools/subset-fonts.py` — and
    /// its ranges did not include U+2318, U+21E7, U+2325 or U+23CE. DejaVu's
    /// `.notdef` is an empty glyph with a normal advance, so on macOS every
    /// menu item, every palette row and the whole Keyboard Shortcuts sheet drew
    /// its modifiers as blank space of the right width. Nothing looked broken;
    /// the shortcuts simply appeared to be bare letters.
    ///
    /// It survived because the platform it is wrong on is the one platform no
    /// test in this repository runs on: on Linux the same function produces
    /// ASCII, so every existing assertion about chord text passed.
    ///
    /// The list below is the subset's non-ASCII coverage, spelled out. A new
    /// modifier symbol — `⌃` for control, say — fails this test on the machine
    /// that adds it rather than on a Mac user's first launch, and the fix is to
    /// add the codepoint to `ci/tools/subset-fonts.py` and regenerate.
    #[test]
    fn every_chord_is_writable_in_the_embedded_font() {
        // U+2318 ⌘, U+21E7 ⇧, U+2325 ⌥, U+23CE ⏎, U+232B ⌫.
        const COVERED: [char; 5] = ['\u{2318}', '\u{21E7}', '\u{2325}', '\u{23CE}', '\u{232B}'];

        for platform in [
            TargetPlatform::MacOS,
            TargetPlatform::Linux,
            TargetPlatform::Windows,
        ] {
            for command in Command::ALL {
                let Some(chord) = command.chord() else {
                    continue;
                };
                let text = chord.describe(platform);
                for ch in text.chars() {
                    assert!(
                        ch.is_ascii() || COVERED.contains(&ch),
                        "{command:?} on {platform:?} is written {text:?}, and U+{:04X} is not \
                         in the embedded font subset — add it to ci/tools/subset-fonts.py and \
                         regenerate the faces, or this draws as blank space on that platform",
                        ch as u32
                    );
                }
            }
        }
    }
}
