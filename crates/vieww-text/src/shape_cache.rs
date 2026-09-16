//! Checklist item 6: shaping the same words twice is the largest repeated cost
//! left in a frame.
//!
//! # The gap
//!
//! [`Paragraph::layout_aligned`](crate::Paragraph::layout_aligned) built a fresh
//! `cosmic_text::Buffer` and called `shape_until_scroll` on **every call**.
//! Nothing above it cached, so the shaper ran again for a label whose text,
//! style and width had not moved — once for the layout pass, again for each
//! intrinsic query, and again on the next frame that laid anything out.
//!
//! Shaping is not cheap: itemisation, font fallback per script, `rustybuzz` per
//! run, line breaking, bidi reordering. It is the single most expensive thing
//! the text layer does, and it was the only phase in the framework with no
//! retention of any kind — every other one has a named mechanism and a test
//! asserting the *absence* of work.
//!
//! # What is cached, and what is not
//!
//! The key is everything the shaped result depends on: the spans (their text
//! **and** their style), the width it wrapped against, the alignment, and the
//! base direction. Nothing else can change the answer — the store's fonts are
//! the only other input, and a store whose fonts changed is a different store.
//!
//! Colour is part of the style and therefore part of the key, which is
//! deliberately conservative: a colour change alone cannot alter a glyph
//! position, so keying on it costs a re-shape that a narrower key would avoid.
//! Narrowing it means splitting `TextStyle` into "affects shaping" and "affects
//! painting" halves, which is a real change to a public type and wants its own
//! measurement rather than being smuggled in here.
//!
//! # Floats in a hash key
//!
//! By bit pattern, the same way `vieww-paint`'s `SceneFlattener` does it: a
//! `f32` is hashed and compared as its `u32` bits, so the key is exactly `Hash +
//! Eq` and a `NaN` width simply never matches itself. A `NaN` that matched would
//! serve a paragraph laid out against a different width.
//!
//! # Collisions are checked, not assumed
//!
//! The map is keyed by a 64-bit hash, and every hit **verifies the full key**
//! before answering. A 64-bit collision is unlikely; a 64-bit collision that
//! draws somebody else's sentence is not a defect worth leaving to probability
//! when the check costs one string comparison on a path that just avoided
//! shaping.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use vieww_foundation::FastHasher;

use vieww_foundation::{TextAlign, TextDirection};

use crate::paragraph::{Paragraph, TextSpan};

/// How many laid-out paragraphs to keep.
///
/// A screen holds tens of distinct strings; a list scrolling through a thousand
/// rows holds more, but reuses the same *styles* and only a window of the text.
/// 256 covers a dense screen several times over and bounds the memory at a few
/// hundred `Vec<GlyphRun>`s.
///
/// Evicted oldest-first rather than least-recently-used: FIFO needs no
/// bookkeeping on the hit path, and the access pattern that would punish it —
/// cycling through exactly `CAPACITY + 1` paragraphs for ever — is not what a
/// scrolling list does.
const CAPACITY: usize = 256;

/// Everything a shaped paragraph depends on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Key {
    spans: Vec<TextSpan>,
    max_width: f32,
    align: TextAlign,
    direction: TextDirection,
}

impl Key {
    pub(crate) fn new(
        spans: &[TextSpan],
        max_width: f32,
        align: TextAlign,
        direction: TextDirection,
    ) -> Self {
        Self {
            spans: spans.to_vec(),
            max_width,
            align,
            direction,
        }
    }

    /// The 64-bit hash the map is bucketed by.
    ///
    /// Hand-written rather than derived because `TextSpan` holds a `TextStyle`
    /// holding four `f32`s, and `f32` is not `Hash` — for the good reason that
    /// `NaN != NaN`. Bits sidestep that without pretending otherwise: two
    /// `NaN`s with the same bits hash the same and then fail the full-key
    /// comparison, which is the correct outcome.
    fn hash(&self) -> u64 {
        // `FastHasher`, not the default one. This key is built by the
        // framework out of its own text and styles, it is hashed on every text
        // layout, and SipHash's collision resistance buys nothing against an
        // input the framework produced itself. The bucket holds full keys and a
        // hit is verified against them, so a collision costs a comparison
        // rather than a wrong answer.
        let mut hasher = FastHasher::default();
        self.max_width.to_bits().hash(&mut hasher);
        (self.align as u8).hash(&mut hasher);
        (self.direction as u8).hash(&mut hasher);
        self.spans.len().hash(&mut hasher);
        for span in &self.spans {
            span.text.hash(&mut hasher);
            let style = &span.style;
            style.size.to_bits().hash(&mut hasher);
            style.line_height.to_bits().hash(&mut hasher);
            style.letter_spacing.to_bits().hash(&mut hasher);
            style.italic.hash(&mut hasher);
            // **Hashed directly.** These three were `format!("{:?}", …)` and
            // then hashed — three heap allocations, one of them rendering a
            // `Color` through its `Debug` impl, on every key, on every text
            // layout, on every frame. A profile of one keystroke in the studio
            // put roughly fifteen hundred allocations a frame right here, in a
            // cache whose entire purpose is to be cheaper than the work it
            // avoids. `Color` and `FontFamily` already derived `Hash`; only
            // `FontWeight` had to gain it.
            style.weight.hash(&mut hasher);
            style.family.hash(&mut hasher);
            style.color.hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// Laid-out paragraphs, by what produced them.
#[derive(Debug, Default)]
pub(crate) struct ShapeCache {
    /// Bucketed by hash; each bucket holds full keys so a hit is verified.
    entries: HashMap<u64, Vec<(Key, Paragraph)>>,
    /// Insertion order, for eviction.
    order: VecDeque<u64>,
    /// How many paragraphs have actually been shaped.
    shapes: usize,
    hits: usize,
}

impl ShapeCache {
    /// The paragraph for `key`, if it has been laid out.
    pub(crate) fn get(&mut self, key: &Key) -> Option<Paragraph> {
        let bucket = self.entries.get(&key.hash())?;
        let found = bucket
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, p)| p.clone());
        if found.is_some() {
            self.hits += 1;
        }
        found
    }

    /// Remember `paragraph` as the result for `key`.
    pub(crate) fn insert(&mut self, key: Key, paragraph: Paragraph) {
        self.shapes += 1;
        let hash = key.hash();
        let bucket = self.entries.entry(hash).or_default();
        // An identical key can arrive twice if two threads — or two calls
        // straddling a `get` — raced. Replace rather than grow the bucket.
        if let Some(slot) = bucket.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = paragraph;
            return;
        }
        bucket.push((key, paragraph));
        self.order.push_back(hash);

        while self.order.len() > CAPACITY {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            // A bucket holds every key that hashed here, and only one of them is
            // being evicted — but which one is not recorded, so the whole bucket
            // goes. Buckets have one entry except under collision, so this is
            // one paragraph in every realistic case.
            self.entries.remove(&oldest);
        }
    }

    /// How many paragraphs have been shaped. Counted work: a claim about cost
    /// is only a claim if something counts.
    pub(crate) const fn shapes(&self) -> usize {
        self.shapes
    }

    /// How many layouts were answered without shaping.
    pub(crate) const fn hits(&self) -> usize {
        self.hits
    }

    /// How many paragraphs are **currently retained**.
    ///
    /// A different question from [`shapes`](Self::shapes), and the distinction
    /// matters enough to be worth stating: `shapes` is cumulative *work* and
    /// only ever grows, so it answers "was this recomputed?" and can never
    /// answer "is this still in memory?". Trimming empties the cache without
    /// un-doing the work that filled it, so `shapes` is exactly the wrong
    /// counter to assert a trim against — which is a mistake
    /// `tests/memory_pressure.rs` made before this existed.
    pub(crate) fn retained(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    /// Throw everything away. For a test that wants a cold cache, and for a
    /// caller that has changed the fonts underneath it.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FontStore;
    use vieww_foundation::{Color, FontFamily, TextStyle};

    fn store() -> FontStore {
        // The embedded font only, so metrics are identical on every machine and
        // a wrap point can be asserted exactly.
        FontStore::embedded_only()
    }

    fn spans(text: &str) -> Vec<TextSpan> {
        vec![TextSpan::new(text, TextStyle::new(14.0))]
    }

    /// Everything observable about a paragraph, as a string.
    ///
    /// The oracle. Asserting only that a cached call is *fast* would pass for a
    /// cache that returned the wrong paragraph, so every test below compares
    /// this instead of trusting the counter.
    fn describe(p: &Paragraph) -> String {
        let mut out = format!(
            "size={:?} widest={} dir={:?} lines={} glyphs={}",
            p.size(),
            p.widest_line(),
            p.direction(),
            p.line_count(),
            p.glyph_count(),
        );
        for run in p.runs() {
            out.push_str(&format!("\n  run at {:?} size={}", run.origin, run.size));
            for glyph in run.glyphs.iter() {
                out.push_str(&format!(" [{} @ {:?}]", glyph.id, glyph.offset));
            }
        }
        out
    }

    #[test]
    fn the_same_paragraph_is_shaped_once() {
        let mut store = store();
        let spans = spans("The quick brown fox");

        let first = Paragraph::layout(&mut store, &spans, 200.0);
        assert_eq!(store.shape_count(), 1);

        let second = Paragraph::layout(&mut store, &spans, 200.0);
        assert_eq!(store.shape_count(), 1, "the second layout re-shaped");
        assert_eq!(store.shape_hits(), 1);

        // And it is the same paragraph, not merely a cheap one.
        assert_eq!(describe(&first), describe(&second));
    }

    /// The cache must be invisible: for every input, what comes out of it has
    /// to equal what a cold store produces.
    /// One case for the two tests below: what to lay out, and how.
    struct Case {
        what: &'static str,
        spans: Vec<TextSpan>,
        width: f32,
        align: TextAlign,
        direction: Option<TextDirection>,
    }

    #[test]
    fn a_cached_paragraph_equals_a_freshly_shaped_one() {
        let cases: Vec<(Vec<TextSpan>, f32, TextAlign, Option<TextDirection>)> = vec![
            (spans("hello"), f32::INFINITY, TextAlign::Start, None),
            (
                spans("hello world, wrapping here"),
                60.0,
                TextAlign::Start,
                None,
            ),
            (spans("centred"), 120.0, TextAlign::Center, None),
            (spans("right"), 120.0, TextAlign::End, None),
            (spans("שלום עולם"), 120.0, TextAlign::Start, None),
            (
                spans("forced"),
                120.0,
                TextAlign::Start,
                Some(TextDirection::Rtl),
            ),
            (spans(""), 120.0, TextAlign::Start, None),
            (spans("tab\tand\nnewline"), 120.0, TextAlign::Start, None),
            (
                vec![
                    TextSpan::new("bold ", TextStyle::new(18.0)),
                    TextSpan::new("and small", TextStyle::new(9.0)),
                ],
                100.0,
                TextAlign::Start,
                None,
            ),
        ];

        for (spans, width, align, direction) in cases {
            // Cold: a store that has never seen this.
            let mut cold = store();
            let expected = Paragraph::layout_aligned(&mut cold, &spans, width, align, direction);

            // Warm: laid out once to fill the cache, then again to read it.
            let mut warm = store();
            let _ = Paragraph::layout_aligned(&mut warm, &spans, width, align, direction);
            let cached = Paragraph::layout_aligned(&mut warm, &spans, width, align, direction);

            assert_eq!(
                describe(&expected),
                describe(&cached),
                "cache disagreed for {spans:?} at {width}"
            );
            assert_eq!(warm.shape_count(), 1);
        }
    }

    /// Every field of the key has to be *in* the key. A field left out serves a
    /// paragraph laid out under different conditions, which is a wrong picture
    /// rather than a slow one.
    #[test]
    fn changing_any_input_reshapes() {
        let base = spans("hello");

        let plain = |what, spans, width, align, direction| Case {
            what,
            spans,
            width,
            align,
            direction,
        };
        let styled = |what, style| Case {
            what,
            spans: vec![TextSpan::new("hello", style)],
            width: 100.0,
            align: TextAlign::Start,
            direction: None,
        };

        let variants = vec![
            plain(
                "different text",
                spans("hallo"),
                100.0,
                TextAlign::Start,
                None,
            ),
            plain(
                "different width",
                base.clone(),
                50.0,
                TextAlign::Start,
                None,
            ),
            plain("different align", base.clone(), 100.0, TextAlign::End, None),
            plain(
                "different direction",
                base.clone(),
                100.0,
                TextAlign::Start,
                Some(TextDirection::Rtl),
            ),
            styled("different size", TextStyle::new(24.0)),
            styled(
                "different colour",
                TextStyle::new(14.0).color(Color::rgb(255, 0, 0)),
            ),
            styled(
                "different family",
                TextStyle::new(14.0).family(FontFamily::Monospace),
            ),
            styled(
                "different letter spacing",
                TextStyle {
                    letter_spacing: 2.0,
                    ..TextStyle::new(14.0)
                },
            ),
            plain(
                "split into two spans",
                vec![
                    TextSpan::new("hel", TextStyle::new(14.0)),
                    TextSpan::new("lo", TextStyle::new(14.0)),
                ],
                100.0,
                TextAlign::Start,
                None,
            ),
        ];

        for Case {
            what,
            spans,
            width,
            align,
            direction,
        } in variants
        {
            let mut store = store();
            let _ = Paragraph::layout_aligned(&mut store, &base, 100.0, TextAlign::Start, None);
            assert_eq!(store.shape_count(), 1);

            let _ = Paragraph::layout_aligned(&mut store, &spans, width, align, direction);
            assert_eq!(
                store.shape_count(),
                2,
                "{what} was served from the cache, so it is not in the key"
            );
        }
    }

    /// An inferred direction and the same direction passed explicitly describe
    /// one paragraph. Keying on the *argument* would shape it twice.
    #[test]
    fn an_inferred_direction_and_the_same_one_passed_in_are_one_entry() {
        let mut store = store();
        let spans = spans("plain latin text");
        let _ = Paragraph::layout_aligned(&mut store, &spans, 200.0, TextAlign::Start, None);
        let _ = Paragraph::layout_aligned(
            &mut store,
            &spans,
            200.0,
            TextAlign::Start,
            Some(TextDirection::Ltr),
        );
        assert_eq!(store.shape_count(), 1);
    }

    /// A `NaN` width is not equal to itself, so it must never hit — the
    /// alternative is serving a paragraph laid out against some other width.
    #[test]
    fn a_nan_width_never_matches_itself() {
        let mut store = store();
        let spans = spans("hello");
        let _ = Paragraph::layout(&mut store, &spans, f32::NAN);
        let _ = Paragraph::layout(&mut store, &spans, f32::NAN);
        assert_eq!(store.shape_count(), 2, "a NaN matched itself");
    }

    #[test]
    fn the_cache_is_bounded() {
        let mut cache = ShapeCache::default();
        let mut store = store();
        let paragraph = Paragraph::layout(&mut store, &spans("x"), 100.0);

        for n in 0..CAPACITY * 2 {
            let key = Key::new(
                &spans(&format!("line {n}")),
                100.0,
                TextAlign::Start,
                TextDirection::Ltr,
            );
            cache.insert(key, paragraph.clone());
        }
        assert!(
            cache.entries.len() <= CAPACITY,
            "{} entries retained, cap is {CAPACITY}",
            cache.entries.len()
        );

        // And the newest is still there.
        let newest = Key::new(
            &spans(&format!("line {}", CAPACITY * 2 - 1)),
            100.0,
            TextAlign::Start,
            TextDirection::Ltr,
        );
        assert!(cache.get(&newest).is_some(), "the newest entry was evicted");
    }

    /// The check that makes a 64-bit hash safe to key on.
    ///
    /// Two different keys cannot be made to hash the same on demand, so the
    /// collision is staged instead: a **foreign entry is put in front of the
    /// real one in the same bucket**, which is exactly the state a real
    /// collision produces. An implementation that answered with the bucket's
    /// first entry — which is what "trust the hash" means in code — returns the
    /// wrong paragraph here.
    #[test]
    fn a_bucket_collision_does_not_serve_the_wrong_paragraph() {
        let mut cache = ShapeCache::default();
        let mut store = store();
        let one = Paragraph::layout(&mut store, &spans("one"), 100.0);
        let two = Paragraph::layout(&mut store, &spans("two"), 100.0);
        assert_ne!(describe(&one), describe(&two), "the fixtures must differ");

        let key_one = Key::new(&spans("one"), 100.0, TextAlign::Start, TextDirection::Ltr);
        let key_two = Key::new(&spans("two"), 100.0, TextAlign::Start, TextDirection::Ltr);
        assert_ne!(key_one, key_two);

        let bucket = cache.entries.entry(key_one.hash()).or_default();
        bucket.push((key_two.clone(), two.clone()));
        bucket.push((key_one.clone(), one.clone()));

        assert_eq!(
            describe(&cache.get(&key_one).expect("the real entry")),
            describe(&one),
            "the bucket's first entry was served instead of the matching one"
        );
        // And a key that is genuinely absent is absent, however the buckets look.
        let key_three = Key::new(&spans("three"), 100.0, TextAlign::Start, TextDirection::Ltr);
        assert!(cache.get(&key_three).is_none());
    }

    #[test]
    fn clearing_forgets_everything() {
        let mut store = store();
        let spans = spans("hello");
        let _ = Paragraph::layout(&mut store, &spans, 100.0);
        store.clear_shape_cache();
        let _ = Paragraph::layout(&mut store, &spans, 100.0);
        assert_eq!(store.shape_count(), 2);
    }

    // ---------------------------------------------------- memory pressure

    /// `retained` counts what is *held*; `shapes` counts what was *done*. The
    /// two are one word apart at the call site and answer opposite questions,
    /// so the difference is pinned rather than left to a reader.
    #[test]
    fn retained_is_a_cache_size_and_shapes_is_cumulative_work() {
        let mut store = store();
        let spans = spans("hello");
        let _ = Paragraph::layout(&mut store, &spans, 100.0);

        assert_eq!(store.retained_shapes(), 1);
        assert_eq!(store.shape_count(), 1);

        store.clear_shape_cache();

        assert_eq!(store.retained_shapes(), 0, "the entry is gone");
        assert_eq!(
            store.shape_count(),
            1,
            "the work still happened; a work counter must not go down"
        );
    }

    /// The claim `crates/vieww/tests/memory_pressure.rs` cannot make, because
    /// an idle frame never lays out and so never reaches the shaper: after a
    /// trim, the *same* text at the *same* width is genuinely shaped again
    /// rather than answered from a cache that only looked empty.
    #[test]
    fn shaping_after_a_trim_is_real_work() {
        use vieww_foundation::{MemoryPressure, Trim};

        let mut store = store();
        let spans = spans("hello");
        let first = Paragraph::layout(&mut store, &spans, 100.0);
        // Proof the cache is doing its job before the trim, or the assertion
        // below would pass for a store that never cached anything.
        let _ = Paragraph::layout(&mut store, &spans, 100.0);
        assert_eq!(store.shape_count(), 1, "the second layout was a hit");
        assert_eq!(store.shape_hits(), 1);

        store.trim(MemoryPressure::Critical);
        let after = Paragraph::layout(&mut store, &spans, 100.0);

        assert_eq!(
            store.shape_count(),
            2,
            "the trimmed paragraph was re-shaped, not recovered"
        );
        // And it is the same paragraph, which is the half a counter cannot say.
        assert_eq!(describe(&after), describe(&first));
    }

    /// A moderate warning is not a lesser version of a critical one *here* —
    /// shaped paragraphs are the biggest derived thing the text layer holds, so
    /// they go at every level. The font handles are what the levels differ on,
    /// and that difference is asserted in `font.rs`.
    #[test]
    fn every_pressure_level_above_none_drops_the_shaped_paragraphs() {
        use vieww_foundation::{MemoryPressure, Trim};

        for level in [
            MemoryPressure::Moderate,
            MemoryPressure::Critical,
            MemoryPressure::Backgrounded,
        ] {
            let mut store = store();
            let _ = Paragraph::layout(&mut store, &spans("hello"), 100.0);
            assert_eq!(store.retained_shapes(), 1);

            store.trim(level);

            assert_eq!(store.retained_shapes(), 0, "{level}");
        }
    }

    #[test]
    fn no_pressure_keeps_the_shaped_paragraphs() {
        use vieww_foundation::{MemoryPressure, Trim};

        let mut store = store();
        let _ = Paragraph::layout(&mut store, &spans("hello"), 100.0);
        store.trim(MemoryPressure::None);
        assert_eq!(store.retained_shapes(), 1);
    }
}
