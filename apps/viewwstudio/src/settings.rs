//! What the studio remembers between launches.
//!
//! # Why this exists at all
//!
//! Until this module the studio persisted **nothing**. The Settings view
//! offered twenty toggles and steppers and every one of them was a `Signal`
//! that died with the process, along with the open workspace, the tab set, the
//! caret, and the window's own size. A user who spent two minutes setting the
//! studio up the way they liked it did that again the next morning.
//!
//! The framework had the missing piece the whole time: `vieww_platform_winit`
//! provides a `Storage` service backed by a file under
//! [`data_dir`](vieww_platform_winit::storage::data_dir), and the studio's
//! `services` field already carried it. Nothing ever called it. This module is
//! the record that goes in, and the parser that reads it back.
//!
//! # Why not a serialisation crate
//!
//! The studio has three third-party dependencies and a stated preference for
//! keeping that number small. More to the point, a config file is a file
//! **people edit by hand**, and a hand-edited file is a file with a typo in it
//! one morning. The format here is `key = value`, one per line, `#` for
//! comments — and the parser's contract is that *nothing in it can fail*:
//!
//! * an unknown key is ignored, so a file written by a newer studio still opens
//!   in an older one;
//! * a value that will not parse leaves that field at its default rather than
//!   discarding the whole file;
//! * a missing file is an empty record, which is every field's default.
//!
//! A settings file that refuses to load because line 34 says `font_size = big`
//! is a settings file that loses the other thirty-three lines, and a studio
//! that will not start because of it is worse than one with no memory at all.
//!
//! # Two records, two lifetimes
//!
//! [`Settings`] is preference — what the user chose. [`Session`] is where they
//! were — workspace, tabs, caret, window. They are separate keys in the store
//! because they are forgotten at different times: "reset my settings" must not
//! close the user's files, and "close everything" must not turn the theme back
//! to dark.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The storage key holding [`Settings`].
pub const SETTINGS_KEY: &str = "studio.settings";

/// The storage key holding [`Session`].
pub const SESSION_KEY: &str = "studio.session";

/// How many workspaces the recent list keeps.
///
/// Ten is a screen's worth on the welcome view and about as far back as anyone
/// recognises a path. The list is most-recent-first and de-duplicated, so a
/// project reopened daily stays at the top rather than filling the list.
pub const RECENT_LIMIT: usize = 10;

// ---------------------------------------------------------------------------
// The wire format
// ---------------------------------------------------------------------------

/// A parsed `key = value` file.
///
/// Public because both records use it and because the tests for the *format*
/// belong to the format, not to either record.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Record {
    values: BTreeMap<String, String>,
}

impl Record {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `text`. Never fails — see the module docs.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut values = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            // A comment, or the blank line between sections.
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                // A line with no `=` is a line somebody was in the middle of
                // typing. Skipped, not fatal.
                continue;
            };
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            values.insert(key.to_owned(), value.trim().to_owned());
        }
        Self { values }
    }

    /// Render back to text, sorted by key so a diff of two settings files is
    /// about what changed rather than about hash order.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (key, value) in &self.values {
            out.push_str(key);
            out.push_str(" = ");
            out.push_str(value);
            out.push('\n');
        }
        out
    }

    pub fn put(&mut self, key: &str, value: impl Into<String>) {
        let value: String = value.into();
        // A value containing a newline would produce a file that reads back as
        // two keys, one of them nonsense. Paths can legally contain one on
        // Unix; this is the one case the writer has to defend against, and
        // dropping the entry is better than writing a file that will not
        // round-trip.
        if value.contains('\n') {
            return;
        }
        self.values.insert(key.to_owned(), value);
    }

    pub fn put_bool(&mut self, key: &str, value: bool) {
        self.put(key, if value { "true" } else { "false" });
    }

    pub fn put_f32(&mut self, key: &str, value: f32) {
        // Not `{value}`: a float formatted by `Display` can come out as
        // `1e-7`, which parses back fine, and as `NaN`, which does not mean
        // anything as a width. Non-finite values are simply not written, so
        // the field keeps its default on the way back in.
        if value.is_finite() {
            self.put(key, format!("{value:.4}"));
        }
    }

    pub fn put_usize(&mut self, key: &str, value: usize) {
        self.put(key, value.to_string());
    }

    /// Store `paths` under `key` as a `\0`-free list.
    ///
    /// The separator is `\u{1}`, not a comma or a colon: both of those are
    /// legal in a path on Unix and a comma is legal on Windows too. `\u{1}` is
    /// not legal in a path on any platform the studio runs on.
    pub fn put_paths(&mut self, key: &str, paths: &[PathBuf]) {
        let joined: Vec<String> = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .filter(|text| !text.contains('\n'))
            .collect();
        self.put(key, joined.join("\u{1}"));
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// The value at `key`, or `fallback` if it is absent or unparseable.
    #[must_use]
    pub fn bool_or(&self, key: &str, fallback: bool) -> bool {
        match self.get(key) {
            Some("true" | "yes" | "1" | "on") => true,
            Some("false" | "no" | "0" | "off") => false,
            _ => fallback,
        }
    }

    /// The value at `key`, clamped into `range`, or `fallback`.
    ///
    /// The clamp is not decoration. A settings file is hand-editable, and
    /// `sidebar_width = 100000` is a studio with no editor in it and no way to
    /// get one back except by finding and deleting the file.
    #[must_use]
    pub fn f32_or(&self, key: &str, fallback: f32, range: (f32, f32)) -> f32 {
        self.get(key)
            .and_then(|text| text.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .map_or(fallback, |value| value.clamp(range.0, range.1))
    }

    #[must_use]
    pub fn usize_or(&self, key: &str, fallback: usize, max: usize) -> usize {
        self.get(key)
            .and_then(|text| text.parse::<usize>().ok())
            .map_or(fallback, |value| value.min(max))
    }

    #[must_use]
    pub fn path(&self, key: &str) -> Option<PathBuf> {
        self.get(key)
            .filter(|text| !text.is_empty())
            .map(PathBuf::from)
    }

    #[must_use]
    pub fn paths(&self, key: &str) -> Vec<PathBuf> {
        self.get(key)
            .filter(|text| !text.is_empty())
            .map(|text| text.split('\u{1}').map(PathBuf::from).collect())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Everything the Settings view can change, in one record.
///
/// Every field has a default, and the defaults are the values the studio used
/// before it could remember anything — so a first launch after this module
/// exists looks exactly like every launch before it.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub dark: bool,
    pub show_insets: bool,
    pub comforts: bool,
    pub indent_guides: bool,
    pub format_on_save: bool,
    pub inline_diagnostics: bool,
    pub release_profile: bool,
    pub auto_render: bool,
    pub minimap: bool,
    pub blink: bool,
    pub highlight: bool,
    pub reduce_motion: bool,
    /// Whether the previewed screen is themed dark. Independent of `dark`,
    /// which is the studio's own chrome — the two answer different questions.
    pub preview_dark: bool,
    /// N-new: wrap long lines in the editor rather than scrolling sideways.
    pub word_wrap: bool,
    /// Accessibility: the studio's own chrome text, one notch larger.
    pub large_ui: bool,
    /// Accessibility: foregrounds pushed towards white or black.
    pub high_contrast: bool,
    /// The studio's accent, by name — one of `theme::ACCENTS`.
    ///
    /// Stored as a name so the file stays readable and survives the palette
    /// being reordered or a stop being adjusted. An unknown name falls back to
    /// the default accent rather than refusing to load the file.
    pub accent: String,
    /// Whether the vieww-specific lint layer contributes to Problems.
    pub lint: bool,
    /// N-new: which colour theme, by name. `"dark"` and `"light"` are built in;
    /// anything else names a file in the themes directory.
    pub theme: String,
    pub font_size: f32,
    pub tab_width: usize,
    pub text_scale: f32,
    pub sidebar_width: f32,
    pub preview_width: f32,
    pub panel_open: bool,
    pub right_open: bool,
    /// The far-left column of view switches. Separate from `sidebar_width`,
    /// because the two hide independently — Ctrl+B and Ctrl+Alt+U.
    pub activity_bar_open: bool,

    // ----- where the mobile SDKs are, and what signs a build ---------------
    //
    // Empty means "ask the environment", which is what the studio did before
    // any of these existed: `ANDROID_HOME` and friends off the process
    // environment, and nothing at all if the studio was launched from a desktop
    // icon that inherited none of a shell's exports. That is the case these
    // close — a GUI application on macOS or Linux started from the Dock has
    // never seen the user's `.zshrc`, so a perfectly installed Android SDK was
    // invisible to it and the checklist said "missing" with no way to argue.
    /// The Android SDK root — the directory holding `platform-tools`.
    pub android_home: String,
    /// The NDK, which is a *versioned directory inside* the SDK.
    pub android_ndk: String,
    /// A JDK 17 or newer, which Gradle needs and Rust does not.
    pub java_home: String,
    /// The keystore an Android release is signed with.
    pub keystore: String,
    /// The alias inside that keystore.
    ///
    /// The **password is deliberately not here.** This file is plain text in a
    /// config directory, and a signing key's password written into it is a
    /// signing key given away with the laptop. `Studio::signing_env` reads it
    /// from `VIEWW_KEYSTORE_PASSWORD` at build time instead, which is what CI
    /// does anyway and what a password manager can fill.
    pub keystore_alias: String,
    /// The Apple Developer team, for `codesign` and provisioning.
    pub apple_team: String,
}

/// A string setting, falling back when the key is absent *or blank*.
///
/// Blank matters here: every one of these is written even when empty, so the
/// file always shows which keys exist — and an empty value has to mean "not
/// set" rather than "set to nothing", or a fresh file would override its own
/// defaults with emptiness.
fn setting_text(record: &Record, key: &str, fallback: &str) -> String {
    record
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .map_or_else(|| fallback.to_owned(), |value| value.trim().to_owned())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            // On, matching `Studio`'s own default — this struct is what a
            // studio with no settings file on disk starts from, so the two
            // disagreeing means the first launch and the second differ. See
            // `Studio::show_insets` for why on is right for a phone frame.
            show_insets: true,
            comforts: true,
            indent_guides: true,
            format_on_save: true,
            inline_diagnostics: true,
            release_profile: false,
            auto_render: false,
            minimap: true,
            blink: true,
            highlight: true,
            reduce_motion: false,
            preview_dark: false,
            word_wrap: false,
            large_ui: false,
            high_contrast: false,
            accent: crate::theme::accent_name(crate::theme::ACCENT).to_owned(),
            lint: true,
            theme: "dark".to_owned(),
            font_size: 13.0,
            tab_width: 4,
            text_scale: 1.0,
            sidebar_width: 248.0,
            preview_width: 460.0,
            panel_open: true,
            right_open: true,
            activity_bar_open: true,
            android_home: String::new(),
            android_ndk: String::new(),
            java_home: String::new(),
            keystore: String::new(),
            keystore_alias: String::new(),
            apple_team: String::new(),
        }
    }
}

impl Settings {
    /// Read a settings file. A file that does not parse is a file of defaults.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let record = Record::parse(text);
        let fallback = Self::default();
        Self {
            dark: record.bool_or("dark", fallback.dark),
            show_insets: record.bool_or("show_insets", fallback.show_insets),
            comforts: record.bool_or("comforts", fallback.comforts),
            indent_guides: record.bool_or("indent_guides", fallback.indent_guides),
            format_on_save: record.bool_or("format_on_save", fallback.format_on_save),
            inline_diagnostics: record.bool_or("inline_diagnostics", fallback.inline_diagnostics),
            release_profile: record.bool_or("release_profile", fallback.release_profile),
            auto_render: record.bool_or("auto_render", fallback.auto_render),
            minimap: record.bool_or("minimap", fallback.minimap),
            blink: record.bool_or("blink", fallback.blink),
            highlight: record.bool_or("highlight", fallback.highlight),
            reduce_motion: record.bool_or("reduce_motion", fallback.reduce_motion),
            preview_dark: record.bool_or("preview_dark", fallback.preview_dark),
            word_wrap: record.bool_or("word_wrap", fallback.word_wrap),
            large_ui: record.bool_or("large_ui", fallback.large_ui),
            high_contrast: record.bool_or("high_contrast", fallback.high_contrast),
            accent: record
                .get("accent")
                .filter(|name| !name.is_empty())
                .map_or_else(|| fallback.accent.clone(), str::to_owned),
            lint: record.bool_or("lint", fallback.lint),
            theme: record
                .get("theme")
                .filter(|name| !name.is_empty())
                .unwrap_or(&fallback.theme)
                .to_owned(),
            // The ranges are the ones the Settings view offers, widened enough
            // that a hand-edited file can pick a value between the steps
            // without being overruled.
            font_size: record.f32_or("font_size", fallback.font_size, (8.0, 32.0)),
            tab_width: record.usize_or("tab_width", fallback.tab_width, 16).max(1),
            text_scale: record.f32_or("text_scale", fallback.text_scale, (0.5, 4.0)),
            sidebar_width: record.f32_or(
                "sidebar_width",
                fallback.sidebar_width,
                (crate::state::MIN_SIDEBAR, crate::state::MAX_SIDEBAR),
            ),
            // **Floored at `MIN_PANE`, not at zero.** A hand-edited file saying
            // `preview_width = 40` used to be honoured, and the studio opened
            // with a pane too narrow for its own toolbar — controls painted
            // past the window edge exactly as the divider bug did, except no
            // amount of dragging could widen it, because the value came back
            // from the file on every launch. The floor the divider enforces is
            // the floor the file meets.
            preview_width: record.f32_or(
                "preview_width",
                fallback.preview_width,
                (crate::state::MIN_PANE, 1600.0),
            ),
            panel_open: record.bool_or("panel_open", fallback.panel_open),
            right_open: record.bool_or("right_open", fallback.right_open),
            activity_bar_open: record.bool_or("activity_bar_open", fallback.activity_bar_open),
            android_home: setting_text(&record, "android_home", &fallback.android_home),
            android_ndk: setting_text(&record, "android_ndk", &fallback.android_ndk),
            java_home: setting_text(&record, "java_home", &fallback.java_home),
            keystore: setting_text(&record, "keystore", &fallback.keystore),
            keystore_alias: setting_text(&record, "keystore_alias", &fallback.keystore_alias),
            apple_team: setting_text(&record, "apple_team", &fallback.apple_team),
        }
    }

    #[must_use]
    pub fn render(&self) -> String {
        let mut record = Record::new();
        record.put_bool("dark", self.dark);
        record.put_bool("show_insets", self.show_insets);
        record.put_bool("comforts", self.comforts);
        record.put_bool("indent_guides", self.indent_guides);
        record.put_bool("format_on_save", self.format_on_save);
        record.put_bool("inline_diagnostics", self.inline_diagnostics);
        record.put_bool("release_profile", self.release_profile);
        record.put_bool("auto_render", self.auto_render);
        record.put_bool("minimap", self.minimap);
        record.put_bool("blink", self.blink);
        record.put_bool("highlight", self.highlight);
        record.put_bool("reduce_motion", self.reduce_motion);
        record.put_bool("preview_dark", self.preview_dark);
        record.put_bool("word_wrap", self.word_wrap);
        record.put_bool("large_ui", self.large_ui);
        record.put_bool("high_contrast", self.high_contrast);
        record.put("accent", self.accent.clone());
        record.put_bool("lint", self.lint);
        record.put("theme", self.theme.clone());
        record.put_f32("font_size", self.font_size);
        record.put_usize("tab_width", self.tab_width);
        record.put_f32("text_scale", self.text_scale);
        record.put_f32("sidebar_width", self.sidebar_width);
        record.put_f32("preview_width", self.preview_width);
        record.put_bool("panel_open", self.panel_open);
        record.put_bool("right_open", self.right_open);
        record.put_bool("activity_bar_open", self.activity_bar_open);
        // Written even when empty, so the file itself is the documentation:
        // somebody opening it sees which keys exist rather than having to know.
        record.put("android_home", self.android_home.clone());
        record.put("android_ndk", self.android_ndk.clone());
        record.put("java_home", self.java_home.clone());
        record.put("keystore", self.keystore.clone());
        record.put("keystore_alias", self.keystore_alias.clone());
        record.put("apple_team", self.apple_team.clone());
        let mut out = String::from("# vieww Studio settings. Edit by hand if you like:\n");
        out.push_str("# an unknown key is ignored and a bad value falls back to its default.\n");
        out.push_str(&record.render());
        out
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// Where the user was when they last closed the studio.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Session {
    /// The workspace root, if one was open.
    pub root: Option<PathBuf>,
    /// Every open buffer with a path, in tab order. Untitled buffers cannot be
    /// restored — they have nowhere to be read back from — so they are not
    /// written, which is also why [`Studio::save_session`] is not a substitute
    /// for saving your work.
    ///
    /// [`Studio::save_session`]: crate::Studio::save_session
    pub open: Vec<PathBuf>,
    /// Which of `open` was active. Clamped on the way back in.
    pub active: usize,
    /// The activity view, by its stable name.
    pub view: String,
    /// The bottom panel's tab, by its stable name.
    pub panel_tab: String,
    /// Workspaces opened before, most recent first.
    pub recent: Vec<PathBuf>,
    /// Window size in logical pixels, if one was recorded.
    pub window: Option<(f32, f32)>,
}

impl Session {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let record = Record::parse(text);
        Self {
            root: record.path("root"),
            open: record.paths("open"),
            active: record.usize_or("active", 0, 4096),
            view: record.get("view").unwrap_or_default().to_owned(),
            panel_tab: record.get("panel_tab").unwrap_or_default().to_owned(),
            recent: record.paths("recent"),
            window: match (record.get("window_width"), record.get("window_height")) {
                (Some(width), Some(height)) => {
                    match (width.parse::<f32>(), height.parse::<f32>()) {
                        // A window smaller than this is one the user cannot
                        // find the close button on, and a stored zero — which
                        // is what a minimised window reports on some platforms
                        // — would restore as an invisible studio.
                        (Ok(width), Ok(height)) if width >= 480.0 && height >= 360.0 => {
                            Some((width, height))
                        }
                        _ => None,
                    }
                }
                _ => None,
            },
        }
    }

    #[must_use]
    pub fn render(&self) -> String {
        let mut record = Record::new();
        if let Some(root) = &self.root {
            record.put("root", root.to_string_lossy().into_owned());
        }
        record.put_paths("open", &self.open);
        record.put_usize("active", self.active);
        record.put("view", self.view.clone());
        record.put("panel_tab", self.panel_tab.clone());
        record.put_paths("recent", &self.recent);
        if let Some((width, height)) = self.window {
            record.put_f32("window_width", width);
            record.put_f32("window_height", height);
        }
        let mut out = String::from("# vieww Studio session. Deleting this forgets your tabs,\n");
        out.push_str("# not your settings — those are in the settings file beside it.\n");
        out.push_str(&record.render());
        out
    }

    /// Put `root` at the front of the recent list, removing any earlier
    /// mention of it and trimming to [`RECENT_LIMIT`].
    ///
    /// De-duplication is by path equality after the caller has canonicalised,
    /// which matters more than it looks: `~/code/app` and `~/code/app/` are the
    /// same project and two different strings.
    pub fn remember(&mut self, root: &Path) {
        self.recent.retain(|existing| existing != root);
        self.recent.insert(0, root.to_path_buf());
        self.recent.truncate(RECENT_LIMIT);
    }

    /// Drop recent entries whose directory no longer exists.
    ///
    /// Run when the welcome view is built rather than when the list is written:
    /// a project on an unmounted drive is not gone, it is not here *now*, and
    /// forgetting it because the volume was unplugged is the wrong answer.
    /// Filtering at display time shows what can actually be opened and keeps
    /// the rest for the next launch.
    #[must_use]
    pub fn openable_recent(&self) -> Vec<PathBuf> {
        self.recent
            .iter()
            .filter(|path| path.is_dir())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_every_default() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(Session::parse(""), Session::default());
    }

    #[test]
    fn settings_round_trip() {
        let settings = Settings {
            dark: false,
            font_size: 15.0,
            tab_width: 2,
            word_wrap: true,
            theme: "solarized".to_owned(),
            sidebar_width: 300.0,
            ..Settings::default()
        };
        let back = Settings::parse(&settings.render());
        assert_eq!(back, settings);
    }

    /// The whole contract of the parser, in one test.
    #[test]
    fn a_bad_line_costs_its_own_field_and_nothing_else() {
        let text = "dark = false\nfont_size = enormous\nminimap = false\n";
        let settings = Settings::parse(text);
        assert!(!settings.dark, "the good line before the bad one");
        assert!(!settings.minimap, "the good line after the bad one");
        assert!(
            (settings.font_size - Settings::default().font_size).abs() < f32::EPSILON,
            "the bad line falls back rather than taking the file down"
        );
    }

    #[test]
    fn an_unknown_key_is_ignored_so_an_older_studio_can_read_a_newer_file() {
        let text = "dark = false\nquantum_mode = true\n";
        assert!(!Settings::parse(text).dark);
    }

    #[test]
    fn comments_and_blank_lines_and_missing_equals_are_all_survivable() {
        let text = "# a comment\n\n   \nnot a setting\ndark = false\n";
        assert!(!Settings::parse(text).dark);
    }

    #[test]
    fn whitespace_around_the_equals_does_not_matter() {
        assert!(!Settings::parse("dark=false").dark);
        assert!(!Settings::parse("   dark   =   false   ").dark);
    }

    /// A hand-edited file cannot lock the user out of their own editor.
    #[test]
    fn absurd_values_are_clamped_rather_than_obeyed() {
        let settings = Settings::parse("sidebar_width = 100000\nfont_size = 900\ntab_width = 0");
        assert!(
            settings.sidebar_width <= 900.0,
            "{}",
            settings.sidebar_width
        );
        assert!(settings.font_size <= 32.0, "{}", settings.font_size);
        assert!(settings.tab_width >= 1, "a zero-width tab is not an indent");
    }

    #[test]
    fn a_non_finite_float_is_not_written_and_so_reads_back_as_the_default() {
        let settings = Settings {
            font_size: f32::NAN,
            ..Settings::default()
        };
        let back = Settings::parse(&settings.render());
        assert!(
            (back.font_size - Settings::default().font_size).abs() < f32::EPSILON,
            "{}",
            back.font_size
        );
    }

    #[test]
    fn session_round_trips_paths_with_spaces_and_punctuation() {
        let session = Session {
            root: Some(PathBuf::from("/home/a b/my project, v2")),
            open: vec![
                PathBuf::from("/home/a b/my project, v2/src/main.rs"),
                PathBuf::from("/home/a b/x = y.rs"),
            ],
            active: 1,
            view: "Explorer".to_owned(),
            panel_tab: "Problems".to_owned(),
            recent: vec![PathBuf::from("/home/a b/my project, v2")],
            window: Some((1440.0, 900.0)),
        };
        assert_eq!(Session::parse(&session.render()), session);
    }

    /// `x = y.rs` is a legal filename and would round-trip as a key if the
    /// path list were not stored as one value.
    #[test]
    fn a_path_containing_an_equals_sign_survives() {
        let session = Session {
            open: vec![PathBuf::from("/tmp/a = b.rs")],
            ..Session::default()
        };
        assert_eq!(Session::parse(&session.render()).open, session.open);
    }

    #[test]
    fn recent_is_most_recent_first_deduplicated_and_bounded() {
        let mut session = Session::default();
        for n in 0..RECENT_LIMIT + 5 {
            session.remember(Path::new(&format!("/p/{n}")));
        }
        assert_eq!(session.recent.len(), RECENT_LIMIT);
        assert_eq!(session.recent[0], PathBuf::from("/p/14"));

        session.remember(Path::new("/p/14"));
        assert_eq!(
            session.recent.len(),
            RECENT_LIMIT,
            "reopening is not a new entry"
        );
        assert_eq!(session.recent[0], PathBuf::from("/p/14"));
    }

    #[test]
    fn a_tiny_window_is_not_restored() {
        assert_eq!(
            Session::parse("window_width = 0\nwindow_height = 0").window,
            None
        );
        assert_eq!(
            Session::parse("window_width = 1200\nwindow_height = 800").window,
            Some((1200.0, 800.0))
        );
    }

    #[test]
    fn a_value_with_a_newline_in_it_is_refused_rather_than_written() {
        let mut record = Record::new();
        record.put("root", "/tmp/one\ntwo");
        assert_eq!(record.get("root"), None, "a two-line value is not a value");
    }

    #[test]
    fn rendering_is_sorted_so_two_settings_files_diff_cleanly() {
        let text = Settings::default().render();
        let keys: Vec<&str> = text
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split(' ').next())
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }
}
