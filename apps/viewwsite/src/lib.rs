//! viewwstudio's product page — a vieww widget tree on a `<canvas>`.
//!
//! # What this is, and why it is not HTML
//!
//! It is the page that sells the studio, built out of the same widgets the
//! studio itself is built out of, laid out by the same layout algorithm,
//! rasterised by the same CPU rasteriser, and presented through
//! `vieww_platform_web` (a wasm32-only dependency, so not linkable from a host
//! `cargo doc`). There is no `<div>` in it. The only HTML on the
//! page is the `<canvas>` this mounts on.
//!
//! That is the argument the page is making, so it had better be true of the
//! page: a framework whose landing page is hand-written HTML is a framework
//! whose author reached for something else the one time it mattered.
//!
//! # What it costs, stated here rather than discovered
//!
//! A canvas is pixels. Text drawn here is not selectable, the browser's find
//! bar cannot see it, and a crawler reads nothing — so the `<canvas>` this
//! mounts on carries the page's real copy in its fallback content, and the
//! host page carries the metadata. That is the honest arrangement, not a
//! workaround: a search engine should be given words, and this backend does
//! not produce any.
//!
//! # Shape
//!
//! - `start` — the entry point wasm-bindgen calls, and the only public item.
//!   (wasm32 only, which is why it is not a link here.)
//! - `Site` — the one widget that reads the scroll signal, so a drag rebuilds
//!   it and nothing else.
//! - everything else — free functions returning [`WidgetNode`], because a
//!   section of a page has no identity worth keeping across a rebuild.

pub mod dom;

use std::time::Duration;

use vieww::element::{Animation, Signal};
use vieww::foundation::{Axis, Cursor, FontFamily};
use vieww::prelude::*;
use vieww::ImageData;
use vieww::{FrameDriver, ScrollController, ScrollPhysics};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

// ─── the palette ──────────────────────────────────────────────────────────
//
// The studio's own, from `apps/viewwstudio/src/theme.rs`: `ACCENT` is `PURPLE`
// (`dark_near` 7E5CE8, `dark_far` B491FF) and the dark scheme's `chrome_0` is
// 181818. Restated here rather than imported because the studio is a binary
// crate this page must not depend on — and because a page that drifted from
// the product it sells would be advertising a colour scheme nobody ships.

// Converted from `src/app/globals.css`'s `.dark` block — the site's own oklch
// tokens, not values picked to look similar. The neutrals there are **warm**
// (hue 60), which is the difference between this page and the one it is meant
// to match: mine were cool and read as a different product.

/// `--background`, `oklch(0.16 0.006 60)`.
const GROUND: Color = Color::rgb(0x0F, 0x0D, 0x0B);
/// `--card`, `oklch(0.21 0.008 60)`.
const SURFACE: Color = Color::rgb(0x1B, 0x17, 0x15);
/// `--secondary`, `oklch(0.27 0.012 60)` — a surface that sits on a surface.
const SURFACE_2: Color = Color::rgb(0x2B, 0x25, 0x21);
/// `--border`, which is white at 9% over the ground.
const LINE: Color = Color::rgb(0x2A, 0x28, 0x26);
/// `--foreground`, `oklch(0.97 0.005 60)`.
const INK: Color = Color::rgb(0xF8, 0xF4, 0xF2);
/// `--muted-foreground`, `oklch(0.7 0.015 60)`.
const INK_2: Color = Color::rgb(0xA6, 0x9C, 0x95);
/// Between the border and the muted foreground: captions and file names.
const INK_3: Color = Color::rgb(0x78, 0x71, 0x6C);

// **Purple, as asked for.** `globals.css` says
// `/* Blue accent — #5B9DF9, the viewwstudio mark's preview panel */`, so the
// published site is blue; this page is purple on instruction. Both are one
// constant, so it is one edit either way.

/// The accent on a dark ground.
const ACCENT: Color = Color::rgb(0xB4, 0x91, 0xFF);
/// The accent a button is filled with.
const ACCENT_DEEP: Color = Color::rgb(0x7E, 0x5C, 0xE8);
/// `bg-primary/10` — what an eyebrow pill and an icon square sit on.
const WASH: Color = Color::rgb(0x22, 0x1C, 0x33);

/// The widest the *page* is allowed to get.
///
/// This used to be 860 — a reading measure — and applying it to the whole page
/// was the mistake: on a 1440 window it left two hundred points of ground down
/// each side and a screenshot shrunk to fit a column of prose. A product page
/// is not an essay. The measure still exists, as [`PROSE`], and now applies to
/// the paragraphs rather than to everything.
const COLUMN: f32 = 1280.0;

/// The widest a *paragraph* is allowed to get.
///
/// A little over the sixty-odd characters a line can hold before a reader
/// starts losing their place on the return sweep. Headings get [`PROSE`] plus
/// some slack, because a heading is scanned rather than read.
const PROSE: f32 = 672.0;

/// `max-w-3xl` — how wide a section heading is allowed to run.
const HEADING_MEASURE: f32 = 768.0;

/// `py-20 md:py-28` — the vertical padding every section carries.
///
/// **This is the number that was wrong.** It was 44, and the reference's is 80
/// on a phone and 112 on a desktop: a page whose sections are two and a half
/// times closer together than they should be reads as cramped no matter what
/// is in them, and "there is no clarity" is what that looks like from the
/// outside.
const SECTION_Y: f32 = 112.0;
/// The same, narrow.
const SECTION_Y_NARROW: f32 = 80.0;

/// The narrowest a two-column band stays two columns.
const BREAKPOINT: f32 = 720.0;

/// The page's own gutter.
const GUTTER: f32 = 28.0;

// ─── the release assets ───────────────────────────────────────────────────

/// The repository the download links point at.
///
/// The fallback, used when the page carries no `vieww-repo` meta tag — see
/// [`repo`], which is what the links actually read. Every URL is
/// `releases/latest/download/<name>`, which resolves only because each release
/// names its assets identically: the names `packaging/package.sh` writes.
const REPO: &str = "__REPO__";

/// One downloadable build.
struct Asset {
    /// Which platform's row this is, in the words a person picking a download
    /// uses — not the target triple.
    label: &'static str,
    /// The file name a release publishes, verbatim.
    file: &'static str,
    /// About how big it is. A stub until a release answers with its own: these
    /// are the sizes `packaging/package.sh` produces on this author's machine,
    /// and they are marked as approximate because they are.
    size: &'static str,
    /// What `detect` matches against.
    os: Os,
}

/// The three platforms the studio is packaged for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Every Linux, plus Android, which downloads the same way.
    Linux,
    /// Both Macs. Which `.dmg` is a question a browser cannot answer.
    MacOs,
    /// Windows on x86-64.
    Windows,
}

impl Os {
    /// The name to put on a button.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
        }
    }
}

/// Every published build, in the order the table lists them.
const ASSETS: &[Asset] = &[
    Asset {
        label: "Debian, Ubuntu",
        file: "viewwstudio-linux-x86_64.deb",
        size: "~410 MB",
        os: Os::Linux,
    },
    Asset {
        label: "Other Linux",
        file: "viewwstudio-linux-x86_64.AppImage",
        size: "~415 MB",
        os: Os::Linux,
    },
    Asset {
        label: "Linux archive",
        file: "viewwstudio-linux-x86_64.tar.gz",
        size: "~395 MB",
        os: Os::Linux,
    },
    Asset {
        label: "macOS, Apple silicon",
        file: "viewwstudio-macos-aarch64.dmg",
        size: "~400 MB",
        os: Os::MacOs,
    },
    Asset {
        label: "macOS, Intel",
        file: "viewwstudio-macos-x86_64.dmg",
        size: "~405 MB",
        os: Os::MacOs,
    },
    Asset {
        label: "Windows",
        file: "viewwstudio-windows-x86_64.msi",
        size: "~420 MB",
        os: Os::Windows,
    },
    Asset {
        label: "Windows archive",
        file: "viewwstudio-windows-x86_64.zip",
        size: "~400 MB",
        os: Os::Windows,
    },
];

/// The repository the links actually point at, this run.
///
/// # Why this is read from the page rather than compiled in
///
/// It used to be [`REPO`] and nothing else, which meant the repository name
/// was inside the wasm — and changing it meant a Rust toolchain, a two-minute
/// build and a new 2 MB binary to deploy. For a string that appears in one
/// place in a `<meta>` tag, that is the wrong trade.
///
/// So the page asks the document it is mounted in:
///
/// ```html
/// <meta name="vieww-repo" content="owner/name">
/// ```
///
/// and falls back to [`REPO`] when the tag is missing or still holds the
/// placeholder. `build-site.sh` writes the tag from `VIEWW_REPO`, so one `sed`
/// over `index.html` now fixes the links in **both** halves of the page — the
/// canvas and the HTML fallback — without recompiling anything.
///
/// Read once and kept: this is asked for every download URL on the page, and
/// the answer cannot change while the page is open.
#[cfg(target_arch = "wasm32")]
pub(crate) fn repo() -> String {
    use std::cell::RefCell;
    thread_local! {
        static CACHED: RefCell<Option<String>> = const { RefCell::new(None) };
    }
    CACHED.with(|cached| {
        cached
            .borrow_mut()
            .get_or_insert_with(|| {
                web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| {
                        document.query_selector("meta[name=\"vieww-repo\"]").ok()?
                    })
                    .and_then(|meta| meta.get_attribute("content"))
                    // A tag left holding the placeholder is the same as no tag:
                    // both mean "nobody has said which repository yet".
                    .filter(|value| !value.is_empty() && !value.starts_with("__"))
                    .unwrap_or_else(|| REPO.to_owned())
            })
            .clone()
    })
}

/// Off the browser there is no document to ask, so the compiled-in default is
/// the whole answer — which is what `examples/render.rs` draws.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn repo() -> String {
    REPO.to_owned()
}

/// The download URL for one asset.
fn url_for(file: &str) -> String {
    format!(
        "https://github.com/{}/releases/latest/download/{file}",
        repo()
    )
}

/// Send the browser somewhere.
///
/// A canvas has no `<a>` in it, so every link on this page is a tap handler
/// and this function. Failures are dropped: a navigation that the browser
/// refused is not something the page can do anything about, and there is no
/// console in a shipped page worth writing to.
#[cfg(target_arch = "wasm32")]
fn navigate(url: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href(url);
    }
}

/// Off the browser there is nowhere to navigate to, and the host renderer only
/// draws the page — so this records nothing and does nothing. It exists so the
/// tree below is one tree rather than two.
#[cfg(not(target_arch = "wasm32"))]
fn navigate(_url: &str) {}

/// Which build to put under the big button.
///
/// A guess, and always overridable — the table below the button is the whole
/// list. `None` when the user agent says nothing recognisable, which is a
/// reason to show the list rather than to pick wrong.
///
/// macOS is deliberately answered as one platform rather than two: a browser
/// will not say whether it is Apple silicon or Intel, and the wrong `.dmg`
/// refuses at first launch with a host-mismatch message. The button opens the
/// Apple-silicon build — every Mac sold since 2020 — and the line under it
/// says so.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn detect() -> Option<Os> {
    let agent = web_sys::window()?.navigator().user_agent().ok()?;
    if agent.contains("Windows") {
        Some(Os::Windows)
    } else if agent.contains("Mac") || agent.contains("iPhone") || agent.contains("iPad") {
        Some(Os::MacOs)
    } else if agent.contains("Linux") || agent.contains("X11") || agent.contains("Android") {
        Some(Os::Linux)
    } else {
        None
    }
}

/// No user agent to read off the browser: the host renderer is told which
/// platform to draw for.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub const fn detect() -> Option<Os> {
    None
}

/// The page's theme: the studio's accent over the catalogue's dark scheme.
///
/// Every control on the page reads this rather than a colour of its own — the
/// download button, the two text buttons, the press tint on a table row. Left
/// at the catalogue default they came out Fluent blue, which is the accent the
/// studio *used* to ship (`theme.rs`'s `BLUE`, kept there for the accent
/// picker) and not the one it ships now. A product page in last release's
/// accent is a small lie told in a conspicuous place.
fn theme() -> ThemeData {
    let mut colors = ColorScheme::dark();
    colors.primary = ACCENT_DEEP;
    colors.on_primary = Color::WHITE;
    colors.surface = SURFACE;
    colors.on_surface = INK;
    colors.surface_variant = SURFACE;
    colors.on_surface_variant = INK_2;
    colors.outline = LINE;
    ThemeData::from_colors(colors)
}

/// The typeface, and the reason the page carries one at all.
///
/// # Why this is not the embedded font
///
/// `vieww-text` embeds DejaVu Sans so the framework always has *a* font. It is
/// a 2004 Bitstream Vera derivative — wide, low contrast, no tight tracking —
/// and it is the single biggest reason a page can have the right layout and
/// still look like nobody chose anything. At a 72pt headline the face is the
/// design.
///
/// So the page ships **Geist** and **Geist Mono**, which is what
/// `studio.vieww.workers.dev` is set in. Both are SIL Open Font Licence 1.1
/// (`assets/fonts/Geist-OFL.txt`), subset to the 143 characters this page
/// actually draws and retagged for vieww's four-step weight model — 20 KB a
/// face rather than 126 KB, five faces for 97 KB before compression.
///
/// The retag is worth knowing about: their headings are CSS `font-semibold`,
/// which is 600, and [`FontWeight`] has no 600. Geist SemiBold is registered as
/// this family's *bold* member, so `Text::bold()` lands on the weight their
/// page uses rather than on a heavier one that would read as shouting.
const GEIST: &[u8] = include_bytes!("../assets/fonts/vw-Geist-Regular.ttf");
const GEIST_MEDIUM: &[u8] = include_bytes!("../assets/fonts/vw-Geist-Medium.ttf");
const GEIST_BOLD: &[u8] = include_bytes!("../assets/fonts/vw-Geist-Bold.ttf");
const GEIST_MONO: &[u8] = include_bytes!("../assets/fonts/vw-GeistMono-Regular.ttf");
const GEIST_MONO_MEDIUM: &[u8] = include_bytes!("../assets/fonts/vw-GeistMono-Medium.ttf");

/// The font store the page is drawn with — Geist in front, the embedded faces
/// behind it so anything outside the subset still renders.
///
/// Handed to [`FrameDriver::set_fonts`] before the first frame, by both
/// [`root`] and `examples/render.rs`, so the browser and the host renders
/// measure text identically.
#[must_use]
pub fn fonts() -> vieww::text::FontStore {
    vieww::text::FontStore::with_application_faces(
        [
            GEIST.to_vec(),
            GEIST_MEDIUM.to_vec(),
            GEIST_BOLD.to_vec(),
            GEIST_MONO.to_vec(),
            GEIST_MONO_MEDIUM.to_vec(),
        ],
        Some("Geist"),
        Some("Geist Mono"),
    )
}

/// The face the code blocks and the file names are set in.
fn mono(size: f32, color: Color) -> TextStyle {
    // Named rather than `monospace()`: the generic resolves through the font
    // *database* on the canvas backend, where this page has registered Geist
    // Mono as the monospace family — but through a CSS generic on the DOM one,
    // where it would land on whatever the platform calls monospace. Naming the
    // family is the only spelling both backends read the same way.
    TextStyle::new(size)
        .family(FontFamily::Named("Geist Mono"))
        .color(color)
}

// ─── the page ─────────────────────────────────────────────────────────────

/// The scrolling page.
///
/// The one widget here that reads a signal, and that is the point: a drag
/// marks this element pending and rebuilds the page's *description*, while the
/// element tree underneath keeps every child it can. Reading the offset in
/// `start`'s closure instead would subscribe nothing at all — a build is the
/// only place a signal read is a subscription.
#[derive(Debug)]
pub struct Site {
    /// Where the page is scrolled to.
    pub scroll: ScrollController,
    /// The surface, from the [`LayoutBuilder`] above this widget. Not read
    /// from anywhere else: a resize changes the constraints, the builder
    /// rebuilds, and this widget is handed the new numbers.
    pub surface: Size,
    /// What [`detect`] answered, resolved once at start-up rather than per
    /// build — the user agent does not change while the page is open.
    pub host: Option<Os>,
    /// 0 to 1, once, over the first second. Every entrance on the page is a
    /// slice of this one animation rather than an animation of its own — see
    /// the private `stage` helper.
    pub entrance: Animation<f32>,
    /// The live demo's counter. A real signal, written by a real tap.
    pub count: Signal<i32>,
    /// The decoded screenshots. Cloning one is an `Arc` bump; see [`Art`].
    pub art: Art,
    /// Where each navigable section ended up, measured. See [`Anchors`].
    pub anchors: Anchors,
}

impl Widget for Site {
    fn debug_name(&self) -> &'static str {
        "Site"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Read here, in a build, which is what subscribes this element to it —
        // and what makes the entrance rebuild *this* widget sixty times a
        // second for one second and nothing at all afterwards.
        let entrance = self.entrance.value();
        let width = self.surface.width;
        let wide = width >= BREAKPOINT;
        let gutter = if wide { GUTTER } else { 18.0 };
        // `clamp` rather than `.min().max()`: clippy's `manual_clamp` fires on
        // the pair, and the two are identical here because `240.0 < COLUMN`.
        // They differ on one input — a NaN `width` used to fall out as
        // `COLUMN` and now stays NaN — which is the honest answer for a
        // surface that has no width, and not a case this can reach.
        let column = (width - gutter * 2.0).clamp(240.0, COLUMN);

        let page = Flex::column()
            // **Stretch, not Start.** With `Start` a section is only as wide as
            // its own widest line and sits against the column's left edge — so
            // a section that centres its heading centres it on *itself* rather
            // than on the page, and every section lands on a slightly different
            // axis depending on how long its longest line happens to be. On the
            // hero that put the mark, the pill and the buttons on one centre
            // and the headline on another, about a hundred points apart. It
            // reads as "nothing quite lines up" long before anyone works out
            // why. `Stretch` gives every section the full column, which is what
            // makes centring mean the same thing twice.
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(0.0)
            .children(children![
                // The nav is drawn over the page, so the page owes it a gap.
                SizedBox::height(NAV_HEIGHT),
                hero(column, wide, self.host, entrance),
                divider(column),
                self.anchors.mark(
                    0,
                    &self.scroll,
                    self.anchors.reveal(
                        0,
                        &self.scroll,
                        demo_band(column, wide, &self.count, entrance),
                    ),
                ),
                divider(column),
                self.anchors.mark(
                    1,
                    &self.scroll,
                    self.anchors
                        .reveal(1, &self.scroll, showcase(column, wide, &self.art)),
                ),
                divider(column),
                self.anchors
                    .reveal(2, &self.scroll, loop_section(column, wide)),
                divider(column),
                self.anchors.reveal(3, &self.scroll, features(column, wide)),
                divider(column),
                self.anchors.mark(
                    2,
                    &self.scroll,
                    self.anchors.reveal(4, &self.scroll, code_section(wide)),
                ),
                divider(column),
                self.anchors.mark(
                    3,
                    &self.scroll,
                    self.anchors
                        .reveal(5, &self.scroll, gallery(wide, &self.art, column)),
                ),
                divider(column),
                self.anchors.mark(
                    4,
                    &self.scroll,
                    self.anchors
                        .reveal(6, &self.scroll, downloads(column, wide, self.host)),
                ),
                divider(column),
                self.anchors.reveal(7, &self.scroll, requirements(column)),
                divider(column),
                self.anchors.reveal(8, &self.scroll, footer(column, wide)),
            ]);

        let scrollable = Scrollable::vertical(self.scroll.offset())
            .viewport(self.surface.height)
            .on_drag(self.scroll.on_drag())
            .on_drag_end(self.scroll.on_drag_end())
            .on_extents(self.scroll.on_extents())
            .child(
                Container::new()
                    .color(GROUND)
                    .padding(EdgeInsets::symmetric(gutter, 0.0))
                    .child(
                        Center::new().child(
                            Constrained::new(Constraints::new(0.0, column, 0.0, f32::INFINITY))
                                .child(page),
                        ),
                    ),
            );

        // The bar is a **sibling** drawn over the scrollable, not a wrapper
        // around it: that is what lets it overlay the last few points of the
        // page without taking a column away from the content.
        Stack::new()
            .fit(StackFit::Expand)
            .children(children![
                scrollable,
                Positioned::new().left(0.0).right(0.0).top(0.0).child(nav(
                    &self.scroll,
                    &self.anchors,
                    column,
                    wide,
                    self.host
                ),),
                Positioned::new()
                    .right(2.0)
                    .top(NAV_HEIGHT)
                    .bottom(2.0)
                    .child(SizedBox::width(BAR_WIDTH).child(Scrollbar {
                        scroll: self.scroll.clone(),
                    }),),
            ])
            .into()
    }
}

widget_node_from!(Site);

// ─── syntax colours ───────────────────────────────────────────────────────
//
// `apps/viewwstudio/src/theme.rs`'s dark `Syntax`, value for value. The code on
// this page should look like the code in the editor it is selling; a second
// palette invented here would be a screenshot of a different product.

/// `.vw-code .tok-kw`, `oklch(0.82 0.15 320)`.
const SYN_KEYWORD: Color = Color::rgb(0xEF, 0xA3, 0xFF);
/// `.tok-type`, `oklch(0.82 0.13 220)`.
const SYN_TYPE: Color = Color::rgb(0x48, 0xD7, 0xFE);
/// `.tok-str`, `oklch(0.78 0.15 155)`.
const SYN_STRING: Color = Color::rgb(0x59, 0xD3, 0x8C);
/// `.tok-num`, `oklch(0.8 0.16 16)`.
const SYN_NUMBER: Color = Color::rgb(0xFF, 0x8F, 0x9A);
/// `.tok-com`, `oklch(0.6 0.01 60)`.
const SYN_COMMENT: Color = Color::rgb(0x85, 0x7F, 0x7A);
/// `.tok-mac`, `oklch(0.82 0.13 65)`.
const SYN_MACRO: Color = Color::rgb(0xFE, 0xB2, 0x63);
/// `.tok-fn`, `oklch(0.85 0.13 65)`.
const SYN_FUNCTION: Color = Color::rgb(0xFF, 0xBB, 0x6D);
/// `.tok-punc`, `oklch(0.7 0.01 60)`.
const SYN_PUNCT: Color = Color::rgb(0xA3, 0x9D, 0x98);

/// The words the highlighter colours as keywords.
///
/// Rust's, plus the two Say lines this page shows. Not a complete Rust keyword
/// list and not trying to be: this highlights *these samples*, and a token it
/// does not know falls through to punctuation rather than to a wrong colour.
const KEYWORDS: &[&str] = &[
    "use", "pub", "fn", "let", "impl", "struct", "enum", "mut", "const", "for", "in", "if", "else",
    "match", "return", "self", "Self", "as", "move", "where", "trait", "true", "false",
    // Say, and only its *structural* words. Its whole point is that it reads
    // as English, so colouring "a", "the" and "to" as keywords turns a sentence
    // into a ransom note — which is what the first pass at this list did.
    "keep", "screen", "when", "tapped", "add", "starting", "which",
];

/// One line of source, as coloured spans.
///
/// A hand-rolled tokeniser rather than the studio's `highlight.rs`, which lives
/// in a binary crate this page must not depend on. It knows five things —
/// comments, strings, numbers, identifiers and everything else — which is
/// exactly enough for the two samples below and honestly less than the editor
/// does. Anything it cannot classify is punctuation, never a guess.
pub(crate) fn highlight(line: &str, size: f32) -> Vec<Span> {
    // **Tokenised once per line, not once per frame.**
    //
    // Every build of this page re-declares its whole widget tree — that is what
    // makes a vieww widget cheap — and a scroll rebuilds on every frame. Doing
    // the tokenising there put 5 ms of string scanning into a 16 ms budget the
    // moment syntax colouring was added: the page's pipeline went from 1.4 ms
    // to 6.2 ms and the profile pointed straight here. The samples are
    // `&'static str` constants that never change, so the answer is computed
    // once and cloned after — a clone of a handful of `Span`s, against a walk
    // over the line and a `String` per token.
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static CACHE: RefCell<HashMap<(String, u32), Vec<Span>>> =
            RefCell::new(HashMap::new());
    }
    let key = (line.to_owned(), size.to_bits());
    if let Some(cached) = CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return cached;
    }
    let spans = highlight_uncached(line, size);
    CACHE.with(|cache| {
        cache.borrow_mut().insert(key, spans.clone());
    });
    spans
}

/// The tokeniser itself. See [`highlight`], which is what callers want.
fn highlight_uncached(line: &str, size: f32) -> Vec<Span> {
    let base = mono(size, SYN_PUNCT);
    let mut spans: Vec<Span> = Vec::new();
    let bytes: Vec<char> = line.chars().collect();
    let mut index = 0;

    // A `//` anywhere outside a string makes the rest of the line a comment,
    // and this tokeniser walks left to right, so reaching one ends the walk.
    while index < bytes.len() {
        let character = bytes[index];

        if character == '/' && bytes.get(index + 1) == Some(&'/') {
            let rest: String = bytes[index..].iter().collect();
            spans.push(Span::new(rest).style(mono(size, SYN_COMMENT)));
            break;
        }

        if character == '"' {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end] != '"' {
                // A backslash escapes the next character, including a quote —
                // without this, `"a \" b"` ends the string in the wrong place
                // and the rest of the line is coloured as code.
                if bytes[end] == '\\' {
                    end += 1;
                }
                end += 1;
            }
            let text: String = bytes[index..(end + 1).min(bytes.len())].iter().collect();
            spans.push(Span::new(text).style(mono(size, SYN_STRING)));
            index = (end + 1).min(bytes.len());
            continue;
        }

        if character.is_ascii_digit() {
            let mut end = index;
            while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == '.') {
                end += 1;
            }
            let text: String = bytes[index..end].iter().collect();
            spans.push(Span::new(text).style(mono(size, SYN_NUMBER)));
            index = end;
            continue;
        }

        if character.is_alphabetic() || character == '_' {
            let mut end = index;
            while end < bytes.len() && (bytes[end].is_alphanumeric() || bytes[end] == '_') {
                end += 1;
            }
            let word: String = bytes[index..end].iter().collect();
            let next = bytes.get(end).copied();
            let colour = if next == Some('!') {
                SYN_MACRO
            } else if KEYWORDS.contains(&word.as_str()) {
                SYN_KEYWORD
            } else if word.starts_with(char::is_uppercase) {
                SYN_TYPE
            } else if next == Some('(') {
                SYN_FUNCTION
            } else {
                SYN_PUNCT
            };
            spans.push(Span::new(word).style(mono(size, colour)));
            index = end;
            continue;
        }

        // Runs of everything else in one span, so a line of `.child(` is not
        // six spans and six glyph runs.
        let mut end = index;
        while end < bytes.len()
            && !bytes[end].is_alphanumeric()
            && bytes[end] != '_'
            && bytes[end] != '"'
            && !(bytes[end] == '/' && bytes.get(end + 1) == Some(&'/'))
        {
            end += 1;
        }
        let text: String = bytes[index..end.max(index + 1)].iter().collect();
        spans.push(Span::new(text).style(base));
        index = end.max(index + 1);
    }

    if spans.is_empty() {
        // An empty line still needs a span, or the row collapses and the block
        // loses its blank lines.
        spans.push(Span::new(" ").style(base));
    }
    spans
}

// ─── the pictures ─────────────────────────────────────────────────────────

/// The screenshots, decoded once.
///
/// Held rather than decoded per build for the obvious reason — a PNG decode in
/// a `build` would run on every frame of a scroll — and passed down the tree by
/// clone, which is an `Arc` bump: `vieww_foundation::Image` is reference
/// counted precisely so a decoded picture can be handed around a widget tree
/// without copying its pixels.
///
/// Every field is an `Option`: a picture that failed to decode leaves its
/// section out of the page rather than taking the page down with it.
#[derive(Debug, Clone, Default)]
pub struct Art {
    /// The studio itself — editor, device preview, Problems panel.
    pub studio: Option<ImageData>,
    /// 28 blend modes over a checkerboard, from `examples/fixtures`.
    pub blend: Option<ImageData>,
    /// One frame of the animation showcase, same source.
    pub anim: Option<ImageData>,
}

impl Art {
    /// Decode the three pictures compiled into this binary.
    ///
    /// `include_bytes!` rather than a fetch: they are 55 KB together, which is
    /// less than the round trips would cost, and a page whose hero image
    /// arrives a second after the hero is worse than one that ships with it.
    #[must_use]
    pub fn embedded() -> Self {
        fn decode(bytes: &[u8]) -> Option<ImageData> {
            vieww_asset::decode(bytes).ok().map(|(image, _)| image)
        }
        // `VIEWWSITE_NO_ART=1` renders the page without its pictures. Only
        // `examples/bench` sets it, and only to answer "how much of a frame is
        // the screenshot" — which is a question worth being able to ask again.
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var("VIEWWSITE_NO_ART").is_ok() {
            return Self::default();
        }
        Self {
            studio: decode(include_bytes!("../assets/studio.png")),
            // The two renderer figures, generated by `examples/figures.rs`.
            // They replaced `blend.png` and `anim.png`, which came out of
            // `examples/fixtures` — correctness pictures, and the wrong thing
            // to put on a product page. See that example's module docs.
            blend: decode(include_bytes!("../assets/figure-blend.png")),
            anim: decode(include_bytes!("../assets/figure-easing.png")),
        }
    }
}

/// The bar across the top, which does not scroll away.
///
/// # Why a canvas has to build this by hand
///
/// `position: sticky` is a CSS declaration; here it is a *sibling in a stack*.
/// The page scrolls underneath it and the bar is drawn over the top, which is
/// the same arrangement as the scrollbar and for the same reason. Its links
/// steer the same [`ScrollController`] the wheel does, so a click and a drag
/// cannot disagree about where the page is.
///
/// The section offsets are constants rather than measurements. Measuring them
/// would mean reporting each section's laid-out position back up out of layout,
/// which is a signal per section and a rebuild of this bar every time any of
/// them moves. For a page whose section order changes when somebody edits this
/// file, a table beside the sections is the honest trade — and `jump_to` clamps,
/// so a stale number lands at the end rather than off it.
fn nav(
    scroll: &ScrollController,
    anchors: &Anchors,
    column: f32,
    wide: bool,
    host: Option<Os>,
) -> WidgetNode {
    let mut items: Vec<WidgetNode> = vec![Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .spacing(9.0)
        .children(children![
            SizedBox::from_size(Size::new(20.0, 20.0)).child(mark(20.0)),
            Text::new("vieww Studio").size(14.0).color(INK).bold(),
            chip("0.1.0", INK_3),
        ])
        .into()];

    if wide {
        let mut links: Vec<WidgetNode> = Vec::new();
        for (index, label) in SECTIONS.iter().enumerate() {
            let target = scroll.clone();
            let anchors = anchors.clone();
            links.push(
                CursorArea::new(Cursor::Pointer)
                    .child(
                        Pressable::sensed(move |sense: Sense| {
                            Container::new()
                                .radius(6.0)
                                .padding(EdgeInsets::symmetric(9.0, 5.0))
                                .color(if sense.emphasis() > 0.0 {
                                    SURFACE
                                } else {
                                    GROUND
                                })
                                .child(
                                    Text::new((*label).to_string())
                                        .size(12.5)
                                        .color(if sense.hover > 0.0 { INK } else { INK_2 }),
                                )
                                .into()
                        })
                        .on_tap(move || target.jump_to(anchors.target(index))),
                    )
                    .into(),
            );
        }
        items.push(
            Flexible::expanded(1)
                .child(
                    Flex::row()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .spacing(2.0)
                        .children(links),
                )
                .into(),
        );
    } else {
        items.push(Flexible::expanded(1).child(SizedBox::shrink()).into());
    }

    if let Some(os) = host {
        let asset = ASSETS
            .iter()
            .find(|asset| asset.os == os)
            .unwrap_or(&ASSETS[0]);
        let target = url_for(asset.file);
        items.push(
            CursorArea::new(Cursor::Pointer)
                .child(
                    Button::new(if wide { "Download" } else { "Get it" })
                        .style(ButtonStyle::Filled)
                        .on_pressed(move || navigate(&target)),
                )
                .into(),
        );
    }

    Container::new()
        // Opaque, and it has to be: the page scrolls *under* this, and a
        // translucent bar over a canvas would need the frame behind it, which
        // is a blur this renderer does not do cheaply.
        .color(GROUND)
        .border(Border::thin(LINE))
        .padding(EdgeInsets::symmetric(0.0, 8.0))
        .child(
            Center::new().child(
                Constrained::new(Constraints::new(0.0, column, 0.0, f32::INFINITY)).child(
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .spacing(12.0)
                        .children(items),
                ),
            ),
        )
        .into()
}

/// The bar's height, which is also the top inset the page is given so its first
/// line is not born underneath it.
const NAV_HEIGHT: f32 = 56.0;

// ─── icons ────────────────────────────────────────────────────────────────
//
// Centreline paths from `apps/viewwstudio/src/ui/icons.rs`, drawn as strokes by
// `Icon::stroke`. Copied rather than imported for the same reason the scrollbar
// was: that module is inside a binary crate. They are the studio's own shapes,
// so a feature card on this page and the activity bar in the screenshot above
// it are drawing the same icon.

/// One 24x24 centreline icon, parsed once and cached by its own path string.
///
/// Keyed by the `d` string rather than by a name, so editing a path is a
/// different key and the cache cannot go stale against the source.
fn icon(data: &'static str) -> IconData {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static PARSED: RefCell<HashMap<&'static str, IconData>> =
            RefCell::new(HashMap::new());
    }
    PARSED.with(|parsed| {
        if let Some(hit) = parsed.borrow().get(data) {
            return hit.clone();
        }
        // A path that does not parse becomes an empty one rather than a panic:
        // a missing icon is a blank square on a page, not a reason to take the
        // page down.
        let built = vieww::foundation::parse_path_data(data)
            .map_or_else(|_| IconData::square24(Path::new()), IconData::square24);
        parsed.borrow_mut().insert(data, built.clone());
        built
    })
}

/// The weight every icon on this page is drawn at. One number, so the set stays
/// even at every size — which is the whole argument for centrelines over filled
/// shapes.
const ICON_WEIGHT: f32 = 1.6;

/// An icon at the page's own weight and size.
fn glyph(data: &'static str, size: f32, color: Color) -> WidgetNode {
    Icon::new(icon(data))
        .size(size)
        .color(color)
        .stroke(ICON_WEIGHT)
        .into()
}

/// `ui::icons::preview` — a phone in a frame.
const I_PREVIEW: &str = "M8.4 3.6h7.2v16.8H8.4zM10.6 5.6h2.8";
/// `ui::icons::code` — angle brackets.
const I_CODE: &str = "M9.2 7.4 3.9 12l5.3 4.6M14.8 7.4 20.1 12l-5.3 4.6";
/// `ui::icons::warning` — the Problems panel's triangle.
const I_WARNING: &str = "M12 4.6 21 19.4H3zM12 10v4M12 16.6v.2";
/// `ui::icons::search`.
const I_SEARCH: &str = "M16.6 10.4a6.2 6.2 0 1 1-12.4 0 6.2 6.2 0 0 1 12.4 0M14.9 14.9l5 5";
/// `ui::icons::book` — the Say guide.
const I_BOOK: &str = "M4 5.2h6.4a1.6 1.6 0 0 1 1.6 1.6v12a1.6 1.6 0 0 0-1.6-1.6H4zM20 \
                      5.2h-6.4a1.6 1.6 0 0 0-1.6 1.6v12a1.6 1.6 0 0 1 1.6-1.6H20z";
/// `ui::icons::shield` — recovery and autosave.
const I_SHIELD: &str = "M12 3.6 19.4 6v6.4c0 4-3.1 6.9-7.4 8-4.3-1.1-7.4-4-7.4-8V6z";
/// `ui::icons::swatch` — the studio drawing itself.
const I_SWATCH: &str = "M4.6 19.4V6a1.4 1.4 0 0 1 1.4-1.4h5.4V18a1.4 1.4 0 0 1-1.4 \
                        1.4zM11.4 12.6 15.2 8.8M11.4 19.4h6.6a1.4 1.4 0 0 0 1.4-1.4v-3.6";
/// `ui::icons::export` — a download arrow.
const I_DOWNLOAD: &str = "M12 4v11M8 11.4l4 4 4-4M4.6 19.4h14.8";

// ─── where the sections actually are ──────────────────────────────────────

/// The sections the nav can jump to, in page order.
///
/// The label is what the nav shows and the index is the slot each section
/// writes its measured position into. One list, so a link and a section cannot
/// drift apart the way they did when the nav held guessed fractions.
pub const SECTIONS: &[&str] = &["Live demo", "The editor", "Code", "Renderer", "Download"];

/// Where each section starts, in document coordinates, once it has been laid
/// out. `0.0` until then.
///
/// # Why this is measured and not calculated
///
/// The nav used to hold a table of fractions — `("Code", 0.46)` — because a
/// widget cannot know its own position: `build` runs before layout, and layout
/// hands a size *up* while the parent assigns position afterwards. The
/// fractions were a guess, they were wrong at every window size, and clicking a
/// link landed somewhere near the section rather than on it. That is the bug
/// this replaces.
///
/// [`Measured`] is the way the answer gets back into the tree. It reports a
/// **global** rectangle — where the section is on screen right now — so the
/// document position is that plus however far the page is already scrolled.
/// It reports only when the rectangle changes, so this does not write a signal
/// every frame.
#[derive(Debug, Clone)]
pub struct Anchors {
    tops: Vec<Signal<f32>>,
    spots: Vec<Signal<f32>>,
}

/// How many blocks on the page can be revealed on scroll. One more than the
/// page currently uses, so adding a band does not mean rewiring this.
const REVEALS: usize = 16;

/// Over how many points of scroll a block goes from invisible to arrived.
///
/// Scroll-linked rather than timed: the scrollable already eases a wheel notch
/// over a couple of hundred milliseconds, so a band this wide reads as the
/// same 0.6s fade the CSS version runs — without a second clock, a ticker per
/// block, or a fade that finishes while the block is still moving.
const REVEAL_BAND: f32 = 150.0;

/// The fraction of the window a block's top has to cross before it starts.
///
/// Slightly inside the bottom edge, so a block begins arriving just before it
/// would otherwise appear rather than a moment after.
const REVEAL_LINE: f32 = 0.92;

/// One block of the page, faded and lifted by where the page is scrolled to.
///
/// This is `Reveal` from `src/components/primitives.tsx` — `opacity 0 → 1` and
/// `y 18 → 0` on an ease-out — with the `IntersectionObserver` replaced by
/// arithmetic on the scroll offset, which this page already has and a canvas
/// does not get for free.
#[derive(Debug, Clone)]
struct Reveal {
    scroll: ScrollController,
    /// Where this block sits in the document, written by the [`Measured`] that
    /// wraps it. Zero until the first layout has reported.
    top: Signal<f32>,
    child: WidgetNode,
}

impl Widget for Reveal {
    fn debug_name(&self) -> &'static str {
        "Reveal"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Both reads subscribe: this element, and nothing above it, rebuilds
        // as the page moves.
        let top = self.top.get();
        let offset = self.scroll.offset();
        let viewport = self.scroll.viewport();

        // Before the first layout there is no viewport and no measurement, and
        // the honest answer is "arrived" — a page that starts blank because it
        // has not measured itself yet is worse than one that never animated.
        let progress = if viewport <= 0.0 {
            1.0
        } else {
            ((offset + viewport * REVEAL_LINE - top) / REVEAL_BAND).clamp(0.0, 1.0)
        };
        if progress >= 1.0 {
            return self.child.clone();
        }

        // Ease out cubic — the same shape as the CSS
        // `cubic-bezier(0.22, 1, 0.36, 1)`, close enough that the difference
        // is not visible at this distance and cheap enough to be free.
        let eased = 1.0 - (1.0 - progress).powi(3);
        Opacity::new(eased)
            .child(
                Transformed::translate(Offset::new(0.0, (1.0 - eased) * 18.0))
                    .child(self.child.clone()),
            )
            .into()
    }
}

widget_node_from!(Reveal);

impl Anchors {
    /// One signal per section, on this tree's runtime, plus a pool of slots
    /// for the scroll reveals — which are the same measurement (a block's
    /// position in the document) put to a different use.
    #[must_use]
    pub fn new(runtime: &vieww::element::Runtime) -> Self {
        Self {
            tops: SECTIONS.iter().map(|_| runtime.signal(0.0_f32)).collect(),
            spots: (0..REVEALS).map(|_| runtime.signal(0.0_f32)).collect(),
        }
    }

    /// Wrap `node` so it fades and lifts in as it scrolls into view.
    ///
    /// Two pieces: a [`Measured`] that writes the block's document position
    /// into `spots[index]` once, and a [`Reveal`] that reads that position and
    /// the live scroll offset. Only the `Reveal` subscribes to the scroll, so a
    /// scroll rebuilds twelve small wrappers rather than the page — and a
    /// wrapper whose progress has reached 1.0 returns its child untouched, so
    /// the finished page carries no compositing layers at all.
    fn reveal(&self, index: usize, scroll: &ScrollController, node: WidgetNode) -> WidgetNode {
        let Some(slot) = self.spots.get(index).cloned() else {
            return node;
        };
        let writer = slot.clone();
        let peeker = scroll.clone();
        Measured::new()
            .on_measured(std::rc::Rc::new(move |rect: Rect| {
                writer.set(rect.top + peeker.peek());
            }))
            .child(Reveal {
                scroll: scroll.clone(),
                top: slot,
                child: node,
            })
            .into()
    }

    /// Wrap a section so it reports where it ends up.
    fn mark(&self, index: usize, scroll: &ScrollController, node: WidgetNode) -> WidgetNode {
        let Some(slot) = self.tops.get(index).cloned() else {
            return node;
        };
        let scroll = scroll.clone();
        Measured::new()
            .on_measured(std::rc::Rc::new(move |rect: Rect| {
                // `peek`, not `offset`: this runs inside layout's report, and
                // reading a signal there would subscribe the *measuring*
                // element to the scroll offset — a rebuild on every frame of
                // every scroll, which is the loop this page cannot afford.
                slot.set(rect.top + scroll.peek());
            }))
            .child(node)
            .into()
    }

    /// Where the nav should scroll to for `index`, allowing for the bar that
    /// would otherwise cover the heading it just jumped to.
    fn target(&self, index: usize) -> f32 {
        self.tops
            .get(index)
            .map_or(0.0, |slot| (slot.get() - NAV_HEIGHT - 12.0).max(0.0))
    }
}

// ─── the scrollbar ────────────────────────────────────────────────────────

/// A scrollbar, because a canvas does not get one.
///
/// # Why this is here at all
///
/// The page scrolls a widget tree, not the document — the document is exactly
/// the height of the window and has nothing to scroll — so the browser draws no
/// scrollbar and there is nothing on screen saying how long the page is or
/// where in it you are. Every canvas application ends up drawing its own for
/// the same reason.
///
/// This is `apps/viewwstudio/src/ui/scrollbar.rs`, narrowed to one axis and
/// with the studio's theme swapped for this page's colours. It is a port rather
/// than a dependency because that module lives in a binary crate — and it is
/// the strongest argument in this file for promoting that widget into
/// `vieww-widget`, where both could use one copy.
///
/// The geometry comes from the same [`ScrollController`] the `Scrollable`
/// reads, so the two cannot disagree about where the content is.
const BAR_WIDTH: f32 = 12.0;
/// The thumb inside the track. Narrower than the track so the grab area is
/// bigger than the thing you are grabbing.
const THUMB_WIDTH: f32 = 6.0;
/// Below this the thumb stops shrinking and only its *position* still means
/// anything — the trade every scrollbar makes on a long page.
const MIN_THUMB: f32 = 32.0;

#[derive(Debug)]
struct Scrollbar {
    scroll: ScrollController,
}

impl Widget for Scrollbar {
    fn debug_name(&self) -> &'static str {
        "Scrollbar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let scroll = self.scroll.clone();
        // Read through `offset()`, not `peek()`: this is the subscription that
        // rebuilds the thumb as the content moves. With `peek` the bar is drawn
        // once and then lies for the rest of the session.
        let offset = scroll.offset();
        let viewport = scroll.viewport();
        let max = scroll.max_offset();

        // Nothing to steer — a window taller than the page. A track that says
        // "there is more" when there is not is worse than no track.
        if max <= 0.5 || viewport <= 0.0 {
            return SizedBox::width(BAR_WIDTH).into();
        }
        let content = viewport + max;

        LayoutBuilder::new(move |constraints: Constraints| {
            let track = constraints.max_height;
            if !track.is_finite() || track <= 0.0 {
                return SizedBox::shrink().into();
            }

            let thumb = ((viewport / content) * track).clamp(MIN_THUMB.min(track), track);
            // The divisor is the *scrollable* extent, not the content: at
            // `max_offset` the thumb's far edge is the track's far edge.
            let travel = (track - thumb).max(0.0);
            let at = (offset / max).clamp(0.0, 1.0) * travel;
            // A drag of `d` moves the content by `d × max/travel`: crossing the
            // whole track scrolls the whole page. That ratio is the whole of a
            // scrollbar — without it the gesture is a nudge rather than "point
            // at the part you want".
            let scale = if travel > 0.0 { max / travel } else { 0.0 };

            let dragging = scroll.clone();
            let paging = scroll.clone();

            // **A column, not padding.** `Positioned` with both `top` and
            // `bottom` hands this subtree a *tight* vertical constraint, and a
            // `Container` passes that straight down — so a thumb given an
            // explicit height was stretched to fill whatever the padding left,
            // and its bottom edge was always the bottom of the track however
            // far the page had scrolled. A `Flex` gives each child the height
            // it asks for and the trailing `Flexible` absorbs the remainder.
            let bar = Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(children![
                    SizedBox::height(at),
                    Container::new()
                        .color(Color::rgb(0x4A, 0x46, 0x55))
                        .radius(THUMB_WIDTH / 2.0)
                        .size(THUMB_WIDTH, thumb),
                    Flexible::expanded(1).child(SizedBox::shrink()),
                ]);

            GestureDetector::new()
                // Zero, because the track is twelve points wide and a touch
                // target grown around it would swallow presses meant for the
                // page beside it.
                .touch_target(0.0)
                .drag_axis(Axis::Vertical)
                .on_drag_update(move |details: vieww::foundation::DragDetails| {
                    dragging.jump_to(dragging.peek() + details.delta.dy * scale);
                })
                .on_tap_down(move |details| {
                    // A press on the track pages towards it. The thumb's own
                    // presses land here too and must move nothing, which is
                    // what the comparison against `at` is for.
                    let point = details.local.dy;
                    if point < at {
                        paging.jump_to(paging.peek() - viewport);
                    } else if point > at + thumb {
                        paging.jump_to(paging.peek() + viewport);
                    }
                })
                .child(
                    // A transparent fill still hit tests, which makes the whole
                    // track clickable without drawing a groove.
                    Container::new().color(Color::rgba(0, 0, 0, 1)).child(bar),
                )
                .into()
        })
        .into()
    }
}

widget_node_from!(Scrollbar);

// ─── the entrance ─────────────────────────────────────────────────────────

/// One element's slice of the page's single entrance animation.
///
/// `t` is the whole animation, `delay` where this element starts and `0.30`
/// how long it takes — so an element at `delay = 0.15` is still at zero when
/// the one before it is half way, which is what reads as a sequence rather
/// than as everything arriving at once.
///
/// One animation and arithmetic, rather than one `Animation` per element:
/// every ticker is a signal to poll and a reason to draw a frame, and eight of
/// them expressing one idea is eight times the bookkeeping for the same
/// second of motion.
fn stage(t: f32, delay: f32) -> f32 {
    ((t - delay) / 0.30).clamp(0.0, 1.0)
}

/// Fade and lift `node` in, at its own moment.
///
/// The lift is a *transform*, not padding: a transform is applied when the
/// subtree is painted, so nothing above it is laid out again on any of the
/// sixty frames this runs for. Animating a margin here would relayout the page
/// per frame to move a heading twelve pixels.
fn enter(t: f32, delay: f32, node: WidgetNode) -> WidgetNode {
    let progress = stage(t, delay);
    if progress >= 1.0 {
        // The finished state is the node itself, with no `Opacity` layer and no
        // transform left in the tree — an entrance that leaves scaffolding
        // behind makes every later frame composite through it.
        return node;
    }
    Opacity::new(progress)
        .child(Transformed::translate(Offset::new(0.0, (1.0 - progress) * 14.0)).child(node))
        .into()
}

// ─── pieces ───────────────────────────────────────────────────────────────

/// A one-pixel rule the width of the column.
fn divider(column: f32) -> WidgetNode {
    Container::new()
        .color(LINE)
        .child(SizedBox::from_size(Size::new(column, 1.0)))
        .into()
}

/// The pill that opens each section.
///
/// `SectionHeading`'s eyebrow: a rounded-full chip with a dot, a 1px accent
/// border at 25% and an accent wash behind it, mono, uppercase, tracked out.
/// It was a bare line of text with a square beside it, which is the same
/// information and none of the emphasis.
fn eyebrow(text: &str) -> WidgetNode {
    Container::new()
        .color(WASH)
        .radius(999.0)
        .border(Border::thin(ACCENT_DEEP))
        .padding(EdgeInsets::symmetric(12.0, 5.0))
        .child(
            Flex::row()
                .main_axis_size(MainAxisSize::Min)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(8.0)
                .children(children![
                    Container::new().color(ACCENT).radius(999.0).size(6.0, 6.0),
                    Text::new(spaced(text)).style(mono(10.5, ACCENT)),
                ]),
        )
        .into()
}

/// A string with a hair space between every character.
///
/// The section labels are set this way, so they read as labels at eleven
/// pixels rather than as small body text.
fn spaced(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for (index, character) in text.chars().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push(character);
    }
    out
}

/// The mark, in the fractions `apps/viewwstudio/src/ui/brand.rs` draws it in:
/// ground at 0.22 radius, the editor panel at (.16, .20) 40x60, the preview
/// panel at (.44, .32) 40x48. One shape, and this is the fifth place it is
/// drawn — the icon, the title bar, the splash, the installer, and here.
fn mark(side: f32) -> WidgetNode {
    let unit = side / 100.0;
    Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            Container::new()
                .color(Color::rgb(0x14, 0x16, 0x1A))
                .radius(22.0 * unit)
                .child(SizedBox::from_size(Size::new(side, side))),
            Positioned::new().left(16.0 * unit).top(20.0 * unit).child(
                Container::new()
                    .color(Color::rgb(0x46, 0x4E, 0x5E))
                    .radius(8.0 * unit)
                    .child(SizedBox::from_size(Size::new(40.0 * unit, 60.0 * unit))),
            ),
            Positioned::new().left(44.0 * unit).top(32.0 * unit).child(
                Container::new()
                    .color(ACCENT_DEEP)
                    .radius(8.0 * unit)
                    .child(SizedBox::from_size(Size::new(40.0 * unit, 48.0 * unit))),
            ),
        ])
        .into()
}

/// A small outlined label: a version, a status, a file kind.
fn chip(text: &str, ink: Color) -> WidgetNode {
    Container::new()
        .color(SURFACE)
        .radius(999.0)
        .border(Border::thin(LINE))
        .padding(EdgeInsets::symmetric(9.0, 3.0))
        .child(Text::new(text.to_string()).style(mono(11.0, ink)))
        .into()
}

/// The hero: centred, and built around the headline.
///
/// # Why this is centred when the rest of the page is not
///
/// It was a two-column band — copy left, the live demo right — which is a
/// perfectly good shape and the wrong one for the top of a product page. The
/// first screenful has one job: say what this is, in the fewest words, at a
/// size nobody can miss. Everything competing with the headline for that job
/// has been moved below it, which is the composition every editor's front page
/// converges on because it is the one that works.
///
/// The live demo did not lose anything by moving: it is now its own band with
/// the facts beside it, where it reads as a demonstration rather than as
/// decoration in the margin.
fn hero(column: f32, wide: bool, host: Option<Os>, entrance: f32) -> WidgetNode {
    let lede = "vieww follows three-tree architecture — Widget → Element → \
                RenderObject — with a renderer that goes all the way down to the \
                pixels. viewwstudio is the desktop editor and device-framed \
                preview for the screens you write.";

    // `text-4xl sm:text-6xl md:text-7xl` — 36, 60, 72. Mine was 64 at the top
    // end and 36 at the bottom, which is close at desktop and a whole step
    // small everywhere else.
    let headline = if wide { 72.0 } else { 36.0 };
    let mark_side = if wide { 84.0 } else { 64.0 };

    // The status pill: a live dot, the version, and the one claim that is worth
    // making before the headline does its work.
    let pill = Container::new()
        .color(SURFACE)
        .radius(999.0)
        .border(Border::thin(LINE))
        .padding(EdgeInsets::symmetric(14.0, 6.0))
        .child(
            Flex::row()
                .main_axis_size(MainAxisSize::Min)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(9.0)
                .children(children![
                    Container::new()
                        .color(Color::rgb(0x3F, 0xB9, 0x50))
                        .radius(999.0)
                        .size(7.0, 7.0),
                    Text::new("v0.1.0").style(mono(11.5, INK)),
                    Text::new("·").style(mono(11.5, INK_3)),
                    Text::new(if wide {
                        "Renders on real Android and iOS hardware"
                    } else {
                        "Android and iOS"
                    })
                    .style(mono(11.5, INK_2)),
                ]),
        );

    Container::new()
        .padding(EdgeInsets::only(
            0.0,
            // `pt-28 sm:pt-36` over the nav, `pb-16 sm:pb-24` under.
            if wide { 96.0 } else { 72.0 },
            0.0,
            if wide { 96.0 } else { 64.0 },
        ))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(0.0)
                .children(children![
                    enter(
                        entrance,
                        0.02,
                        SizedBox::from_size(Size::new(mark_side, mark_side))
                            .child(mark(mark_side))
                            .into()
                    ),
                    SizedBox::height(22.0),
                    enter(entrance, 0.10, pill.into()),
                    SizedBox::height(if wide { 30.0 } else { 24.0 }),
                    enter(
                        entrance,
                        0.16,
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .spacing(2.0)
                            .children(children![
                                Text::new("A UI framework in Rust.")
                                    .size(headline)
                                    .color(INK)
                                    .bold()
                                    .align(TextAlign::Center),
                                Text::new("Three trees, one job each.")
                                    .size(headline)
                                    .color(ACCENT)
                                    .bold()
                                    .align(TextAlign::Center),
                            ])
                            .into()
                    ),
                    SizedBox::height(24.0),
                    enter(
                        entrance,
                        0.26,
                        Constrained::new(Constraints::new(
                            0.0,
                            column.min(660.0),
                            0.0,
                            f32::INFINITY
                        ))
                        .child(
                            // `text-lg sm:text-xl` — 18, then 20.
                            Text::new(lede)
                                .size(if wide { 20.0 } else { 17.0 })
                                .color(INK_2)
                                .align(TextAlign::Center)
                        )
                        .into()
                    ),
                    SizedBox::height(30.0),
                    enter(entrance, 0.34, download_button(host, column)),
                ]),
        )
        .into()
}

/// The live demo and the four facts, as their own band under the hero.
fn demo_band(column: f32, wide: bool, count: &Signal<i32>, entrance: f32) -> WidgetNode {
    let body: WidgetNode = if wide {
        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .spacing(40.0)
            .children(children![
                Flexible::expanded(1).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(14.0)
                        .children(children![
                            eyebrow("LIVE DEMO"),
                            Text::new("This is not a screenshot.")
                                .size(24.0)
                                .color(INK)
                                .bold(),
                            Constrained::new(Constraints::new(0.0, PROSE, 0.0, f32::INFINITY))
                                .child(
                                    Text::new(
                                        "The screen beside this is a vieww widget tree, \
                                         running in your browser: a Signal, a Button and a \
                                         Text, compiled to WebAssembly and drawn by vieww's \
                                         own rasteriser. Tap it. Everything else on this \
                                         page is drawn the same way."
                                    )
                                    .size(14.5)
                                    .color(INK_2)
                                ),
                            facts(wide),
                        ])
                ),
                demo(count),
            ])
            .into()
    } else {
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(20.0)
            .children(children![
                eyebrow("LIVE DEMO"),
                Text::new("This is not a screenshot.")
                    .size(21.0)
                    .color(INK)
                    .bold(),
                Text::new(
                    "The screen below is a vieww widget tree, running in your \
                     browser. Tap it."
                )
                .size(14.0)
                .color(INK_2),
                demo(count),
                facts(wide),
            ])
            .into()
    };

    let _ = column;
    Container::new()
        .padding(EdgeInsets::symmetric(0.0, 44.0))
        .child(enter(entrance, 0.42, body))
        .into()
}

/// The live demo: a vieww screen, in a device frame, on the page.
///
/// This is the same screen the code sample further down builds, running. It is
/// the one thing on the page that a screenshot could not have been, which is
/// why it is above the fold and why the caption under it says so plainly
/// rather than cleverly.
fn demo(count: &Signal<i32>) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(10.0)
        .children(children![
            Device {
                count: count.clone(),
            },
            Text::new("A real vieww screen, running here. Tap it.").style(mono(11.0, INK_3)),
        ])
        .into()
}

/// The device frame and the screen inside it.
///
/// A widget rather than a function because it **reads the counter**, and a
/// signal read is only a subscription inside a `build`. That is also what
/// keeps a tap cheap: the element that reads `count` is this one, so pressing
/// the button rebuilds a phone-sized subtree and leaves the rest of the page —
/// the tables, the code blocks, the eight cards — untouched.
#[derive(Debug)]
struct Device {
    count: Signal<i32>,
}

impl Widget for Device {
    fn debug_name(&self) -> &'static str {
        "Device"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let count = self.count.get();
        let bump = self.count.clone();
        let reset = self.count.clone();

        let screen = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(0.0)
            .children(children![
                // The status bar, which is the frame's own furniture rather
                // than the screen's: it is what makes 393x852 read as a phone.
                Container::new()
                    .padding(EdgeInsets::symmetric(16.0, 9.0))
                    .child(
                        Flex::row()
                            .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .children(children![
                                Text::new("9:41").style(mono(10.0, INK_2)),
                                Text::new("iOS  393x852").style(mono(10.0, INK_3)),
                            ])
                    ),
                Container::new().padding(EdgeInsets::all(18.0)).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(12.0)
                        .children(children![
                            Text::new("Counter").size(22.0).color(INK).bold(),
                            // The number, big, because it is the thing that
                            // changes and the reason to tap.
                            Text::new(count.to_string()).size(46.0).color(ACCENT).bold(),
                            Text::new(format!(
                                "Tapped {count} time{}",
                                if count == 1 { "" } else { "s" }
                            ))
                            .size(13.0)
                            .color(INK_2),
                            SizedBox::height(2.0),
                            CursorArea::new(Cursor::Pointer).child(
                                Button::new("Add one")
                                    .style(ButtonStyle::Filled)
                                    .on_pressed(move || bump.update(|value| *value += 1)),
                            ),
                            CursorArea::new(Cursor::Pointer).child(
                                Button::new("Reset")
                                    .style(ButtonStyle::Text)
                                    .on_pressed(move || reset.set(0)),
                            ),
                        ])
                ),
            ]);

        // Two containers: the body of the phone, and the screen inside its
        // bezel. The radii differ by the bezel width, which is what stops the
        // inner corners looking loose inside the outer ones.
        Container::new()
            .color(Color::rgb(0x08, 0x08, 0x0A))
            .radius(30.0)
            .border(Border::thin(LINE))
            .padding(EdgeInsets::all(9.0))
            .child(
                Container::new()
                    .color(SURFACE)
                    .radius(21.0)
                    .child(SizedBox::from_size(Size::new(258.0, 344.0)).child(screen)),
            )
            .into()
    }
}

widget_node_from!(Device);

/// The counter screen on its own, with no phone around it.
///
/// What the DOM page mounts into its `<canvas>` island: there the bezel is a
/// `border-radius` and a `box-shadow` on the element wrapping the canvas, so
/// the widget tree inside it is only the screen. Same tree, same signal, same
/// rasteriser as the private `Device` widget — the frame moved out to CSS,
/// where it belongs on
/// that target.
#[must_use]
pub fn counter_screen(count: Signal<i32>) -> WidgetNode {
    // Wrapped in the page's own theme: without it the `Button` inside picks up
    // `ThemeData`'s default primary, and the one control in the demo comes out
    // a different colour from every control around it — which reads as the
    // island being someone else's component rather than this page's.
    Theme::new(theme()).child(Device { count }).into()
}

/// The download buttons: the detected platform filled, the other two outlined.
///
/// One button was the first shape of this, and it hid the fact that the studio
/// ships for three platforms behind a user-agent guess — a Mac user on a
/// Windows machine, or anyone whose browser lies, saw a page that appeared to
/// offer one build. Showing all three and *emphasising* the guess says both
/// things at once, which is what every editor's download page does.
fn download_button(host: Option<Os>, available: f32) -> WidgetNode {
    /// Three buttons plus their gaps. Measured, not guessed: the overflow
    /// warning named it exactly.
    const ROW_WIDTH: f32 = 580.0;
    let side_by_side = available >= ROW_WIDTH;
    let each = [Os::Linux, Os::MacOs, Os::Windows];
    let mut buttons: Vec<WidgetNode> = Vec::new();

    for os in each {
        let asset = ASSETS
            .iter()
            .find(|asset| asset.os == os)
            .unwrap_or(&ASSETS[0]);
        let target = url_for(asset.file);
        let mine = host == Some(os);
        buttons.push(
            CursorArea::new(Cursor::Pointer)
                .child(
                    Button::new(format!("Download for {}", os.name()))
                        .style(if mine {
                            ButtonStyle::Filled
                        } else {
                            ButtonStyle::Outlined
                        })
                        .on_pressed(move || navigate(&target)),
                )
                .into(),
        );
    }

    let note = match host {
        Some(Os::MacOs) => {
            "Apple silicon. Intel, and every archive, are in the list below.".to_owned()
        }
        Some(os) => {
            let asset = ASSETS.iter().find(|a| a.os == os).unwrap_or(&ASSETS[0]);
            format!(
                "{} · {} · every build is in the list below",
                asset.file, asset.size
            )
        }
        None => "Pick a platform, or see every build in the list below.".to_owned(),
    };

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(10.0)
        .children(children![
            // Three buttons side by side need about 580 points. The hero's
            // copy column on a tablet is 430, so this cannot key off the page's
            // `wide` flag — it has to be the width this particular row was
            // actually given. Stacked, they are three full-width rows, which is
            // what a phone download page looks like anyway.
            if side_by_side {
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .spacing(8.0)
                    .children(buttons)
            } else {
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .spacing(8.0)
                    .children(buttons)
            },
            Text::new(note).style(mono(11.5, INK_3)),
        ])
        .into()
}

/// Four numbers a person deciding whether to download wants before they do.
fn facts(wide: bool) -> WidgetNode {
    let entries = [
        ("RUST NEEDED", "None. It carries its own"),
        ("PREVIEWS", "iOS, Android, Desktop"),
        ("SHIPS TO", "Desktop, Windows, APK, iOS"),
        ("LICENCE", "Apache-2.0"),
    ];
    let cells: Vec<WidgetNode> = entries
        .iter()
        .map(|(label, value)| {
            Flexible::expanded(1)
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(3.0)
                        .children(children![
                            Text::new(spaced(label)).style(mono(9.5, INK_3)),
                            Text::new((*value).to_string()).size(12.5).color(INK),
                        ]),
                )
                .into()
        })
        .collect();

    let row = if wide {
        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(16.0)
            .children(cells)
    } else {
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(12.0)
            .children(cells)
    };

    Container::new()
        .padding(EdgeInsets::only(0.0, 18.0, 0.0, 0.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .spacing(16.0)
                .children(children![
                    Container::new()
                        .color(LINE)
                        .child(SizedBox::height(1.0).child(SizedBox::shrink())),
                    row,
                ]),
        )
        .into()
}

/// A section: a pill eyebrow, a heading, a lead, and whatever follows.
///
/// Sizes and widths are `primitives.tsx`'s `SectionHeading` and `Section`,
/// converted: heading `md:text-[2.75rem]` at `max-w-3xl` with balanced wrap,
/// lead `sm:text-lg` at `max-w-2xl`, both centred, over `py-20 md:py-28`.
fn section(eyebrow_text: &str, heading: &str, body: Option<&str>, rest: WidgetNode) -> WidgetNode {
    section_at(eyebrow_text, heading, body, rest, true)
}

/// [`section`], with the width switch it needs for its own padding.
fn section_at(
    eyebrow_text: &str,
    heading: &str,
    body: Option<&str>,
    rest: WidgetNode,
    wide: bool,
) -> WidgetNode {
    let mut head: Vec<WidgetNode> = vec![
        eyebrow(eyebrow_text),
        SizedBox::height(6.0).into(),
        Constrained::new(Constraints::new(0.0, HEADING_MEASURE, 0.0, f32::INFINITY))
            .child(
                Text::new(heading.to_string())
                    .size(if wide { 44.0 } else { 30.0 })
                    .color(INK)
                    .bold()
                    .align(TextAlign::Center),
            )
            .into(),
    ];
    if let Some(body) = body {
        head.push(SizedBox::height(14.0).into());
        head.push(
            Constrained::new(Constraints::new(0.0, PROSE, 0.0, f32::INFINITY))
                .child(
                    Text::new(body.to_string())
                        .size(if wide { 18.0 } else { 15.5 })
                        .color(INK_2)
                        .align(TextAlign::Center),
                )
                .into(),
        );
    }

    Container::new()
        .padding(EdgeInsets::symmetric(
            0.0,
            if wide { SECTION_Y } else { SECTION_Y_NARROW },
        ))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(0.0)
                .children(children![
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(head),
                    SizedBox::height(44.0),
                    // The body of the section is full width and left-aligned;
                    // only the heading block is centred, which is what keeps a
                    // table or a grid from looking like a poster.
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(children![rest]),
                ]),
        )
        .into()
}

/// The screenshot, in a frame that quotes the studio's own chrome.
///
/// The one section that had to exist and did not: a product page for a *visual*
/// tool with no picture of the tool on it is a page arguing with itself. It
/// goes directly under the hero because it is the most persuasive thing here —
/// more than the copy, more than the code, and more than the live counter,
/// which proves the framework rather than the product.
fn showcase(column: f32, wide: bool, art: &Art) -> WidgetNode {
    let Some(image) = art.studio.clone() else {
        // No picture rather than a broken one: a decode that failed leaves the
        // section out, and the page below it is unaffected.
        return SizedBox::shrink().into();
    };

    // Guarded: a zero-height decode would divide by zero, and the ratio is the
    // one number here that comes from outside this file.
    let ratio = if image.height() == 0 {
        16.0 / 10.0
    } else {
        image.width() as f32 / image.height() as f32
    };

    let bar = |text: &str| -> WidgetNode {
        Container::new()
            .color(Color::rgb(0x1F, 0x1E, 0x23))
            .padding(EdgeInsets::symmetric(12.0, 7.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(8.0)
                    .children(children![
                        Container::new()
                            .color(LINE)
                            .radius(999.0)
                            .child(SizedBox::from_size(Size::new(8.0, 8.0))),
                        Text::new(text.to_string()).style(mono(11.0, INK_3)),
                    ]),
            )
            .into()
    };

    let frame = Container::new()
        .color(SURFACE)
        .radius(12.0)
        .border(Border::thin(LINE))
        .child(
            Clip::new(ClipShape::RRect { radius: 12.0 })
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(children![
                            bar("vieww Studio — scratch.rs"),
                            picture(image, ratio, column - 40.0),
                            Container::new()
                                .color(Color::rgb(0x1F, 0x1E, 0x23))
                                .padding(EdgeInsets::symmetric(12.0, 7.0))
                                .child(
                                    Text::new(
                                        "iOS  393x852     Safe area on     Problems 0 — the buffer compiled clean"
                                    )
                                    .style(mono(10.5, INK_3))
                                ),
                        ]),
                ),
        );

    section(
        "THE EDITOR",
        "The editor on the left. Your screen on the right.",
        Some(
            "A file tree, a Rust buffer, and the widget tree that buffer builds — \
             mounted at an iPhone's metrics, inside its safe area, with the \
             Problems panel reporting on the compile that produced it.",
        ),
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(10.0)
            .children(children![
                SizedBox::height(6.0),
                frame,
                Text::new(if wide {
                    "Every pixel of that window is a vieww widget tree — and so is every pixel of this page."
                } else {
                    "That window is a vieww widget tree. So is this page."
                })
                .style(mono(11.0, INK_3)),
            ])
            .into(),
    )
    .pipe(|node| {
        let _ = column;
        node
    })
}

/// A picture, drawn at its own size wherever there is room for it.
///
/// # Why this is not just an `AspectRatio`
///
/// A picture scaled by even two pixels is a picture the rasteriser has to
/// *filter*: four texel fetches, four premultiplies and three interpolations
/// for every destination pixel. At its natural size every sample lands on a
/// texel centre, `vieww-paint` takes its exact fast path, and the same picture
/// costs half as much — measured at 25 ms against 47 for one 860x538
/// screenshot. On a page that scrolls, that is the difference between drawing
/// it and not being able to afford to.
///
/// So: natural size when it fits, scaled only when the window is genuinely
/// narrower than the picture. `AspectRatio` is still what handles the second
/// case, and is also why an `Image` left alone letterboxes — it asks for its
/// intrinsic pixel height and `Contain` centres the picture inside it.
fn picture(image: ImageData, ratio: f32, available: f32) -> WidgetNode {
    let natural = image.width() as f32;
    if available >= natural {
        return Center::new()
            .child(
                SizedBox::from_size(Size::new(natural, image.height() as f32))
                    .child(Image::new(image).fit(BoxFit::None)),
            )
            .into();
    }
    AspectRatio::new(ratio)
        .child(Image::new(image).fit(BoxFit::Cover))
        .into()
}

/// Two fixtures, as proof that the renderer is real.
///
/// Both come out of `cargo run --release -p fixtures`, which is where a
/// rendering change in this framework is reviewed. They are on the page for the
/// same reason they exist: a claim about a rasteriser that shows no output of
/// it is a claim with nothing under it.
fn gallery(wide: bool, art: &Art, column: f32) -> WidgetNode {
    // What one plate's picture gets: the column, less the gap between two of
    // them and the card's own padding. Passed down so `picture` can tell
    // whether the fixture fits at its own size.
    let plate_width = if wide {
        (column - 14.0) / 2.0 - 28.0
    } else {
        column - 28.0
    };
    let plate =
        |image: Option<ImageData>, title: &str, caption: &str| -> Option<WidgetNode> {
            let image = image?;
            // **One ratio for both plates, not each picture's own.** The blend
            // matrix is 1.75:1 and the animation frame is square, and a row of two
            // cards whose pictures disagree about their shape is a row that looks
            // broken rather than varied. `Cover` crops to fill, which is the right
            // choice for a fixture: no part of either is load-bearing.
            const PLATE: f32 = 1.55;
            Some(
                Container::new()
                    .color(SURFACE)
                    .radius(10.0)
                    .border(Border::thin(LINE))
                    .child(
                        Clip::new(ClipShape::RRect { radius: 10.0 }).child(
                            Flex::column()
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .children(children![
                                    Container::new()
                                        .color(Color::rgb(0x0E, 0x0E, 0x10))
                                        .padding(EdgeInsets::all(14.0))
                                        .child(
                                            Clip::new(ClipShape::RRect { radius: 6.0 })
                                                .child(picture(image, PLATE, plate_width)),
                                        ),
                                    Container::new().padding(EdgeInsets::all(12.0)).child(
                                        Flex::column()
                                            .cross_axis_alignment(CrossAxisAlignment::Start)
                                            .spacing(3.0)
                                            .children(children![
                                                Text::new(title.to_string())
                                                    .size(13.0)
                                                    .color(INK)
                                                    .bold(),
                                                Text::new(caption.to_string())
                                                    .style(mono(10.5, INK_3)),
                                            ])
                                    ),
                                ]),
                        ),
                    )
                    .into(),
            )
        };

    let plates: Vec<WidgetNode> = [
        plate(
            art.blend.clone(),
            "28 blend modes",
            "composited over alpha, one pass",
        ),
        plate(
            art.anim.clone(),
            "Easing and transitions",
            "one frame of animation_showcase",
        ),
    ]
    .into_iter()
    .flatten()
    .collect();

    if plates.is_empty() {
        return SizedBox::shrink().into();
    }

    let body: WidgetNode = if wide {
        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(14.0)
            .children(
                plates
                    .into_iter()
                    .map(|plate| Flexible::expanded(1).child(plate).into())
                    .collect::<Vec<WidgetNode>>(),
            )
            .into()
    } else {
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(12.0)
            .children(plates)
            .into()
    };

    section(
        "RENDERER",
        "No wgpu. No Skia. vieww's own, all the way down.",
        Some(
            "The scene goes through vieww's render graph to a CPU rasteriser, or to \
             vieww-gpu's Vulkan backend — verified against that rasteriser pixel for \
             pixel. Both pictures below came out of it, and so did this page.",
        ),
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children![SizedBox::height(6.0), body])
            .into(),
    )
}

/// The three steps, which really are a sequence — which is why they are
/// numbered and nothing else on this page is.
fn loop_section(column: f32, wide: bool) -> WidgetNode {
    let steps = [
        (
            "01 · WRITE",
            "One file, one screen()",
            "A buffer compiles as a standalone crate, so screen() and everything \
             it calls live in that file. Define exactly one field-less widget and \
             the Problems panel offers to paste the missing screen() for you.",
        ),
        (
            "02 · RENDER",
            "Or just stop typing",
            "Render compiles the buffer to a cdylib and loads it. Screen state is \
             carried across a Render when the edit left the tree's shape alone, so \
             a counter you were half way through does not reset.",
        ),
        (
            "03 · JUDGE",
            "At the size it will be seen",
            "The tree mounts inside a device frame with that platform's metrics and \
             safe-area insets. A Flex that overflows here overflows on the device; a \
             Text that wraps wrong here wraps wrong there.",
        ),
    ];

    let cells: Vec<WidgetNode> = steps
        .iter()
        .map(|(number, title, body)| {
            Flexible::expanded(1)
                .child(
                    Container::new()
                        .padding(EdgeInsets::only(14.0, 2.0, 0.0, 2.0))
                        .child(
                            Flex::row()
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .spacing(14.0)
                                .children(children![
                                    Container::new().color(LINE).child(SizedBox::width(2.0)),
                                    Flexible::expanded(1).child(
                                        Flex::column()
                                            .cross_axis_alignment(CrossAxisAlignment::Start)
                                            .spacing(7.0)
                                            .children(children![
                                                Text::new((*number).to_string())
                                                    .style(mono(10.5, ACCENT).bold()),
                                                Text::new((*title).to_string())
                                                    .size(15.0)
                                                    .color(INK)
                                                    .bold(),
                                                Text::new((*body).to_string())
                                                    .size(13.0)
                                                    .color(INK_2),
                                            ])
                                    ),
                                ]),
                        ),
                )
                .into()
        })
        .collect();

    let body = if wide {
        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(18.0)
            .children(cells)
    } else {
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(20.0)
            .children(cells)
    };

    section(
        "THE LOOP",
        "Edit a line, see the picture change, edit the next line.",
        Some(
            "That is the whole thing this studio exists to shorten. A buffer needs \
             exactly one thing to be previewable: a function named screen that \
             returns something to draw.",
        ),
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(0.0)
            .children(children![SizedBox::height(6.0), body])
            .into(),
    )
    .pipe(|node| {
        let _ = column;
        node
    })
}

/// A card in the feature grid.
fn card(mark: &'static str, tag: &str, title: &str, body: &str) -> WidgetNode {
    Container::new()
        .color(SURFACE)
        .radius(10.0)
        .border(Border::thin(LINE))
        .padding(EdgeInsets::all(18.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(9.0)
                .children(children![
                    // The icon in its own tinted square, which is what stops a
                    // grid of cards reading as a wall of paragraphs.
                    Container::new()
                        .color(WASH)
                        .radius(8.0)
                        .padding(EdgeInsets::all(7.0))
                        .child(glyph(mark, 18.0, ACCENT)),
                    Text::new(spaced(tag)).style(mono(9.5, INK_3)),
                    Text::new(title.to_string()).size(14.5).color(INK).bold(),
                    Text::new(body.to_string()).size(13.0).color(INK_2),
                ]),
        )
        .into()
}

/// Eight things the studio does, two across when there is room.
fn features(_column: f32, wide: bool) -> WidgetNode {
    let cards = [
        (
            I_PREVIEW,
            "PREVIEW",
            "Three platforms, one switch",
            "iOS, Android and Desktop frames, with safe area, dark mode and a live \
             toggle. Switching platform changes the metrics and which theme \
             conventions apply, with no if platform == anywhere in your code.",
        ),
        (
            I_DOWNLOAD,
            "TOOLCHAIN",
            "It brings its own compiler",
            "The bundle carries the rustc it was built by and the vieww rlibs it was \
             linked against, so host and guest are one compilation by construction. \
             You do not need Rust installed.",
        ),
        (
            I_CODE,
            "EDITOR",
            "Rust editing, not a text box",
            "Highlighting, folding, find and replace, multi-buffer tabs, completion \
             through an LSP when one is on the machine, and a command palette where \
             every action is named once so menus and shortcuts cannot disagree.",
        ),
        (
            I_WARNING,
            "PANELS",
            "Problems, Output, Run, Tasks, Rustc, Timings",
            "Compiler diagnostics land against the line that caused them. Timings \
             says where the last Render went, so a slow loop is a number rather \
             than a feeling.",
        ),
        (
            I_SEARCH,
            "INSPECTOR",
            "The tree, and the Rust behind it",
            "Inspect the mounted element tree beside the preview, switch to Devices \
             to change the frame, or read the Generated Rust tab to see exactly what \
             was compiled.",
        ),
        (
            I_BOOK,
            "SAY",
            "A second way to write a screen",
            "A .say buffer is an English-facing screen language. The studio generates \
             the same Rust file you would have written by hand, then compiles it the \
             ordinary way.",
        ),
        (
            I_SHIELD,
            "SAFETY",
            "It will not lose your buffer",
            "Autosaved recovery files, a close that asks before dropping a modified \
             buffer, undo history per buffer, and a session scratch that survives a \
             restart.",
        ),
        (
            I_SWATCH,
            "PROOF",
            "The studio is built with vieww",
            "Every pixel of its window is a vieww widget tree drawn by vieww's own \
             rasteriser. So is this page. The editor is the largest application built \
             with the framework it edits.",
        ),
    ];

    let body: WidgetNode = if wide {
        let rows: Vec<WidgetNode> = cards
            .chunks(2)
            .map(|pair| {
                let mut cells: Vec<WidgetNode> = pair
                    .iter()
                    .map(|(mark, tag, title, text)| {
                        Flexible::expanded(1)
                            .child(card(mark, tag, title, text))
                            .into()
                    })
                    .collect();
                // An odd last row keeps its column width rather than spreading
                // one card across the whole band.
                if pair.len() == 1 {
                    cells.push(Flexible::expanded(1).child(SizedBox::shrink()).into());
                }
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .spacing(14.0)
                    .children(cells)
                    .into()
            })
            .collect();
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(14.0)
            .children(rows)
            .into()
    } else {
        let cells: Vec<WidgetNode> = cards
            .iter()
            .map(|(mark, tag, title, text)| card(mark, tag, title, text))
            .collect();
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(12.0)
            .children(cells)
            .into()
    };

    section(
        "IN THE WINDOW",
        "An editor that knows it is editing a screen.",
        None,
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children![SizedBox::height(6.0), body])
            .into(),
    )
}

/// A block of code, with the chrome `primitives.tsx`'s `CodeBlock` gives it.
///
/// Three traffic lights, the file name, a language chip, and a line-number
/// gutter. None of that is decoration: the gutter is what makes a code block
/// read as *code* at a glance rather than as an indented paragraph, and it was
/// the difference between this section and the reference's.
fn code_block(lines: &[&str], size: f32, filename: &str, lang: &str) -> WidgetNode {
    let dot = |color: Color| -> WidgetNode {
        Container::new()
            .color(color)
            .radius(999.0)
            .size(9.0, 9.0)
            .into()
    };

    let bar = Container::new()
        .color(SURFACE_2)
        .padding(EdgeInsets::symmetric(14.0, 9.0))
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(7.0)
                .children(children![
                    dot(Color::rgb(0xE0, 0x6C, 0x60)),
                    dot(Color::rgb(0xE0, 0xA8, 0x4E)),
                    dot(Color::rgb(0x5C, 0xB8, 0x60)),
                    SizedBox::width(6.0),
                    Text::new(filename.to_string()).style(mono(11.5, INK_2)),
                    Flexible::expanded(1).child(SizedBox::shrink()),
                    Container::new()
                        .color(SURFACE)
                        .radius(6.0)
                        .border(Border::thin(LINE))
                        .padding(EdgeInsets::symmetric(7.0, 2.0))
                        .child(Text::new(spaced(lang)).style(mono(9.0, INK_3))),
                ]),
        );

    let body: Vec<WidgetNode> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(14.0)
                .children(children![
                    // Fixed width, right-aligned, so the code starts on one
                    // column whether the file has nine lines or ninety.
                    SizedBox::width(20.0).child(
                        Text::new(format!("{}", index + 1))
                            .style(mono(size - 1.0, Color::rgb(0x4A, 0x45, 0x41)))
                            .align(TextAlign::Right),
                    ),
                    Flexible::expanded(1).child(RichText::new(highlight(line, size))),
                ])
                .into()
        })
        .collect();

    Container::new()
        .color(Color::rgb(0x12, 0x10, 0x0F))
        .radius(12.0)
        .border(Border::thin(LINE))
        .child(
            Clip::new(ClipShape::RRect { radius: 12.0 }).child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children![
                        bar,
                        Container::new().padding(EdgeInsets::all(16.0)).child(
                            Flex::column()
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .spacing(3.0)
                                .children(body),
                        ),
                    ]),
            ),
        )
        .into()
}

/// What a buffer looks like.
///
/// Two samples per language: a phone column is about forty monospace
/// characters wide, and a line longer than its column wraps rather than
/// scrolls — vieww has no horizontally scrollable text box, and a wrapped code
/// line reads as a syntax error. So the narrow variant is the same program
/// written to fit, not the same string in a smaller size.
fn code_section(wide: bool) -> WidgetNode {
    let rust_narrow = [
        "use vieww::prelude::*;",
        "",
        "// One function, named screen.",
        "pub fn screen() -> impl Widget {",
        "    Flex::column()",
        "        .spacing(12.0)",
        "        .children(children![",
        "            Text::new(\"Counter\"),",
        "            Text::new(\"Tapped 0 times\"),",
        "            Button::new(\"Add one\")",
        "                .on_pressed(|| {}),",
        "        ])",
        "}",
    ];
    let say_narrow = [
        "// The same screen in Say.",
        "",
        "keep a whole number called",
        "  count starting at 0",
        "",
        "screen \"Home\":",
        "    a column, spaced 16:",
        "        a heading \"Counter\"",
        "        a label \"Tapped",
        "          \\(count) times\"",
        "        a button \"Add one\"",
        "          which when tapped:",
        "            add 1 to count",
    ];
    let rust = [
        "use vieww::prelude::*;",
        "",
        "// Everything a preview needs: one function named screen.",
        "pub fn screen() -> impl Widget {",
        "    Container::new()",
        "        .color(theme.colors.surface)",
        "        .padding(EdgeInsets::all(24.0))",
        "        .child(",
        "            Flex::column()",
        "                .spacing(12.0)",
        "                .children(children![",
        "                    Text::new(\"Counter\").style(theme.text.headline),",
        "                    Text::new(\"Tapped 0 times\"),",
        "                    Button::new(\"Add one\").on_pressed(|| {}),",
        "                ]),",
        "        )",
        "}",
    ];
    let say = [
        "// The same screen in Say, which the studio compiles",
        "// to the Rust above before anything else happens.",
        "",
        "keep a whole number called count starting at 0",
        "",
        "screen \"Home\":",
        "    a column, spaced 16, children aligned to the start:",
        "        a heading \"Counter\"",
        "        a label \"Tapped \\(count) times\"",
        "        a button \"Add one\" which when tapped:",
        "            add 1 to count",
    ];

    section(
        "CODE",
        "Composition by method call. No macros to learn.",
        Some(
            "A widget is a cheap, immutable description of intent. You build one by \
             calling methods on it and hand it children with children![].",
        ),
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(12.0)
            .children(children![
                SizedBox::height(6.0),
                code_block(
                    if wide { &rust } else { &rust_narrow },
                    if wide { 13.0 } else { 11.5 },
                    "screen.rs",
                    "rust"
                ),
                code_block(
                    if wide { &say } else { &say_narrow },
                    if wide { 13.0 } else { 11.5 },
                    "counter.say",
                    "say"
                ),
            ])
            .into(),
    )
}

/// One row of the download table, tappable.
fn asset_row(asset: &'static Asset, wide: bool) -> WidgetNode {
    let target = url_for(asset.file);

    // The whole row is the target, not just the file name: a link that is one
    // line of eleven-pixel text is a link nobody hits on a phone. `Pressable`
    // builds its subtree from how pressed it is, which is where the tint comes
    // from — a canvas has no `:active`, so the feedback has to be drawn.
    // `CursorArea` around it, because `cursor_at` asks the render tree what
    // shape belongs under a point and nothing else on this page answers: a
    // canvas is one element, so the browser cannot infer a hand from a link
    // the way it would in HTML. Without this every tappable thing on the page
    // shows an arrow.
    CursorArea::new(Cursor::Pointer)
        .child(
            Pressable::sensed(move |sense: Sense| {
                let left = Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(3.0)
                    .children(children![
                        Text::new(asset.label.to_string()).size(13.5).color(INK),
                        Text::new(asset.file.to_string()).style(mono(11.5, ACCENT)),
                    ]);

                let row: WidgetNode = if wide {
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .spacing(12.0)
                        .children(children![
                            Flexible::expanded(1).child(left),
                            Text::new(asset.size.to_string()).style(mono(12.0, INK_3)),
                            chip("Pending", INK_3),
                        ])
                        .into()
                } else {
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(6.0)
                        .children(children![
                            left,
                            Flex::row().spacing(8.0).children(children![
                                Text::new(asset.size.to_string()).size(12.0).color(INK_3),
                                chip("Pending", INK_3),
                            ]),
                        ])
                        .into()
                };

                // `AnimatedContainer` rather than `Container`: hover is a state a
                // pointer enters and leaves, and a fill that snaps between two colours
                // reads as a flicker when the pointer crosses a row on its way
                // somewhere else. The press is not animated separately — it is the
                // same fill, further along.
                AnimatedContainer::new()
                    .duration(Duration::from_millis(120))
                    .curve(Curve::EASE_OUT)
                    .color(press_tint(sense.press.max(sense.hover * 0.55)))
                    .padding(EdgeInsets::symmetric(16.0, 13.0))
                    .child(row)
                    .into()
            })
            .on_tap(move || navigate(&target)),
        )
        .into()
}

/// The row's fill at a given press amount: the surface, moving toward the
/// accent wash. Flat at rest, so an untouched table is a table and not a
/// stack of buttons.
fn press_tint(press: f32) -> Color {
    let mix = press.clamp(0.0, 1.0);
    let lerp = |from: u8, to: u8| -> u8 {
        (f32::from(from) + (f32::from(to) - f32::from(from)) * mix).round() as u8
    };
    Color::rgb(lerp(0x18, 0x2A), lerp(0x18, 0x23), lerp(0x1B, 0x40))
}

/// The download section: the button, then every file.
fn downloads(column: f32, wide: bool, host: Option<Os>) -> WidgetNode {
    let mut rows: Vec<WidgetNode> = Vec::new();
    for (index, asset) in ASSETS.iter().enumerate() {
        if index > 0 {
            rows.push(
                Container::new()
                    .color(LINE)
                    .child(SizedBox::height(1.0))
                    .into(),
            );
        }
        rows.push(asset_row(asset, wide));
    }

    let table = Container::new()
        .color(SURFACE)
        .radius(10.0)
        .border(Border::thin(LINE))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(rows),
        );

    let checksums = url_for("SHA256SUMS");

    section(
        "DOWNLOAD",
        "One file, and a linker.",
        Some(
            "Every build carries the compiler and the libraries inside it, which is \
             why each one is around 400 MB and why there is nothing to configure \
             after it lands. Every row below is a link.",
        ),
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(14.0)
            .children(children![
                SizedBox::height(2.0),
                download_button(host, column),
                table,
                // In a row of its own, because the column around it stretches
                // its children and a stretched button centres its label.
                Flex::row()
                    .main_axis_alignment(MainAxisAlignment::Start)
                    .children(children![CursorArea::new(Cursor::Pointer).child(
                        Button::new("Checksums (SHA256SUMS)")
                            .style(ButtonStyle::Text)
                            .on_pressed(move || navigate(&checksums)),
                    ),]),
            ])
            .into(),
    )
}

/// The one thing the download cannot carry.
fn requirements(_column: f32) -> WidgetNode {
    let panel = Container::new()
        .color(WASH)
        .radius(10.0)
        .border(Border::thin(ACCENT_DEEP))
        .padding(EdgeInsets::all(16.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(9.0)
                .children(children![
                    Text::new("A C toolchain, and nothing else")
                        .size(14.5)
                        .color(INK)
                        .bold(),
                    Text::new(
                        "The download carries the rustc it was built with and the vieww \
                         libraries it was linked against, so you do not need Rust \
                         installed: the preview compiles against exactly this build, by \
                         construction."
                    )
                    .size(13.0)
                    .color(INK_2),
                    Text::new(
                        "What it cannot carry is a linker. A preview is a shared library, \
                         and rustc links it through cc. Install build-essential on Linux, \
                         the Xcode Command Line Tools on macOS, or the MSVC Build Tools on \
                         Windows."
                    )
                    .size(13.0)
                    .color(INK_2),
                ]),
        );

    section(
        "WHAT YOU NEED",
        "Before the first Render.",
        None,
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children![SizedBox::height(6.0), panel])
            .into(),
    )
}

/// The end of the page: three columns of links, then the small print.
fn footer(column: f32, wide: bool) -> WidgetNode {
    let base = format!("https://github.com/{}", repo());
    let groups: [(&str, Vec<(&str, String)>); 3] = [
        (
            "STUDIO",
            vec![
                ("Download", url_for("viewwstudio-linux-x86_64.deb")),
                ("Checksums", url_for("SHA256SUMS")),
                ("All releases", format!("{base}/releases")),
            ],
        ),
        (
            "FRAMEWORK",
            vec![
                ("Source", base.clone()),
                ("Guide", format!("{base}/blob/main/docs/guide/README.md")),
                (
                    "Architecture",
                    format!("{base}/blob/main/docs/architecture/README.md"),
                ),
            ],
        ),
        (
            "PROJECT",
            vec![
                ("What is verified", format!("{base}/blob/main/TRACKER.md")),
                ("What is not", format!("{base}/blob/main/PENDING.md")),
                ("Issues", format!("{base}/issues")),
            ],
        ),
    ];

    let columns: Vec<WidgetNode> = groups
        .into_iter()
        .map(|(heading, links)| {
            let mut items: Vec<WidgetNode> = vec![
                Text::new(spaced(heading)).style(mono(9.5, INK_3)).into(),
                SizedBox::height(4.0).into(),
            ];
            for (label, url) in links {
                items.push(
                    CursorArea::new(Cursor::Pointer)
                        .child(
                            Pressable::sensed(move |sense: Sense| {
                                Text::new(label.to_string())
                                    .size(13.0)
                                    .color(if sense.emphasis() > 0.0 {
                                        ACCENT
                                    } else {
                                        INK_2
                                    })
                                    .into()
                            })
                            .on_tap(move || navigate(&url)),
                        )
                        .into(),
                );
            }
            Flexible::expanded(1)
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(7.0)
                        .children(items),
                )
                .into()
        })
        .collect();

    let link_row: WidgetNode = if wide {
        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(24.0)
            .children(columns)
            .into()
    } else {
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(22.0)
            .children(columns)
            .into()
    };

    Container::new()
        .padding(EdgeInsets::symmetric(0.0, 40.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .spacing(28.0)
                .children(children![
                    link_row,
                    Container::new()
                        .color(LINE)
                        .child(SizedBox::from_size(Size::new(column, 1.0))),
                    // Two mono lines and a mark come to 715 points; a phone
                    // column is 334. On narrow they stack.
                    if wide {
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .spacing(9.0)
                            .children(children![
                                SizedBox::from_size(Size::new(18.0, 18.0)).child(mark(18.0)),
                                Text::new("vieww Studio 0.1.0 — Apache-2.0")
                                    .style(mono(11.5, INK_3)),
                                Flexible::expanded(1).child(SizedBox::shrink()),
                                Text::new(
                                    "Rendered by vieww-paint's CPU rasteriser, presented with put_image_data."
                                )
                                .style(mono(10.5, INK_3)),
                            ])
                    } else {
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(8.0)
                            .children(children![
                                Flex::row()
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .spacing(9.0)
                                    .children(children![
                                        SizedBox::from_size(Size::new(18.0, 18.0))
                                            .child(mark(18.0)),
                                        Text::new("vieww Studio 0.1.0 — Apache-2.0")
                                            .style(mono(11.5, INK_3)),
                                    ]),
                                Text::new(
                                    "Rendered by vieww-paint's CPU rasteriser."
                                )
                                .style(mono(10.5, INK_3)),
                            ])
                    },
                ]),
        )
        .into()
}

/// A tiny helper so a section builder can be written as an expression.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

/// The whole page, mounted on `driver`.
///
/// Takes the driver rather than the pieces because everything the page needs
/// has to come off *this* tree: a signal created on another runtime marks
/// nothing pending, and an animation attached to no tickers never advances.
/// Both are mistakes that produce a page which looks right and does not move,
/// so there is one function that cannot make either.
///
/// `LayoutBuilder` rather than a bare [`Site`] because the page has to be laid
/// out against the surface's real width and rebuilt when it changes — and
/// because that is the only place the width is known without asking a render
/// object. `Site` then reads the scroll offset and the entrance, which is what
/// makes a drag and the first second rebuild the page and nothing above it.
pub fn root(driver: &mut FrameDriver, host: Option<Os>) -> WidgetNode {
    // Before anything is built: a font store swapped after the first layout
    // would remeasure every string on the page.
    driver.set_fonts(fonts());

    let runtime = driver.elements().runtime().clone();

    let scroll = ScrollController::new(&runtime, ScrollPhysics::android());
    let count = runtime.signal(0_i32);

    // One second, once. `EASE_OUT` because everything it moves is *arriving*:
    // it has to leave quickly and settle, rather than accelerate away.
    let entrance = Animation::new(
        &runtime,
        Tween::new(0.0_f32, 1.0),
        Duration::from_millis(1000),
    )
    .curve(Curve::EASE_OUT);

    // Both attached before either is started. A controller nobody attached
    // scrolls under the finger and stops dead when it lifts; an animation
    // nobody attached sits at zero, which on this page means an invisible
    // hero.
    scroll.attach(driver.tickers());
    entrance.attach(driver.tickers());
    entrance.forward(Duration::ZERO);

    let anchors = Anchors::new(&runtime);
    page(scroll, count, entrance, host, Art::embedded(), anchors)
}

/// The tree, with everything it needs already built.
///
/// Split from [`root`] so a caller that has its own animation clock — a test,
/// or `examples/render.rs`, which has to render the page *after* the entrance
/// rather than during it — can supply the pieces itself.
#[must_use]
pub fn page(
    scroll: ScrollController,
    count: Signal<i32>,
    entrance: Animation<f32>,
    host: Option<Os>,
    art: Art,
    anchors: Anchors,
) -> WidgetNode {
    Theme::new(theme())
        .child(LayoutBuilder::new(move |constraints: Constraints| {
            // **The tree lays out in logical points, and nothing here knows
            // about the device-pixel ratio.**
            //
            // It used to: the backend handed down the canvas's device-pixel
            // buffer, this divided by the ratio and wrapped the page in
            // `Transformed::scale(ratio)` to put the painting back at device
            // resolution. That is correct everywhere except below a repaint
            // boundary — and a `Scrollable` is one. A boundary records into a
            // scene of its own starting from an identity canvas and composites
            // back into its parent through an `Offset`, which carries a
            // translation and drops a scale, so the whole page was laid out at
            // twice the window and painted at 1:1.
            //
            // The ratio now belongs to the backend, which applies it to the
            // finished, flattened scene — `Scene::scaled`, after compositing,
            // where there are no boundaries left to lose it. Glyph outlines
            // still go through the matrix, so the sharpness is the same.
            let surface = Size::new(constraints.max_width, constraints.max_height);
            SizedBox::from_size(surface)
                .child(Site {
                    scroll: scroll.clone(),
                    surface,
                    host,
                    entrance: entrance.clone(),
                    count: count.clone(),
                    art: art.clone(),
                    anchors: anchors.clone(),
                })
                .into()
        }))
        .into()
}

// ─── the entry point ──────────────────────────────────────────────────────

/// Mount the page on the `<canvas>` with id `vieww`.
///
/// Called by the browser through wasm-bindgen's start shim, so the host page
/// is one `<script type="module">` and nothing else.
///
/// # Errors
///
/// Anything [`WebSurface::by_id`] or [`WebApp::mount_with`] can fail with,
/// surfaced as a `JsValue` so the failure reaches the console rather than
/// leaving a blank canvas with no explanation.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    let host = detect();

    // **DOM, not canvas.** The page is a vieww widget tree either way; this
    // hands it to `vieww-platform-web-dom`, which walks the element tree and
    // emits elements and CSS instead of scene commands. What that buys is in
    // that crate's documentation, and the short version is: the browser's own
    // type rasteriser, `backdrop-filter`, selectable text, a screen reader, a
    // crawler, and a payload a fraction of the rasteriser's.
    //
    // The rasteriser is still here. `dom::demo` puts a `Canvas` in the page,
    // and where it lands a real `vieww-platform-web` application is mounted —
    // the same `Device` tree, painted by `vieww-paint`, a few hundred points
    // below the headline that claims it can be.
    vieww_platform_web_dom::DomApp::mount("app", dom::page(host))
        .map(std::mem::forget)
        .map_err(|error| error.to_string())?;
    Ok(())
}
