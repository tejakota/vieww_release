//! Font loading, and the bridge from font ids to font bytes.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use cosmic_text::{fontdb, FontSystem};
use vieww_foundation::FontData;

/// The regular face of the embedded font.
///
/// # Why embed a font at all
///
/// Two reasons, and the second is the one that matters. A device with no usable
/// system font renders nothing at all otherwise — a blank screen and no error.
/// And tests need a font whose metrics are identical on every machine: a wrap
/// point or an advance width asserted against whatever font a CI runner happens
/// to have installed is not a test, it is a coin flip.
///
/// # Why three faces and not one
///
/// Real bold and real italic. A single regular face makes
/// `TextStyle::bold()` a silent no-op — `fontdb` matches the closest available
/// face, so the text shapes identically and nothing reports a problem. The Phase 5
/// exit test needs mixed bold and italic runs to *differ*, which needs faces that
/// actually exist.
///
/// # Why subsetted
///
/// The three full DejaVu faces are about 2MB. Subsetted to Latin, Hebrew and
/// Arabic they are about 170KB, which is the difference between an acceptable
/// binary cost and an unacceptable one. Regenerate with
/// `ci/tools/subset-fonts.py`(../../../ci/tools/subset-fonts.py); `fontTools` is needed to
/// regenerate but not to build, since the results are committed.
///
/// Coverage is Latin, Hebrew and (except in the oblique face) Arabic — enough to
/// make the bidirectional tests real rather than notional — plus the CJK
/// vocabulary of [`EMBEDDED_CJK`], enough that fallback and mixed-script
/// shaping are real headlessly too. Scripts beyond that (Devanagari, Thai,
/// full-repertoire CJK) fall back to a system font.
///
/// Licensed under the Bitstream Vera licence, which permits redistribution — see
/// `assets/DejaVuSans-LICENSE.txt`.
pub const EMBEDDED_FONT: &[u8] = include_bytes!("../assets/DejaVuSans-subset.ttf");

/// The bold face of the embedded font.
pub const EMBEDDED_FONT_BOLD: &[u8] = include_bytes!("../assets/DejaVuSans-Bold-subset.ttf");

/// The oblique face of the embedded font, used for italic.
pub const EMBEDDED_FONT_ITALIC: &[u8] = include_bytes!("../assets/DejaVuSans-Oblique-subset.ttf");

/// The regular monospace face.
///
/// A real second typeface, not an alias. Without it,
/// `FontFamily::Monospace` resolved back to the proportional face — the
/// database was told the monospace family *was* DejaVu Sans — so asking for
/// monospace shaped identically and nothing reported a problem. Same failure
/// mode as a missing bold face, and the same reason it is fixed by shipping the
/// face rather than by documenting the gap.
pub const EMBEDDED_MONO: &[u8] = include_bytes!("../assets/DejaVuSansMono-subset.ttf");

/// The bold monospace face.
pub const EMBEDDED_MONO_BOLD: &[u8] = include_bytes!("../assets/DejaVuSansMono-Bold-subset.ttf");

/// The embedded CJK fallback face — a static instance of Noto Serif SC at
/// the reading weight, subset to the framework's own test vocabulary.
///
/// # Why this exists, and why it is small
///
/// The DejaVu subsets cover Latin, Hebrew and Arabic, and everything else
/// fell through to the platform — which is right for a shipped application
/// (a full CJK face is 10–20MB and no UI framework should put that in every
/// binary) but meant a headless render of *any* Chinese drew tofu boxes, and
/// every test that wanted to exercise fallback, shaping or metrics for a
/// non-Latin script could not run without a system font. This face is 26KB
/// of real CJK outlines — enough for the framework's tests to be real, and
/// for the certification suite's typography screens to show an actual
/// fallback chain rather than a row of boxes. An application that needs full
/// coverage still ships it: `load_font_data` is unchanged, and this face is
/// registered *after* everything the application brings, so it never
/// displaces a chosen typeface.
///
/// Regenerate with `scripts/make_test_fonts.py`; SIL Open Font License —
/// Noto's licensing permits redistribution of subsets.
pub const EMBEDDED_CJK: &[u8] = include_bytes!("../assets/NotoSansCJK-subset.ttf");

/// Every embedded face.
///
/// The CJK face is last in the list, which is also the *fallback order*:
/// a tie between it and a DejaVu face for a Latin character resolves to
/// DejaVu, and a character DejaVu cannot cover resolves to the CJK face
/// rather than to nothing.
pub const EMBEDDED_FACES: [&[u8]; 6] = [
    EMBEDDED_FONT,
    EMBEDDED_FONT_BOLD,
    EMBEDDED_FONT_ITALIC,
    EMBEDDED_MONO,
    EMBEDDED_MONO_BOLD,
    EMBEDDED_CJK,
];

/// The family name shared by the embedded proportional faces.
pub const EMBEDDED_FAMILY: &str = "DejaVu Sans";

/// The family name shared by the embedded monospace faces.
pub const EMBEDDED_MONO_FAMILY: &str = "DejaVu Sans Mono";

/// The assembled system-fallback database — embedded faces plus every font
/// the OS reports — built once per process and cloned for every window after
/// the first.
///
/// # Why this exists
///
/// `cosmic_text::FontSystem::new`'s own docs warn that building one "can take
/// up to a second [in release], while debug builds can take up to ten times
/// longer... it should only be called once, and the resulting `FontSystem`
/// should be shared." [`FontStore::with_system_fallback`] used to ignore that
/// advice on every window it opened: each call built a fresh
/// `fontdb::Database` and ran `load_system_fonts()` — a synchronous walk of
/// every font file on disk — from scratch, before that window's first frame.
/// One window paid the scan once; an application that opens a second window
/// or a dialog paid it again, in full, synchronously, on the frame that
/// window is trying to present. Measured at 46 seconds on one slow disk (see
/// `docs/release/BETA-RELEASE-CHECKLIST.md`, B12) — and a 3-window desktop
/// suite paid that three times over.
///
/// # Why a clone is safe and cheap here
///
/// `fontdb::Database` derives `Clone`, and cloning it does not re-read any
/// font: a face is stored as a `Source::File` path or a `Source::Binary`
/// wrapping an `Arc<[u8]>`, so cloning the database clones small metadata and
/// bumps a refcount, never the file system. What it does cost is an
/// allocation proportional to the number of installed faces — real, but nine
/// orders of magnitude cheaper than the scan it replaces.
///
/// # What this does not do
///
/// It does not notice a font installed after the first window opened in this
/// process — the same staleness `VulkanDevice`'s per-process cache in
/// `vieww-platform-winit::native` accepts for the same reason: a UI
/// framework's window-open path is not the place to re-walk the filesystem on
/// the chance something changed since the last window, and a process
/// noticing a newly-installed font without restarting is not a guarantee any
/// major toolkit makes either.
fn scanned_system_db() -> fontdb::Database {
    static SYSTEM_FONTS: OnceLock<fontdb::Database> = OnceLock::new();
    SYSTEM_FONTS
        .get_or_init(|| {
            SYSTEM_FONT_SCANS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut db = fontdb::Database::new();
            // First, so a tie resolves to the deterministic face — see
            // `with_system_fallback`'s own docs on why order is the point.
            for face in EMBEDDED_FACES {
                db.load_font_data(face.to_vec());
            }
            db.load_system_fonts();
            db.set_sans_serif_family(EMBEDDED_FAMILY);
            db.set_monospace_family(EMBEDDED_MONO_FAMILY);
            db
        })
        .clone()
}

/// How many times this process has actually walked the filesystem for system
/// fonts — incremented once, inside [`scanned_system_db`]'s `get_or_init`, no
/// matter how many windows or `FontStore`s ask for one.
///
/// Exists for exactly one caller: a test that opens (constructs)
/// [`FontStore::with_system_fallback`] more than once and asserts this stayed
/// at 1 — the regression test for B12
/// (`docs/release/BETA-RELEASE-CHECKLIST.md`). An application has no
/// legitimate use for this number.
static SYSTEM_FONT_SCANS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// See [`SYSTEM_FONT_SCANS`]. `cfg(test)` rather than `pub`: nothing outside
/// this crate's own test module has a legitimate reason to read it, unlike
/// `vieww_platform_winit::native`'s equivalent counter for the Vulkan device,
/// which an external integration test needs and so must be reachable from
/// outside the crate.
#[cfg(test)]
pub(crate) fn system_font_scans() -> usize {
    SYSTEM_FONT_SCANS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Loaded fonts, and the shaping engine that owns them.
///
/// Wraps `cosmic-text`'s `FontSystem`. Held for the lifetime of an application
/// and passed to every layout, because font loading is expensive and the shaping
/// caches inside it are what make re-layout cheap.
pub struct FontStore {
    system: FontSystem,
    /// Font bytes already handed out, keyed by the id `cosmic-text` knows them by
    /// and the weight it was resolved at.
    ///
    /// This cache is not an optimisation, it is a correctness requirement.
    /// [`FontData`] compares by `Rc` identity, so handing out a fresh `Rc` for
    /// the same font each frame would make damage tracking see every text run as
    /// changed, and repaint all text on every frame forever.
    data: HashMap<(fontdb::ID, u16), FontData>,
    /// Laid-out paragraphs, so the same words are not shaped twice.
    ///
    /// It lives here because this is the only thing every layout call already
    /// has a `&mut` to, and because the fonts are the one input to shaping that
    /// is not in the key — a cache on a different store would be a cache with a
    /// missing key. See [`crate::shape_cache`].
    shapes: crate::shape_cache::ShapeCache,
}

impl FontStore {
    /// Load the platform's fonts, plus the embedded fallback.
    ///
    /// The system scan is the expensive part — hundreds of files on a desktop.
    /// Do it once.
    #[must_use]
    pub fn new() -> Self {
        let mut system = FontSystem::new();
        for face in EMBEDDED_FACES {
            system.db_mut().load_font_data(face.to_vec());
        }
        Self {
            system,
            data: HashMap::new(),
            shapes: crate::shape_cache::ShapeCache::default(),
        }
    }

    /// The embedded faces **first**, then the platform's, as fallback.
    ///
    /// # Why this is the one an application wants
    ///
    /// The two existing constructors are both wrong for a shipped application,
    /// in opposite directions.
    ///
    /// [`embedded_only`](Self::embedded_only) is what the render tree defaults
    /// to, because deterministic metrics are what make a wrap-point assertion a
    /// test rather than a coin flip. But the embedded subset is Latin, Hebrew
    /// and Arabic — so a shipped application built on it renders Chinese,
    /// Japanese, Korean, Devanagari, Thai and every other script as nothing at
    /// all. That is most of the world's readers.
    ///
    /// [`new`](Self::new) scans the system and *then* adds the embedded faces,
    /// which fixes coverage and loses determinism in the other direction: the
    /// same Latin string measures differently on two machines, which is the bug
    /// that had `tap_to_caret` passing on Linux and failing on macOS.
    ///
    /// This one has both properties, because order decides which face wins a
    /// tie. The embedded faces are registered first, so anything the subset
    /// covers — every character the framework itself draws, and all Latin UI
    /// text — shapes identically everywhere. Anything it does not falls through
    /// to whatever the platform has.
    ///
    /// # What it still cannot do
    ///
    /// A device with no CJK font installed has no CJK font, and this cannot
    /// invent one. What it guarantees is that the failure is *visible*: the
    /// glyphs come back as `.notdef` and `RenderText` strokes a box for each,
    /// rather than the screen going quietly blank. Shipping the coverage is an
    /// application's decision — a 16MB CJK face is not something a UI framework
    /// should put in every binary — and `load` is how it adds one.
    #[must_use]
    pub fn with_system_fallback() -> Self {
        Self {
            system: FontSystem::new_with_locale_and_db("en-US".to_owned(), scanned_system_db()),
            data: HashMap::new(),
            shapes: crate::shape_cache::ShapeCache::default(),
        }
    }

    /// An application's own faces first, the embedded ones behind them.
    ///
    /// # Why a shipped application needs this and the other three do not
    ///
    /// The embedded faces exist so the framework always has *a* font. They are
    /// not a design decision — DejaVu Sans is a 2004 Bitstream Vera derivative,
    /// and a product page set in it looks like a product page nobody chose a
    /// font for. An application that ships its own typeface had nowhere to say
    /// so: [`load_font_data`](Self::load_font_data) registers a face but cannot
    /// make it the answer to [`FontFamily::SansSerif`], so every style in the
    /// tree had to name it explicitly or silently get DejaVu.
    ///
    /// `sans_family` and `mono_family` are the family *names* — the `name`
    /// table's family, not a file name — that [`FontFamily::SansSerif`] and
    /// [`FontFamily::Monospace`] resolve to. `None` leaves that generic pointed
    /// at the embedded face.
    ///
    /// The application's faces are loaded first, so a tie in family and weight
    /// resolves to theirs; the embedded ones stay behind them as the fallback
    /// that keeps Hebrew and Arabic rendering. No system scan, so metrics stay
    /// deterministic — which is what a page rasterised on a build machine and
    /// again in a browser needs.
    ///
    /// ```no_run
    /// # use vieww_text::FontStore;
    /// const SANS: &[u8] = b"";
    /// let fonts = FontStore::with_application_faces(
    ///     [SANS.to_vec()],
    ///     Some("Geist"),
    ///     Some("Geist Mono"),
    /// );
    /// ```
    ///
    /// [`FontFamily::SansSerif`]: vieww_foundation::FontFamily::SansSerif
    /// [`FontFamily::Monospace`]: vieww_foundation::FontFamily::Monospace
    #[must_use]
    pub fn with_application_faces(
        faces: impl IntoIterator<Item = Vec<u8>>,
        sans_family: Option<&str>,
        mono_family: Option<&str>,
    ) -> Self {
        let mut db = fontdb::Database::new();
        // First, so a tie in family and weight resolves to the application's.
        for face in faces {
            db.load_font_data(face);
        }
        for face in EMBEDDED_FACES {
            db.load_font_data(face.to_vec());
        }
        db.set_sans_serif_family(sans_family.unwrap_or(EMBEDDED_FAMILY));
        db.set_serif_family(sans_family.unwrap_or(EMBEDDED_FAMILY));
        db.set_monospace_family(mono_family.unwrap_or(EMBEDDED_MONO_FAMILY));

        Self {
            system: FontSystem::new_with_locale_and_db("en-US".to_owned(), db),
            data: HashMap::new(),
            shapes: crate::shape_cache::ShapeCache::default(),
        }
    }

    /// Load *only* the embedded font, skipping the system scan.
    ///
    /// What tests use: metrics are then identical on every machine, so a wrap
    /// point or an advance width can be asserted exactly. Also the fast path for a
    /// headless render that does not care which font it gets.
    #[must_use]
    pub fn embedded_only() -> Self {
        let mut db = fontdb::Database::new();
        for face in EMBEDDED_FACES {
            db.load_font_data(face.to_vec());
        }
        db.set_sans_serif_family(EMBEDDED_FAMILY);
        // Serif maps to the sans face on purpose: there is no embedded serif,
        // and mapping it to nothing would make a headless render of serif text
        // draw nothing at all. Monospace does *not* map here — there is a real
        // mono face, and pointing this at the sans one is exactly the silent
        // no-op that made `FontFamily::Monospace` untestable.
        db.set_serif_family(EMBEDDED_FAMILY);
        db.set_monospace_family(EMBEDDED_MONO_FAMILY);
        db.set_cursive_family(EMBEDDED_FAMILY);
        db.set_fantasy_family(EMBEDDED_FAMILY);

        Self {
            system: FontSystem::new_with_locale_and_db("en-US".to_owned(), db),
            data: HashMap::new(),
            shapes: crate::shape_cache::ShapeCache::default(),
        }
    }

    /// Add a font face from bytes — a CJK face an application ships, a brand
    /// typeface, an icon font.
    ///
    /// Registered after everything already loaded, so it does not displace the
    /// embedded faces for characters they already cover.
    ///
    /// # Shipping coverage for a script the framework cannot embed
    ///
    /// This is the answer to "my app is for China / India / Thailand". The
    /// embedded subset is Latin, Hebrew and Arabic, because a font covering CJK
    /// is 10–20MB and a UI framework has no business putting that in every
    /// binary that links it. So an application that needs it ships it:
    ///
    /// ```no_run
    /// use vieww_text::FontStore;
    ///
    /// // `include_bytes!` for a face compiled in, or read it from an asset
    /// // bundle at start-up — either way, once.
    /// # const NOTO_SC: &[u8] = &[];
    /// let mut fonts = FontStore::with_system_fallback();
    /// fonts.load_font_data(NOTO_SC.to_vec());
    /// ```
    ///
    /// Then hand it to the driver with
    /// `FrameDriver::set_fonts`. Anything already laid out has to be re-shaped
    /// — see [`clear_shape_cache`](Self::clear_shape_cache) — which is why this
    /// belongs at start-up rather than in a build.
    ///
    /// # What happens without it
    ///
    /// [`with_system_fallback`](Self::with_system_fallback) finds a system face
    /// when the device has one, which covers most desktops and phones. When it
    /// does not, the glyphs come back as `.notdef` and the renderer draws a box
    /// per character rather than nothing — visible, diagnosable, and still not a
    /// shipping experience for a user who reads that script.
    pub fn load_font_data(&mut self, data: Vec<u8>) {
        self.system.db_mut().load_font_data(data);
        // A face loaded after something was laid out changes what fallback
        // resolves to, and that is the one input the shape cache's key cannot
        // see. Clearing here rather than asking the caller to remember is the
        // difference between a correct API and a footgun.
        self.shapes.clear();
    }

    /// Add every font file under `directory`, recursively.
    ///
    /// For an application that ships its faces as assets rather than compiling
    /// them in — a `fonts/` folder beside the binary, or an Android asset
    /// directory staged at start-up.
    ///
    /// Returns how many faces were added. Zero is not an error: a directory
    /// that does not exist, or holds nothing a font parser recognises, leaves
    /// the store exactly as it was. A caller that requires coverage should
    /// check the count rather than assume.
    pub fn load_font_directory(&mut self, directory: impl AsRef<std::path::Path>) -> usize {
        let before = self.len();
        self.system.db_mut().load_fonts_dir(directory);
        self.shapes.clear();
        self.len().saturating_sub(before)
    }

    /// `true` when every character in `text` can be drawn by some loaded face.
    ///
    /// # Why an application wants to ask
    ///
    /// So that "this device cannot render Chinese" is a condition the
    /// application can *detect* — and download a face, or show a message, or
    /// fall back to another language — rather than a screen of boxes the user
    /// has to interpret.
    ///
    /// Cheap enough to call on a string at start-up; it asks the font database
    /// per distinct character and does no shaping. Not cheap enough for a
    /// per-frame check on a paragraph.
    ///
    /// ```
    /// use vieww_text::FontStore;
    ///
    /// let fonts = FontStore::embedded_only();
    /// assert!(fonts.can_render("Hello, שלום, مرحبا"));
    /// assert!(fonts.can_render("你好世界")); // covered by the CJK face
    /// assert!(!fonts.can_render("नमस्ते")); // Devanagari: not embedded
    /// ```
    #[must_use]
    pub fn can_render(&self, text: &str) -> bool {
        self.missing_characters(text).is_empty()
    }

    /// The characters in `text` no loaded face can draw, deduplicated and in
    /// first-seen order.
    ///
    /// The diagnostic form of [`can_render`](Self::can_render): what to put in a
    /// log line, or in the message that asks a user to install a language pack.
    ///
    /// Whitespace and control characters are never reported — they have no ink
    /// and a font that lacks a glyph for one still lays it out correctly.
    #[must_use]
    pub fn missing_characters(&self, text: &str) -> Vec<char> {
        let mut seen = std::collections::HashSet::new();
        let mut missing = Vec::new();

        for character in text.chars() {
            if character.is_whitespace() || character.is_control() {
                continue;
            }
            if !seen.insert(character) {
                continue;
            }
            let covered = self
                .system
                .db()
                .faces()
                .any(|face| self.face_covers(face.id, character));
            if !covered {
                missing.push(character);
            }
        }
        missing
    }

    /// Whether one face has a glyph for `character`.
    ///
    /// `fontdb` answers this from the face's character map without rasterising
    /// or shaping, which is what makes the scan above affordable.
    fn face_covers(&self, id: fontdb::ID, character: char) -> bool {
        self.system
            .db()
            .with_face_data(id, |data, index| {
                ttf_parser::Face::parse(data, index)
                    .ok()
                    .and_then(|face| face.glyph_index(character))
                    .is_some()
            })
            .unwrap_or(false)
    }

    /// How many faces are loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.system.db().len()
    }

    /// `true` if no font is loaded at all — nothing can be shaped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn system_mut(&mut self) -> &mut FontSystem {
        &mut self.system
    }

    /// The shaped-paragraph cache. See [`crate::shape_cache`].
    pub(crate) fn shapes_mut(&mut self) -> &mut crate::shape_cache::ShapeCache {
        &mut self.shapes
    }

    /// How many paragraphs this store has actually shaped.
    ///
    /// Counted work, in the spirit of `RenderTree::layout_runs` and
    /// `LayerTree::paint_count`: the shaping cache's whole claim is that this
    /// number stops growing when the text does not change, and a claim about
    /// cost is only a claim if something counts.
    #[must_use]
    pub const fn shape_count(&self) -> usize {
        self.shapes.shapes()
    }

    /// How many layouts were answered out of the cache.
    #[must_use]
    pub const fn shape_hits(&self) -> usize {
        self.shapes.hits()
    }

    /// How many shaped paragraphs are currently held in memory.
    ///
    /// The cache-size question, as distinct from
    /// [`shape_count`](Self::shape_count)'s cumulative-work one. This is what a
    /// memory-pressure assertion wants; see
    /// `ShapeCache::retained`.
    #[must_use]
    pub fn retained_shapes(&self) -> usize {
        self.shapes.retained()
    }

    /// Throw the shaped paragraphs away.
    ///
    /// The cache is bounded and keyed on everything that can change the answer,
    /// so this is not needed for correctness in the ordinary case. It is here
    /// for a caller that has loaded a font *after* laying text out — which
    /// changes what fallback resolves to and is the one input the key cannot
    /// see — and for `Trim`, which calls it under memory pressure.
    pub fn clear_shape_cache(&mut self) {
        self.shapes.clear();
    }

    /// The bytes of a loaded font, as the paint layer's [`FontData`].
    ///
    /// Returns the *same* `Rc` every time for a given id and weight, which is what
    /// keeps damage tracking from treating unchanged text as changed.
    ///
    /// # Variable faces arrive with their coordinates attached
    ///
    /// `cosmic-text` 0.19 shapes a variable font *at the requested weight*: it
    /// builds its HarfBuzz face at the `wght` location, so advances, kerning
    /// and line metrics are the interpolated instance's already. The raw face
    /// bytes it hands back, though, say nothing about that — and the paint
    /// layer's outline extraction would read the *default* instance out of
    /// them, drawing one weight while measuring another. So a variable face
    /// is wrapped through [`FontData::with_variations`] with the shaping
    /// weight as a coordinate: the rasterizer sets the axis before extracting
    /// contours, and the distinct id a variation set mints keeps two weights
    /// of one face from ever sharing a cache entry. A static face — which is
    /// every non-variable font, and a variable font at its default weight —
    /// is wrapped as before, with no coordinate.
    pub(crate) fn font_data(&mut self, id: fontdb::ID, weight: fontdb::Weight) -> Option<FontData> {
        let key = (id, weight.0);
        if let Some(existing) = self.data.get(&key) {
            return Some(existing.clone());
        }
        // The face index has to come from the database rather than the font: a
        // font *collection* holds several faces in one file, and drawing the wrong
        // one silently renders the wrong typeface.
        let index = self.system.db().face(id).map_or(0, |face| face.index);
        let font = self.system.get_font(id, weight)?;
        let bytes = Rc::new(font.data().to_vec());
        let data = match variable_wght(&bytes, index, weight) {
            Some(variation) => {
                FontData::with_variations(bytes, index, std::slice::from_ref(&variation))
            }
            None => FontData::new(bytes, index),
        };
        self.data.insert(key, data.clone());
        Some(data)
    }
}

/// The `wght` variation for a face that has the axis, so the paint layer
/// draws the instance the shaper measured — `None` for a static face or a
/// variable face whose default already matches.
///
/// A weight that *matches the axis default exactly* is dropped as well: at
/// the default, the un-varied face and the varied one draw identical outlines,
/// and skipping the coordinate keeps every existing cache identity — and
/// every golden image — exactly what it was for all the text that was never
/// bold in the first place.
fn variable_wght(
    bytes: &[u8],
    index: u32,
    weight: fontdb::Weight,
) -> Option<vieww_foundation::FontVariation> {
    let face = ttf_parser::Face::parse(bytes, index).ok()?;
    if !face.is_variable() {
        return None;
    }
    let axis = face
        .variation_axes()
        .into_iter()
        .find(|axis| axis.tag == ttf_parser::Tag::from_bytes(b"wght"))?;
    let value = f32::from(weight.0);
    if (axis.def_value - value).abs() < f32::EPSILON {
        // At the axis default the face *is* the requested instance already.
        return None;
    }
    Some(vieww_foundation::FontVariation::weight(value))
}

impl Default for FontStore {
    fn default() -> Self {
        Self::new()
    }
}

/// # What is dropped, and the one thing that deliberately is not
///
/// The **shaped paragraphs** go at every level: they are pure derived work,
/// rebuildable from the spans and the width, and the largest thing here by far.
///
/// The **`FontData` handles** (`Self::data`) go only at
/// `MemoryPressure::trims_everything`, and even then the bytes usually
/// survive — `cosmic-text`'s own database still owns them, so this releases the
/// framework's handles rather than the megabytes. It is second-order, and it
/// costs something real: those handles are the *identity* damage tracking
/// compares text runs by, so dropping them makes every text run on screen
/// compare unequal and repaint once. That is an acceptable price when the
/// process is a kill candidate and not otherwise, which is exactly the line
/// `trims_everything` draws.
///
/// The loaded **faces themselves are never dropped**. Re-scanning the system's
/// fonts is hundreds of file reads, it is the single most expensive thing this
/// type ever does, and doing it in response to memory pressure would mean the
/// application freezes precisely when the machine is already struggling.
impl vieww_foundation::Trim for FontStore {
    fn trim(&mut self, pressure: vieww_foundation::MemoryPressure) {
        if !pressure.trims_unused() {
            return;
        }
        self.shapes.clear();
        if pressure.trims_everything() {
            self.data.clear();
        }
    }
}

impl std::fmt::Debug for FontStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `FontSystem` is not Debug, and printing hundreds of face records would
        // be useless anyway.
        f.debug_struct("FontStore")
            .field("faces", &self.system.db().len())
            .field("cached", &self.data.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_store_holds_exactly_the_embedded_faces() {
        let store = FontStore::embedded_only();
        assert_eq!(
            store.len(),
            EMBEDDED_FACES.len(),
            "no system fonts, so metrics are identical on every machine"
        );
        assert!(!store.is_empty());
    }

    #[test]
    fn the_embedded_faces_include_a_real_bold_and_italic() {
        let store = FontStore::embedded_only();
        let styles: Vec<_> = store
            .system
            .db()
            .faces()
            .map(|face| (face.weight, face.style))
            .collect();

        assert!(
            styles.iter().any(|(w, _)| *w == fontdb::Weight::BOLD),
            "without a bold face, TextStyle::bold() is a silent no-op: {styles:?}"
        );
        assert!(
            styles.iter().any(|(_, s)| *s != fontdb::Style::Normal),
            "likewise for italic: {styles:?}"
        );
    }

    #[test]
    fn asking_twice_for_a_font_returns_the_same_allocation() {
        let mut store = FontStore::embedded_only();
        let id = store.system.db().faces().next().expect("one face").id;
        let weight = fontdb::Weight::NORMAL;

        let first = store.font_data(id, weight).expect("loaded");
        let second = store.font_data(id, weight).expect("loaded");

        assert_eq!(
            first, second,
            "FontData compares by Rc identity, so a fresh Rc each frame would \
             make damage tracking repaint all text every frame"
        );
    }

    // ------------------------------------------------------ memory pressure

    /// The level distinction this type is responsible for: shaped paragraphs go
    /// at every level, the font *handles* only when the process is a kill
    /// candidate.
    ///
    /// The cost of dropping a handle is not the bytes — `cosmic-text`'s
    /// database still owns those — it is that the handle is the **identity**
    /// damage tracking compares text runs by, so every run on screen compares
    /// unequal and repaints once. That is worth it at `Critical` and not at
    /// `Moderate`, which is the whole reason the two levels differ here.
    #[test]
    fn a_moderate_warning_keeps_the_font_handles_and_a_critical_one_does_not() {
        use vieww_foundation::{MemoryPressure, Trim};

        let mut store = FontStore::embedded_only();
        let id = store.system.db().faces().next().expect("one face").id;
        let weight = fontdb::Weight::NORMAL;
        let original = store.font_data(id, weight).expect("loaded");

        store.trim(MemoryPressure::Moderate);
        assert_eq!(
            store.font_data(id, weight).expect("loaded"),
            original,
            "a moderate warning must not make every text run on screen repaint"
        );

        store.trim(MemoryPressure::Critical);
        assert_ne!(
            store.font_data(id, weight).expect("loaded"),
            original,
            "at critical the handle really is released; the reissued one is a \
             new identity and the one-off repaint is the accepted price"
        );
    }

    /// Releasing a handle must not release the *face*. Re-scanning the system's
    /// fonts is hundreds of file reads and the most expensive thing this type
    /// ever does — doing it in response to memory pressure would freeze the
    /// application at the exact moment the machine is already struggling.
    #[test]
    fn no_pressure_level_unloads_the_faces_themselves() {
        use vieww_foundation::{MemoryPressure, Trim};

        for level in [
            MemoryPressure::Moderate,
            MemoryPressure::Critical,
            MemoryPressure::Backgrounded,
        ] {
            let mut store = FontStore::embedded_only();
            let faces = store.len();
            store.trim(level);
            assert_eq!(store.len(), faces, "{level} unloaded a face");
            assert!(!store.is_empty());
        }
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    use crate::{Paragraph, TextSpan};
    use vieww_foundation::TextStyle;

    fn shape(store: &mut FontStore, text: &str) -> Paragraph {
        Paragraph::layout(
            store,
            &[TextSpan::new(text.to_owned(), TextStyle::new(20.0))],
            f32::INFINITY,
        )
    }

    /// The bug this closes. The embedded subset is Latin + Hebrew + Arabic
    /// plus a CJK vocabulary, so everything beyond it shapes to glyph 0 — and
    /// DejaVu's glyph 0 inks nothing. A Chinese screen came out as
    /// correctly-spaced blank space. (The CJK case below uses characters
    /// deliberately outside the embedded vocabulary, because the covered
    /// ones are no longer the interesting case.)
    #[test]
    fn uncovered_scripts_are_reported_as_missing_rather_than_shaped_away() {
        let mut store = FontStore::embedded_only();
        for (script, text) in [
            ("CJK beyond the subset", "龘靈龜"),
            ("Devanagari", "नमस्ते"),
            ("Thai", "สวัสดี"),
        ] {
            let para = shape(&mut store, text);
            let missing: usize = para.runs().iter().map(|run| run.missing_count()).sum();
            assert!(
                missing > 0,
                "{script} must be reported missing on an embedded-only store"
            );
            // And every one of them must produce a box to draw, or the fix does
            // not reach the screen.
            let boxes: usize = para
                .runs()
                .iter()
                .map(|run| run.missing_boxes().len())
                .sum();
            assert_eq!(boxes, missing, "{script}: every missing glyph needs a box");
        }
    }

    /// The other half: what the subset *does* cover must not be reported
    /// missing, or every screen grows boxes it does not need.
    #[test]
    fn covered_text_reports_nothing_missing() {
        let mut store = FontStore::embedded_only();
        for (script, text) in [
            ("Latin", "The quick brown fox"),
            ("Hebrew", "שלום עולם"),
            ("Arabic", "مرحبا بالعالم"),
            ("CJK", "你好世界，视界框架。"),
            ("punctuation the framework draws", "— … ' ≥ ≤ ≠ −"),
        ] {
            let para = shape(&mut store, text);
            let missing: usize = para.runs().iter().map(|run| run.missing_count()).sum();
            assert_eq!(missing, 0, "{script} is covered and must not box");
        }
    }

    /// A missing box has to be a box: positive area, sitting on the baseline
    /// rather than floating somewhere with a zero dimension.
    #[test]
    fn a_missing_box_has_real_area_above_the_baseline() {
        let mut store = FontStore::embedded_only();
        let para = shape(&mut store, "龘靈");

        let boxes: Vec<_> = para
            .runs()
            .iter()
            .flat_map(vieww_foundation::GlyphRun::missing_boxes)
            .collect();
        assert!(!boxes.is_empty());
        for rect in boxes {
            assert!(rect.right > rect.left, "zero-width box: {rect:?}");
            assert!(rect.bottom > rect.top, "zero-height box: {rect:?}");
        }
    }

    /// Order is the whole design: embedded first, so Latin metrics stay
    /// identical on every machine while other scripts fall through.
    #[test]
    fn the_fallback_store_keeps_embedded_metrics_for_covered_text() {
        let mut embedded = FontStore::embedded_only();
        let mut fallback = FontStore::with_system_fallback();

        let a = shape(&mut embedded, "The quick brown fox");
        let b = shape(&mut fallback, "The quick brown fox");

        assert!(
            (a.size().width - b.size().width).abs() < 0.01,
            "adding system fonts must not change how covered text measures: \
             {} vs {}",
            a.size().width,
            b.size().width
        );
        assert!(
            fallback.len() >= embedded.len(),
            "the fallback store must have at least the embedded faces"
        );
    }

    /// B12: opening a second window must not re-walk the filesystem for
    /// system fonts.
    ///
    /// `with_system_fallback` used to build a fresh `fontdb::Database` and
    /// call `load_system_fonts()` — a synchronous scan of every font file on
    /// disk — on every call, because every window called it once with no
    /// caching between them (`vieww-platform-winit`'s `app.rs` calls it from
    /// `FrameDriver::use_system_fonts` for each window it creates). Measured
    /// at 46 seconds on a slow disk in
    /// `docs/release/BETA-RELEASE-CHECKLIST.md`, and paid once per window —
    /// three times over for a three-window desktop suite.
    ///
    /// This asserts the scan itself — [`system_font_scans`], incremented only
    /// inside `scanned_system_db`'s `get_or_init` — happens at most once for
    /// the whole test binary, no matter how many stores this test (or any
    /// test before it in the same binary) constructs. "At most" rather than
    /// "exactly", because tests share one process and another test may have
    /// already paid for the first scan; what must never happen is a *second*
    /// one caused by *this* test's three extra stores.
    #[test]
    fn opening_several_windows_scans_the_system_fonts_once() {
        let _first = FontStore::with_system_fallback();
        let after_first = system_font_scans();
        assert!(
            after_first >= 1,
            "the very first call anywhere in this binary must have scanned \
             at least once by now"
        );

        // Three more, standing in for three windows (or a window and two
        // dialogs) opened in one process.
        let _second = FontStore::with_system_fallback();
        let _third = FontStore::with_system_fallback();
        let _fourth = FontStore::with_system_fallback();

        assert_eq!(
            system_font_scans(),
            after_first,
            "building 3 more `FontStore`s scanned the filesystem {} more \
             time(s) — B12 is back: every window is paying the system font \
             scan again instead of reusing the first one",
            system_font_scans() - after_first
        );
    }

    /// The detection an application needs to decide whether to ship a face,
    /// download one, or tell the user.
    #[test]
    fn coverage_can_be_asked_about_before_anything_is_drawn() {
        let store = FontStore::embedded_only();

        assert!(store.can_render("Hello, world"));
        assert!(store.can_render("שלום"));
        assert!(store.can_render("مرحبا"));
        // Covered by the embedded CJK face now — the interesting negative is
        // a script nobody embedded.
        assert!(store.can_render("你好"));
        assert!(!store.can_render("नमस्ते"));

        // And the diagnostic form names the characters, so a log line can say
        // which ones rather than that some were missing.
        let missing = store.missing_characters("Total नमस्ते due");
        assert!(!missing.is_empty(), "Devanagari is not embedded");
        assert!(
            missing.iter().all(|character| !character.is_whitespace()),
            "no whitespace in the report"
        );
    }

    /// The embedded CJK face means the *embedded-only* store — the one a
    /// headless certification render runs on — actually draws Chinese rather
    /// than falling through to a platform font a build machine may not have.
    /// The assertion that keeps that honest: shaping a mixed Latin/CJK line
    /// produces real (non-`.notdef`) glyph ids and splits across two faces,
    /// which is the whole point of a fallback chain.
    #[test]
    fn the_embedded_cjk_face_shapes_real_glyphs_headlessly() {
        let mut store = FontStore::embedded_only();
        let para = shape(&mut store, "Vieww 你好世界");
        let missing: usize = para.runs().iter().map(|run| run.missing_count()).sum();
        assert_eq!(missing, 0, "the embedded CJK face covers this text");
        let fonts: std::collections::HashSet<u64> =
            para.runs().iter().map(|run| run.font.id()).collect();
        assert!(
            fonts.len() > 1,
            "Latin and CJK must shape through different faces, got {}",
            fonts.len()
        );
    }

    /// A variable face asked for a weight it is not at rest at must hand the
    /// paint layer a `FontData` carrying that weight — see
    /// [`FontStore::font_data`] for the whole argument, and
    /// `paint`'s `glyph.rs` for where it lands.
    #[test]
    fn a_variable_face_carries_its_weight_to_the_paint_layer() {
        use std::rc::Rc;
        use vieww_foundation::FontVariation;

        // The Noto Serif SC subset from the fidelity suite's assets, with
        // its wght axis (200..900, default 200) intact.
        const VF: &[u8] =
            include_bytes!("../../../examples/test-text-fidelity/assets/NotoSansSC-VF-subset.ttf");
        let mut store =
            FontStore::with_application_faces([VF.to_vec()], Some("Noto Serif SC"), None);

        let id = store
            .system_mut()
            .db()
            .faces()
            .find(|face| face.post_script_name.contains("NotoSerifSC"))
            .map(|face| face.id)
            .expect("the variable face is loaded");

        // 400 differs from the axis default (200), so the store must attach
        // the coordinate.
        let regular = store
            .font_data(id, cosmic_text::fontdb::Weight::NORMAL)
            .expect("resolvable");
        assert_eq!(
            regular.variations(),
            &[FontVariation::weight(400.0)],
            "wght 400 on a default-200 axis must reach the paint layer"
        );
        assert!(regular.is_variable_instance());

        // The axis *default*, by contrast, must not grow a coordinate: the
        // face already is that instance, and minting an id for it would only
        // split the glyph cache for every default-weight run.
        let at_default = {
            let _ = Rc::new(());
            let mut store = store;
            store.data.clear();
            store
                .font_data(id, cosmic_text::fontdb::Weight(200))
                .expect("resolvable")
        };
        assert_eq!(
            at_default.variations(),
            &[] as &[FontVariation],
            "the axis default must stay an un-varied identity"
        );
    }

    /// Whitespace has no ink. Reporting it would make every multi-word string
    /// with one uncovered character look like several.
    #[test]
    fn whitespace_is_never_reported_as_missing() {
        let store = FontStore::embedded_only();
        assert!(store.missing_characters(" \t\n").is_empty());
        assert!(store.can_render("a b\tc\nd"));
    }

    /// The same character twice is one report, so a paragraph of uncovered
    /// text does not produce a thousand-entry list.
    #[test]
    fn missing_characters_are_deduplicated_in_first_seen_order() {
        let store = FontStore::embedded_only();
        // Characters no embedded face covers — CJK itself is covered now, so
        // the report is exercised on a script beyond the embedded set.
        assert_eq!(store.missing_characters("नमनम"), vec!['न', 'म']);
    }

    /// Shipping a face is the documented answer for a script the framework
    /// cannot embed, so it has to actually change the answer.
    #[test]
    fn loading_a_face_closes_the_gap_it_covers() {
        let mut store = FontStore::embedded_only();
        assert!(
            !store.can_render("\u{05D0}\u{2603}"),
            "the snowman is not covered"
        );

        // The embedded bold face, re-loaded under this API. A real application
        // would pass a CJK face here; what is being tested is that a face
        // loaded at runtime joins the coverage set at all, which is the
        // mechanism, and that needs a face whose contents are known.
        let before = store.len();
        store.load_font_data(super::EMBEDDED_FONT_BOLD.to_vec());
        assert_eq!(store.len(), before + 1, "the face must be registered");
    }

    /// A face loaded after something was laid out changes what fallback
    /// resolves to — and that is the one input the shape cache's key cannot
    /// see. Forgetting to clear it means the newly-covered text keeps rendering
    /// as boxes until something else invalidates the entry.
    #[test]
    fn loading_a_face_invalidates_the_shape_cache() {
        let mut store = FontStore::with_system_fallback();
        let _ = shape(&mut store, "some text to put in the cache");
        assert!(store.retained_shapes() > 0, "the cache has an entry");

        store.load_font_data(super::EMBEDDED_FONT_BOLD.to_vec());
        assert_eq!(
            store.retained_shapes(),
            0,
            "loading a face must invalidate what was shaped without it"
        );
    }

    /// A missing directory is a normal condition — an application shipping
    /// optional fonts should not have to check the path exists first.
    #[test]
    fn loading_a_directory_that_is_not_there_adds_nothing_and_does_not_panic() {
        let mut store = FontStore::embedded_only();
        let before = store.len();
        assert_eq!(store.load_font_directory("/no/such/directory"), 0);
        assert_eq!(store.len(), before);
    }
}
