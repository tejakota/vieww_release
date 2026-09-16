//! Every piece of studio state, in signals, held above the tree.
//!
//! The same contract the framework's own control catalogue uses: no widget in
//! this application holds state. A region renders what it is given and reports
//! what it wants, which is what makes a region testable by building it against
//! a hand-made `Studio` rather than by driving the window.
//!
//! M0 holds the shell's state only. The editor buffer (M1), the compile state
//! machine (M3) and the diagnostics list (M5) arrive as further fields here,
//! not as state hidden inside a widget.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use vieww_element::{Memo, Runtime, ScrollController, Signal};
use vieww_foundation::{TextDecoration, TextLayoutProbe, TextRange};
use vieww_gestures::ScrollPhysics;

use crate::buffer::{Buffer, BufferTab, Workspace};
use crate::builds::{self, Builds, Kind, Profile};
use crate::caret::{Active, Blink, SharedBlink};
use crate::command::Command;
use crate::compile::{self, Job, Progress, Session, Toolchain, ToolchainError};
use crate::find::{self, Query};
use crate::highlight::Highlighter;
use crate::loaded::Preview;
use crate::toolchains;

// The rest of `impl Studio`, one file per subject.
//
// **This module was 11,188 lines**: 170 signals and 320 methods, with the
// subjects separated by comment rules. The evaluation called it a god object and
// it was one — but the god *object* is deliberate and the god *file* was not.
// `Studio` holds every signal in one place so that any widget can read any of
// them without a chain of props threaded through the tree, which is the whole
// reason the studio's widgets own no state. Splitting the type would mean
// inventing ownership boundaries the interface does not have.
//
// So the code is split and the type is not. An inherent `impl` may be written in
// as many blocks as it has subjects, in as many files; the comment rules became
// module boundaries and nothing else changed — no signature, no visibility, no
// field. `use super::*` in each gives them the same imports this file has, so a
// reader following a method from here lands somewhere with the same vocabulary.
mod buffers;
mod commands;
mod editing;
mod export;
mod files;
mod language;
mod panels;
mod picker;
mod session;
mod vcs;

/// Which sidebar view the activity bar has selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Explorer,
    Search,
    Snippets,
    Problems,
    Inspector,
    /// Editable lessons — one per feature in `examples/features/*`, written
    /// as `pub fn screen()` so a click in the sidebar puts the source in the
    /// editor and Render shows what the example demonstrated.
    ///
    /// Beside [`Self::Docs`] (a book) because the two are different things:
    /// Docs answers "what does this mean" and Learn answers "show me one".
    /// See [`crate::lessons`].
    Learn,
    /// The reference: five pages that answer the questions of the first hour.
    ///
    /// In the bar rather than in a menu because the questions it answers arrive
    /// while looking at code, and a menu item is a thing you have to remember
    /// exists.
    Docs,
    Toolchain,
    /// Source control: what changed, and a box to commit it with.
    Source,
    /// N5: what this project can be packaged as, and what is plugged in.
    Export,
    /// N8: the theme's tokens, read out of the live theme.
    Tokens,
    Settings,
}

impl View {
    /// Every view, in activity-bar order. The bar is built from this rather
    /// than from a second list that could disagree with the enum.
    pub const ALL: [Self; 12] = [
        Self::Explorer,
        Self::Search,
        Self::Snippets,
        Self::Problems,
        Self::Inspector,
        Self::Learn,
        Self::Docs,
        Self::Toolchain,
        Self::Source,
        Self::Export,
        Self::Tokens,
        Self::Settings,
    ];

    /// The sidebar's header, and the activity button's tooltip.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Explorer => "Explorer",
            Self::Search => "Search",
            Self::Snippets => "Snippets",
            Self::Problems => "Problems",
            Self::Inspector => "Inspector",
            Self::Learn => "Learn",
            Self::Docs => "Docs",
            Self::Toolchain => "Toolchain",
            Self::Source => "Source Control",
            Self::Export => "Export",
            Self::Tokens => "Theme Tokens",
            Self::Settings => "Settings",
        }
    }

    /// One line saying what is inside, for the tooltip on the activity bar.
    ///
    /// # Not the hover tooltip any more
    ///
    /// This was the second line of the activity bar's tooltip, on the argument
    /// that a card reading "Snippets" over the snippets icon confirms a guess
    /// and teaches nothing. Two lines is why the card flickered:
    /// [`Tooltip`](vieww_widget::Tooltip) places itself using a **one-line**
    /// height — its own docs say a wrapped message "will overlap its anchor by
    /// the difference" — so the card landed on the icon, took the hover away
    /// from it, and dropped itself. `ui::tooltip` now shows the title alone,
    /// and `ui::tooltip`'s `IgnorePointer` makes the overlap harmless in any
    /// case.
    ///
    /// Kept because the sentence is worth having and the sidebar is the place
    /// for it: a header for the panel a click *opens* is read once, on purpose,
    /// rather than flashed under a moving pointer. Nothing renders it today.
    #[must_use]
    pub const fn note(self) -> &'static str {
        match self {
            Self::Explorer => "The files in this workspace, and the buffers you have open.",
            Self::Search => "Find text across every file, and replace it with a preview first.",
            Self::Snippets => "Templates you can drop into the buffer at the caret.",
            Self::Problems => "Every rustc error and warning from the last build, by file.",
            Self::Inspector => "The previewed screen's widget tree, and what each node measured.",
            Self::Learn => "Editable lessons — one per feature, ready to Render.",
            Self::Docs => {
                "Five short pages: the first screen, the shapes, and what Rust you can write."
            }
            Self::Toolchain => "Rust, Android, Apple: what is installed, and what to install next.",
            Self::Source => "What changed since the last commit, and a box to commit it with.",
            Self::Export => "What this project can be packaged as, and what is plugged in.",
            Self::Tokens => "The live theme's colours, type and spacing, read out of it.",
            Self::Settings => "Fonts, behaviour, shortcuts, and where the studio keeps things.",
        }
    }

    /// Where this view sits in [`Self::ALL`] — its own independent scroll
    /// position in [`Studio::sidebar_scroll`], so switching views does not
    /// reset (or share) where each one was left scrolled to.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Explorer => 0,
            Self::Search => 1,
            Self::Snippets => 2,
            Self::Problems => 3,
            Self::Inspector => 4,
            Self::Learn => 5,
            Self::Docs => 6,
            Self::Toolchain => 7,
            Self::Source => 8,
            Self::Export => 9,
            Self::Tokens => 10,
            Self::Settings => 11,
        }
    }

    /// The view whose [`title`](Self::title) is `name`.
    ///
    /// The inverse of `title`, and the reason the session file stores a name
    /// rather than an index: an index is a promise that [`Self::ALL`] never
    /// changes order, and this enum has grown twice already. A name that no
    /// longer exists restores as `None`, which every caller reads as "leave the
    /// default alone" — so a session written by a newer studio naming a view
    /// this one does not have opens on the Explorer instead of failing.
    #[must_use]
    pub fn from_title(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.title() == name)
    }

    /// The two letters drawn in the activity bar until the icon set grows.
    ///
    /// `vieww-widget`'s `icons` module ships ten paths — check, close, the four
    /// chevrons, add, remove. None of them is a folder or a magnifier. Rather
    /// than draw seven icons from SVG path data in M0 and get the shell wrong
    /// while doing it, the bar carries initials and M8 replaces this function.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Explorer => "Ex",
            Self::Search => "Se",
            Self::Snippets => "Sn",
            Self::Problems => "Pr",
            Self::Inspector => "In",
            Self::Learn => "Ln",
            Self::Docs => "Do",
            Self::Toolchain => "To",
            Self::Source => "Gt",
            Self::Export => "Pk",
            Self::Tokens => "Tk",
            Self::Settings => "St",
        }
    }
}

/// `from` moved `amount` of the way towards `to`.
///
/// Straight in sRGB bytes rather than in a perceptual space. That is the wrong
/// place to interpolate colour in general, and it is the right place here: the
/// two endpoints are a grey and pure white or pure black, so there is no hue to
/// travel through and nothing to go muddy.
#[must_use]
fn towards_color(
    from: vieww_foundation::Color,
    to: vieww_foundation::Color,
    amount: f32,
) -> vieww_foundation::Color {
    let mix = |a: u8, b: u8| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "channels are 0..=255 and the result is clamped by construction"
        )]
        {
            (f32::from(a) + (f32::from(b) - f32::from(a)) * amount.clamp(0.0, 1.0)) as u8
        }
    };
    vieww_foundation::Color::rgba(
        mix(from.r, to.r),
        mix(from.g, to.g),
        mix(from.b, to.b),
        from.a,
    )
}

/// Programs that want a terminal, and so cannot be run from the Run box.
///
/// Not exhaustive and not meant to be: the point is to catch the handful people
/// reach for first and say why, rather than to police the list. Anything not
/// here that turns out to want a terminal simply produces no output and can be
/// cancelled from the Tasks panel — which is a worse experience than a refusal
/// and a much better one than a hang with no cancel.
const INTERACTIVE: [&str; 10] = [
    "vim", "vi", "nvim", "nano", "emacs", "less", "top", "htop", "ssh", "python",
];

/// Split a command line into a program and its arguments.
///
/// Whitespace-separated, with double quotes grouping — so a path with a space
/// in it is one argument. Returns `None` for unbalanced quotes, which is the
/// one case where guessing produces a command the user did not write.
///
/// Deliberately **not** a shell: no globbing, no pipes, no redirection, no
/// variable expansion. Implementing a quarter of a shell is how an editor ends
/// up with a command line that works until it does not, in a way nobody can
/// predict from looking at it.
#[must_use]
pub fn split_command(line: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut any = false;

    for character in line.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut current));
                    any = false;
                }
            }
            c => {
                current.push(c);
                any = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if any {
        out.push(current);
    }
    Some(out)
}

/// How much larger the chrome's text is drawn, from [`Studio::large_ui`].
///
/// A multiplier rather than a second font size, so every call site that already
/// passes a size gets it right by multiplying rather than by remembering a
/// second constant. 1.15 is one notch — enough to help, small enough that a
/// 248-point sidebar still fits the words it has to fit. Anything larger wants
/// a reflowing chrome, which is a bigger change than a setting.
pub const LARGE_UI_SCALE: f32 = 1.15;

/// How far "high contrast" moves a foreground towards white or black.
///
/// 0.35, not 1.0: the point is to clear the WCAG bar on the greys the chrome
/// uses for secondary text, not to turn every label into pure white. A theme
/// pushed all the way loses the hierarchy that made it readable in the first
/// place, which is a different accessibility problem.
pub const CONTRAST_LIFT: f32 = 0.35;

/// One device the preview can be framed at.
///
/// # Why presets rather than the three platform defaults
///
/// The preview had exactly three sizes, one per `Platform`, and the prototype
/// modelled a Devices tab beside Preview and Inspector that the studio never
/// had. Three sizes is enough to answer "does this look right on a phone" and
/// nothing else — not "does the safe area eat my header on a notched phone",
/// not "does this reflow on a tablet", and not "is the small-phone case the one
/// that overflows", which is the one that actually breaks.
///
/// The list is deliberately short and each entry is a real device class rather
/// than a model name that dates: a studio listing "iPhone 15 Pro" is a studio
/// with a wrong list in eighteen months.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Device {
    pub name: &'static str,
    pub width: f32,
    pub height: f32,
    /// Which platform's conventions this device implies, so choosing a device
    /// also sets what the previewed tree is told it is running on.
    pub platform: Platform,
    /// Whether this is the device on its side.
    ///
    /// # Why the orientation is carried rather than inferred
    ///
    /// [`rotated`](Self::rotated) swapped the width and the height and nothing
    /// else, so a landscape frame was told it had a **47-point inset at the
    /// top and 34 at the bottom** — the portrait numbers, on a device whose
    /// notch and home indicator had just moved to the short edges. A `SafeArea`
    /// screen checked in landscape got its padding on the two edges that were
    /// safe and none on the two that were not, which is worse than no
    /// simulation: it is a confident wrong answer to the exact question the
    /// mode exists to answer.
    ///
    /// It cannot be inferred from `width > height` — a tablet in portrait is
    /// 834×1194 and a phone in landscape is 852×393, but the *Desktop* presets
    /// are wider than they are tall while sitting the right way up. The frame
    /// knows which way it was turned; it may as well say so.
    pub landscape: bool,
}

impl Device {
    /// Every preset, grouped by platform in the order the tab lists them.
    pub const ALL: [Self; 8] = [
        Self {
            name: "Phone (small)",
            width: 360.0,
            height: 780.0,
            platform: Platform::Ios,
            landscape: false,
        },
        Self {
            name: "Phone",
            width: 393.0,
            height: 852.0,
            platform: Platform::Ios,
            landscape: false,
        },
        Self {
            name: "Phone (large)",
            width: 430.0,
            height: 932.0,
            platform: Platform::Ios,
            landscape: false,
        },
        Self {
            name: "Tablet",
            width: 834.0,
            height: 1194.0,
            platform: Platform::Ios,
            landscape: false,
        },
        Self {
            name: "Android phone",
            width: 412.0,
            height: 915.0,
            platform: Platform::Android,
            landscape: false,
        },
        Self {
            name: "Android tablet",
            width: 800.0,
            height: 1280.0,
            platform: Platform::Android,
            landscape: false,
        },
        Self {
            name: "Laptop",
            width: 1280.0,
            height: 800.0,
            platform: Platform::Desktop,
            landscape: false,
        },
        Self {
            name: "Desktop",
            width: 1680.0,
            height: 1050.0,
            platform: Platform::Desktop,
            landscape: false,
        },
    ];

    /// The same device turned on its side.
    #[must_use]
    pub const fn rotated(self) -> Self {
        Self {
            width: self.height,
            height: self.width,
            landscape: !self.landscape,
            ..self
        }
    }

    /// `"393 × 852"`, for the row.
    #[must_use]
    pub fn size_label(self) -> String {
        format!("{} \u{00d7} {}", self.width.round(), self.height.round())
    }

    /// The metrics the previewed screen is told, when this device is chosen.
    ///
    /// Mirrors [`Platform::view_metrics`] but uses *this* device's width and
    /// height, so a screen that asks `ViewMetrics::size` learns the device it
    /// is actually drawn at rather than the platform's default. Rotation is
    /// the caller's concern — pass [`Self::rotated`] when the studio is in
    /// landscape.
    ///
    /// `device_pixel_ratio` comes from [`Self::platform`], because DPR is a
    /// property of the device *class* and there is no per-device table.
    #[must_use]
    pub fn view_metrics(self) -> vieww_foundation::ViewMetrics {
        vieww_foundation::ViewMetrics {
            size: vieww_foundation::Size::new(self.width, self.height),
            device_pixel_ratio: match self.platform {
                Platform::Ios => 3.0,
                Platform::Android => 2.625,
                Platform::Desktop => 1.0,
            },
            safe_area: self.safe_area(),
            // No soft keyboard is simulated — see `Platform::view_metrics`.
            view_insets: vieww_foundation::EdgeInsets::ZERO,
        }
    }

    /// Where this device, in this orientation, does not let an app draw.
    ///
    /// Portrait is the platform's own `(top, bottom)`. Landscape is not those
    /// two numbers rotated by accident — turning a phone moves the cutout to a
    /// **long** edge and shortens the home indicator's reservation:
    ///
    /// * The notch or island now sits on one side, and both platforms inset
    ///   *both* sides by the same amount so a layout does not jump when the
    ///   phone is turned the other way round. That is what iOS reports and what
    ///   Android's display cutout API produces for a symmetric layout.
    /// * The indicator keeps the bottom edge but needs less of it, because in
    ///   landscape there is no status bar stacked above it.
    ///
    /// Desktop has no unsafe region in either orientation, and a tablet's
    /// rounded corners are not modelled — those are the two simplifications,
    /// stated rather than silently taken.
    #[must_use]
    fn safe_area(self) -> vieww_foundation::EdgeInsets {
        let (top, bottom) = self.platform.insets();
        if !self.landscape || matches!(self.platform, Platform::Desktop) {
            return vieww_foundation::EdgeInsets::only(0.0, top, 0.0, bottom);
        }
        let side = match self.platform {
            // A notched iPhone reports 59 points on each side in landscape.
            Platform::Ios => 59.0,
            // Android's punch-hole is small enough that the status bar's own
            // height is what an app is actually kept clear of.
            Platform::Android => top,
            Platform::Desktop => 0.0,
        };
        let indicator = match self.platform {
            Platform::Ios => 21.0,
            Platform::Android => bottom,
            Platform::Desktop => 0.0,
        };
        vieww_foundation::EdgeInsets::only(side, 0.0, side, indicator)
    }
}

/// The platform the preview simulates. Read at Render time, never live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Ios,
    Android,
    Desktop,
}

impl Platform {
    pub const ALL: [Self; 3] = [Self::Ios, Self::Android, Self::Desktop];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ios => "iOS",
            Self::Android => "Android",
            Self::Desktop => "Desktop",
        }
    }

    /// The logical size of the simulated screen.
    #[must_use]
    pub const fn screen(self) -> (f32, f32) {
        match self {
            Self::Ios => (393.0, 852.0),
            Self::Android => (412.0, 915.0),
            Self::Desktop => (1280.0, 800.0),
        }
    }

    /// What the previewed tree is told it is running on.
    ///
    /// The whole point of the picker: `ThemeData::adaptive` branches on this,
    /// and so does anything in the previewed screen that asks. Desktop reports
    /// the platform the studio is actually running on rather than inventing
    /// one, because that is what a desktop build of the screen would see.
    #[must_use]
    pub fn target(self) -> vieww_foundation::TargetPlatform {
        match self {
            Self::Ios => vieww_foundation::TargetPlatform::IOS,
            Self::Android => vieww_foundation::TargetPlatform::Android,
            Self::Desktop => vieww_foundation::TargetPlatform::current(),
        }
    }

    /// The metrics a screen on this device would be handed.
    #[must_use]
    pub fn view_metrics(self) -> vieww_foundation::ViewMetrics {
        let (width, height) = self.screen();
        let (top, bottom) = self.insets();
        vieww_foundation::ViewMetrics {
            size: vieww_foundation::Size::new(width, height),
            device_pixel_ratio: match self {
                Self::Ios => 3.0,
                Self::Android => 2.625,
                Self::Desktop => 1.0,
            },
            safe_area: vieww_foundation::EdgeInsets::only(0.0, top, 0.0, bottom),
            // No soft keyboard is simulated: the studio has no way to raise one
            // and a fake inset that never changes would be a worse lie than an
            // absent one.
            view_insets: vieww_foundation::EdgeInsets::ZERO,
        }
    }

    /// Safe-area insets as `(top, bottom)`, in logical pixels.
    ///
    /// Simulated, and stated as such in the caption under the preview: these
    /// are the insets a device of this shape reports, not insets read from a
    /// device. Desktop has none.
    #[must_use]
    pub const fn insets(self) -> (f32, f32) {
        match self {
            Self::Ios => (47.0, 34.0),
            Self::Android => (30.0, 24.0),
            Self::Desktop => (0.0, 0.0),
        }
    }

    /// What the status bar says about the simulated device.
    #[must_use]
    pub fn describe(self) -> String {
        let (w, h) = self.screen();
        format!("{} · {w:.0}×{h:.0}", self.label())
    }
}

/// Where the compile pipeline is. M0 shows these; M3 drives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewState {
    /// Nothing has been rendered yet this session.
    Empty,
    /// A compile is in flight. The Render button is disabled.
    Compiling,
    /// The last render succeeded and is what the preview shows.
    Rendered,
    /// The last render failed. The preview still shows the *previous* frame,
    /// and says so — the pane never blanks (plan §2.2).
    Failed,
    /// The project's vieww is not this studio's, so nothing was mounted.
    ///
    /// Separate from [`Failed`](Self::Failed) because it is not a failure of
    /// the code in the buffer and cannot be fixed by editing it. Plan 2 §8.2:
    /// the preview `dlopen`s a library into the studio's own process, which is
    /// sound only when both sides are the same compilation of vieww — so a
    /// mismatch is a *refusal*, not a warning, and a widget crossing that
    /// boundary is undefined behaviour rather than a compatibility risk.
    ///
    /// It is also the one preview outcome with a way forward that does not
    /// involve changing anything: Build and Run has no ABI constraint at all.
    /// §8.2 asks for that offer to be made rather than for the pane to say no
    /// and stop, which is what this state exists to let the UI do.
    AbiRefused,
}

impl PreviewState {
    /// The status-bar phrase.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Empty => "No render yet",
            Self::Compiling => "Compiling…",
            Self::Rendered => "Rendered",
            Self::Failed => "Failed",
            Self::AbiRefused => "ABI mismatch",
        }
    }

    /// Whether the preview is showing a frame older than the buffer.
    #[must_use]
    pub const fn is_stale(self) -> bool {
        matches!(self, Self::Failed | Self::AbiRefused)
    }

    /// The word for the status light, when the *preview* is the thing being
    /// reported. Distinguished from [`label`](Self::label) only in that
    /// "Failed" says what failed: the status bar is shared with the build
    /// pipeline, and a bare "Failed" beside a finished build is the exact
    /// ambiguity [`Activity`] exists to remove.
    #[must_use]
    pub const fn status_label(self) -> &'static str {
        match self {
            Self::Failed => "Render failed",
            other => other.label(),
        }
    }
}

/// Which pipeline the status light is reporting on.
///
/// The studio runs two long operations that are unrelated to each other:
/// the **preview** (`rustc` on one buffer, `dlopen`, mount) and the **build**
/// (`cargo build`, and possibly the binary it produced). Each has its own
/// state machine, and neither is a stage of the other.
///
/// One status light reports both, so it needs to know which one to report.
/// The rule is the one every status bar uses: the most recent thing wins. A
/// build that has just finished is what the user was watching, and a render
/// that failed before it is history — history the Problems panel still has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Activity {
    /// The preview pipeline moved last.
    #[default]
    Preview,
    /// The build pipeline moved last.
    Build,
}

/// What the palette's input is filtering, chosen by its first character.
///
/// One field with four meanings rather than four fields, because the thing
/// being replaced is a *mode switch the user has to find*: a separate Go to
/// File command, a separate Go to Symbol command, and a person who knows about
/// neither. A prefix is discoverable from the placeholder and costs one
/// keystroke from inside the palette they already have open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    /// Bare text: files in the workspace.
    Files,
    /// `>`: the command registry, which is what the palette used to be only.
    Commands,
    /// `@`: definitions in the active buffer.
    Symbols,
    /// `:`: a line number in the active buffer.
    Line,
}

impl PaletteMode {
    /// The mode a query is in, and the query with its prefix removed.
    #[must_use]
    pub fn parse(query: &str) -> (Self, &str) {
        match query.as_bytes().first() {
            Some(b'>') => (Self::Commands, &query[1..]),
            Some(b'@') => (Self::Symbols, &query[1..]),
            Some(b':') => (Self::Line, &query[1..]),
            _ => (Self::Files, query),
        }
    }

    /// What the field says when it is empty.
    #[must_use]
    pub const fn placeholder(self) -> &'static str {
        match self {
            Self::Files => "Go to file \u{2014} > commands, @ symbols, : line",
            Self::Commands => "Run a command",
            Self::Symbols => "Go to a definition in this file",
            Self::Line => "Go to line",
        }
    }
}

/// One row of the palette, whatever it is currently listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntry {
    Command(Command),
    /// A file in the workspace tree. `label` is the name, `detail` its folder.
    File {
        path: std::path::PathBuf,
        label: String,
        detail: String,
    },
    /// A definition in the active buffer.
    Symbol {
        label: String,
        detail: String,
        line: u32,
    },
    /// A line number, valid or not — an out-of-range one still shows, and says
    /// so, because silently listing nothing looks like the palette is broken.
    Line {
        line: u32,
        detail: String,
    },
}

/// Which right-pane tab is showing.
///
/// # Why the right pane grew tabs
///
/// It had one thing in it — the preview — and the prototype's `S.rightTab`
/// lists four. The Inspector in particular has nowhere else sensible to live:
/// it describes *the screen next to it*, and reading a tree in a 248-point
/// sidebar while the thing it describes is on the other side of the window is
/// the arrangement that made it easy to leave as a stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Preview,
    /// The previewed screen's render tree, read out of the frame.
    Inspector,
    /// N-new: the viewport the preview is framed at. The prototype modelled
    /// this tab and the studio had two where it drew three.
    Devices,
    /// The Rust a Say buffer compiles to. A view, not a fork — named
    /// separately so it never reads as an alternative source of truth.
    GeneratedRust,
}

impl RightTab {
    pub const ALL: [Self; 4] = [
        Self::Preview,
        Self::Inspector,
        Self::Devices,
        Self::GeneratedRust,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preview => "Preview",
            Self::Inspector => "Inspector",
            Self::Devices => "Devices",
            Self::GeneratedRust => "Generated Rust",
        }
    }
}

/// Which bottom-panel tab is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTab {
    Problems,
    Output,
    /// N-new: start a command in the workspace. See [`Studio::run_command`]
    /// for what this is and, more importantly, what it is not.
    Run,
    /// §8.1's queue, made visible: every child process the studio has started,
    /// running or recently ended, with a cancel on each.
    Tasks,
    Rustc,
    Timings,
}

impl PanelTab {
    pub const ALL: [Self; 6] = [
        Self::Problems,
        Self::Output,
        Self::Run,
        Self::Tasks,
        Self::Rustc,
        Self::Timings,
    ];

    /// Where this tab sits in [`Self::ALL`] — its own scroll position in
    /// [`Studio::panel_scroll`], so reading a long build log and switching to
    /// Problems and back finds the log where it was left.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Problems => 0,
            Self::Output => 1,
            Self::Run => 2,
            Self::Tasks => 3,
            Self::Rustc => 4,
            Self::Timings => 5,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Problems => "Problems",
            Self::Output => "Output",
            Self::Run => "Run",
            Self::Tasks => "Tasks",
            Self::Rustc => "rustc",
            Self::Timings => "Timings",
        }
    }

    /// The tab whose [`label`](Self::label) is `name`, for the session file.
    /// Same contract as [`View::from_title`]: an unknown name is `None`, not an
    /// error.
    #[must_use]
    pub fn from_label(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.label() == name)
    }
}

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One `rustc` diagnostic, already mapped back to the buffer.
///
/// M5 builds these from `--error-format=json`; M0 carries a fixed list so the
/// panel is laid out against real content rather than against a placeholder
/// that turns out to be the wrong shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Which buffer it is about. M0 flagged lines by number alone, which meant
    /// the marks stayed put when the editor switched files — a diagnostic
    /// pointing at line 17 of whatever happens to be open is worse than none.
    pub file: String,
    pub severity: Severity,
    /// `E0308`, `unused_imports` — whatever rustc called it.
    pub code: String,
    pub message: String,
    /// rustc's `help:` line, when it gave one.
    pub help: Option<String>,
    pub line: u32,
    pub column: u32,
    /// The last line and column the span covers, one past the offending text.
    ///
    /// Kept because a mark under *the text that is wrong* is a different thing
    /// from a mark under the line it is on, and only the second was possible
    /// before: `line`/`column` alone say where a span begins and nothing about
    /// where it ends. rustc and cargo both report `line_end`/`column_end` and
    /// this had been throwing them away.
    ///
    /// Defaults to the start when a record has no end — a zero-width range
    /// draws nothing, which is the right answer for a diagnostic that names a
    /// point rather than a span.
    pub end_line: u32,
    pub end_column: u32,
}

impl Diagnostic {
    /// The byte range this diagnostic covers in `text`, if it covers one.
    ///
    /// # Why this is computed here and not carried
    ///
    /// rustc reports `byte_start`/`byte_end` too, and they are offsets into the
    /// file **it** compiled. For a project build that is the file on disk and
    /// they would be usable; for the preview it is a temp file holding the
    /// buffer plus an appended entry point, so the offsets are correct for a
    /// string the editor never has. Line and column are the same in both, which
    /// is why they are what is stored and this is what resolves them.
    #[must_use]
    pub fn span_in(&self, text: &str) -> Option<vieww_foundation::TextRange> {
        let start = byte_of(text, self.line, self.column)?;
        let end = byte_of(text, self.end_line, self.end_column)?;
        (end > start).then(|| vieww_foundation::TextRange::new(start, end))
    }
}

/// The byte offset of a one-based line and column in `text`.
///
/// Column is counted in **characters**, because that is what rustc counts in —
/// a column of 14 on a line with an em dash in it is not byte 14.
fn byte_of(text: &str, line: u32, column: u32) -> Option<usize> {
    let line = usize::try_from(line).ok()?.checked_sub(1)?;
    let column = usize::try_from(column).ok()?.checked_sub(1)?;
    let mut offset = 0;
    for (index, contents) in text.split_inclusive('\n').enumerate() {
        if index == line {
            let within = contents
                .char_indices()
                .nth(column)
                .map_or(contents.trim_end_matches('\n').len(), |(at, _)| at);
            return Some(offset + within);
        }
        offset += contents.len();
    }
    None
}

/// An export part-way through: what it is doing, and which step is next.
#[derive(Debug, Clone)]
pub struct ExportRun {
    pub plan: crate::export::Plan,
    /// The index of the step to start next. Equal to `plan.steps.len()` once
    /// every step has been started.
    pub next: usize,
}

/// Which line `offset` falls on, counting from zero.
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count()
}

/// `formatted`, with the caret put back on `line`.
///
/// The column is dropped rather than carried across: a formatter's whole job
/// is to move columns, so a column from the old text is a number about a
/// string that no longer exists. Start-of-line is the honest place to land.
fn reformatted_value(formatted: &str, line: usize) -> vieww_foundation::TextEditingValue {
    let offset = formatted
        .split('\n')
        .take(line)
        .map(|l| l.len() + 1)
        .sum::<usize>()
        .min(formatted.len());
    let mut value = vieww_foundation::TextEditingValue::new(formatted);
    value.selection = vieww_foundation::TextSelection::collapsed(offset);
    value
}

/// A vertical rule at every indent level a line sits inside.
///
/// # Why the columns are computed here and the positions are not
///
/// *Which* columns get a guide is a question about the source: the indent of
/// the enclosing blocks, quantised to the tab width. *Where* column `n` falls
/// on screen is a question about the shaped paragraph, and the answer is
/// different for every font. Splitting it this way is what lets the studio
/// answer the first and hand the second to `TextDecorationShape::Guide`,
/// rather than multiplying a column by an advance it would have to assume was
/// uniform — true of the monospaced face the code pane asks for, and false the
/// moment one glyph falls back to a face that is not.
///
/// # Blank lines
///
/// A blank line has no indent of its own, so it inherits the *smaller* of its
/// nearest non-blank neighbours' depths. That is what every editor that does
/// this well settles on: guides run through a gap inside a block and stop at
/// the gap that closes one. A blank line also has no character to hang a mark
/// on, which is the honest limit of drawing these as decorations — the run
/// resumes on the next line that reaches the column.
fn indent_guides(
    text: &str,
    tab_width: usize,
    color: vieww_foundation::Color,
) -> Vec<TextDecoration> {
    let tab_width = tab_width.max(1);
    let lines: Vec<&str> = text.split('\n').collect();

    let depths: Vec<Option<usize>> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                None
            } else {
                Some(leading_columns(line, tab_width))
            }
        })
        .collect();

    let mut marks = Vec::new();
    let mut offset = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let depth = depths[index].unwrap_or_else(|| blank_line_depth(&depths, index));
        // A guide *at* a line's own indent would sit on its first character,
        // which is a rule through the code rather than beside it. Only the
        // levels strictly inside it are marked.
        let mut column = tab_width;
        while column < depth {
            if let Some(byte) = byte_at_column(line, column, tab_width) {
                let width = line[byte..].chars().next().map_or(1, char::len_utf8);
                marks.push(
                    TextDecoration::guide(
                        TextRange::new(offset + byte, offset + byte + width),
                        color,
                    )
                    .thickness(1.0),
                );
            }
            column += tab_width;
        }
        // `+ 1` for the newline `split` consumed. The last line has none, and
        // nothing reads past it.
        offset += line.len() + 1;
    }
    marks
}

/// How many columns of indent `line` opens with, counting a tab as the jump to
/// the next multiple of `tab_width` rather than as a single column.
fn leading_columns(line: &str, tab_width: usize) -> usize {
    let mut columns = 0;
    for ch in line.chars() {
        match ch {
            ' ' => columns += 1,
            '\t' => columns += tab_width - (columns % tab_width),
            _ => break,
        }
    }
    columns
}

/// The byte offset of the character occupying `column`, if the line reaches
/// that far and a character starts exactly there.
///
/// `None` when a tab straddles the column: there is no glyph at that position
/// to position a mark against, and drawing the rule on the tab's own left edge
/// would put it a whole level out.
fn byte_at_column(line: &str, column: usize, tab_width: usize) -> Option<usize> {
    let mut columns = 0;
    for (byte, ch) in line.char_indices() {
        if columns == column {
            return Some(byte);
        }
        if columns > column {
            return None;
        }
        columns += if ch == '\t' {
            tab_width - (columns % tab_width)
        } else {
            1
        };
    }
    None
}

/// The depth a blank line inherits: the smaller of the nearest non-blank line
/// above and below it, and zero when there is none on either side.
fn blank_line_depth(depths: &[Option<usize>], index: usize) -> usize {
    let above = depths[..index].iter().rev().find_map(|depth| *depth);
    let below = depths[index + 1..].iter().find_map(|depth| *depth);
    match (above, below) {
        (Some(above), Some(below)) => above.min(below),
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => 0,
    }
}

/// Every *other* occurrence of the identifier the caret is inside.
///
/// # Why whole-word, and why not the one under the caret
///
/// Substring matching turns a variable called `n` into a highlight on every
/// word containing the letter, which is noise rather than information. And the
/// occurrence under the caret is the one already being looked at: marking it
/// says nothing, and on a file with one use it would draw a mark that appears
/// to promise a second one somewhere.
///
/// Nothing is marked for a selection or for a caret in whitespace, which is
/// what stops the editor flickering with underlines while someone arrows
/// through indentation.
fn occurrences_of_word_at(text: &str, caret: usize) -> Vec<vieww_foundation::TextRange> {
    const MIN: usize = 2;

    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    if caret > text.len() || !text.is_char_boundary(caret) {
        return Vec::new();
    }
    let start = text[..caret]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(caret, |(at, _)| at);
    let end = text[caret..]
        .char_indices()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(caret, |(at, c)| caret + at + c.len_utf8());

    let word = &text[start..end];
    if word.len() < MIN || word.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find(word) {
        let from = at + found;
        let to = from + word.len();
        at = to;

        let before_is_word = text[..from].chars().next_back().is_some_and(is_word);
        let after_is_word = text[to..].chars().next().is_some_and(is_word);
        if before_is_word || after_is_word || from == start {
            continue;
        }
        out.push(vieww_foundation::TextRange::new(from, to));
    }
    out
}

impl Diagnostic {
    #[must_use]
    pub fn location(&self) -> String {
        format!("{}:{}", self.line, self.column)
    }
}

/// Turn a parsed [`ChordSpec`] into a real [`Chord`].
///
/// Lives here rather than in `customise` so that module stays free of the
/// widget types and can be checked by the std-only harness — the same split
/// `folding` and `picker` follow.
///
/// [`ChordSpec`]: crate::customise::ChordSpec
/// [`Chord`]: crate::command::Chord
#[must_use]
fn chord_from_spec(spec: &crate::customise::ChordSpec) -> Option<crate::command::Chord> {
    use vieww_foundation::{LogicalKey, NamedKey};
    let key = match (&spec.character, &spec.named) {
        (Some(character), _) => LogicalKey::Character(character.clone()),
        (None, Some(named)) => LogicalKey::Named(match named.as_str() {
            "Enter" => NamedKey::Enter,
            "Escape" => NamedKey::Escape,
            "Tab" => NamedKey::Tab,
            "Space" => NamedKey::Space,
            "Backspace" => NamedKey::Backspace,
            "Delete" => NamedKey::Delete,
            // `parse_chord` produces only the names above, so this is
            // unreachable in practice and is an unbind rather than a panic if
            // that list ever grows without this one.
            _ => return None,
        }),
        // Neither: the `none` spelling, which unbinds.
        (None, None) => return None,
    };
    Some(crate::command::Chord {
        shortcut: spec.shortcut,
        shift: spec.shift,
        alt: spec.alt,
        key,
    })
}

/// An open completion list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completions {
    /// What the server offered, already filtered by the prefix.
    pub items: Vec<crate::lsp::Completion>,
    /// Which one is highlighted.
    pub index: usize,
    /// The byte range the accepted item replaces — the identifier the caret is
    /// in the middle of. Recorded when the list opens rather than recomputed on
    /// accept, because by then the caret may have moved.
    pub replacing: std::ops::Range<usize>,
}

impl Completions {
    /// How many rows the list draws before it scrolls.
    pub const VISIBLE: usize = 9;

    #[must_use]
    pub fn selected(&self) -> Option<&crate::lsp::Completion> {
        self.items.get(self.index)
    }
}

/// Which tabs a bulk close is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabScope {
    /// Everything but the active tab.
    Others,
    /// Everything after the active tab.
    ToTheRight,
    /// Everything with no unsaved edits, active tab included.
    Saved,
}

/// An open context menu in the Explorer.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    /// Where it was opened, in window coordinates. The menu is positioned here
    /// rather than beside the row, because the pointer is where the user is
    /// looking and a menu that appears somewhere else is a menu they have to
    /// find.
    pub at: vieww_foundation::Offset,
    /// What it is about.
    pub path: std::path::PathBuf,
    /// Whether `path` is a directory — "New File" belongs inside a folder and
    /// beside a file, which is two different parents.
    pub is_dir: bool,
}

/// What a name is being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    NewFile,
    NewFolder,
    Rename,
    /// N2: scaffolding a new vieww project. Validates against Cargo's crate
    /// name rules rather than the file-name rules the other three variants
    /// use — a crate name may contain `-`, may not start with a digit, and
    /// may not be a Rust keyword. The validation lives in
    /// [`crate::scaffold::check_name`], which the file-name variants do not
    /// share because they are about a leaf on disk rather than a Cargo
    /// package.
    NewProject,
    /// Naming a buffer that has never been on disk, so it can be written.
    ///
    /// # The bug this closes
    ///
    /// `File > New File` opens a buffer with no `path`, and Save was gated on
    /// `path.is_some()` — so the one command whose entire job is "do not lose
    /// this" was permanently greyed out on exactly the buffers that only exist
    /// in memory. Type a screen into a new file, press Save, and nothing
    /// happened; close the window and the work was gone. The disabled reason
    /// even said "open a folder first", which is not what was wrong and not
    /// something that would have fixed it.
    SaveAs,
}

impl NameKind {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::NewFile => "New File",
            Self::NewFolder => "New Folder",
            Self::Rename => "Rename",
            Self::NewProject => "New Project",
            Self::SaveAs => "Save As",
        }
    }
}

/// A pending "what should it be called?".
#[derive(Debug, Clone, PartialEq)]
pub struct NamePrompt {
    pub kind: NameKind,
    /// The directory a new item goes in, or the file being renamed.
    pub target: std::path::PathBuf,
    pub text: String,
    /// Why the current text will not do, if it will not.
    pub error: Option<String>,
    /// What a new project's screens are written in. Only read when
    /// [`NameKind::NewProject`]; Say is the default, because the first thing
    /// a stranger meets should be Say — a default, not a question.
    pub project_kind: crate::scaffold::ProjectKind,
}

/// What the folder picker is being asked to choose.
///
/// See [`Studio::picker_mode`] for the longer story. The short version: the
/// picker is one widget that serves two flows, and confirming in each does a
/// different thing. The mode is what `picker_choose` reads to decide which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickerMode {
    /// Pick a folder to **open** as the workspace. The default, and the
    /// behaviour the File → Open Folder… command wants.
    #[default]
    Open,
    /// Pick a folder to **create a new project inside**. Confirming closes the
    /// picker and opens the name prompt with the picked folder as the parent,
    /// so the second step (the name) is what the user actually typed and the
    /// folder they picked is where the project lands.
    NewProject,
}

impl PickerMode {
    /// What the picker's header shows, beside the breadcrumb.
    ///
    /// Two distinct titles rather than one generic "Pick a folder" because the
    /// title is what tells the user what confirming will do — and "Open Folder"
    /// vs "Create Project In" are different actions that need different
    /// affordances even though the picker that asks is the same widget.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Open => "Open Folder",
            Self::NewProject => "Create Project In",
        }
    }

    /// What the confirm button says.
    ///
    /// "Open this folder" vs "Choose this folder": the second is deliberately
    /// softer, because confirming in New Project mode does not *open*
    /// anything — it closes the picker and opens the name prompt. A button
    /// that said "Open this folder" would lie about what comes next.
    #[must_use]
    pub const fn confirm_label(self) -> &'static str {
        match self {
            Self::Open => "Open this folder",
            Self::NewProject => "Choose this folder",
        }
    }
}

/// One remembered lint pass. See [`Studio::lint_findings`].
///
/// `Clone` because `Studio` is — the studio handle is cloned into every widget
/// that holds one. Cloning a remembered pass copies the text it was keyed on
/// and bumps a refcount on the findings; the clone is then a cache that is
/// warm rather than one that has to be refilled.
#[derive(Debug, Clone)]
struct LintPass {
    /// Which buffer it was run over — the same file name the diagnostics carry.
    name: String,
    /// The exact text it was run over. Compared by length and then in full, the
    /// way the highlighter's cache key is, so a large unchanged file is an
    /// integer compare.
    text: String,
    findings: Rc<Vec<Diagnostic>>,
}

/// The fold-region scan, cached against the text it was run over.
///
/// A named type because the shape appears in three places — the studio, the
/// gutter's inputs, and the constructor that hands one to the other — and a
/// nested `Rc<RefCell<Option<(..)>>>` written out three times is three chances
/// to write a different one.
type FoldCache = Rc<RefCell<Option<(String, Rc<Vec<crate::folding::Region>>)>>>;

/// Whether a derivation's signal reads should subscribe the caller.
///
/// `GutterSource::rows` runs in two places with the same arithmetic and
/// opposite tracking needs: inside a [`Memo`], where every read must record a
/// dependency, and from [`Studio::gutter_rows_now`], which is the oracle a test
/// compares the memo against and must subscribe nothing. One function with a
/// flag rather than two copies of the arithmetic is what keeps the oracle
/// honest — an oracle that has drifted from the thing it checks is worse than
/// no oracle at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tracking {
    /// `get` — the memo's path.
    On,
    /// `peek` — the oracle's.
    Off,
}

/// The inputs the gutter is derived from.
///
/// Handles rather than a `Studio`, for two reasons. A memo's closure holding
/// the studio that owns the memo is a reference cycle; and the honest list of
/// what the gutter actually depends on is worth being able to read, which
/// `&self` hides.
#[derive(Debug, Clone)]
struct GutterSource {
    buffers: Signal<Rc<Vec<Buffer>>>,
    active_buffer: Signal<usize>,
    folds: Signal<Rc<crate::folding::Folds>>,
    diagnostics: Signal<Rc<Vec<Diagnostic>>>,
    lint_enabled: Signal<bool>,
    lint_cache: Rc<RefCell<Option<LintPass>>>,
    fold_cache: FoldCache,
}

impl GutterSource {
    /// Every foldable region of `text`, memoised on the text itself.
    ///
    /// `folding::regions` walks the whole buffer, and this derivation runs on
    /// every caret move and every fold toggle as well as on every edit — only
    /// the edit can have changed the answer. Keyed the same way [`LintPass`]
    /// is: length first, so the common "nothing changed" case is an integer
    /// compare.
    fn fold_regions(&self, text: &str) -> Rc<Vec<crate::folding::Region>> {
        {
            let cached = self.fold_cache.borrow();
            if let Some((held, regions)) = cached.as_ref() {
                if held.len() == text.len() && held == text {
                    return Rc::clone(regions);
                }
            }
        }
        let regions = Rc::new(crate::folding::regions(text));
        *self.fold_cache.borrow_mut() = Some((text.to_owned(), Rc::clone(&regions)));
        regions
    }

    /// The lines carrying a diagnostic, from the compiler and the linter alike.
    ///
    /// The lint half reads the cache rather than running the pass: this is a
    /// *derivation*, and running a tree-sitter walk inside one would make a
    /// caret move cost a parse. A cold cache contributes no lint marks for one
    /// frame, and the pass that fills it is the editor's own, which runs anyway.
    fn flagged(&self, buffer: &Buffer, tracking: Tracking) -> Vec<u32> {
        let published = match tracking {
            Tracking::On => self.diagnostics.get(),
            Tracking::Off => self.diagnostics.peek(),
        };
        let lint_on = match tracking {
            Tracking::On => self.lint_enabled.get(),
            Tracking::Off => self.lint_enabled.peek(),
        };
        let mut lines: Vec<u32> = published
            .iter()
            .filter(|d| d.file == buffer.name)
            .map(|d| d.line)
            .collect();
        if lint_on && buffer.language.highlighted() {
            let cached = self.lint_cache.borrow();
            if let Some(pass) = cached.as_ref() {
                if pass.name == buffer.name
                    && pass.text.len() == buffer.value.text.len()
                    && pass.text == buffer.value.text
                {
                    lines.extend(pass.findings.iter().map(|d| d.line));
                }
            }
        }
        lines
    }

    /// The rows themselves.
    fn rows(&self, tracking: Tracking) -> Vec<GutterRow> {
        let (open, index, folds) = match tracking {
            Tracking::On => (
                self.buffers.get(),
                self.active_buffer.get(),
                self.folds.get(),
            ),
            Tracking::Off => (
                self.buffers.peek(),
                self.active_buffer.peek(),
                self.folds.peek(),
            ),
        };
        let Some(buffer) = open.get(index) else {
            return Vec::new();
        };
        let view = crate::folding::project(&buffer.value.text, &folds);
        let foldable = self.fold_regions(&buffer.value.text);
        let flagged = self.flagged(buffer, tracking);
        let (caret_line, _) = buffer.caret();

        view.lines
            .iter()
            .map(|&source_line| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a line index in a file an editor will open fits u32"
                )]
                let number = source_line as u32 + 1;
                // The outermost region starting here, which is the one a marker
                // on this line means.
                let region = foldable
                    .iter()
                    .filter(|region| region.header == source_line)
                    .max_by_key(|region| region.last);
                GutterRow {
                    number,
                    flagged: flagged.contains(&number),
                    active: number == caret_line,
                    fold: region.map(|_| folds.contains(&source_line)),
                    fold_line: source_line,
                }
            })
            .collect()
    }
}

/// One row of the editor's gutter: everything it draws, and nothing that
/// produced it.
///
/// # Why the gutter has a published shape at all
///
/// It is one fixed-height row per source line, and on a 62-line file that is
/// **175 elements** — the largest single region in the studio's keystroke
/// rebuild once the minimap stopped being a widget tree. It was rebuilt on
/// every character typed because it derives from the buffer's *text*: the
/// folded projection, the foldable regions, the diagnostics and the caret line
/// all come out of it.
///
/// But almost none of that actually *changes* when a character is typed inside
/// a line. The line count is the same, the caret is on the same line, the
/// regions start and end on the same lines, and the diagnostics are whatever
/// the last compile said. So the derivation is done once per write and
/// published with
/// [`set_if_changed`](vieww_element::Signal::set_if_changed) — the same shape
/// as [`crate::buffer::BufferTab`] — and typing produces an
/// identical `Vec` that notifies nobody.
///
/// Derived by a [`Memo`] over exactly those four inputs, so nothing has to
/// remember to recompute it and there is no door to forget.
/// `the_gutter_matches_a_fresh_derivation` stays as the oracle that the
/// *definition* is right — a memo is only as correct as what it computes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GutterRow {
    /// The **source** line number, counting from 1.
    ///
    /// Not the row's position: a folded file's gutter reads 1, 2, 9, 10 —
    /// the numbers that exist — because renumbering it would make every
    /// diagnostic, every jump and every `git blame` disagree with the screen.
    pub number: u32,
    /// Carries a diagnostic, so the row shows an error mark.
    pub flagged: bool,
    /// The caret is on this line.
    pub active: bool,
    /// A foldable region starts here: `Some(true)` while it is folded,
    /// `Some(false)` while it is open, `None` where there is nothing to fold.
    pub fold: Option<bool>,
    /// The line to toggle when the marker is pressed — the region's header,
    /// which is this row's own source line.
    pub fold_line: usize,
}

/// Everything the shell reads, and the only thing a widget here is given.
#[derive(Debug, Clone)]
pub struct Studio {
    /// Dark chrome. A signal because the theme toggle has to reach the root.
    pub dark: Signal<bool>,
    pub view: Signal<View>,
    /// Search text used by the Search activity view.
    pub search_query: Signal<String>,
    /// Which severities the Problems panel lists. `None` is all of them.
    ///
    /// A filter and not a sort, because the question it answers is "is
    /// anything actually broken" — and on a project mid-refactor that question
    /// is unanswerable through forty warnings. It lives here rather than in
    /// the panel for the reason every other piece of state does: a widget in
    /// this application holds none.
    pub problem_filter: Signal<Option<Severity>>,
    /// Sidebar width, in logical pixels. Dragged by the sidebar's own divider.
    pub sidebar_width: Signal<f32>,
    /// Preview pane width, in logical pixels. Dragged by the centre divider.
    pub preview_width: Signal<f32>,
    /// Whether the sidebar's seam is mid-drag.
    ///
    /// One per divider rather than one shared `Option<Seam>`, because two
    /// signals that are never both true cost less than one signal every
    /// divider in the shell subscribes to: with the shared field, moving
    /// either seam rebuilds both.
    /// How tall the bottom panel's body is, in points. Dragged through
    /// `ui::divider::HorizontalDivider`; the tab strip's collapse leaves it
    /// alone, so a panel that comes back comes back the size it was.
    /// Whether the note under the device frame is showing.
    ///
    /// Dismissible, because it is a paragraph that answers a question once and
    /// then sits in the pane for ever — and restorable from the palette, because
    /// a control that hides something with no way back is a trap. See
    /// `ui::preview::caption`.
    pub preview_note: Signal<bool>,
    pub panel_height: Signal<f32>,
    /// True while the panel's seam is being dragged.
    pub panel_dragging: Signal<bool>,
    /// True while the pointer rests on the panel's seam.
    pub panel_seam_hovered: Signal<bool>,
    pub sidebar_dragging: Signal<bool>,
    /// True while the pointer rests on the sidebar seam, so its grip can show
    /// before anything is dragged. See `ui::divider`.
    pub sidebar_seam_hovered: Signal<bool>,
    /// Whether the editor/preview seam is mid-drag.
    pub preview_dragging: Signal<bool>,
    /// True while the pointer rests on the preview seam. See `ui::divider`.
    pub preview_seam_hovered: Signal<bool>,
    pub platform: Signal<Platform>,
    pub preview: Signal<PreviewState>,
    /// The buffer has been edited since the last render — the dot on Render.
    pub dirty: Signal<bool>,
    pub panel_tab: Signal<PanelTab>,
    pub panel_open: Signal<bool>,
    /// Whether the preview simulates the device's unsafe regions.
    ///
    /// On, and it was off. Both halves of that matter:
    ///
    /// * **What it controls is the simulation, not a tint.** When it is off the
    ///   preview publishes zero safe-area insets, a guest's `SafeArea` is a
    ///   no-op, and the frame draws no island and no home bar — a clean,
    ///   full-bleed rectangle, which is what somebody iterating on a layout
    ///   wants when they ask for the chrome to get out of the way.
    /// * **Off is the wrong default for a phone frame.** The studio's own
    ///   project template opens with a heading in the top-left corner, and the
    ///   default preview is an iPhone. Off meant the first render of the first
    ///   project either ran under a black pill (when the island was drawn
    ///   regardless, which is what shipped) or was drawn in a frame with no
    ///   island at all, on a device that has one. A preview whose entire claim
    ///   is "this is what it looks like on the device" should start by telling
    ///   the truth about the device; the toggle is there for the moment
    ///   somebody wants to set that aside.
    pub show_insets: Signal<bool>,
    /// The platform's capabilities, published above the tree by [`crate::Shell`].
    ///
    /// The clipboard lives in here. `RenderEditableText` implements cut, copy
    /// and paste in full and reads its `Clipboard` out of the inherited scope;
    /// a tree with no services is a tree where those keys are reported
    /// unhandled. Empty by default so a test builds a shell without touching
    /// the machine's pasteboard, and filled in by `main`.
    pub services: Rc<vieww_foundation::SharedServices>,
    /// The wrapper that makes the previewed screen's `AssetBundle` resolve
    /// against the open workspace's `assets/` first, then the studio's own
    /// bundle. Installed once at startup; the project half is swapped through
    /// a `RefCell` when the workspace changes — see [`crate::assets`].
    pub workspace_bundle: Option<Rc<crate::assets::WorkspaceBundle>>,
    /// The caret's blink, shared with whatever is painting it.
    ///
    /// Driven by `Tickers`, like the scroll physics beside it — see
    /// [`crate::caret`] for why the framework leaves this to the caller.
    pub blink: SharedBlink,
    /// Whether the caret blinks at all.
    ///
    /// A setting because a blinking caret is a real accessibility problem for
    /// some people, and because it is the one thing in this window that asks
    /// for a frame twice a second forever.
    pub blink_enabled: Signal<bool>,
    /// Wrap long lines at the pane's edge rather than scrolling sideways.
    ///
    /// A setting and not a mode: the horizontal scroll it replaces still works
    /// for anyone who prefers it, and the two disagree about what a "line" is
    /// often enough that switching between them has to be one click.
    pub word_wrap: Signal<bool>,
    /// Which colour theme is in use, by name.
    ///
    /// `"dark"` and `"light"` are built in. Anything else names a file in the
    /// themes directory beside the settings file — see `crate::theme::Custom`.
    /// A name whose file has gone falls back to the built-in dark theme rather
    /// than to a half-loaded palette.
    pub theme_name: Signal<String>,
    /// Where the Android SDK, its NDK and a JDK are, when the environment does
    /// not say — and what signs a release.
    ///
    /// # Why the studio holds these at all
    ///
    /// `toolchains::detect` reads `ANDROID_HOME`, `ANDROID_NDK_HOME` and
    /// `JAVA_HOME` off the process environment, and a desktop application has
    /// no process environment worth the name: launched from the Dock, from a
    /// `.desktop` entry or from Explorer it inherits the *session's* variables,
    /// not the shell's, so an Android SDK installed by a `.zshrc` export is
    /// invisible to it. The checklist then said "missing" about something that
    /// was plainly installed, and offered no way to say so.
    ///
    /// Empty means "ask the environment", so a studio launched from a terminal
    /// behaves exactly as it always has.
    pub android_home: Signal<String>,
    pub android_ndk: Signal<String>,
    pub java_home: Signal<String>,
    /// The keystore an Android release is signed with, and the alias in it.
    ///
    /// The password is **not** here and not in the settings file — see
    /// [`crate::settings::Settings::keystore_alias`]. It comes from
    /// `VIEWW_KEYSTORE_PASSWORD` at build time.
    pub keystore: Signal<String>,
    pub keystore_alias: Signal<String>,
    /// The Apple Developer team id, for signing an iOS build.
    pub apple_team: Signal<String>,
    /// Workspaces opened before, most recent first. Restored from the session
    /// file at launch and written back whenever a folder is opened.
    pub recent: Signal<Rc<Vec<std::path::PathBuf>>>,
    /// A close was asked for and there is unsaved work.
    ///
    /// The window does **not** shut while this is `Some`: `main`'s close hook
    /// vetoes the first request and sets this, the shell draws the dialog over
    /// everything, and the answer either saves and closes, discards and closes,
    /// or clears the flag. Before this existed, clicking the window's close
    /// button on a modified buffer lost it with no dialog and no recovery file
    /// — the single worst outcome the studio allowed.
    pub quit_prompt: Signal<Option<Rc<Vec<String>>>>,
    /// Set once the user has answered the quit prompt with "close anyway", so
    /// the second close request is not vetoed as well.
    pub quit_confirmed: Rc<std::cell::Cell<bool>>,
    /// Chords the user's keymap file overrides, by command.
    ///
    /// `None` as a value means *unbound* — the only way to free a shortcut
    /// whose default is in the way — which is why this is a map to an
    /// `Option<Chord>` and not simply an absent entry.
    pub keymap: Rc<std::collections::HashMap<Command, Option<crate::command::Chord>>>,
    /// Snippets from the user's snippets file, shown after the built-in ones.
    pub user_snippets: Signal<Rc<Vec<crate::customise::UserSnippet>>>,
    /// A workspace-wide replace that has been planned and not yet confirmed.
    ///
    /// See [`crate::find::Plan`] for why there is a plan at all: there is no
    /// undo across files, so a replace-all with no preview is a change to
    /// thirty files that cannot be taken back from inside the editor.
    pub replace_plan: Signal<Option<Rc<crate::find::Plan>>>,
    /// The outstanding goto-definition or find-references request, and which of
    /// the two it was.
    ///
    /// The pair is needed because the two replies are indistinguishable on the
    /// wire — both are a list of `Location`s — and what the studio does with
    /// them differs. See `Studio::ask_for_places`.
    pub jump_request: Signal<Option<(u32, bool)>>,
    /// Which device preset the preview is framed at, if one was chosen.
    ///
    /// `None` means "whatever the platform's own default is", which is what
    /// the preview did before the Devices tab existed.
    pub device: Signal<Option<Device>>,
    /// Whether the chosen device is on its side.
    pub landscape: Signal<bool>,
    /// Scale the studio's *own* chrome text up.
    ///
    /// Distinct from `preview_text_scale`, which is a property of the simulated
    /// device. This one is about the window the user is looking at, and until
    /// the Accessibility group existed there was no way to ask for it.
    pub large_ui: Signal<bool>,
    /// Push the chrome's contrast up.
    ///
    /// Not a separate theme: it lightens the foregrounds and darkens the
    /// grounds of whichever theme is in use, so it composes with a custom one
    /// rather than replacing it.
    pub high_contrast: Signal<bool>,
    /// The last lint pass, and the buffer it was run over.
    ///
    /// See [`Studio::lint_findings`]: four callers per frame, one answer.
    lint_cache: Rc<RefCell<Option<LintPass>>>,
    /// The last fold-region scan, and the text it was run over.
    ///
    /// `folding::regions` walks the whole buffer, and the gutter memo asks for
    /// it on **every** settle — which includes a caret move and a fold toggle,
    /// neither of which changes the text. Keyed the same way [`LintPass`] is:
    /// length first, so the common "nothing changed" case is an integer
    /// compare.
    fold_cache: FoldCache,
    /// The studio's accent, by name — one of [`crate::theme::ACCENTS`].
    ///
    /// # Why a name and not a `Brand`
    ///
    /// This is written to the settings file and read back by a studio that may
    /// be a different version. `"Teal"` survives a palette being reordered, a
    /// stop being adjusted, and a person editing the file by hand; four hex
    /// numbers and two alphas survive none of those and mean nothing to the
    /// person reading them. An unrecognised name falls back to the default
    /// rather than failing — see [`crate::theme::accent_named`].
    pub accent: Signal<String>,
    /// What the Run box holds, and what it has been given before.
    ///
    /// This is not a terminal and does not claim to be — see
    /// [`Studio::run_command`]. It is a way to start a command in the workspace
    /// and read its output in a panel that already exists.
    pub run_input: Signal<String>,
    /// Commands run before, most recent first, so the obvious ones are one
    /// click away rather than retyped.
    pub run_history: Signal<Rc<Vec<String>>>,
    /// The completion list, when one is open.
    pub completion: Signal<Option<Rc<Completions>>>,
    /// The request id whose reply the completion list is waiting for.
    ///
    /// A reply carrying any other id is a reply to a request the user has
    /// already typed past, and showing it is a list for a prefix they have
    /// moved on from — which looks exactly like a list that is wrong.
    pub completion_request: Rc<std::cell::Cell<Option<u32>>>,
    /// Whether the vieww-specific lint layer contributes to Problems.
    ///
    /// On by default: the rules are few and each one describes a real cost.
    /// Off is a real choice, and it is a setting rather than a build flag
    /// because the honest false-positive story on `signal-write-in-build` is
    /// that a closure inside a `build` is lexically inside it.
    pub lint_enabled: Signal<bool>,
    /// The About dialog is up.
    pub about: Signal<bool>,
    /// The welcome overlay is up.
    ///
    /// Not a sidebar view. A view would put "how do I start?" behind the
    /// activity bar the new user has not learned yet — the welcome has to be
    /// the thing in front of them, and it is raised automatically on a launch
    /// that has no workspace and nothing restored.
    /// Which activity-bar button the pointer is over, if any.
    ///
    /// The bar is ten icons and no words, and it had no tooltip: the only way
    /// to find out what the fourth one down was for was to press it and see
    /// what the sidebar became. A hover writes this, `ui::tooltip` reads it, and
    /// nothing else in the studio cares.
    /// Which reference page the Docs view has open.
    pub docs_page: Signal<usize>,
    pub hovered_view: Signal<Option<View>>,
    /// Where each of those buttons ended up on screen.
    ///
    /// A tooltip needs a rectangle to point at, and a widget cannot work out
    /// its own position: `build` runs before layout, and position is assigned by
    /// the parent afterwards. `Measured` reports it, which is one frame late and
    /// harmless — a tooltip is opened by a pointer that has already been resting
    /// on the button.
    ///
    /// Deliberately **not** a signal. It is written on every layout of the bar,
    /// and a signal written every layout is a rebuild every layout; the tooltip
    /// reads it at build time, by which point the value is there.
    pub view_anchors: Rc<RefCell<[vieww_foundation::Rect; View::ALL.len()]>>,
    pub welcome: Signal<bool>,
    /// Whether this is a launch, and so whether the splash is mounted.
    ///
    /// Set true in `main.rs`'s `.run` closure and false again when somebody
    /// clicks the splash away. Nothing else reads it, and it deliberately does
    /// *not* say how far the animation has got.
    ///
    /// # Why a `bool`, when it started as an `Option<Instant>`
    ///
    /// It held the launch instant so the splash could work out its own progress
    /// from `Instant::now()`, and that is exactly why the splash never
    /// appeared: a signal that is written once marks its reader pending once,
    /// so the overlay built a single time — at zero elapsed, where every reveal
    /// is still fully transparent — and nothing ever rebuilt it. A widget's
    /// progress through an animation belongs to the widget's own state, where
    /// `Animated` keeps it and where the frame loop already knows to tick it;
    /// this signal answers the only question the *studio* has, which is whether
    /// the studio has just started.
    pub splash: Signal<bool>,
    /// The window's logical size, refreshed once a frame.
    ///
    /// Needed by anything that has to decide where to draw relative to the
    /// window rather than to its parent — a context menu at the pointer, which
    /// has to flip when it would run off the bottom — and by the session file,
    /// which restores the geometry. Nothing in a widget can ask for this: a
    /// build sees constraints, not the window.
    pub window_size: Signal<vieww_foundation::Size>,
    /// The Explorer's context menu: where it is, and what it is about.
    ///
    /// `None` on almost every frame. The studio had no context menu of any
    /// kind and no secondary-click handling anywhere — which is why
    /// `file_tree::rename` and `delete_to_trash` were written, tested, and
    /// called by nothing. This is the wire.
    pub context_menu: Signal<Option<ContextMenu>>,
    /// A rename or new-file prompt: what is being named, and the text so far.
    pub name_prompt: Signal<Option<NamePrompt>>,
    /// Files awaiting a delete confirmation.
    pub pending_delete: Signal<Option<std::path::PathBuf>>,
    /// Open buffers whose file changed on disk while they had unsaved edits.
    ///
    /// Empty on almost every frame. Non-empty raises the conflict dialog, which
    /// is the only thing standing between the user and silently overwriting
    /// whatever changed the file.
    pub conflicts: Signal<Rc<Vec<std::path::PathBuf>>>,
    /// The dialog's answer asked for the window to go. Drained by `main`.
    ///
    /// A flag rather than a call, because closing a window is the platform's
    /// job and the only handle to it lives in `main`. The frame hook drains
    /// this exactly the way it drains a finished compile.
    pub close_requested: Rc<std::cell::Cell<bool>>,
    /// Whether the editor paints syntax-highlighted spans.
    ///
    /// A setting, and a diagnostic. The spans a highlighter produces are handed
    /// to `TextField::spans`, and the render object lays the *paragraph* out
    /// from them — so they decide hit testing as well as colour. Being able to
    /// turn them off is how "is the caret landing in the wrong place because of
    /// highlighting?" becomes a question with an answer.
    pub highlight_enabled: Signal<bool>,
    /// Whether the code pane draws a vertical rule at each indent level.
    ///
    /// Drawn as `TextDecorationShape::Guide` marks on the field itself rather
    /// than as positioned children over it: a guide has to sit at the *shaped*
    /// x of a column, and after a change to the font, the tab width or the
    /// text, only the paragraph knows where that is. Doing it as an overlay
    /// would mean the application multiplying a column by an advance it had to
    /// guess — which is right for a monospaced font and silently wrong for
    /// every other one.
    pub indent_guides: Signal<bool>,
    /// N6: which regions of the active buffer are folded, by header line.
    ///
    /// Per-buffer would be better and is a change to `Buffer`; this is
    /// per-*editor*, and switching tabs clears it. Said out loud rather than
    /// left to be discovered: a fold set carried onto a different file would
    /// hide lines chosen for another one, which is worse than losing it.
    pub folds: Signal<Rc<crate::folding::Folds>>,
    /// How many columns one level of indentation is, for the guides above and
    /// for anything else that has to turn a tab into a position.
    ///
    /// A setting rather than a constant because it is a property of the *file*
    /// — the studio's own source is four spaces, and a project that uses two
    /// would otherwise get guides in the wrong places on every line.
    pub tab_width: Signal<usize>,
    /// Whether each diagnostic's message is drawn at the end of the line it is
    /// on, rather than only in the Problems panel.
    ///
    /// This one *is* an overlay, because it is text — and text is not
    /// something a decoration can be. It is positioned from
    /// [`layout_probe`](Self::layout_probe), which is the field telling the
    /// application where its lines actually ended.
    pub inline_diagnostics: Signal<bool>,
    /// What the code field measured at its last layout.
    ///
    /// Not a `Signal`: a report is *read* during a build and must never be
    /// subscribed to, or a field that publishes during layout would invalidate
    /// the build that produced it, forever. The rebuild that follows a change
    /// is asked for explicitly, once, through
    /// [`layout_generation`](Self::layout_generation).
    pub layout_probe: TextLayoutProbe,
    /// Bumped when — and only when — the probe's report actually changes.
    ///
    /// The editor reads this, so a changed measurement rebuilds the pane; the
    /// next layout of unchanged text publishes an equal report, which changes
    /// nothing and wakes nobody. That is what makes "read the previous frame's
    /// geometry" settle after one extra frame instead of spinning.
    pub layout_generation: Signal<u64>,
    /// The caret the horizontal scroll last followed, as
    /// `(layout_generation, cursor.left, cursor.width)`.
    ///
    /// **The reason horizontal scrolling exists at all.** The inline-diagnostics
    /// overlay used to call [`ScrollController::reveal`] for the caret on
    /// *every* rebuild of the code pane — and a scroll of the pane is itself a
    /// rebuild, because the pane subscribes to the scroll offset. So the frame
    /// after the user dragged sideways, the reveal measured the caret the drag
    /// had just pushed off screen and dragged the window straight back to it.
    /// Wheel, trackpad and the scrollbar all publish the same way, so every
    /// path of sideways travel sprang back to where it started: reported as
    /// "the horizontal scroll in the editor is not functional".
    ///
    /// The reveal is now for *following a moving caret*, which is the only
    /// thing it was ever for: it runs when the caret's measured rectangle
    /// changes, and stays out of the way when the user is the one moving the
    /// window. `(u64, f32, f32)` rather than a cursor rect so an unchanged
    /// report compares equal without a `Rect` method of its own; started at
    /// generation `0` and `NAN`s, which compare unequal to anything including
    /// themselves, so the first frame always follows.
    pub revealed_caret: std::cell::Cell<(u64, f32, f32)>,
    /// Whether the far-left activity bar is showing.
    ///
    /// A bool rather than the sidebar's width trick, for the reason
    /// `MainColumn` gives for the right pane: the activity bar has no divider
    /// on its far side to keep the place of, so there is nothing to restore
    /// except "it was there". Toggled by `Command::ToggleActivityBar`, the
    /// way the sidebar is toggled by Ctrl+B.
    pub activity_bar_open: Signal<bool>,
    /// Every open file, text included.
    ///
    /// **Read this only if you need the text.** Everything else — a name, a
    /// dirty dot, a language, a read-only flag — is on [`tabs`](Self::tabs),
    /// which a keystroke does not touch. See [`BufferTab`] for the measurement.
    ///
    /// Written through `Studio::put_buffers` and nowhere else, so
    /// the two can never disagree.
    pub buffers: Signal<Rc<Vec<Buffer>>>,
    /// The same files, without their text.
    ///
    /// A [`Memo`] over [`buffers`](Self::buffers): re-derived when the buffer
    /// list is written, published **only when a field of a [`BufferTab`]
    /// actually differs**. Typing changes text and nothing else, so it notifies
    /// nobody. This is what the tab strip, the explorer's open-buffer list, the
    /// Save button and the status bar read.
    ///
    /// It used to be a second `Signal` written by hand beside `buffers` at
    /// every one of twenty-one call sites, with a test whose only job was to
    /// catch the site somebody forgot. A memo cannot be forgotten: the
    /// derivation *is* the definition.
    pub tabs: Memo<Rc<Vec<BufferTab>>>,
    /// The gutter's rows.
    ///
    /// A [`Memo`] over the buffer, the caret, the folds and the diagnostics —
    /// see [`GutterRow`] for what it costs and why it is worth it. Typing
    /// inside a line moves none of them, so the derived `Vec` compares equal
    /// and the gutter's 175 elements are never told.
    pub gutter: Memo<Rc<Vec<GutterRow>>>,
    pub active_buffer: Signal<usize>,
    /// Where the files came from, for the explorer's header. `None` is the
    /// scratch workspace.
    pub root: Signal<Option<std::path::PathBuf>>,
    /// The recursive directory tree the Explorer renders (N1). Empty for a
    /// scratch workspace.
    pub tree: Signal<crate::file_tree::FileTree>,
    /// Whether `root` is a vieww project — what lights up Build/Export per the
    /// plan's §5.3. `false` for a scratch workspace.
    pub is_vieww: Signal<bool>,
    /// A poll-based external-change watcher (N1). Held in an `Rc` rather than
    /// directly so the studio — which is `Clone` for the widget tree — can carry
    /// it without cloning the thread handle. Its events are drained each frame
    /// by [`Studio::drain_watcher`].
    pub watcher: Option<std::rc::Rc<crate::file_tree::Watcher>>,
    /// The previewed screen's state, taken before it was replaced.
    ///
    /// Not a signal: nothing on screen reads it, and it lives for exactly the
    /// two frames between a render finishing and the new tree being built.
    /// See [`Studio::capture_preview_state`].
    pub preview_state: Rc<RefCell<Vec<(String, String)>>>,
    /// Set when a render lands, cleared when the state has been put back.
    pub preview_restore_due: Rc<std::cell::Cell<bool>>,
    /// Set when a render starts, cleared once the old tree has been snapshotted.
    pub preview_capture_due: Rc<std::cell::Cell<bool>>,
    /// What has already been written to a recovery file, by origin.
    ///
    /// Not a signal: nothing on screen reads it. It exists so
    /// [`poll_recovery`](Studio::poll_recovery) can skip a write when a dirty
    /// buffer has not changed since the last one.
    pub recovered_state:
        Rc<RefCell<std::collections::BTreeMap<Option<std::path::PathBuf>, String>>>,
    /// The editor's vertical scroll. A controller rather than a plain signal
    /// because a fling has to be simulated after the finger lifts, and only a
    /// controller attached to `Tickers` advances one.
    pub editor_scroll: ScrollController,
    /// And the horizontal one, which did not exist.
    ///
    /// `TextField::wrap(false)` is deliberate — the gutter is one fixed row per
    /// *source* line, and a wrapped line puts every row under it out of step —
    /// but a field that neither wraps nor scrolls sideways simply hides the
    /// text past the right edge. In the recording this was written from, line 1
    /// reached column 388 and the diagnostic's own column was off screen with
    /// no way to reach it. Real Rust passes 100 columns after two levels of
    /// builder chaining, so this is not an edge case.
    pub editor_scroll_x: ScrollController,
    /// The editor tab strip's sideways scroll.
    ///
    /// A tab strip is a row of items whose count the user chooses, which makes
    /// it the same shape as any other list and means it needs the same answer.
    /// Before this it was a bare `Flex::row`: nine open files overflowed it by
    /// 690 logical pixels, the save and new-file controls at the end of the row
    /// were pushed off screen, and the *active* tab could be one of the ones no
    /// longer drawn — so the strip stopped answering the one question it exists
    /// to answer.
    pub tabs_scroll: ScrollController,
    /// The Devices tab's own scroll.
    ///
    /// Nine device rows and a paragraph is 387 logical pixels, and the right
    /// pane on a 1366x679 window has 341 — so `Desktop` and `Laptop` were
    /// simply cut off the bottom, on exactly the machines somebody would be
    /// choosing `Laptop` on.
    pub devices_scroll: ScrollController,
    /// The Generated Rust pane's scroll position, separate from the device
    /// list's so browsing one never moves the other.
    pub generated_scroll: ScrollController,
    /// The preview stage's own scroll, for the frames that no longer shrink.
    ///
    /// The stage used to answer "the pane is shorter than the phone" by scaling
    /// the phone down until it fit, with no floor — which on a 1366x679 window
    /// with the panel open put the frame at 20%: every dimension correct, every
    /// glyph inside it a smudge, and a picture nobody can review. The scale has
    /// a floor now ([`crate::ui::preview::MIN_SCALE`]) and whatever does not fit
    /// under it is reached by scrolling, which is the answer the Devices tab and
    /// the bottom panel already give to the same question.
    pub stage_scroll: ScrollController,
    /// One scroll position per bottom-panel tab.
    ///
    /// **The panel body did not scroll at all.** It was a fixed 176-pixel box,
    /// so a build log past nine lines, a rustc JSON dump, a task list and a
    /// diagnostic with a long `help` were all simply cut off with nothing to
    /// say so and no way to reach the rest — on the one surface whose entire
    /// job is telling the user what went wrong.
    pub panel_scroll: [ScrollController; PanelTab::ALL.len()],
    /// A dropped menu's own scroll.
    ///
    /// One controller for all of them rather than one each: only one menu is
    /// open at a time, and a menu that reopened halfway down where it was left
    /// is a menu whose first item is not where the user is looking. Reset by
    /// [`Studio::open_menu`].
    pub menu_scroll: ScrollController,
    /// A short line for the status bar: what just happened, or why it did not.
    ///
    /// Cleared by the next thing the user does rather than on a timer. A
    /// message that vanishes mid-sentence is one somebody has to reproduce to
    /// finish reading.
    pub notice: Signal<Option<String>>,
    /// The Open Folder picker, when it is open.
    ///
    /// A route in all but name — see [`crate::picker`] for why the studio has
    /// to draw its own. `None` is closed, which is nearly always.
    pub picker: Signal<Option<Rc<crate::picker::Picker>>>,
    /// What the picker is being asked to choose.
    ///
    /// The picker is reused for two flows that look the same but answer
    /// different questions:
    ///
    /// - [`PickerMode::Open`] — *which folder should I open as the workspace?*
    ///   Confirming opens the picked folder as the workspace. This is the
    ///   File → Open Folder… command and the default.
    /// - [`PickerMode::NewProject`] — *where should the new project live?*
    ///   Confirming does **not** open anything — it closes the picker and
    ///   opens the name prompt with the picked folder as the parent. This is
    ///   the File → New Project command, and the second step (the name
    ///   prompt) is what actually creates the project.
    ///
    /// A signal because the picker UI rebuilds on it: the title and the
    /// confirm button's label both depend on the mode, and changing them
    /// without rebuilding would leave the modal's text out of sync with
    /// what confirming will do.
    pub picker_mode: Signal<PickerMode>,
    /// What the right pane is showing.
    pub right_tab: Signal<RightTab>,
    /// The Rust a Say buffer compiles to, shown by the Generated Rust tab.
    ///
    /// Set by [`Studio::render`] when the active buffer is Say and the
    /// codegen step succeeded; cleared whenever a Rust buffer renders. A
    /// view, not a fork — the buffer is still the only thing the user edits.
    pub generated_rust: Signal<Option<Rc<String>>>,
    /// Whether the right pane is shown at all.
    ///
    /// The sidebar and the panel have had this since M0; the preview never
    /// did, so the one pane a person might want out of the way on a laptop was
    /// the one that could not be closed.
    pub right_open: Signal<bool>,
    /// The render tree as of the last frame, for the Inspector.
    ///
    /// # Why a snapshot rather than a live read
    ///
    /// A widget cannot reach the render tree during `build`: the tree it would
    /// read is the one its own build is about to replace, and the borrow does
    /// not exist to be taken anyway. So `main`'s `before_frame` hook — the same
    /// place the compile poll and the file watcher live — reads it and leaves
    /// a description here, and the Inspector draws that.
    ///
    /// One frame behind, which is the same contract `TextLayoutProbe` states
    /// and for the same reason. An inspector showing the previous frame's tree
    /// is telling the truth about a frame the user has already seen.
    ///
    /// A `RefCell` beside a generation `Signal`, like `jobs`: the value is a
    /// vector that would be cloned into a signal on every frame, and the pane
    /// only needs to know that it *changed*.
    pub tree_snapshot: Rc<RefCell<Vec<(usize, vieww_render::NodeDescription)>>>,
    pub tree_generation: Signal<u64>,
    /// Which row of the Inspector is expanded.
    pub inspect_selected: Signal<Option<usize>>,
    /// Paint the regions the last frame actually repainted.
    ///
    /// The framework's central performance claim is damage-driven repaint — a
    /// 20×20 change on a 200×200 surface touches under 5% of it — and nothing
    /// in any application could *see* it. A number in a panel is a number you
    /// believe or do not; a rectangle over the thing that repainted is the
    /// claim itself.
    pub show_damage: Signal<bool>,
    /// Paint a box round every node in the semantics tree.
    ///
    /// The studio is the framework's evidence that its accessibility story
    /// works. This is how somebody building a screen checks theirs.
    pub show_semantics: Signal<bool>,
    /// Click anything to select it in the Inspector.
    pub pick_mode: Signal<bool>,
    /// A point somebody clicked in pick mode, waiting for a frame to resolve it.
    ///
    /// Resolved in `before_frame`, where there is a driver to hit-test against —
    /// a handler has a position and no tree. Same shape as the tree capture.
    pub pick_request: Signal<Option<(f32, f32)>>,
    /// The last frame's damage, in window coordinates.
    pub damage_snapshot: Rc<RefCell<Vec<vieww_foundation::Rect>>>,
    /// The last frame's semantics nodes: where, what, and what it is called.
    pub semantics_snapshot: Rc<RefCell<Vec<(vieww_foundation::Rect, String)>>>,
    /// Bumped when either of the two above changes.
    pub overlay_generation: Signal<u64>,
    /// N7: `rust-analyzer`, when it is running.
    ///
    /// A `RefCell` beside the studio rather than a field of it, for the reason
    /// `jobs` is: `Studio` is `Clone` because the widget tree carries it, and a
    /// child process is not a thing to clone.
    pub analyzer: Rc<RefCell<Option<crate::lsp::Client>>>,
    /// What the status cell says about it.
    pub analyzer_state: Signal<&'static str>,
    /// N10: recompile and remount when the buffer settles.
    pub auto_render: Signal<bool>,
    /// When the debounce is up. `None` when there is nothing pending.
    ///
    /// An `Instant` rather than a frame count: an idle window draws nothing, so
    /// a count would stop advancing at exactly the moment the debounce is
    /// supposed to expire — the trap `Reloader::wake_on_change` documents from
    /// the other side.
    pub render_due: Rc<RefCell<Option<std::time::Instant>>>,
    /// Token overrides, by name. Empty until somebody nudges one.
    ///
    /// A map rather than a whole edited `ColorScheme`, because what the export
    /// and the reset both want to know is *which* tokens were changed — a
    /// scheme with every field present cannot tell an edit from a default.
    pub token_edits: Signal<Rc<std::collections::BTreeMap<String, u32>>>,
    /// Which token group the Tokens view is showing.
    pub token_group: Signal<crate::tokens::Group>,
    /// Preview zoom, or `None` for "fit the pane".
    ///
    /// `None` rather than a number equal to the fit, because the fit changes
    /// with the pane and a zoom that silently stopped following it would be a
    /// bug nobody could describe. Set by the zoom control; cleared by Fit.
    pub preview_zoom: Signal<Option<f32>>,
    /// The editor's font size, in points.
    pub font_size: Signal<f32>,
    /// The accessibility text scale the *previewed* screen is told about.
    ///
    /// The framework applies `text_scale` to every `Text`, and
    /// `vieww-foundation`'s accessibility docs say the design has to survive
    /// 2×. Nothing in the studio could ask for it, so nothing was ever looked
    /// at at 2× — which is the failure mode that note describes: it breaks and
    /// nobody notices until it ships.
    pub preview_text_scale: Signal<f32>,
    /// Whether the previewed screen is told to reduce motion.
    pub preview_reduce_motion: Signal<bool>,
    /// Whether the previewed screen is themed dark.
    ///
    /// `ThemeData::adaptive(platform, dark)`'s `dark` argument used to be
    /// hardcoded to `false`, which meant the preview only ever showed the light
    /// scheme and "does this work in dark mode" was a question the studio could
    /// not answer. This is the toggle a toolbar button beside the platform
    /// picker writes to, and the value `device()` reads when it builds the
    /// theme the previewed screen sees.
    pub preview_dark: Signal<bool>,
    /// Whether the live demo is showing instead of a compiled screen.
    pub live_preview: Signal<bool>,
    /// Whether the live-preview caution dialog is open.
    pub live_caution: Signal<bool>,
    /// True while the "there is no live.rs" dialog is up.
    pub live_missing: Signal<bool>,
    /// The live demo's state — route, counter, toggle, selected item.
    pub live_state: crate::live::LiveState,
    /// Every screen this session has rendered, by the file it came from.
    ///
    /// The live preview's `mount` reads it. A `RefCell` rather than a `Signal`
    /// because a rebuild is driven by the render that filled it, not by the map
    /// itself, and because a loaded `Preview` is an `Rc` around a guest widget
    /// rather than a value worth diffing.
    pub live_mounts: std::rc::Rc<
        std::cell::RefCell<std::collections::HashMap<std::path::PathBuf, crate::loaded::Preview>>,
    >,
    /// Whether the minimap is drawn.
    pub minimap: Signal<bool>,
    /// The sidebar's own scroll position, one per [`View`] — see
    /// [`View::index`]. Every view used to clip rather than scroll, `Scrollable`
    /// wired to a shared, hardcoded `0.0` (see `ui/sidebar.rs`'s history):
    /// nothing owned an offset to give it. Separate controllers rather than one
    /// shared across views, because sharing one would mean switching from
    /// Explorer to Toolchain moved Explorer's scroll position too — reusable,
    /// but wrong in the way that only shows up once both are scrolled.
    pub sidebar_scroll: [ScrollController; View::ALL.len()],
    /// The folder picker's own scroll position.
    ///
    /// **The cap this replaces.** The picker drew eleven rows and then a note
    /// saying "11 of 34 folders shown — go into one to narrow it", which is a
    /// modal telling the user to navigate somewhere else to reach a folder it
    /// is already listing. Eleven was defended as "tens of entries at the very
    /// worst"; `/usr/lib`, `~/Library`, `node_modules` and every monorepo's
    /// package directory are all hundreds.
    pub picker_scroll: ScrollController,
    /// The folder picker's volume-strip scroll position.
    ///
    /// One row of chips across the top of the modal, each one a different
    /// mounted volume — see [`crate::picker::volumes`]. Most machines have two
    /// or three, which fit without scrolling; a server with fifteen mounts
    /// does not, and a row that clipped its overflow was a row whose last
    /// volumes were unreachable. This is the horizontal mirror of
    /// [`picker_scroll`](Self::picker_scroll): same physics, same per-session
    /// persistence, separate controller because sharing one with the entry
    /// list would mean the two axes fought for the same offset.
    pub picker_volume_scroll: ScrollController,
    pub diagnostics: Signal<Rc<Vec<Diagnostic>>>,
    /// Caret position, shown in the status bar. Derived from the active
    /// buffer's selection on every edit rather than tracked separately, so the
    /// two cannot disagree.
    pub caret: Signal<(u32, u32)>,
    /// Parser/cache retained across editor rebuilds.
    pub highlighter: Rc<RefCell<Highlighter>>,

    // ----- M3: the compile pipeline -------------------------------------
    /// The compiler and the `.rlib`s, or why there are none.
    ///
    /// An `Rc<Result<..>>` rather than an `Option`, because "no toolchain" is a
    /// state the UI has to *explain*, not one it can silently treat as "not
    /// rendered yet".
    pub toolchain: Rc<Result<Toolchain, ToolchainError>>,
    /// This run's temp directory. Dropped with the studio, taking its files.
    pub session: Rc<Option<Session>>,
    /// Which of the toolchain's candidate `vieww` rlibs is being used.
    ///
    /// A workspace holds more than one compilation of vieww, and only one of
    /// them shares this process's types. The first Render tries the best guess;
    /// a fingerprint mismatch advances this and retries, so the studio finds
    /// the right one by itself instead of rendering blank. See
    /// [`crate::compile::vieww_candidates`].
    pub candidate: Rc<std::cell::Cell<usize>>,
    /// The screen the preview is showing, once one has compiled.
    pub preview_screen: Signal<Option<Preview>>,
    /// `rustc`'s side of the conversation, for the Output panel.
    pub output: Signal<Rc<Vec<String>>>,
    /// What the last Render cost, for the Timings panel.
    pub timings: Signal<Rc<Vec<(String, String)>>>,
    /// Which buffer the last successful render was of, for the status bar.
    pub rendered_at: Signal<Option<String>>,
    /// What the last successful render cost.
    ///
    /// `None` until one has happened, and the status bar leaves the cell out
    /// rather than showing a number — that cell used to read `1.92s` from a
    /// string literal, on a studio that had never compiled anything.
    pub last_render: Signal<Option<std::time::Duration>>,
    /// `rustc`'s JSON, verbatim, so a diagnostic the parser dropped is still
    /// reachable by a person.
    pub rustc_json: Signal<Rc<Vec<String>>>,

    // ----- the compile now running, if any -------------------------------
    /// The worker thread's handle, polled once a frame by
    /// [`poll_compile`](Self::poll_compile).
    ///
    /// A `RefCell` rather than a `Signal` because a `Job` is neither `Clone`
    /// nor comparable, and a signal's whole job is to hand out copies and
    /// notice changes. What the *UI* watches is `preview`, which the poll
    /// writes — so a compile starting and finishing still rebuilds the tree,
    /// through the state machine that was always meant to drive it.
    pub job: Rc<RefCell<Option<Job>>>,
    /// Asks the platform for a frame from another thread.
    ///
    /// Without it a compile that finishes on an idle window sits in its channel
    /// until the user happens to move the mouse. A default-constructed `Waker`
    /// is detached and drops wakes silently, which is exactly right for a test:
    /// there is no loop to wake and no frame that would go stale.
    pub waker: vieww_platform_winit::Waker,

    // ----- find and replace ----------------------------------------------
    pub find_open: Signal<bool>,
    /// Whether the replace row is showing under the find row.
    pub find_replacing: Signal<bool>,
    pub find_query: Signal<String>,
    pub find_replacement: Signal<String>,
    pub find_case_sensitive: Signal<bool>,
    pub find_whole_word: Signal<bool>,
    /// Which match of the current query is selected, counted from zero.
    pub find_index: Signal<usize>,

    // ----- the command palette -------------------------------------------
    pub palette_open: Signal<bool>,
    pub palette_query: Signal<String>,
    /// Which palette row the arrow keys have moved to.
    pub palette_index: Signal<usize>,
    /// Which title-bar menu is open, if any.
    pub menu_open: Signal<Option<crate::command::Menu>>,

    // ----- N3/N4: building the whole application --------------------------
    //
    // Kept apart from the preview's own state above, and deliberately so: plan
    // 2 §4.3 is that *preview renders a widget, build produces an application*,
    // and the surest way for a UI never to blur two things is for them not to
    // share a field.
    /// The in-flight build, its output and its diagnostics. Plain data behind a
    /// `RefCell` rather than signals, because it is written from a frame hook
    /// and read by widgets — see [`crate::builds`].
    /// §8.1: every child process the studio has started, in one list.
    ///
    /// An `Rc<RefCell<_>>` rather than a signal for the same reason `builds`
    /// is: a `Task` is not `Clone` and the queue is mutated from a frame hook,
    /// not from a build. What the UI subscribes to is
    /// [`jobs_generation`](Self::jobs_generation).
    pub jobs: Rc<RefCell<crate::jobs::Queue>>,
    /// Bumped whenever the queue changed, so the Tasks panel and the status
    /// bar rebuild and nothing else does.
    pub jobs_generation: Signal<u64>,
    /// The queue id of the build currently registered, so the one `Builds`
    /// owns can be resolved when its state machine says it is over.
    ///
    /// A `Cell` rather than a signal: it is bookkeeping between two frame
    /// hooks and nothing draws it.
    pub build_job: Rc<std::cell::Cell<Option<u64>>>,
    /// The export being run, if any: its plan and how far through it is.
    ///
    /// A plan half-run is application state, which is why it lives here and
    /// not in [`export`](crate::export) — that module is a planner and knows
    /// nothing about what happened to a plan it handed over.
    pub export_run: Rc<RefCell<Option<ExportRun>>>,
    /// The chosen export format, for the export sheet's selection.
    pub export_format: Signal<crate::export::Format>,
    /// Devices the last scan found, and whether one has ever run.
    pub devices: Signal<Rc<Vec<crate::export::Device>>>,
    pub devices_scanned: Signal<bool>,
    /// The last thing a build or an export produced.
    ///
    /// `NEXT-VIEWWSTUDIO.md` §6: *"`Follow::Produced` reports the artefact and
    /// nothing offers to install it. The wire exists — `Studio::install_on` —
    /// and the button does not."* This is the missing half: the path was
    /// printed to the Output panel and dropped, so `install_on` had a device
    /// and no artefact to put on it.
    pub artefact: Signal<Option<std::path::PathBuf>>,
    /// What `git status` last said. Empty until a scan has run.
    pub git: Signal<Rc<crate::git::Status>>,
    /// Whether a scan has ever completed, so an empty list can say which kind
    /// of empty it is — clean, or not looked at.
    pub git_scanned: Signal<bool>,
    /// The commit box's contents.
    pub commit_message: Signal<String>,
    /// Whether saving runs `rustfmt` over the file first.
    ///
    /// On by default, which is the convention in every Rust project that has
    /// an opinion — and off is one click away, because a formatter that
    /// rewrites a file somebody is mid-thought in is the single most
    /// complained-about default an editor has.
    pub format_on_save: Signal<bool>,
    pub builds: Rc<RefCell<Builds>>,
    /// [`Builds::state`], mirrored into a signal by [`Studio::poll_builds`] so
    /// the toolbar and status bar can read it like everything else.
    pub build_state: Signal<builds::State>,
    /// Which of the two pipelines reported most recently.
    ///
    /// # The defect this exists for
    ///
    /// The status light showed the *preview* pipeline's outcome whenever no
    /// build was in flight. So a session that went "press Render on `main.rs`
    /// (no `screen()`, fails) → press Build (succeeds in five minutes)" ended
    /// with the Output panel reading `Build finished in 336.8s` and the light
    /// in the corner reading a red **Failed** — two surfaces reporting opposite
    /// verdicts about the same session. Every user who saw it read it as the
    /// build having failed, because the build is what they had just watched.
    ///
    /// The status light is a "what just happened" cell, so it has to be told
    /// what happened *last*. This is that: set by whichever pipeline moved,
    /// read by the status bar to choose which one to report.
    pub activity: Signal<Activity>,
    /// Which profile the Build command uses.
    pub profile: Signal<Profile>,
    /// Whether the editor's own edits — auto-indent, bracket pairing — are on.
    ///
    /// A setting because these are the features people most reliably disagree
    /// about, and because an editor that rewrites what you typed and cannot be
    /// told to stop is worse than one that never did.
    pub comforts: Signal<bool>,
    /// Which target Build and Export act on.
    pub target: Signal<toolchains::Target>,
    /// What the Toolchains view lists. Detected once, at construction: probing
    /// the filesystem on every frame to draw a sidebar would be absurd, and
    /// nothing here changes without the user installing something, at which
    /// point [`Command::ShowToolchain`]
    /// re-runs it.
    pub toolchains: Signal<Rc<Vec<toolchains::Report>>>,
    /// The environment the reports above were read from.
    pub build_env: Rc<toolchains::Env>,

    /// The platform whose keyboard conventions the shortcuts follow.
    ///
    /// The *host's*, not the preview's: ⌘S saves on the machine the studio is
    /// running on regardless of which device the preview is pretending to be.
    /// A field rather than a call to `TargetPlatform::current()` so a test can
    /// check the Mac bindings on a Linux runner.
    pub host: vieww_foundation::TargetPlatform,
}

/// The narrowest each pane may be dragged. Below this the editor stops being
/// an editor and the preview stops being able to hold its own toolbar.
/// The most palette rows built at once. The list scrolls a window of nine, so
/// anything past this is unreachable by arrowing anyway and only costs the
/// filter that produced it.
pub const PALETTE_MAX: usize = 200;

pub const MIN_PANE: f32 = 300.0;

/// The bottom panel's body height when the studio opens, and the height it had
/// been fixed at before the seam above it could be dragged.
pub const PANEL_HEIGHT: f32 = 176.0;

/// The shortest the panel body can be dragged.
///
/// Three rows of output and its padding. Below that the panel is a strip that
/// says a build is running without being able to say anything about it, and the
/// tab strip's collapse is the better way to ask for the space back — that one
/// leaves the tabs, so the panel is one click from returning.
pub const MIN_PANEL: f32 = 72.0;

/// The tallest, as a share of the window.
///
/// A number rather than a constant for the same reason the preview's cap is:
/// on a tall monitor a flat ceiling hands the rest to the editor whether or not
/// that is what somebody is reading. Two thirds, so the editor keeps a usable
/// column at every window size.
#[must_use]
pub fn max_panel(window_height: f32) -> f32 {
    (window_height * 0.66).max(MIN_PANEL + 1.0)
}

/// How often unsaved buffers are written to the recovery directory.
///
/// The number is "how much work is a crash allowed to cost", and five seconds
/// is about a sentence of code. Shorter turns a fast typist's session into a
/// stream of small writes for no gain; longer starts to be a real amount of
/// thinking to redo.
pub const RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
pub const MIN_SIDEBAR: f32 = 180.0;
pub const MAX_SIDEBAR: f32 = 480.0;

/// The width the sidebar comes back at.
///
/// The pane folds to zero — `Command::ToggleSidebar`, Zen mode, and the
/// activity bar's folding press on the view already showing — and unfolds to
/// this, which is also the width a fresh session starts with. One number so
/// "open" means the same width from every switch.
pub const OPEN_SIDEBAR: f32 = 248.0;

impl Studio {
    /// The state a fresh session starts in, with nothing open but scratch.
    #[must_use]
    pub fn new(runtime: &Runtime) -> Self {
        Self::with_workspace(runtime, Workspace::scratch())
    }

    /// The same, holding the files `workspace` found.
    #[must_use]
    pub fn with_workspace(runtime: &Runtime, workspace: Workspace) -> Self {
        // **The same choice `adopt_workspace` makes**, and it has to be made
        // here too: this is the constructor a *launch* goes through — the
        // studio started with a folder on the command line, and the studio
        // reopening the folder it had last time — while `adopt_workspace` is
        // the one Open Folder and New Project go through. Fixing only the
        // second left the first opening on `src/main.rs`, which is the one file
        // in a vieww project that Render refuses, so the studio's own first
        // frame carried a failed render. See `Workspace::preferred_buffer`.
        let active = workspace.preferred_buffer();
        let caret = workspace.buffers.get(active).map_or((1, 1), Buffer::caret);
        // Read once here rather than on every frame that draws the Toolchains
        // view: this walks `PATH` and stats directories, and nothing it finds
        // changes without the user installing something.
        let build_env = toolchains::Env::host();

        // Made before the struct, because two memos derive from them and a
        // memo's closure has to hold the signals rather than the studio — a
        // derivation that captured `self` would be a reference cycle with the
        // thing that owns it.
        let buffers = runtime.signal(Rc::new(workspace.buffers));
        let active_buffer = runtime.signal(active);
        let folds = runtime.signal(Rc::new(crate::folding::Folds::new()));
        let diagnostics = runtime.signal(Rc::new(Vec::new()));
        let lint_enabled = runtime.signal(true);
        let lint_cache: Rc<RefCell<Option<LintPass>>> = Rc::new(RefCell::new(None));
        let fold_cache: FoldCache = Rc::new(RefCell::new(None));
        let gutter_source = GutterSource {
            buffers: buffers.clone(),
            active_buffer: active_buffer.clone(),
            folds: folds.clone(),
            diagnostics: diagnostics.clone(),
            lint_enabled: lint_enabled.clone(),
            lint_cache: Rc::clone(&lint_cache),
            fold_cache: Rc::clone(&fold_cache),
        };

        let studio = Self {
            // The watcher is built before the struct literal so `workspace.root`
            // is not borrowed after it is moved into the `root` signal below.
            preview_state: Rc::new(RefCell::new(Vec::new())),
            preview_restore_due: Rc::new(std::cell::Cell::new(false)),
            preview_capture_due: Rc::new(std::cell::Cell::new(false)),
            recovered_state: Rc::new(RefCell::new(std::collections::BTreeMap::new())),
            watcher: workspace.root.as_ref().and_then(|root| {
                crate::file_tree::Watcher::start(
                    root.clone(),
                    std::time::Duration::from_millis(750),
                )
                .map(std::rc::Rc::new)
            }),
            root: runtime.signal(workspace.root),
            tree: runtime.signal(workspace.tree),
            is_vieww: runtime.signal(workspace.is_vieww),
            caret: runtime.signal(caret),
            highlighter: Rc::new(RefCell::new(Highlighter::new())),
            tabs: {
                let buffers = buffers.clone();
                runtime.memo(move || {
                    Rc::new(buffers.with(|open| open.iter().map(BufferTab::of).collect()))
                })
            },
            gutter: runtime.memo(move || Rc::new(gutter_source.rows(Tracking::On))),
            buffers: buffers.clone(),
            // iOS physics, not the platform default: the editor is a desktop
            // surface driven by a wheel and a trackpad, and `Clamp` pins the
            // offset at zero rather than letting an overscroll show.
            editor_scroll: ScrollController::new(runtime, ScrollPhysics::ios()),
            // **Horizontal constructor, not the default.** `new` builds a
            // vertical controller; this one feeds a horizontal `Scrollable`,
            // and a vertical controller reading a horizontal event takes the
            // `dy` of a sideways swipe — zero — so a wheel that worked on the
            // scrollbar and nowhere else. Both halves of that report were
            // true. See `tests/scroll_gestures.rs`.
            editor_scroll_x: ScrollController::horizontal(runtime, ScrollPhysics::ios()),
            tabs_scroll: ScrollController::horizontal(runtime, ScrollPhysics::ios()),
            devices_scroll: ScrollController::new(runtime, ScrollPhysics::ios()),
            generated_scroll: ScrollController::new(runtime, ScrollPhysics::ios()),
            stage_scroll: ScrollController::new(runtime, ScrollPhysics::ios()),
            panel_scroll: std::array::from_fn(|_| {
                ScrollController::new(runtime, ScrollPhysics::ios())
            }),
            menu_scroll: ScrollController::new(runtime, ScrollPhysics::ios()),
            notice: runtime.signal(None),
            picker: runtime.signal(None),
            picker_mode: runtime.signal(PickerMode::Open),
            generated_rust: runtime.signal(None),
            right_tab: runtime.signal(RightTab::Preview),
            right_open: runtime.signal(true),
            tree_snapshot: Rc::new(RefCell::new(Vec::new())),
            tree_generation: runtime.signal(0),
            inspect_selected: runtime.signal(None),
            show_damage: runtime.signal(false),
            show_semantics: runtime.signal(false),
            pick_mode: runtime.signal(false),
            pick_request: runtime.signal(None),
            damage_snapshot: Rc::new(RefCell::new(Vec::new())),
            semantics_snapshot: Rc::new(RefCell::new(Vec::new())),
            overlay_generation: runtime.signal(0),
            analyzer: Rc::new(RefCell::new(None)),
            analyzer_state: runtime.signal("off"),
            auto_render: runtime.signal(false),
            render_due: Rc::new(RefCell::new(None)),
            token_edits: runtime.signal(Rc::new(std::collections::BTreeMap::new())),
            token_group: runtime.signal(crate::tokens::Group::Colors),
            preview_zoom: runtime.signal(None),
            font_size: runtime.signal(13.0),
            preview_text_scale: runtime.signal(1.0),
            preview_reduce_motion: runtime.signal(false),
            // Light by default: the previewed screen is its own world, and a
            // designer iterating on it asks for dark explicitly rather than
            // inheriting the studio's chrome. This stays independent of the
            // studio's `dark` signal on purpose — the chrome and the preview
            // answer different questions.
            preview_dark: runtime.signal(false),
            live_preview: runtime.signal(false),
            live_caution: runtime.signal(false),
            live_missing: runtime.signal(false),
            live_state: crate::live::LiveState::new(runtime),
            live_mounts: std::rc::Rc::new(
                std::cell::RefCell::new(std::collections::HashMap::new()),
            ),
            minimap: runtime.signal(true),
            // One per view, all built the same way `editor_scroll` is — iOS
            // physics for the same reason, a wheel/trackpad-driven desktop
            // surface where `Clamp` would hide the overscroll rather than show
            // it.
            sidebar_scroll: View::ALL.map(|_| ScrollController::new(runtime, ScrollPhysics::ios())),
            picker_scroll: ScrollController::new(runtime, ScrollPhysics::ios()),
            // The volume chips row is a horizontal `Scrollable`; same reasoning
            // as `editor_scroll_x` above.
            picker_volume_scroll: ScrollController::horizontal(runtime, ScrollPhysics::ios()),
            dark: runtime.signal(true),
            view: runtime.signal(View::Explorer),
            search_query: runtime.signal(String::new()),
            problem_filter: runtime.signal(None),
            sidebar_width: runtime.signal(OPEN_SIDEBAR),
            preview_width: runtime.signal(460.0),
            preview_note: runtime.signal(true),
            panel_height: runtime.signal(PANEL_HEIGHT),
            panel_dragging: runtime.signal(false),
            panel_seam_hovered: runtime.signal(false),
            sidebar_dragging: runtime.signal(false),
            sidebar_seam_hovered: runtime.signal(false),
            preview_dragging: runtime.signal(false),
            preview_seam_hovered: runtime.signal(false),
            platform: runtime.signal(Platform::Ios),
            preview: runtime.signal(PreviewState::Empty),
            dirty: runtime.signal(true),
            panel_tab: runtime.signal(PanelTab::Problems),
            panel_open: runtime.signal(true),
            show_insets: runtime.signal(true),
            highlight_enabled: runtime.signal(true),
            word_wrap: runtime.signal(false),
            theme_name: runtime.signal("dark".to_owned()),
            android_home: runtime.signal(String::new()),
            android_ndk: runtime.signal(String::new()),
            java_home: runtime.signal(String::new()),
            keystore: runtime.signal(String::new()),
            keystore_alias: runtime.signal(String::new()),
            apple_team: runtime.signal(String::new()),
            recent: runtime.signal(Rc::new(Vec::new())),
            quit_prompt: runtime.signal(None),
            quit_confirmed: Rc::new(std::cell::Cell::new(false)),
            close_requested: Rc::new(std::cell::Cell::new(false)),
            conflicts: runtime.signal(Rc::new(Vec::new())),
            keymap: Rc::new(std::collections::HashMap::new()),
            user_snippets: runtime.signal(Rc::new(Vec::new())),
            replace_plan: runtime.signal(None),
            jump_request: runtime.signal(None),
            run_input: runtime.signal(String::new()),
            run_history: runtime.signal(Rc::new(Vec::new())),
            completion: runtime.signal(None),
            completion_request: Rc::new(std::cell::Cell::new(None)),
            large_ui: runtime.signal(false),
            high_contrast: runtime.signal(false),
            lint_cache: Rc::clone(&lint_cache),
            fold_cache: Rc::clone(&fold_cache),
            accent: runtime.signal(crate::theme::accent_name(crate::theme::ACCENT).to_owned()),
            device: runtime.signal(None),
            landscape: runtime.signal(false),
            lint_enabled: lint_enabled.clone(),
            about: runtime.signal(false),
            welcome: runtime.signal(false),
            docs_page: runtime.signal(0),
            hovered_view: runtime.signal(None),
            view_anchors: Rc::new(RefCell::new(
                [vieww_foundation::Rect::ZERO; View::ALL.len()],
            )),
            splash: runtime.signal(false),
            window_size: runtime.signal(crate::WINDOW),
            context_menu: runtime.signal(None),
            name_prompt: runtime.signal(None),
            pending_delete: runtime.signal(None),
            indent_guides: runtime.signal(true),
            folds: folds.clone(),
            tab_width: runtime.signal(crate::edit_ops::INDENT.len()),
            inline_diagnostics: runtime.signal(true),
            layout_probe: TextLayoutProbe::new(),
            layout_generation: runtime.signal(0),
            revealed_caret: std::cell::Cell::new((0, f32::NAN, f32::NAN)),
            activity_bar_open: runtime.signal(true),
            services: Rc::new(vieww_foundation::SharedServices::new(
                vieww_foundation::Services::new(),
            )),
            // `None` until `main` calls `with_services`. Tests that build a
            // studio without services have no workspace bundle, which is the
            // honest state — a previewed screen in such a test asks for an
            // asset and gets `NotFound`, exactly as it did before this field
            // existed.
            workspace_bundle: None,
            blink: Rc::new(RefCell::new(Blink::new())),
            blink_enabled: runtime.signal(true),
            active_buffer: active_buffer.clone(),
            diagnostics: diagnostics.clone(),
            toolchain: Rc::new(Err(ToolchainError::NoRustc(
                "the studio was built without a toolchain".into(),
            ))),
            session: Rc::new(None),
            candidate: Rc::new(std::cell::Cell::new(0)),
            preview_screen: runtime.signal(None),
            output: runtime.signal(Rc::new(Vec::new())),
            timings: runtime.signal(Rc::new(Vec::new())),
            rendered_at: runtime.signal(None),
            last_render: runtime.signal(None),
            rustc_json: runtime.signal(Rc::new(Vec::new())),

            job: Rc::new(RefCell::new(None)),
            waker: vieww_platform_winit::Waker::default(),

            find_open: runtime.signal(false),
            find_replacing: runtime.signal(false),
            find_query: runtime.signal(String::new()),
            find_replacement: runtime.signal(String::new()),
            find_case_sensitive: runtime.signal(false),
            find_whole_word: runtime.signal(false),
            find_index: runtime.signal(0),

            palette_open: runtime.signal(false),
            palette_query: runtime.signal(String::new()),
            palette_index: runtime.signal(0),
            menu_open: runtime.signal(None),

            jobs: Rc::new(RefCell::new(crate::jobs::Queue::new())),
            jobs_generation: runtime.signal(0),
            build_job: Rc::new(std::cell::Cell::new(None)),
            export_run: Rc::new(RefCell::new(None)),
            export_format: runtime.signal(crate::export::Format::DesktopBinary),
            devices: runtime.signal(Rc::new(Vec::new())),
            devices_scanned: runtime.signal(false),
            artefact: runtime.signal(None),
            git: runtime.signal(Rc::new(crate::git::Status::default())),
            git_scanned: runtime.signal(false),
            commit_message: runtime.signal(String::new()),
            format_on_save: runtime.signal(true),
            builds: Rc::new(RefCell::new(Builds::new())),
            build_state: runtime.signal(builds::State::Idle),
            activity: runtime.signal(Activity::Preview),
            profile: runtime.signal(Profile::Debug),
            comforts: runtime.signal(true),
            target: runtime.signal(toolchains::Target::Desktop),
            toolchains: runtime.signal(Rc::new(toolchains::detect(&build_env))),
            build_env: Rc::new(build_env),

            host: vieww_foundation::TargetPlatform::current(),
        };

        // The one wire that cannot be part of the literal above: the probe has
        // to be told to wake a signal that does not exist until the literal is
        // finished. See `layout_generation` for why the wake is conditional
        // and why that is what stops it spinning.
        studio.layout_probe.set_on_change({
            let generation = studio.layout_generation.clone();
            // A signal and nothing else. The publish happens during layout, on
            // the thread the frame is running on, so marking a subscriber is
            // all that is needed to get another frame — `waker` is for waking
            // this thread from another one, and reaching for it here would
            // also capture whichever waker existed at construction rather than
            // the one `with_waker` installs afterwards.
            Rc::new(move || generation.update(|n| *n = n.wrapping_add(1)))
        });
        studio
    }

    /// The same, told which platform's keyboard conventions to follow.
    ///
    /// What `main` does not need — the host is the host — and what a test needs
    /// to check that ⌘S saves on a Mac without running on one.
    #[must_use]
    pub const fn with_host(mut self, host: vieww_foundation::TargetPlatform) -> Self {
        self.host = host;
        self
    }

    /// Give the studio the platform's capabilities — clipboard, storage, assets.
    #[must_use]
    pub fn with_services(mut self, services: vieww_foundation::SharedServices) -> Self {
        self.services = Rc::new(services);
        self
    }

    /// Give the studio the [`WorkspaceBundle`] that wraps the platform's
    /// `AssetBundle` so a previewed screen resolves `assets/` against the open
    /// workspace first. Built in `main` from the platform's `Services`; the
    /// studio points it at the workspace through [`Self::adopt_workspace`].
    ///
    /// [`WorkspaceBundle`]: crate::assets::WorkspaceBundle
    #[must_use]
    pub fn with_workspace_bundle(mut self, bundle: Rc<crate::assets::WorkspaceBundle>) -> Self {
        self.workspace_bundle = Some(bundle);
        self
    }

    /// Give the studio a way to ask for a frame from a worker thread.
    #[must_use]
    pub fn with_waker(mut self, waker: vieww_platform_winit::Waker) -> Self {
        self.waker = waker;
        self
    }

    /// Move the preview state machine, and record that it is what moved.
    ///
    /// **Every transition of [`Studio::preview`] goes through here.** Setting
    /// the signal directly is what let the two pipelines get out of order in
    /// the first place: the status light read `preview` unconditionally, so a
    /// stale `Failed` from a render nobody remembered outlived a build that
    /// succeeded afterwards and reported the render's verdict as the session's.
    /// Pairing the two writes in one method is what keeps
    /// [`Studio::activity`] true by construction rather than by everybody
    /// remembering.
    pub fn set_preview(&self, state: PreviewState) {
        self.preview.set(state);
        self.activity.set(Activity::Preview);
    }

    /// Give the studio a compiler to work with.
    ///
    /// Separate from construction so a test can build the whole shell without
    /// one — and so the failure to find one is a value the UI renders rather
    /// than a panic at startup.
    #[must_use]
    pub fn with_toolchain(
        mut self,
        toolchain: Result<Toolchain, ToolchainError>,
        session: Option<Session>,
    ) -> Self {
        self.toolchain = Rc::new(toolchain);
        self.session = Rc::new(session);
        self
    }

    /// Start compiling the active buffer.
    ///
    /// Returns immediately. `rustc` runs on a worker thread and the result is
    /// picked up by [`poll_compile`](Self::poll_compile) on a later frame — see
    /// [`Job`] for why the synchronous version had to go. The visible
    /// consequence is that `Compiling` is now a state somebody can actually
    /// see, which is what the state machine was drawn for in the first place.
    ///
    /// # One at a time
    ///
    /// A second call while a job is running does nothing, per the plan's §4.5.
    /// The Render button is disabled in that state, so this is a guard rather
    /// than a path — but the shortcut and the menu reach the same command, and
    /// a keystroke does not know a button was greyed out.
    pub fn render(&self) {
        if self.job.borrow().is_some() {
            return;
        }
        // **Exit live preview before compiling.** The live demo and a compiled
        // screen are mutually exclusive — the device frame shows one or the
        // other — so clicking Render while the demo is up returns the preview
        // to the compiled path.
        self.live_preview.set(false);
        let Some(buffer) = self.active() else {
            return;
        };
        // **A buffer containing `// vieww:flow` triggers the live preview
        // instead of compiling.** This is the "flow.rs" convention: a file that
        // names the whole app rather than one screen. The studio cannot compile
        // a whole app into a single `screen()` — the preview is one screen at
        // a time — so a flow file bypasses the build and mounts the built-in
        // demo app, which shows a complete UX with navigation, buttons, state
        // and transitions. The caution dialog explains that it is a visual
        // representation, not the user's code.
        if buffer.value.text.contains("// vieww:flow") {
            if self.has_live_file() {
                self.live_caution.set(true);
            } else {
                self.live_missing.set(true);
            }
            return;
        }
        // **A Say buffer becomes Rust before anything else runs.**
        //
        // The plan's whole shape lives in this branch: Say changes *what the
        // buffer contains*, not how a buffer becomes a preview. Codegen runs
        // before the entry-point check (a raw `.say` file has no `fn
        // screen(` for rustc to find), and its diagnostics — with real line
        // and column numbers — land in the Problems panel without rustc ever
        // seeing the file. Only what escapes Say's own checks needs the
        // `// say:` source map.
        let source: String = if buffer.language == crate::language::Language::Say {
            match vieww_say_codegen::compile(&buffer.name, &buffer.value.text) {
                Ok(generated) => {
                    self.generated_rust
                        .set(Some(Rc::new(generated.rust.clone())));
                    generated.rust
                }
                Err(diagnostics) => {
                    let mapped: Vec<Diagnostic> = diagnostics
                        .into_iter()
                        .map(|d| Diagnostic {
                            file: (*d.file).clone(),
                            severity: match d.severity {
                                vieww_say_codegen::Severity::Error => Severity::Error,
                                vieww_say_codegen::Severity::Warning => Severity::Warning,
                            },
                            code: d.code.to_owned(),
                            message: d.message,
                            help: None,
                            line: d.line,
                            column: d.column,
                            end_line: d.line,
                            end_column: d.column,
                        })
                        .collect();
                    self.diagnostics.set(Rc::new(mapped));
                    self.set_preview(PreviewState::Failed);
                    self.panel_tab.set(PanelTab::Problems);
                    self.panel_open.set(true);
                    return;
                }
            }
        } else {
            self.generated_rust.set(None);
            buffer.value.text.clone()
        };

        // **Refused before rustc, because rustc cannot explain this one.**
        //
        // The preview appends a hidden entry point that calls `screen()`. A
        // buffer without one is a `cannot find function` error against a line
        // number past the end of the file the user is looking at. See
        // `compile::entry_point`. Generated Say always defines one, so this
        // check only ever refuses hand-written Rust.
        if !crate::compile::entry_point(&source) {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a line count that overflows u32 is not an editable buffer"
            )]
            let lines = buffer.value.text.lines().count() as u32;
            self.diagnostics
                .set(Rc::new(vec![crate::compile::missing_entry_for(
                    &buffer.name,
                    lines,
                    &source,
                )]));
            self.set_preview(PreviewState::Failed);
            self.panel_tab.set(PanelTab::Problems);
            self.panel_open.set(true);
            self.output.set(Rc::new(vec![format!(
                "{} has no `screen()` function, so there is nothing for the preview to build.",
                buffer.name
            )]));
            return;
        }

        // Armed here rather than taken here: the snapshot needs the element
        // tree, which only the frame hook can reach. The tree it will find is
        // still the one the person has been interacting with — a compile takes
        // hundreds of frames, and this is the first of them. See
        // `capture_preview_state`.
        self.preview_capture_due.set(true);
        let Ok(toolchain) = self.toolchain.as_ref() else {
            self.set_preview(PreviewState::Failed);
            self.output.set(Rc::new(vec![self
                .toolchain
                .as_ref()
                .as_ref()
                .err()
                .map_or_else(String::new, ToolchainError::to_string)]));
            return;
        };
        let Some(session) = self.session.as_ref() else {
            self.set_preview(PreviewState::Failed);
            self.output.set(Rc::new(vec![
                "no session directory to compile into — the studio could not \
                 create one in the system temp directory"
                    .to_string(),
            ]));
            return;
        };

        let waker = self.waker.clone();
        let wake = move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        };

        // The candidate the fingerprint search has reached. `with_candidate`
        // returns `None` once they are exhausted, and then the original stands
        // so the failure is reported rather than silently skipped.
        let candidate = self.candidate.get();
        let toolchain = toolchain
            .with_candidate(candidate)
            .unwrap_or_else(|| toolchain.clone());

        self.output.update(|output| {
            let mut next = (**output).clone();
            next.push(format!(
                "ABI candidate {candidate}: {}",
                toolchain.vieww_rlib.display()
            ));
            *output = Rc::new(next);
        });

        // **The open project's own library, when the buffer needs it.**
        //
        // A screen file in a real application starts with `use crate::…`, and a
        // preview compiled with `--extern vieww` alone cannot resolve one — so
        // until now the preview worked on files that depended on nothing and
        // refused everything else, which is most of a real application. See
        // `compile::ProjectLib` and `compile::with_project`.
        //
        // Only consulted when the buffer actually reaches for the project.
        // Linking an rlib a file does not use costs a `--extern` rustc ignores,
        // but finding it costs a directory walk on every Render.
        //
        // **Never for Say.** `needs_project`'s heuristic is `crate::` or
        // `use super::` — written for hand-authored Rust, where the second one
        // means "this file reaches into its parent module in the user's own
        // crate". Say's own codegen emits `use super::SayState;` too, but that
        // `super` is the generated file's own root module, not the project's —
        // the same false positive on every Say screen, which would otherwise
        // both raise a "no cargo project is open" diagnostic against a buffer
        // that needs nothing of the kind and, worse, swap the compile onto the
        // *project's* `vieww` build (below) whenever a project happens to be
        // open, which is exactly the mismatched-ABI failure this whole check
        // exists to catch before it reaches `dlopen`. Say's v1 vocabulary has
        // no way to reach outside the generated file at all, so it never needs
        // the project's library.
        let is_say = buffer.language == crate::language::Language::Say;
        let project = if is_say {
            None
        } else {
            self.project_lib(&source)
        };
        // **And the refusal is skipped for Say too, not only the lookup.**
        //
        // This is the second half of the paragraph above, and it was missing.
        // Forcing `project` to `None` stops a Say screen from being compiled
        // against the project's `vieww`, which is what that paragraph is about
        // — but `project_lib_refusal` was still asked, with `source` being the
        // *generated* Rust and `project` now unconditionally `None`. Its own
        // first line is `needs_project(source)`, the `crate::`-or-`use super::`
        // heuristic, and generated Say always emits `use super::SayState;`. So
        // it always matched, always found no project, and always produced:
        //
        //     this file uses `crate::`, so the preview needs <name>'s own
        //     library built
        //
        // against a file containing neither `crate::` nor any `super` of the
        // user's own — the exact false positive the paragraph above describes
        // and suppresses one line too early.
        //
        // What that cost is the whole first run. `vieww new` scaffolds a Say
        // project whose `src/screens/home.say` is `counter.say`, whose own
        // header comment reads "a counter that renders straight away, with no
        // project, no setup and nothing to read first" — and pressing Render on
        // it, the first thing a new user does, was refused with an instruction
        // to go and run a twelve-minute `cargo build` that would not have
        // helped, because the file needs nothing from the project.
        //
        // Say's v1 vocabulary has no way to reach outside the generated file,
        // so there is no case where a Say buffer legitimately needs the
        // project's library and this suppression hides a real problem.
        let refusal = if is_say {
            None
        } else {
            self.project_lib_refusal(&source, project.as_ref())
        };
        if let Some(reason) = refusal {
            self.set_preview(PreviewState::Failed);
            self.diagnostics.set(Rc::new(vec![reason]));
            self.panel_tab.set(PanelTab::Problems);
            self.panel_open.set(true);
            return;
        }

        // The project's own vieww, when there is a project: see
        // `Toolchain::with_vieww`. Without this the studio's candidate ordering
        // and the project's build can disagree, and the disagreement surfaces
        // as an error about the wrong crate.
        let toolchain = match project.as_ref().and_then(|p| p.vieww_rlib.clone()) {
            Some(rlib) => toolchain.with_vieww(rlib),
            None => toolchain,
        };

        match Job::spawn(&toolchain, session, &source, &buffer.name, project, wake) {
            Ok(job) => {
                self.set_preview(PreviewState::Compiling);
                *self.job.borrow_mut() = Some(job);
            }
            Err(error) => {
                self.output
                    .set(Rc::new(vec![format!("could not start rustc: {error}")]));
                self.set_preview(PreviewState::Failed);
            }
        }
    }

    /// Stop the compile that is running, if one is.
    ///
    /// The job is dropped here rather than waited on: the worker notices the
    /// cancel flag, kills `rustc` and finds nobody listening when it tries to
    /// send. Waiting would block the UI thread for exactly as long as the
    /// thing the user just asked to stop.
    pub fn cancel_render(&self) {
        let Some(job) = self.job.borrow_mut().take() else {
            return;
        };
        job.cancel();
        self.output.update(|output| {
            let mut next = (**output).clone();
            next.push("render cancelled".into());
            *output = Rc::new(next);
        });
        // Back to whatever the preview was before, not to `Failed`: nothing
        // failed, and a red light for a compile the user stopped on purpose is
        // a lie the status bar would carry until the next Render.
        self.set_preview(if self.preview_screen.peek().is_some() {
            PreviewState::Rendered
        } else {
            PreviewState::Empty
        });
    }

    /// Whether a compile is in flight. What disables the Render button.
    #[must_use]
    pub fn is_compiling(&self) -> bool {
        self.job.borrow().is_some()
    }

    /// Pick up a finished compile, if one has finished.
    ///
    /// Called once per frame from the platform hook. Returns whether anything
    /// changed, so the caller can tell a frame that did something from one that
    /// only looked.
    ///
    /// Everything below this line used to be the back half of `render`, and it
    /// is unchanged in what it does: diagnostics, then the library, then the
    /// load, and a failure at any point keeps the previous screen on the
    /// preview (plan §2.2). What changed is only *when* it runs.
    pub fn poll_compile(&self) -> bool {
        let progress = {
            let borrowed = self.job.borrow();
            let Some(job) = borrowed.as_ref() else {
                return false;
            };
            job.poll()
        };

        match progress {
            Progress::Running => false,
            Progress::Lost => {
                self.job.borrow_mut().take();
                self.output.update(|output| {
                    let mut next = (**output).clone();
                    next.push("the compile worker stopped without answering".into());
                    *output = Rc::new(next);
                });
                self.set_preview(PreviewState::Failed);
                true
            }
            Progress::Done(result) => {
                let job = self.job.borrow_mut().take();
                let file_name = job.map_or_else(String::new, |job| job.file_name);
                self.finish(&file_name, result);
                true
            }
        }
    }

    /// Turn a finished compile into diagnostics, timings and a screen.
    fn finish(&self, file_name: &str, result: std::io::Result<compile::Compiled>) {
        let compiled = match result {
            Ok(compiled) => compiled,
            Err(error) => {
                self.output
                    .set(Rc::new(vec![format!("could not run rustc: {error}")]));
                self.set_preview(PreviewState::Failed);
                return;
            }
        };

        let mut log = compiled.log.clone();
        self.diagnostics.set(Rc::new(compiled.diagnostics.clone()));
        self.rustc_json.set(Rc::new(compiled.json.clone()));

        let Some(library) = compiled.library.as_ref() else {
            log.push("keeping the previous screen; the buffer has errors".into());
            self.output.set(Rc::new(log));
            self.set_preview(PreviewState::Failed);
            // **Go to the first error.**
            //
            // A failed render used to light up four surfaces — the status bar,
            // the activity badge, the Problems count, the minimap mark — and
            // leave the editor exactly where it was, which for a mistake made
            // thirty lines further down is a window insisting something is
            // wrong while showing code that is fine. Every compiler people
            // arrive from opens on the first error.
            //
            // Only when it is in the buffer on screen. A render compiles the
            // *active* file, so a diagnostic naming another one is a knock-on
            // error, and jumping the user into a file they did not ask to see
            // is worse than not moving at all.
            if let Some(first) = self
                .diagnostics
                .peek()
                .iter()
                .find(|diagnostic| diagnostic.severity == Severity::Error)
            {
                if self
                    .active()
                    .is_some_and(|buffer| buffer.name == first.file)
                {
                    self.jump_to(first.line, first.column);
                }
            }
            return;
        };

        // **The toolchain's libstd, mapped before the preview asks for it.**
        // A studio started outside cargo — a desktop entry, a double click —
        // gives the loader nowhere to find `libstd-<hash>.so`, and the Render
        // that compiled clean dies at `dlopen`. Cheap (once per process) and
        // harmless when the studio already links libstd itself, which is the
        // other half of the fix and lives in `.cargo/config.toml`. See
        // `loaded::prime_libstd`.
        if let Ok(toolchain) = self.toolchain.as_ref() {
            toolchain.prime_libstd();
        }

        let loaded = std::time::Instant::now();
        match Preview::load(library) {
            Ok(preview) => {
                let load = loaded.elapsed();
                log.push(format!("loaded {}", library.display()));
                self.output.set(Rc::new(log));
                self.timings.set(Rc::new(vec![
                    (
                        "rustc".into(),
                        format!("{:.2}s", compiled.duration.as_secs_f32()),
                    ),
                    (
                        "dlopen + entry".into(),
                        format!("{:.1}ms", load.as_secs_f32() * 1000.0),
                    ),
                    (
                        "diagnostics".into(),
                        format!("{}", compiled.diagnostics.len()),
                    ),
                ]));
                // **Kept by path as well as by "the current one".** A
                // `mount "screens/card_grid.rs"` in `live.rs` places the screen
                // that file last rendered, so every Render quietly updates the
                // flow the live preview draws. See `livedoc::LiveItem::Mount`.
                if let Some(path) = self.active().and_then(|buffer| buffer.path.clone()) {
                    self.live_mounts.borrow_mut().insert(path, preview.clone());
                }
                self.preview_screen.set(Some(preview));
                self.set_preview(PreviewState::Rendered);
                // The new tree is built during *this* frame; the state goes
                // back into it on the next one. See `restore_preview_state`.
                self.preview_restore_due.set(true);
                self.dirty.set(false);
                self.rendered_at.set(Some(file_name.to_string()));
                // The one number the status bar used to make up. It is the
                // cost of the render that just happened, and until one has
                // happened the cell is absent rather than invented.
                self.last_render.set(Some(compiled.duration));
                // The cost of the never-unload rule, once it is worth saying.
                // Silent for the first several hours of a session — see
                // `loaded::budget_warning`, and the module header above it for
                // why this number was never being read.
                if let Some(warning) = crate::loaded::budget_warning() {
                    self.notify(&warning);
                }
            }
            Err(crate::loaded::LoadError::AbiMismatch { host, guest }) => {
                // Not the buffer's fault, and not the end of the attempt: the
                // studio picked the wrong one of several vieww rlibs. Advance
                // and try the next, which is how it finds the right one without
                // anybody being told to pass a flag.
                let next = self.candidate.get() + 1;
                // **No candidate walk when the project chose the rlib.**
                //
                // A buffer that links the project's library has exactly one
                // usable `vieww` — the project's own, forced by
                // `Toolchain::with_vieww` — so advancing the candidate compiles
                // the same thing again and mismatches again, once per rlib in
                // the directory, before arriving at the refusal it could have
                // reached immediately.
                let forced = self
                    .active()
                    .as_ref()
                    .and_then(|buffer| self.project_lib(&buffer.value.text))
                    .and_then(|project| project.vieww_rlib)
                    .is_some();
                let remaining = !forced
                    && self
                        .toolchain
                        .as_ref()
                        .as_ref()
                        .ok()
                        .is_some_and(|t| next < t.candidates.len());

                log.push(format!(
                    "vieww ABI mismatch (host {host:#018x}, preview {guest:#018x})"
                ));
                if remaining {
                    self.candidate.set(next);
                    log.push(format!("retrying against candidate {next}"));
                    self.output.set(Rc::new(log));
                    // Straight back round. The job slot is already clear — the
                    // poll took it — so this starts a fresh compile.
                    self.render();
                    return;
                }

                log.push(crate::loaded::LoadError::AbiMismatch { host, guest }.to_string());
                log.push(
                    "Build and Run has no ABI constraint and works on this project.".to_string(),
                );
                self.output.set(Rc::new(log));
                // Every candidate rlib has been tried. This is the refusal, not
                // a failure: see `PreviewState::AbiRefused`.
                self.set_preview(PreviewState::AbiRefused);
            }
            Err(error) => {
                log.push(error.to_string());
                self.output.set(Rc::new(log));
                self.diagnostics.update(|diagnostics| {
                    let mut next = (**diagnostics).clone();
                    next.push(Diagnostic {
                        file: file_name.to_string(),
                        severity: Severity::Error,
                        code: "load".into(),
                        message: error.to_string(),
                        help: None,
                        line: 1,
                        column: 1,
                        end_line: 1,
                        end_column: 1,
                    });
                    *diagnostics = Rc::new(next);
                });
                self.set_preview(PreviewState::Failed);
            }
        }
    }

    /// Switch to the buffer called `name`, if it is open.
    pub fn open_named(&self, name: &str) {
        if let Some(index) = self.buffers.get().iter().position(|b| b.name == name) {
            self.focus_buffer(index);
        }
    }

    /// Put the caret at `line`:`column` of the active buffer.
    ///
    /// What a click in the Problems panel does. Counted in `char`s, like
    /// everything else that talks about columns here.
    pub fn jump_to(&self, line: u32, column: u32) {
        let Some(buffer) = self.active() else {
            return;
        };

        let mut offset = 0usize;
        for (index, text) in buffer.value.text.split('\n').enumerate() {
            let number = index as u32 + 1;
            if number == line {
                let column = (column.saturating_sub(1)) as usize;
                let within = text
                    .char_indices()
                    .nth(column)
                    .map_or(text.len(), |(byte, _)| byte);
                offset += within;
                break;
            }
            offset += text.len() + 1;
        }

        let mut value = buffer.value.clone();
        value.selection = vieww_foundation::TextSelection::collapsed(offset.min(value.text.len()));
        self.edit(value);
        self.reveal_line(line);
    }

    /// Scroll the code pane the least distance that brings `line` into view.
    ///
    /// # Why this was missing, and what it cost
    ///
    /// `jump_to` moved the caret and nothing else, so clicking a problem on
    /// line 63 put the caret on line 63 and left the reader looking at line 1 —
    /// the panel appearing to do nothing at all. The fix needed a framework
    /// change: `ScrollController` could be dragged and flung but not *told
    /// where to go*, so there was no way for an application to reveal
    /// anything. `ScrollController::reveal` is that, and it is general — a
    /// find-next, a focus change and a list scrolling to a selection are the
    /// same operation.
    ///
    /// One source line is one row of `ui::editor::code_line`, which holds only
    /// because the code pane does not wrap. If it ever does, this becomes a
    /// question only the shaped paragraph can answer.
    ///
    /// `peek` rather than `get` on the font size: this runs from a command
    /// handler, not from a build, and subscribing a scroll to the font size
    /// would be a dependency edge that means nothing.
    /// Set the active buffer *and* scroll the tab strip so its tab is visible.
    ///
    /// # Why both, always, through one call
    ///
    /// Making the strip scrollable stopped the tabs painting over the toolbar
    /// and left a subtler version of the same bug: with nine files open,
    /// `Ctrl+]` walked the selection off the right-hand end and the strip sat
    /// still. The user had switched file, the editor had changed under them,
    /// and the one control that says which file they are in showed no change at
    /// all. A scrollable list with a selection has to follow its selection.
    pub fn focus_buffer(&self, index: usize) {
        self.active_buffer.set(index);
        self.reveal_tab(index);
    }

    /// Bring one tab into the strip's viewport, by the shortest scroll.
    ///
    /// The offsets are estimates — see `crate::ui::editor::tab_extent` — and
    /// the margin is one tab's worth of chrome, so the revealed tab lands
    /// clear of the edge rather than flush against it.
    pub fn reveal_tab(&self, index: usize) {
        let buffers = self.buffers.peek();
        let Some(buffer) = buffers.get(index) else {
            return;
        };
        let start: f32 = buffers
            .iter()
            .take(index)
            .map(|buffer| crate::ui::editor::tab_extent(&buffer.name))
            .sum();
        self.tabs_scroll
            .reveal(start, crate::ui::editor::tab_extent(&buffer.name), 24.0);
    }

    /// Reveal whichever tab is active. Safe to call every time the strip is
    /// laid out: [`ScrollController::reveal`] is a no-op when the tab is
    /// already inside the viewport.
    pub fn reveal_active_tab(&self) {
        self.reveal_tab(self.active_buffer.peek());
    }

    pub fn reveal_line(&self, line: u32) {
        let line_box = crate::ui::editor::code_line(self.font_size.peek());
        let top = f32::from(u16::try_from(line.saturating_sub(1)).unwrap_or(u16::MAX)) * line_box;
        // Three lines of context either side, so a revealed line lands with
        // something above it rather than flush against the edge.
        self.editor_scroll.reveal(top, line_box, line_box * 3.0);
    }

    /// The scroll controller for one bottom-panel tab.
    #[must_use]
    pub fn panel_scroll(&self, tab: PanelTab) -> &ScrollController {
        &self.panel_scroll[tab.index()]
    }

    /// The scroll position for one sidebar view.
    #[must_use]
    pub fn sidebar_scroll(&self, view: View) -> &ScrollController {
        &self.sidebar_scroll[view.index()]
    }

    /// `true` if `other` is a handle to this same studio.
    ///
    /// `Studio` is `Clone` and every clone addresses the same signals, so this
    /// is the identity test a widget's
    /// [`same_configuration`](vieww_widget::Widget::same_configuration) needs:
    /// two handles that compare equal here are interchangeable in every way
    /// that a build can observe.
    #[must_use]
    pub fn same_studio(&self, other: &Self) -> bool {
        self.buffers.runtime().ptr_eq(other.buffers.runtime())
            && Rc::ptr_eq(&self.services, &other.services)
    }

    /// The buffer the editor is showing, **text and all**.
    ///
    /// Subscribes the calling element to [`buffers`](Self::buffers), so it
    /// rebuilds on every keystroke. That is right for the editor field, the
    /// minimap and the file statistics, and wrong for everything else — reach
    /// for [`active_tab`](Self::active_tab) unless the text is what you came
    /// for. The `omnibar_hint` in the title bar asked for a *file name* here
    /// and rebuilt the whole title bar on every character typed as a result.
    #[must_use]
    pub fn active(&self) -> Option<Buffer> {
        let buffers = self.buffers.get();
        buffers.get(self.active_buffer.get()).cloned()
    }

    /// The active file's name, path, dirty flag, language, encoding, endings
    /// and read-only state — everything except its text.
    ///
    /// Subscribes to [`tabs`](Self::tabs), which typing does not change. This
    /// is what a status cell, a tab label, a breadcrumb or a Save button wants.
    #[must_use]
    pub fn active_tab(&self) -> Option<BufferTab> {
        let tabs = self.tabs.get();
        tabs.get(self.active_buffer.get()).cloned()
    }

    /// Replace the active buffer's text, and update everything derived from it.
    ///
    /// One entry point for every edit — typing, a paste, a click that only
    /// moves the caret — so `dirty`, the caret readout and the tab's dot can
    /// never be a keystroke behind the text.
    pub fn edit(&self, value: vieww_foundation::TextEditingValue) {
        // N6: if this change was one character being typed, the comforts get
        // to replace it — auto-indent after Return, the closing half of a
        // bracket, a quote wrapped round a selection. `after_typing` answers
        // `None` for everything else, including its own output, so this cannot
        // recurse and does not touch a paste or a delete. See `edit_ops`.
        let value = self.with_comforts(value);
        let index = self.active_buffer.get();
        let mut buffers = (*self.buffers.get()).clone();
        let Some(buffer) = buffers.get_mut(index) else {
            return;
        };

        // A keystroke or a caret move that lands in the blink's off phase looks
        // like it was dropped. Every editor restarts the blink here.
        self.blink.borrow_mut().wake();

        let changed = buffer.value.text != value.text;
        // Recorded *before* the value is replaced, because the history's
        // coalescing rule compares where the buffer is with where it is going.
        // A caret-only move records nothing but does keep the newest entry's
        // caret in step — see `history::History::record`.
        buffer.history.record(&value);
        buffer.value = value;
        if changed {
            buffer.dirty = true;
        }
        self.caret.set(buffer.caret());
        self.put_buffers(buffers);

        if changed {
            // An edit invalidates the render, exactly as a platform switch
            // does. Moving the caret alone does not.
            self.mark_dirty();
            // And the two things outside the editor that care about text: the
            // auto-render debounce and the language server. Both keyed on
            // `changed` for the same reason `mark_dirty` is — a caret move is
            // not an edit.
            self.buffer_changed();
        }
    }
}

/// The command a checklist line offers to run, if it offers one at all.
///
/// `None` for the lines that are instructions rather than commands — a path
/// through Xcode's menus is not something a shell can be handed, and an Install
/// button beside one would be a button that cannot do what it says.
#[must_use]
pub fn runnable(line: &str) -> Option<String> {
    line.contains('`').then(|| strip_backticks(line))
}

/// The command inside a checklist line, without the prose around it.
///
/// `toolchains.rs` writes the install lines to be *read* — "Android Studio, or
/// `sdkmanager --install \"platform-tools\"`; then set ANDROID_HOME" — so the
/// runnable part is whatever sits between backticks, and a line with none is
/// taken whole.
#[must_use]
pub fn strip_backticks(line: &str) -> String {
    // **Every span, not the outermost pair.** Several requirements offer one
    // command per package manager — ``\`apt install mingw-w64\`, \`dnf install
    // mingw64-gcc\`, or \`brew install mingw-w64\``` — and a first-to-last cut
    // through that produces one long string containing all three and the prose
    // between them, which is what the Install button first offered to run.
    let spans: Vec<&str> = line
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|span| !span.is_empty())
        .collect();

    if spans.is_empty() {
        return line.trim().to_owned();
    }

    // Where there is a choice, the one whose program is actually on this
    // machine: offering `apt install` on Fedora is a button that fails in a way
    // the user then has to diagnose.
    if let Some(present) = spans.iter().find(|span| {
        span.split_whitespace()
            .next()
            .is_some_and(|program| crate::setup::which(program).is_some())
    }) {
        return (*present).to_owned();
    }

    // **Nothing on the line is installed. Fall back by host, not by order.**
    //
    // The old fallback was `spans[0]`, which is whichever package manager the
    // requirement happened to name first — and every one of these lines was
    // written Linux-first. On Windows with no Java, the JDK row's Install button
    // therefore offered `apt install default-jdk`: a command from another
    // operating system, on a machine that has never had `apt`. The studio runs
    // on three hosts and the line has an option for each; choosing among them is
    // this function's job, not the reader's.
    //
    // Still `spans[0]` when the line has nothing for this host — a Homebrew-only
    // instruction on Linux stays runnable, and the user is about to install
    // something anyway. Better a command they can read and adapt than none.
    spans
        .iter()
        .find(|span| {
            span.split_whitespace()
                .next()
                .is_some_and(host_package_manager)
        })
        .map_or_else(|| spans[0].to_owned(), |span| (*span).to_owned())
}

/// Whether `program` is a package manager that belongs to the host this studio
/// was built for.
///
/// Used only when nothing on an install line is present, to decide which of
/// several offered commands is worth showing. `cargo`, `rustup` and `sdkmanager`
/// are not here: they are cross-platform and are found by the check above when
/// they exist at all.
fn host_package_manager(program: &str) -> bool {
    if cfg!(target_os = "windows") {
        matches!(program, "winget" | "choco" | "scoop")
    } else if cfg!(target_os = "macos") {
        matches!(program, "brew" | "port")
    } else {
        matches!(
            program,
            "apt" | "apt-get" | "dnf" | "yum" | "pacman" | "zypper" | "snap"
        )
    }
}

#[cfg(test)]
mod decoration_tests {
    use super::*;
    use vieww_element::Runtime;
    use vieww_foundation::{TextDecorationShape, TextEditingValue};

    fn studio_with(text: &str, caret: usize) -> (Runtime, Studio) {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let mut value = TextEditingValue::new(text);
        value.selection = vieww_foundation::TextSelection::collapsed(caret);
        studio.edit(value);
        (runtime, studio)
    }

    fn shapes(marks: &[TextDecoration], shape: TextDecorationShape) -> Vec<TextRange> {
        marks
            .iter()
            .filter(|m| m.shape == shape)
            .map(|m| m.range)
            .collect()
    }

    #[test]
    fn a_span_resolves_to_the_bytes_it_covers() {
        let text = "let x = 1;\nlet yy = 2;\n";
        let diagnostic = Diagnostic {
            file: "a.rs".into(),
            severity: Severity::Error,
            code: "E0425".into(),
            message: String::new(),
            help: None,
            line: 2,
            column: 5,
            end_line: 2,
            end_column: 7,
        };
        let range = diagnostic.span_in(text).expect("a two-column span");
        assert_eq!(&text[range.start..range.end], "yy");
    }

    #[test]
    fn a_point_span_covers_nothing_rather_than_the_line() {
        // rustc reports plenty of diagnostics whose end equals their start.
        // Marking to the end of the line would be a confident claim about text
        // the compiler said nothing about.
        let diagnostic = Diagnostic {
            file: "a.rs".into(),
            severity: Severity::Warning,
            code: "w".into(),
            message: String::new(),
            help: None,
            line: 1,
            column: 3,
            end_line: 1,
            end_column: 3,
        };
        assert_eq!(diagnostic.span_in("let x = 1;"), None);
    }

    #[test]
    fn a_column_is_counted_in_characters_not_bytes() {
        // rustc counts columns in characters. A byte count puts the mark a
        // third of the way through the next word on any line with an em dash
        // or an accent in it, and every such line is a comment or a string —
        // which is exactly where a wrong mark is least likely to be noticed.
        let text = "// naïve — x\nlet x = 1;";
        let diagnostic = Diagnostic {
            file: "a.rs".into(),
            severity: Severity::Error,
            code: "e".into(),
            message: String::new(),
            help: None,
            line: 1,
            column: 12,
            end_line: 1,
            end_column: 13,
        };
        let range = diagnostic.span_in(text).expect("one character");
        assert_eq!(&text[range.start..range.end], "x");
    }

    #[test]
    fn the_bracket_at_the_caret_and_its_partner_are_both_boxed() {
        let (_runtime, studio) = studio_with("fn main() { let x = 1; }", 11);
        let marks = studio.editor_decorations(crate::theme::StudioTheme::dark());
        let boxes = shapes(&marks, TextDecorationShape::Box);
        assert_eq!(boxes.len(), 2, "a pair is two marks, not one");
        assert_eq!(boxes[0].end - boxes[0].start, 1);
        assert_ne!(boxes[0].start, boxes[1].start);
    }

    #[test]
    fn an_unbalanced_file_is_the_normal_state_and_marks_nothing() {
        // A file being typed into is unbalanced most of the time. Marking the
        // opening brace of every unfinished block would be a permanent box.
        let (_runtime, studio) = studio_with("fn main() {", 11);
        let marks = studio.editor_decorations(crate::theme::StudioTheme::dark());
        assert!(shapes(&marks, TextDecorationShape::Box).is_empty());
    }

    #[test]
    fn other_occurrences_of_the_word_are_underlined_and_the_caret_s_own_is_not() {
        let text = "let count = 1;\nlet other = count + count;";
        // Caret inside the first `count`.
        let (_runtime, studio) = studio_with(text, 6);
        let marks = studio.editor_decorations(crate::theme::StudioTheme::dark());
        let lines = shapes(&marks, TextDecorationShape::Underline);
        assert_eq!(
            lines.len(),
            2,
            "the two later uses, not the one at the caret"
        );
        for range in lines {
            assert_eq!(&text[range.start..range.end], "count");
            assert!(range.start > 10, "the caret's own occurrence was marked");
        }
    }

    #[test]
    fn a_substring_of_a_longer_identifier_is_not_an_occurrence() {
        // The reason the match is whole-word. Without it a variable called
        // `xy` underlines every word containing those letters, which is noise.
        let text = "let xy = 1;\nlet xys = [xy];\nlet maxy = 2;";
        let (_runtime, studio) = studio_with(text, 5);
        let marks = studio.editor_decorations(crate::theme::StudioTheme::dark());
        let lines = shapes(&marks, TextDecorationShape::Underline);
        assert_eq!(
            lines.len(),
            1,
            "only the bare `xy` inside the brackets, not `xys` and not `maxy`"
        );
        assert_eq!(&text[lines[0].start..lines[0].end], "xy");
    }

    #[test]
    fn a_caret_in_whitespace_underlines_nothing() {
        let (_runtime, studio) = studio_with("let x = 1;\n\nlet x = 2;", 11);
        let marks = studio.editor_decorations(crate::theme::StudioTheme::dark());
        assert!(
            shapes(&marks, TextDecorationShape::Underline).is_empty(),
            "arrowing through indentation must not flicker with underlines"
        );
    }

    #[test]
    fn a_one_character_word_is_left_alone() {
        // Two characters is the floor. A single letter matches too often to
        // carry information, and it is the commonest kind of loop variable.
        let (_runtime, studio) = studio_with("let i = 1; let j = i;", 5);
        let marks = studio.editor_decorations(crate::theme::StudioTheme::dark());
        assert!(shapes(&marks, TextDecorationShape::Underline).is_empty());
    }
    // ===================== indent guides =================================

    /// The columns a guide was drawn at on each line, keyed by line number.
    ///
    /// Asserted on columns rather than on byte offsets because a column is
    /// what the feature is about: a rule at the third level of indentation is
    /// right whatever the bytes on that line happen to be.
    fn guide_columns(text: &str, tab_width: usize) -> Vec<(usize, Vec<usize>)> {
        let marks = indent_guides(text, tab_width, vieww_foundation::Color::WHITE);
        let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
        for mark in marks {
            let before = &text[..mark.range.start];
            let line = before.matches('\n').count();
            let line_start = before.rfind('\n').map_or(0, |at| at + 1);
            let column = text[line_start..mark.range.start].chars().count();
            match out.iter_mut().find(|(n, _)| *n == line) {
                Some((_, columns)) => columns.push(column),
                None => out.push((line, vec![column])),
            }
        }
        out
    }

    #[test]
    fn a_guide_marks_every_level_inside_a_line_and_not_its_own() {
        // Eight columns of indent at a tab width of four is two levels
        // enclosing this line — and the guide at column 8 would sit on the
        // line's first character, which is a rule through the code.
        let text = "fn f() {\n    if x {\n        body();\n    }\n}";
        let columns = guide_columns(text, 4);
        assert_eq!(columns, vec![(2, vec![4])]);
    }

    #[test]
    fn deeper_nesting_gets_one_guide_per_level() {
        let text = "a\n            deep();\n";
        assert_eq!(guide_columns(text, 4), vec![(1, vec![4, 8])]);
    }

    #[test]
    fn the_tab_width_decides_the_levels() {
        let text = "a\n        deep();\n";
        assert_eq!(guide_columns(text, 4), vec![(1, vec![4])]);
        // The same line at a tab width of two is four levels deep, so three
        // guides rather than one. A file indented two-wide read against a
        // four-wide setting is the bug this proves is a setting.
        assert_eq!(guide_columns(text, 2), vec![(1, vec![2, 4, 6])]);
    }

    #[test]
    fn a_tab_character_counts_to_the_next_stop() {
        // One tab and one four-space run are the same indent, so the guides
        // must agree. They do not agree if a tab is counted as one column.
        let spaces = guide_columns("a\n        x;\n", 4);
        let tabs = guide_columns("a\n\t\tx;\n", 4);
        assert_eq!(spaces.len(), 1);
        assert_eq!(tabs.len(), 1);
        assert_eq!(spaces[0].1.len(), tabs[0].1.len());
    }

    #[test]
    fn a_blank_line_inside_a_block_keeps_its_guides() {
        // The gap between two statements in the same block, which is where a
        // run of guides that stopped dead would be most obviously wrong.
        let text = "fn f() {\n    if x {\n        one();\n\n        two();\n    }\n}";
        let columns = guide_columns(text, 4);
        let blank = columns.iter().find(|(line, _)| *line == 3);
        assert!(
            blank.is_none(),
            "a blank line has no character to mark, so it draws nothing itself"
        );
        // What matters is that the lines either side are unchanged.
        assert_eq!(
            columns.iter().filter(|(_, c)| c == &vec![4]).count(),
            2,
            "both statements keep their single enclosing guide"
        );
    }

    #[test]
    fn a_blank_line_that_closes_a_block_takes_the_shallower_side() {
        // Above is four columns deep, below is zero. The smaller wins, which
        // is what stops a guide hanging in the air after a block ends.
        let depths = [Some(4), None, Some(0)];
        assert_eq!(blank_line_depth(&depths, 1), 0);
        // Inside a block, both sides agree and the guides continue.
        let depths = [Some(4), None, Some(4)];
        assert_eq!(blank_line_depth(&depths, 1), 4);
        // Nothing on either side is depth zero rather than a panic.
        assert_eq!(blank_line_depth(&[None], 0), 0);
    }

    #[test]
    fn a_column_inside_a_tab_has_nothing_to_mark() {
        // A tab at a width of four spans columns 0-3. Column 2 is inside it:
        // there is no glyph starting there, and a rule on the tab's own left
        // edge would be a whole level out.
        assert_eq!(byte_at_column("\tx", 2, 4), None);
        assert_eq!(byte_at_column("\tx", 4, 4), Some(1));
        assert_eq!(byte_at_column("ab", 5, 4), None);
    }

    #[test]
    fn guides_are_off_when_the_setting_is_off() {
        let (_runtime, studio) = studio_with("fn f() {\n    if x {\n        y();\n", 0);
        let with = studio.editor_decorations(crate::theme::StudioTheme::dark());
        assert!(!shapes(&with, TextDecorationShape::Guide).is_empty());
        studio.indent_guides.set(false);
        let without = studio.editor_decorations(crate::theme::StudioTheme::dark());
        assert!(shapes(&without, TextDecorationShape::Guide).is_empty());
    }

    #[test]
    fn a_guide_names_exactly_one_grapheme() {
        // The shape reads a position and ignores the extent, and an empty
        // range draws nothing — so a guide that named a span or named nothing
        // would be a guide that either misled a future reader or vanished.
        let marks = indent_guides("a\n        x;\n", 4, vieww_foundation::Color::WHITE);
        assert!(!marks.is_empty());
        for mark in marks {
            assert_eq!(mark.range.end - mark.range.start, 1);
        }
    }
    // ===================== format on save ================================

    #[test]
    fn a_format_puts_the_caret_back_on_the_line_it_was_on() {
        // A formatter moves every byte after its first change, so the *offset*
        // is meaningless afterwards. The line is not.
        let before = "fn main(){\nlet x=1;\nprintln!(\"{x}\");\n}\n";
        let caret = before.find("println").expect("in the fixture");
        assert_eq!(line_of(before, caret), 2);

        let after = "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n";
        let value = reformatted_value(after, 2);
        assert_eq!(value.text, after);
        assert!(
            after[value.selection.cursor().offset..].starts_with("    println"),
            "the caret is at the start of the line it was on"
        );
    }

    #[test]
    fn a_caret_past_the_end_of_the_formatted_text_is_clamped() {
        // rustfmt can delete lines — a file that was only blank lines becomes
        // one line — and a caret on a line that no longer exists must land
        // somewhere real rather than panic on a slice.
        let value = reformatted_value("fn main() {}\n", 40);
        assert_eq!(value.selection.cursor().offset, "fn main() {}\n".len());
    }

    #[test]
    fn line_of_counts_from_zero_and_survives_a_bad_offset() {
        assert_eq!(line_of("a\nb\nc", 0), 0);
        assert_eq!(line_of("a\nb\nc", 2), 1);
        assert_eq!(line_of("a\nb\nc", 4), 2);
        // Clamped rather than panicking: the caret and the text are two
        // signals and a frame can see them one step apart.
        assert_eq!(line_of("a\nb\nc", 999), 2);
    }

    #[test]
    fn formatting_a_file_that_is_not_rust_says_so_rather_than_running_rustfmt() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let mut buffers = (*studio.buffers.get()).clone();
        buffers[0].name = "Cargo.toml".to_string();
        studio.put_buffers(buffers);

        studio.format_active();
        assert_eq!(
            studio.jobs.borrow().jobs().len(),
            0,
            "no process was started"
        );
        assert!(
            studio
                .output
                .get()
                .iter()
                .any(|line| line.contains("not a .rs file")),
            "and the reason is in the Output panel: {:?}",
            studio.output.get()
        );
    }

    #[test]
    fn the_tasks_panel_is_a_tab_like_any_other() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        studio.run(Command::ShowTasks);
        assert_eq!(studio.panel_tab.get(), PanelTab::Tasks);
        assert!(studio.panel_open.get());
    }

    #[test]
    fn clearing_and_cancelling_tasks_are_offered_only_with_a_queue() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        assert!(!studio.can_run(Command::ClearFinishedTasks));
        assert!(!studio.can_run(Command::CancelAllTasks));
        studio.jobs.borrow_mut().adopt("Build", "cargo build");
        assert!(studio.can_run(Command::ClearFinishedTasks));
        assert!(studio.can_run(Command::CancelAllTasks));
    }
    // ===================== folding =======================================

    fn folded_studio(text: &str, caret: usize) -> (Runtime, Studio) {
        let (runtime, studio) = studio_with(text, caret);
        (runtime, studio)
    }

    const NESTED: &str = "fn main() {\n    if x {\n        one();\n    }\n}\nstruct S;";

    #[test]
    fn folding_at_the_caret_takes_the_innermost_region() {
        // Standing inside a nested `if`, "fold" means the `if` — not the
        // function around it. Folding the outer one is one more press.
        let caret = NESTED.find("one()").expect("in the fixture");
        let (_runtime, studio) = folded_studio(NESTED, caret);
        studio.toggle_fold();
        assert_eq!(
            studio.folds.get().iter().copied().collect::<Vec<_>>(),
            vec![1],
            "line 2 is the `if`"
        );
        assert!(studio.folded_view().text.contains("if x {"));
        assert!(!studio.folded_view().text.contains("one()"));
    }

    #[test]
    fn folding_the_same_place_twice_unfolds_it() {
        let (_runtime, studio) = folded_studio(NESTED, 0);
        studio.toggle_fold();
        assert!(!studio.folds.get().is_empty());
        studio.toggle_fold();
        assert!(
            studio.folds.get().is_empty(),
            "the caret is on a collapsed header, so the second press opens it"
        );
    }

    #[test]
    fn an_edit_made_while_folded_reaches_the_real_buffer_intact() {
        // The property the whole feature rests on: nothing hidden is lost.
        let (_runtime, studio) = folded_studio(NESTED, 0);
        studio.fold_all(true);
        let view = studio.folded_view();
        assert!(!view.is_complete(NESTED));

        let edited = view.text.replace("struct S;", "struct Renamed;");
        let mut value = vieww_foundation::TextEditingValue::new(edited);
        value.selection = vieww_foundation::TextSelection::collapsed(0);
        studio.edit_projected(value);

        let text = studio.active().expect("a buffer").value.text;
        assert_eq!(text, NESTED.replace("struct S;", "struct Renamed;"));
        assert!(
            text.contains("one();"),
            "the folded body survived the edit: {text:?}"
        );
    }

    #[test]
    fn a_caret_move_while_folded_is_not_an_edit() {
        let (_runtime, studio) = folded_studio(NESTED, 0);
        studio.toggle_fold();
        let before = studio.active().expect("a buffer").value.text;
        let view = studio.folded_view();

        let mut value = vieww_foundation::TextEditingValue::new(view.text.clone());
        value.selection = vieww_foundation::TextSelection::collapsed(view.text.len());
        studio.edit_projected(value);

        assert_eq!(studio.active().expect("a buffer").value.text, before);
        assert!(!studio.folds.get().is_empty(), "and the fold is still on");
    }

    #[test]
    fn unfolding_everything_restores_the_whole_file() {
        let (_runtime, studio) = folded_studio(NESTED, 0);
        studio.fold_all(true);
        studio.fold_all(false);
        assert_eq!(studio.folded_view().text, NESTED);
    }

    #[test]
    fn the_fold_commands_are_offered_only_when_there_is_something_to_fold() {
        let (_runtime, studio) = folded_studio("let x = 1;", 0);
        assert!(
            !studio.can_run(Command::ToggleFold),
            "a flat file folds nothing"
        );
        assert!(!studio.can_run(Command::UnfoldAll));

        let (_runtime, studio) = folded_studio(NESTED, 0);
        assert!(studio.can_run(Command::ToggleFold));
        assert!(!studio.can_run(Command::UnfoldAll), "nothing is folded yet");
        studio.fold_all(true);
        assert!(studio.can_run(Command::UnfoldAll));
    }
    // ===================== source control ================================

    #[test]
    fn source_control_is_offered_only_where_there_is_a_repository() {
        // A folder that is not a repository is a normal thing to have open,
        // and a Commit button over it would be a button that cannot work.
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        assert!(!studio.is_repository(), "a scratch workspace has no root");
        assert!(!studio.can_run(Command::ShowSource));
        assert!(!studio.can_run(Command::RefreshGit));
        assert!(!studio.can_run(Command::Commit));
    }

    #[test]
    fn committing_with_nothing_staged_says_so_rather_than_running_git() {
        // git's own message for this is four lines of advice about `git add`,
        // printed into a panel, after a process launch.
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        studio.commit_message.set("a message".into());
        studio.commit();
        assert_eq!(studio.jobs.borrow().jobs().len(), 0);
    }

    #[test]
    fn the_branch_cell_reads_what_git_reported() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        studio.git.set(Rc::new(crate::git::parse_status(
            "## main...origin/main [ahead 3]\0 M src/theme.rs\0",
        )));
        assert_eq!(studio.git.get().describe(), "main \u{2191}3");
        assert_eq!(studio.git.get().entries.len(), 1);
        assert_eq!(
            studio.git.get().staged(),
            0,
            "a worktree change is not staged"
        );
    }

    #[test]
    fn a_git_letter_is_looked_up_by_the_path_relative_to_the_root() {
        // git reports `src/theme.rs`; the tree holds an absolute path. A
        // lookup that compared them directly would never match, and every
        // letter in the explorer would be missing.
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        studio.root.set(Some(std::path::PathBuf::from("/repo")));
        studio.git.set(Rc::new(crate::git::parse_status(
            "## main\0 M src/theme.rs\0",
        )));
        assert_eq!(
            studio.git_change(Path::new("/repo/src/theme.rs")),
            Some(crate::git::Change::Modified)
        );
        assert_eq!(
            studio.git_change(Path::new("/elsewhere/src/theme.rs")),
            None
        );
    }
    // ===================== multiple carets ===============================

    const THREE: &str = "one\ntwo\nthree";

    #[test]
    fn add_cursor_below_grows_a_column_downwards() {
        // Five presses make five carets in a column, which is the whole point
        // — a version that grew from the *primary* every time would make two
        // carets and then fight over one line.
        let (_runtime, studio) = studio_with(THREE, 1);
        studio.add_caret_line(true);
        studio.add_caret_line(true);
        let carets = studio.active().expect("a buffer").value.sorted_carets();
        assert_eq!(
            carets
                .iter()
                .map(|caret| line_of(THREE, caret.start()))
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn add_cursor_below_at_the_last_line_adds_nothing() {
        // Rather than adding a caret at the same place, which the model would
        // merge away — leaving a key that appears to do nothing at random.
        let (_runtime, studio) = studio_with(THREE, THREE.len());
        studio.add_caret_line(true);
        assert_eq!(studio.caret_count(), 1);
    }

    #[test]
    fn a_column_clamps_to_a_shorter_line() {
        // Column 4 of "one" does not exist. The end of it does.
        let text = "longer line\none";
        let (_runtime, studio) = studio_with(text, 8);
        studio.add_caret_line(true);
        let carets = studio.active().expect("a buffer").value.sorted_carets();
        assert_eq!(carets.len(), 2);
        assert_eq!(carets[1].start(), text.len(), "clamped to the end of `one`");
    }

    #[test]
    fn the_first_press_of_add_at_next_occurrence_selects_the_word() {
        // Which is what gives the second press something to look for.
        let text = "let total = total + 1;";
        let (_runtime, studio) = studio_with(text, 5);
        studio.add_caret_at_next_occurrence();
        let value = studio.active().expect("a buffer").value;
        assert_eq!(value.caret_count(), 1);
        assert_eq!(value.selected_text(), "total");
    }

    #[test]
    fn the_second_press_adds_the_next_occurrence() {
        let text = "let total = total + 1;";
        let (_runtime, studio) = studio_with(text, 5);
        studio.add_caret_at_next_occurrence();
        studio.add_caret_at_next_occurrence();
        let value = studio.active().expect("a buffer").value;
        assert_eq!(value.caret_count(), 2);
        for caret in value.carets() {
            assert_eq!(&text[caret.range().start..caret.range().end], "total");
        }
    }

    #[test]
    fn adding_occurrences_wraps_rather_than_stopping() {
        // A key that stops working two thirds of the way down a file, for no
        // visible reason, is worse than one that wraps.
        let text = "aa bb aa";
        let (_runtime, studio) = studio_with(text, 6);
        studio.add_caret_at_next_occurrence();
        assert_eq!(
            studio.active().expect("a buffer").value.selected_text(),
            "aa"
        );
        studio.add_caret_at_next_occurrence();
        assert_eq!(studio.caret_count(), 2, "it found the one at the start");
    }

    #[test]
    fn escape_clears_the_extra_carets_before_anything_else() {
        // With carets placed, Escape is overwhelmingly "stop doing that".
        let (_runtime, studio) = studio_with(THREE, 1);
        studio.add_caret_line(true);
        studio.find_open.set(true);
        assert_eq!(studio.caret_count(), 2);

        studio.escape();
        assert_eq!(studio.caret_count(), 1);
        assert!(studio.find_open.get(), "and the find bar is still open");

        studio.escape();
        assert!(!studio.find_open.get());
    }

    #[test]
    fn the_editor_comforts_stand_down_for_multiple_carets() {
        // They read one keystroke against one caret and rewrite the text
        // around it. Applied to several, they would indent one caret's line
        // and leave the rest.
        let (_runtime, studio) = studio_with("fn f() {", 8);
        studio.add_caret_line(false);
        let carets = studio.caret_count();
        assert!(studio.comforts.get(), "the setting is still on");

        let mut value = studio.active().expect("a buffer").value;
        value.apply(&vieww_foundation::TextIntent::Insert("x".into()));
        studio.edit(value);
        assert_eq!(
            studio.active().expect("a buffer").value.text,
            "fn f() {x",
            "plain insertion, with no auto-close after the brace"
        );
        let _ = carets;
    }

    #[test]
    fn clearing_is_offered_only_when_there_is_something_to_clear() {
        let (_runtime, studio) = studio_with(THREE, 1);
        assert!(!studio.can_run(Command::ClearCursors));
        studio.add_caret_line(true);
        assert!(studio.can_run(Command::ClearCursors));
        assert!(studio.clear_extra_carets());
        assert!(!studio.clear_extra_carets(), "and again is a no-op");
    }

    /// **A selection never changes the text.** The regression test for the
    /// worst defect in the studio's history: a click-and-drag across the buffer
    /// deleted 247 of its 420 characters, in bursts of up to 86, with no key
    /// pressed. Nothing asserted this, in either crate.
    #[test]
    fn dragging_a_selection_leaves_the_buffer_alone() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let before = studio.active().expect("the scratch buffer").value.text;
        assert!(before.len() > 100, "the fixture has to be worth destroying");

        // Every offset the drag passes through, forwards then backwards —
        // including across the newlines and the `{`, `(` and `"` characters
        // that used to reach `typed_bracket`'s surround-a-selection branch.
        let anchor = before.len() / 2;
        for extent in (0..before.len()).step_by(3).chain((0..anchor).rev()) {
            studio.select(vieww_foundation::TextSelection::new(anchor, extent));
            assert_eq!(
                studio.active().expect("still open").value.text,
                before,
                "a drag to offset {extent} changed the text"
            );
        }
        // And it really did select, rather than doing nothing safely.
        let value = studio.active().expect("still open").value;
        assert_ne!(value.selection.base, value.selection.extent);
    }

    /// The comforts read one typed character. A report that carries the text
    /// the buffer already holds carries no typed character — and used to be
    /// able to reach `typed_bracket` anyway, whenever the previous selection
    /// happened to be exactly one character long, which a drag passes through.
    #[test]
    fn the_comforts_do_not_fire_on_a_selection_only_change() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let text = "fn f() {\n    let x = (1);\n}\n".to_owned();
        let mut value = vieww_foundation::TextEditingValue::new(text.clone());
        value.selection = vieww_foundation::TextSelection::collapsed(0);
        studio.edit(value);

        // One character at a time across the whole buffer, each step leaving a
        // one-character selection behind it — the exact shape that fired.
        for at in 0..text.len().saturating_sub(1) {
            let mut value = studio.active().expect("open").value;
            value.selection = vieww_foundation::TextSelection::new(at, at + 1);
            studio.edit(value);
            assert_eq!(
                studio.active().expect("open").value.text,
                text,
                "selecting the character at {at} rewrote the buffer"
            );
        }
    }

    #[test]
    fn restoring_the_sample_is_one_undo_step() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let mut value = studio.active().expect("open").value;
        value.text = String::new();
        value.selection = vieww_foundation::TextSelection::collapsed(0);
        studio.edit(value);
        assert_eq!(studio.active().expect("open").value.text, "");

        studio.restore_sample();
        assert_eq!(
            studio.active().expect("open").value.text,
            crate::buffer::SCRATCH
        );
        assert!(studio.can_run(Command::Undo));
    }

    /// A refusal has to reach a person. Before this, the sidebar row simply did
    /// nothing and the parse error turned up one compile later.
    #[test]
    fn a_refused_snippet_says_so_and_changes_nothing() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let mut value = studio.active().expect("open").value;
        value.text = String::new();
        value.selection = vieww_foundation::TextSelection::collapsed(0);
        studio.edit(value);

        let expression = crate::edit_ops::SNIPPETS
            .iter()
            .find(|s| matches!(s.kind, crate::edit_ops::SnippetKind::Expression))
            .expect("an expression entry");
        studio.insert_snippet(expression);
        assert_eq!(studio.active().expect("open").value.text, "");
        let notice = studio.notice.peek().clone().expect("no notice was shown");
        assert!(notice.contains("Widget shell"), "{notice}");

        // And the entry that *can* go there does, on its own lines. By name
        // rather than "the first item": the library grew a group of
        // function-level entries and the first of those is now the `screen()`
        // contract, which is a different shape.
        let item = crate::edit_ops::SNIPPETS
            .iter()
            .find(|s| s.name == "Widget shell")
            .expect("an item entry");
        studio.insert_snippet(item);
        let text = studio.active().expect("open").value.text;
        assert!(text.contains("impl Widget for Screen"), "{text}");
        assert!(text.lines().map(str::len).max().unwrap_or(0) < 80, "{text}");
    }

    /// Every command in the menus is reachable, and a disabled one explains
    /// itself. `why_disabled` answers from `can_run`, so this also pins that
    /// the two cannot drift into disagreeing.
    #[test]
    fn every_disabled_command_says_why() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        for command in Command::ALL {
            let disabled = !studio.can_run(command);
            let reason = studio.why_disabled(command);
            assert_eq!(
                disabled,
                reason.is_some(),
                "{} disagrees with its own reason",
                command.title()
            );
            if let Some(reason) = reason {
                assert!(
                    !reason.is_empty() && reason.ends_with('.'),
                    "{}: {reason}",
                    command.title()
                );
            }
        }
    }

    #[test]
    fn the_three_lost_editing_commands_are_reachable() {
        for command in [Command::ToggleComment, Command::Indent, Command::Outdent] {
            assert!(
                Command::ALL.contains(&command),
                "{} is not in ALL, so no menu, palette or key can reach it",
                command.title()
            );
        }
    }

    #[test]
    fn opening_a_folder_is_a_command_with_a_chord() {
        assert!(Command::ALL.contains(&Command::OpenFolder));
        assert!(Command::OpenFolder.chord().is_some());
    }

    /// **Typing must not disturb the tab metadata.** This is the invariant the
    /// whole `tabs`/`buffers` split rests on: if a keystroke changed a
    /// `BufferTab`, `set_if_changed` would notify anyway and the tab strip, the
    /// Explorer's open-buffer list, the Save button and the status bar would be
    /// back to rebuilding on every character — the 492-element rebuild
    /// `docs/PERFORMANCE.md` describes.
    ///
    /// Pinned by *value*, because value is what decides it: a `Memo` notifies
    /// exactly when its recomputed value differs from the last one, and
    /// `signal.rs`'s own tests pin that half. So the thing to assert here is
    /// the studio's half — that a keystroke produces metadata equal to what
    /// was already published.
    ///
    /// **The oracle for the gutter's derivation.** `Studio::gutter` used to be
    /// a signal republished by a `refresh_gutter` that every write touching one
    /// of its four inputs had to remember to call, and this test existed
    /// because a missed call does not fail loudly — it leaves the caret mark on
    /// the wrong line, or a fold chevron on a line that no longer folds.
    ///
    /// It is a `Memo` now and there is nothing left to forget, but the test
    /// stays: what it actually asserts is that the *derivation* is right, and
    /// that is as easy to get wrong through a memo as through a door. It
    /// compares the published value against a from-scratch derivation after
    /// every kind of change.
    #[test]
    fn the_gutter_matches_a_fresh_derivation() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let check = |where_: &str| {
            assert_eq!(
                **studio.gutter.get(),
                studio.gutter_rows_now(),
                "the gutter is stale after {where_}"
            );
        };
        check("construction");

        let value = studio.active().expect("a buffer").value;
        studio.edit(vieww_foundation::TextEditingValue {
            text: format!("{}\nfn added() {{\n    0\n}}\n", value.text),
            ..value
        });
        check("an edit");

        studio.jump_to(2, 1);
        check("a caret move");

        studio.toggle_fold();
        check("a fold");

        studio.diagnostics.set(Rc::new(vec![Diagnostic {
            file: studio.active().expect("a buffer").name,
            severity: Severity::Error,
            code: "E0308".to_owned(),
            message: "probe".to_owned(),
            help: None,
            line: 2,
            column: 1,
            end_line: 2,
            end_column: 1,
        }]));
        check("a diagnostic");
    }

    /// The performance invariant the split exists for: a character typed inside
    /// a line changes no line number, no fold boundary, no diagnostic and not
    /// the caret's *line*, so the derived rows are equal and `set_if_changed`
    /// tells the gutter's 175 elements nothing.
    #[test]
    fn typing_inside_a_line_does_not_change_the_gutter() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        // Past the first edit, which flips `dirty` and is a real change.
        let first = studio.active().expect("a buffer").value;
        studio.edit(vieww_foundation::TextEditingValue {
            text: format!("// one{}", first.text),
            ..first
        });

        let before: Vec<GutterRow> = (*studio.gutter.get()).clone();
        let value = studio.active().expect("a buffer").value;
        studio.edit(vieww_foundation::TextEditingValue {
            // Inside the first line: no newline, so no row appears or moves.
            text: format!("x{}", value.text),
            ..value
        });

        assert_eq!(*studio.gutter.get(), before, "typing moved the gutter");
        // And a change that *does* alter the shape still gets through, or the
        // assertion above is a cache that never invalidates.
        let value = studio.active().expect("a buffer").value;
        studio.edit(vieww_foundation::TextEditingValue {
            text: format!("\n{}", value.text),
            ..value
        });
        assert_ne!(*studio.gutter.get(), before, "a new line was not noticed");
    }

    #[test]
    fn typing_does_not_change_the_tab_metadata() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        // The dirty flag *is* on a `BufferTab`, so the first edit of a clean
        // buffer legitimately republishes. Get past it, then measure.
        let first = studio.active().expect("a buffer").value;
        studio.edit(vieww_foundation::TextEditingValue {
            text: format!("{}\n// one", first.text),
            ..first
        });

        let before: Vec<BufferTab> = (*studio.tabs.get()).clone();
        let value = studio.active().expect("a buffer").value;
        studio.edit(vieww_foundation::TextEditingValue {
            text: format!("{}\n// two", value.text),
            ..value
        });

        assert_eq!(
            *studio.tabs.get(),
            before,
            "a keystroke changed the tab metadata, so every reader of it was told"
        );
        // And the text really did change, or the assertion above is vacuous.
        assert!(studio
            .active()
            .expect("a buffer")
            .value
            .text
            .contains("// two"));
    }

    /// And the other half: something that *is* metadata does change it, or the
    /// split would be a cache that never invalidates.
    #[test]
    fn renaming_a_buffer_changes_the_tab_metadata() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let before: Vec<BufferTab> = (*studio.tabs.get()).clone();
        let name_before = before.first().expect("a buffer").name.clone();

        let mut buffers = (*studio.buffers.peek()).clone();
        buffers[0].name = format!("{name_before}.renamed");
        studio.put_buffers(buffers);

        let after = studio.tabs.get();
        assert_ne!(**after, before, "a rename left the tab metadata alone");
        assert_eq!(after[0].name, format!("{name_before}.renamed"));
    }

    /// The two signals describe the same files, in the same order, always.
    /// `put_buffers` is the only writer precisely so this cannot drift.
    #[test]
    fn the_tabs_always_match_the_buffers() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        let check = |where_: &str| {
            let buffers = studio.buffers.peek();
            let tabs = studio.tabs.get();
            assert_eq!(buffers.len(), tabs.len(), "{where_}: different lengths");
            for (buffer, tab) in buffers.iter().zip(tabs.iter()) {
                assert_eq!(*tab, BufferTab::of(buffer), "{where_}: {}", buffer.name);
            }
        };
        check("at rest");

        let value = studio.active().expect("a buffer").value;
        studio.edit(vieww_foundation::TextEditingValue {
            text: format!("{}\n// typed", value.text),
            ..value
        });
        check("after an edit");

        studio.run(Command::NewFile);
        check("after New File");
    }

    #[test]
    fn the_picker_opens_navigates_and_closes() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        assert!(studio.picker.peek().is_none());

        studio.run(Command::OpenFolder);
        let open = studio
            .picker
            .peek()
            .clone()
            .expect("the picker did not open");
        assert!(open.cwd.is_absolute());

        studio.picker_up();
        studio.escape();
        assert!(studio.picker.peek().is_none(), "Escape did not close it");
    }

    /// The case the volume strip exists for: a picker that opened in `$HOME`
    /// can reach a project that lives on a different volume, without going
    /// up through `/` (which `picker_up` cannot do, because
    /// `Path::parent("/").is_none()`).
    ///
    /// `picker_to` is the wire a volume chip uses, so this is also the test
    /// that a chip click reaches a project — and the picker it lands on still
    /// offers every other volume, which is what keeps the user from being
    /// stranded on the volume they just jumped to.
    #[test]
    fn the_picker_can_jump_across_volumes() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);

        studio.run(Command::OpenFolder);
        let first = studio
            .picker
            .peek()
            .clone()
            .expect("the picker did not open");
        assert!(!first.volumes.is_empty(), "volumes must be offered");

        // Pick the first volume that is not the one the picker opened on —
        // which is the realistic case, because the picker opens on `$HOME`
        // and the volume the user wants is by definition a different one.
        let target = first
            .volumes
            .iter()
            .find(|volume| volume.path != first.cwd)
            .or_else(|| first.volumes.first())
            .expect("at least one volume is always present")
            .path
            .clone();

        studio.picker_to(&target);
        let moved = studio
            .picker
            .peek()
            .clone()
            .expect("the picker is still open");
        assert_eq!(
            moved.cwd, target,
            "picker_to moved the picker to the volume's root"
        );
        assert!(
            !moved.volumes.is_empty(),
            "the volume strip is still populated"
        );
        // The picker it landed on can navigate further, which is the contract
        // a chip click depends on: it is the same wire a directory row uses.
        studio.escape();
        assert!(studio.picker.peek().is_none(), "Escape closed it");
    }

    #[test]
    fn a_second_can_run_check() {
        let (_runtime, studio) = studio_with(THREE, 1);
        studio.add_caret_line(true);
        assert!(studio.can_run(Command::ClearCursors));
        assert!(studio.clear_extra_carets());
        assert!(!studio.clear_extra_carets(), "and again is a no-op");
    }

    /// **The behaviour this pins:** New Project opens the **folder picker
    /// first** (mode = `NewProject`), and confirming in that mode opens the
    /// name prompt with the picked folder as the parent. This is the
    /// two-step flow the user asked for: pick *where* before typing *what*.
    ///
    /// A regression that reverts to opening the name prompt immediately would
    /// land the project in a directory the user did not choose, which is the
    /// exact thing the picker step exists to prevent.
    #[test]
    fn new_workspace_opens_the_folder_picker_before_the_name_prompt() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        assert!(studio.picker.peek().is_none(), "no picker before");
        assert!(studio.name_prompt.peek().is_none(), "no name prompt before");
        assert_eq!(studio.picker_mode.get(), PickerMode::Open, "default mode");

        studio.run(Command::NewProject);

        // Step 1: the picker opens in NewProject mode. The name prompt is
        // *not* open yet — the folder has to be chosen first.
        let picker = studio
            .picker
            .peek()
            .clone()
            .expect("New Project opened the folder picker");
        assert_eq!(
            studio.picker_mode.get(),
            PickerMode::NewProject,
            "the picker is in NewProject mode"
        );
        assert!(
            studio.name_prompt.peek().is_none(),
            "the name prompt is not open until a folder is picked"
        );
        assert!(
            picker.cwd.is_absolute(),
            "the picker opened on an absolute path"
        );

        // Confirming in NewProject mode does not open the workspace — it
        // closes the picker and opens the name prompt with the picked folder
        // as the target parent.
        studio.picker_choose();

        assert!(
            studio.picker.peek().is_none(),
            "the picker closed on confirm"
        );
        let prompt = studio
            .name_prompt
            .peek()
            .clone()
            .expect("the name prompt opened after the picker confirmed");
        assert_eq!(prompt.kind, NameKind::NewProject);
        assert!(
            prompt.text.is_empty(),
            "the name field is empty, not pre-filled"
        );
        assert!(prompt.target.is_absolute(), "the parent is absolute");

        // And Escape closes the name prompt, which is the contract every
        // other modal honours.
        studio.escape();
        assert!(studio.name_prompt.peek().is_none(), "Escape closed it");
    }

    /// `open_picker` resets the mode to `Open`, so a studio that previously
    /// opened the picker in `NewProject` mode and was cancelled does not
    /// keep that mode for the next File → Open Folder… invocation — which
    /// would open a workspace when the user expected to be asked for a name.
    #[test]
    fn open_picker_resets_the_mode_to_open() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);

        // Put the studio in NewProject mode, the way `new_project` does.
        studio.run(Command::NewProject);
        assert_eq!(studio.picker_mode.get(), PickerMode::NewProject);
        studio.close_picker();

        // The mode is left as NewProject after close — that is fine, because
        // `open_picker` is the one that resets it. The File → Open Folder…
        // command goes through `open_picker`, not through a bare
        // `picker.set(Some(...))`.
        assert_eq!(
            studio.picker_mode.get(),
            PickerMode::NewProject,
            "close_picker does not reset the mode — open_picker does"
        );

        studio.open_picker();
        assert_eq!(
            studio.picker_mode.get(),
            PickerMode::Open,
            "open_picker resets to Open mode, so File → Open Folder… never \
             accidentally creates a project"
        );
    }

    /// A name that violates Cargo's crate rules greys the prompt with a
    /// reason, before anything is written to disk. This is the rule that makes
    /// the prompt more useful than the old auto-naming path: a user who typed
    /// `1app` or `loop` is told why at the field, rather than twenty seconds
    /// into a build against a manifest the studio generated for them.
    ///
    /// This test reaches the name prompt directly via `ask_for_name`, because
    /// the full flow goes through the picker first and would not exercise the
    /// validation without a real directory to confirm against.
    #[test]
    fn a_workspace_name_is_checked_against_cargo_rules_as_it_is_typed() {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime);
        // Open the name prompt directly, the way `picker_choose` does in
        // NewProject mode after the user has picked a folder.
        studio.ask_for_name(NameKind::NewProject, std::path::Path::new("/tmp"));

        // `1app` starts with a digit, which `cargo` rejects.
        studio.set_prompt_name("1app".to_owned());
        assert!(
            studio
                .name_prompt
                .peek()
                .expect("prompt still open")
                .error
                .is_some(),
            "a name starting with a digit must show an error"
        );

        // `loop` is a Rust keyword, which would compile as `mod loop;` and fail.
        studio.set_prompt_name("loop".to_owned());
        assert!(
            studio
                .name_prompt
                .peek()
                .expect("prompt still open")
                .error
                .is_some(),
            "a Rust keyword must show an error"
        );

        // `my-app` is a fine crate name and a fine directory name.
        studio.set_prompt_name("my-app".to_owned());
        assert!(
            studio
                .name_prompt
                .peek()
                .expect("prompt still open")
                .error
                .is_none(),
            "a valid crate name must not show an error"
        );
    }
}
