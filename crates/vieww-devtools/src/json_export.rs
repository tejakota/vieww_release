//! A minimal, hand-rolled JSON writer, so a browser-based devtools UI can
//! consume a real export of everything else in this crate.
//!
//! # Why hand-rolled rather than a dependency
//!
//! This workspace has no `serde` anywhere (`grep -rn serde crates/*/Cargo.toml`
//! finds nothing), and pulling in `serde` plus `serde_json` for one crate's
//! export format would be the first heavyweight dependency of that shape in
//! the whole tree. `vieww-asset`'s `svg` module sets the precedent this
//! follows: it hand-rolls its own SVG-subset parser rather than pulling in a
//! full one, because the actual surface it needs is small and known, and a
//! general-purpose parser would bring a great deal it does not. The JSON
//! this crate needs to write is exactly as small and known: numbers,
//! strings, bools, `null`, arrays, and objects with string keys — never a
//! JSON value read back into a general-purpose data structure, only ever
//! written out from types this crate already owns.
//!
//! # Why the escaping is tested directly and separately
//!
//! A hand-rolled JSON writer's entire risk is silently corrupting the one
//! input nobody tried: a quote, a backslash, an embedded control character,
//! a non-ASCII character. [`escape_str`]'s tests exercise each of those on
//! its own, byte by byte against the exact expected output, rather than
//! trusting a full round trip through [`ToJson`] to notice a subtle
//! escaping bug buried in a larger structure.

use std::fmt::Write as _;
use std::time::Duration;

use vieww_paint::FrameStats;
use vieww_render::{Role, SemanticsNode};

use crate::damage_inspector::DamageReport;
use crate::frame_timeline::FrameTimeline;
use crate::gpu_inspector::{DriverStats, GpuReport};
use crate::inspector::InspectorNode;
use crate::render_graph_inspector::{PassReport, RenderGraphReport};
use crate::semantics_inspector::SemanticsReport;

#[cfg(feature = "snapshots")]
use crate::memory_inspector::MemoryReport;

/// Escape `input` for embedding inside a JSON string literal's quotes.
///
/// Covers exactly what the JSON grammar requires: the two characters that
/// would otherwise end the string or start an escape (`"` and `\`), the
/// named short escapes JSON defines for the common control characters, and
/// a `\u00XX` escape for every other control character (`< 0x20`). Every
/// other `char` — including the whole of Unicode outside the control-
/// character range — passes through unescaped: JSON strings are UTF-8 by
/// specification, so there is nothing to encode there, unlike (for
/// instance) JavaScript string literals, which is a different grammar this
/// function does not need to satisfy.
#[must_use]
pub fn escape_str(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// A complete JSON string literal for `input`, quotes included.
#[must_use]
pub fn json_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    out.push_str(&escape_str(input));
    out.push('"');
    out
}

/// A JSON number for `value`, or the literal `null` if it is not finite.
///
/// `NaN` and the infinities have no representation in the JSON grammar —
/// writing one out as a bare token would produce output no JSON parser
/// accepts, and writing it as a string would silently change its type for
/// whatever reads this back. `null` is the honest answer: "there was a
/// number here and it was not one a document can hold."
#[must_use]
pub fn json_finite(value: f64) -> String {
    if value.is_finite() {
        // `f64`'s `Display` produces the shortest decimal that round-trips
        // exactly, in plain decimal notation — always valid JSON number
        // syntax, never a form (`NaN`, `inf`) JSON does not define.
        value.to_string()
    } else {
        "null".to_owned()
    }
}

/// A JSON array from already-serialized element fragments, each of which
/// must already be valid JSON on its own — see [`ToJson::to_json`].
#[must_use]
pub fn json_array(items: impl IntoIterator<Item = String>) -> String {
    let mut out = String::from("[");
    let mut first = true;
    for item in items {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&item);
    }
    out.push(']');
    out
}

/// Something that can render itself as one JSON value.
///
/// Implemented directly on every type this crate reports — the existing
/// [`InspectorNode`] and all five new inspector report types — plus a small
/// set of primitives ([`bool`], the integer and float types, [`str`],
/// [`Option`], slices, and [`Duration`]) so a struct's own `to_json` can be
/// built by composing field values rather than hand-writing every brace.
pub trait ToJson {
    fn to_json(&self) -> String;
}

impl ToJson for bool {
    fn to_json(&self) -> String {
        self.to_string()
    }
}

macro_rules! impl_to_json_for_integer {
    ($($t:ty),+) => {
        $(
            impl ToJson for $t {
                fn to_json(&self) -> String {
                    self.to_string()
                }
            }
        )+
    };
}
impl_to_json_for_integer!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl ToJson for f32 {
    /// Formatted through `f32`'s own `Display`, not widened to `f64` first —
    /// widening changes the value: `12.35f32 as f64` is
    /// `12.350000381469727`, the exact `f64` for the *rounded* `f32` bit
    /// pattern, and printing that leaks the rounding error into the output.
    /// `f32::to_string` produces the shortest decimal that round-trips back
    /// to the same `f32`, which is the number a caller actually wrote.
    fn to_json(&self) -> String {
        if self.is_finite() {
            self.to_string()
        } else {
            "null".to_owned()
        }
    }
}

impl ToJson for f64 {
    fn to_json(&self) -> String {
        json_finite(*self)
    }
}

impl ToJson for str {
    fn to_json(&self) -> String {
        json_string(self)
    }
}

impl ToJson for String {
    fn to_json(&self) -> String {
        json_string(self)
    }
}

impl<T: ToJson> ToJson for Option<T> {
    fn to_json(&self) -> String {
        match self {
            Some(value) => value.to_json(),
            None => "null".to_owned(),
        }
    }
}

impl<T: ToJson> ToJson for [T] {
    fn to_json(&self) -> String {
        json_array(self.iter().map(ToJson::to_json))
    }
}

impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> String {
        self.as_slice().to_json()
    }
}

/// Rendered as milliseconds — the unit every duration in this crate's
/// reports (`FrameStats`'s phase timings included) is most naturally read
/// in, and a single consistent choice rather than making every caller
/// remember which of a report's numbers are seconds and which are millis.
impl ToJson for Duration {
    fn to_json(&self) -> String {
        json_finite(self.as_secs_f64() * 1000.0)
    }
}

/// A small builder for a JSON object, so a `to_json` implementation reads as
/// a list of fields rather than a hand-balanced string of braces and
/// commas.
#[derive(Debug, Default, Clone)]
pub struct JsonObject {
    fields: Vec<(String, String)>,
}

impl JsonObject {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a field whose value is anything implementing [`ToJson`].
    ///
    /// Generic rather than `&dyn ToJson`: [`ToJson`] is implemented directly
    /// on unsized types like `str` (so a `&'static str` field can be passed
    /// straight through with no extra reference), and a trait-object
    /// parameter cannot accept those without an explicit, easy-to-forget
    /// second layer of indirection. Monomorphizing here costs nothing a
    /// devtools report needs to care about.
    #[must_use]
    pub fn field<T: ToJson + ?Sized>(mut self, key: &str, value: &T) -> Self {
        self.fields.push((key.to_owned(), value.to_json()));
        self
    }

    /// Add a field from an already-serialized JSON fragment — for nesting
    /// another [`JsonObject`]/array whose text you already have.
    #[must_use]
    pub fn raw_field(mut self, key: &str, raw_json: impl Into<String>) -> Self {
        self.fields.push((key.to_owned(), raw_json.into()));
        self
    }

    #[must_use]
    pub fn build(self) -> String {
        let mut out = String::from("{");
        for (index, (key, value)) in self.fields.into_iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&json_string(&key));
            out.push(':');
            out.push_str(&value);
        }
        out.push('}');
        out
    }
}

impl ToJson for JsonObject {
    fn to_json(&self) -> String {
        self.clone().build()
    }
}

// ---------------------------------------------------------------- InspectorNode

impl ToJson for InspectorNode {
    fn to_json(&self) -> String {
        JsonObject::new()
            .field("id", &self.id.to_string())
            .field("widget_name", self.widget_name)
            .field("builds", &self.builds)
            .field("recent_builds", &self.recent_builds)
            .raw_field(
                "bounds",
                self.bounds.map_or_else(|| "null".to_owned(), rect_json),
            )
            .raw_field(
                "children",
                json_array(self.children.iter().map(ToJson::to_json)),
            )
            .build()
    }
}

fn rect_json(rect: vieww_foundation::Rect) -> String {
    JsonObject::new()
        .field("left", &rect.left)
        .field("top", &rect.top)
        .field("right", &rect.right)
        .field("bottom", &rect.bottom)
        .build()
}

// ---------------------------------------------------------------- FrameTimeline

impl ToJson for FrameStats {
    fn to_json(&self) -> String {
        JsonObject::new()
            .field("number", &self.number)
            .field("timestamp_ms", &self.timestamp)
            .field("animate_ms", &self.animate)
            .field("build_ms", &self.build)
            .field("layout_ms", &self.layout)
            .field("paint_ms", &self.paint)
            .field("composite_ms", &self.composite)
            .field("total_ms", &self.total)
            .field("budget_ms", &self.budget)
            .field("over_budget", &self.over_budget())
            .field("damage_area", &self.damage_area)
            .field("damage_regions", &self.damage_regions)
            .build()
    }
}

impl ToJson for FrameTimeline {
    fn to_json(&self) -> String {
        let samples = self.samples_oldest_first();
        JsonObject::new()
            .field("capacity", &self.capacity())
            .field("len", &self.len())
            .field("total_recorded", &self.total_recorded())
            .raw_field("samples", json_array(samples.iter().map(ToJson::to_json)))
            .build()
    }
}

// -------------------------------------------------------- RenderGraphReport

impl ToJson for PassReport {
    fn to_json(&self) -> String {
        JsonObject::new()
            .field("id", &self.id.index())
            .field("name", self.name)
            .field("kind", self.kind_name)
            .raw_field("reads", json_array(self.reads.iter().map(|s| s.to_json())))
            .raw_field(
                "writes",
                json_array(self.writes.iter().map(|s| s.to_json())),
            )
            .field("culled", &self.culled)
            .field("concurrency_batch", &self.concurrency_batch)
            .build()
    }
}

impl ToJson for RenderGraphReport {
    fn to_json(&self) -> String {
        JsonObject::new()
            .raw_field(
                "passes",
                json_array(self.passes.iter().map(ToJson::to_json)),
            )
            .field("declared_resource_count", &self.declared_resource_count)
            .field("physical_slot_count", &self.physical_slot_count)
            .field("culled_count", &self.culled_count)
            .field("concurrency_batch_count", &self.concurrency_batch_count)
            .build()
    }
}

// -------------------------------------------------------------- DamageReport

impl ToJson for DamageReport {
    fn to_json(&self) -> String {
        JsonObject::new()
            .field("region_count", &self.region_count)
            .field("covered_area", &self.covered_area)
            .field("surface_area", &self.surface_area)
            .field("coverage_percent", &self.coverage_percent)
            .field("is_everything", &self.is_everything)
            .field("is_clean", &self.is_clean)
            .build()
    }
}

// ------------------------------------------------------------- SemanticsReport

/// The JSON object key for one [`Role`] in a histogram.
///
/// `Role`'s `Debug` output (`"Button"`, `"Custom(\"status\")"`) is used
/// directly: it is already unique per role (including per distinct
/// [`Role::Custom`] name), stable, and human-readable, so deriving a second
/// naming scheme just for JSON keys would be a second thing to keep in sync
/// with the enum for no reader benefit.
fn role_key(role: Role) -> String {
    format!("{role:?}")
}

impl ToJson for SemanticsReport {
    fn to_json(&self) -> String {
        // Sorted by key so the output is deterministic byte-for-byte across
        // runs — `role_histogram` is a `FastMap`, whose iteration order is
        // not something a consumer (or a test asserting exact output)
        // should have to tolerate varying.
        let mut roles: Vec<(String, usize)> = self
            .role_histogram
            .iter()
            .map(|(role, count)| (role_key(*role), *count))
            .collect();
        roles.sort();
        let mut histogram = JsonObject::new();
        for (key, count) in &roles {
            histogram = histogram.field(key, count);
        }

        JsonObject::new()
            .field("total_nodes", &self.total_nodes)
            .raw_field("role_histogram", histogram.build())
            .field(
                "unlabeled_interactive_count",
                &self.unlabeled_interactive_count,
            )
            .field("focusable_count", &self.focusable_count)
            .field("live_region_count", &self.live_region_count)
            .build()
    }
}

/// Exposed for a caller that wants one node's own JSON, rather than only the
/// aggregate [`SemanticsReport`] — e.g. a devtools panel drilling from the
/// histogram into the actual nodes of one role.
impl ToJson for SemanticsNode {
    fn to_json(&self) -> String {
        JsonObject::new()
            .field("role", &role_key(self.role))
            .field("label", &self.label)
            .field("value", &self.value)
            .field("toggled", &self.toggled)
            .field("enabled", &self.enabled)
            .field("focusable", &self.focusable)
            .field("focused", &self.focused)
            .raw_field("bounds", rect_json(self.bounds))
            .build()
    }
}

// -------------------------------------------------------------------- GpuReport

impl ToJson for DriverStats {
    fn to_json(&self) -> String {
        JsonObject::new()
            .field(
                "driver_reported_vram_bytes",
                &self.driver_reported_vram_bytes,
            )
            .field(
                "driver_reported_vram_budget_bytes",
                &self.driver_reported_vram_budget_bytes,
            )
            .field("gpu_frame_time_micros", &self.gpu_frame_time_micros)
            .field(
                "driver_pipeline_cache_hit_rate",
                &self.driver_pipeline_cache_hit_rate,
            )
            .build()
    }
}

impl ToJson for GpuReport {
    fn to_json(&self) -> String {
        JsonObject::new()
            .field("used_bytes", &self.used_bytes)
            .field("budget_bytes", &self.budget_bytes)
            .field("budget_used_percent", &self.budget_used_percent)
            .field("pending_uploads", &self.pending_uploads)
            .field("pending_readbacks", &self.pending_readbacks)
            .raw_field("driver", self.driver.to_json())
            .build()
    }
}

// ----------------------------------------------------------------- MemoryReport

#[cfg(feature = "snapshots")]
impl ToJson for MemoryReport {
    fn to_json(&self) -> String {
        let pool = JsonObject::new()
            .field("reused", &self.target_pool.reused)
            .field("allocated", &self.target_pool.allocated)
            .field("kept", &self.target_pool.kept)
            .field("dropped", &self.target_pool.dropped)
            .build();
        let glyphs = JsonObject::new()
            .field("hits", &self.glyph_outline_cache.hits)
            .field("misses", &self.glyph_outline_cache.misses)
            .field("evictions", &self.glyph_outline_cache.evictions)
            .build();
        let images = JsonObject::new()
            .field("hits", &self.image_cache.hits)
            .field("misses", &self.image_cache.misses)
            .field("evictions", &self.image_cache.evictions)
            .build();

        JsonObject::new()
            .raw_field("target_pool", pool)
            .raw_field("glyph_outline_cache", glyphs)
            .raw_field("image_cache", images)
            .field("image_cache_bytes", &self.image_cache_bytes)
            .field("total_estimated_bytes", &self.total_estimated_bytes())
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------- a minimal reader

    /// A JSON value, parsed back for exact round-trip assertions.
    ///
    /// Deliberately as small as [`ToJson`]'s own writer: this only ever
    /// needs to read back what this module's own writer produced, not
    /// arbitrary JSON from the wild — the same "only the surface actually
    /// needed" reasoning the module docs give for hand-rolling the writer.
    #[derive(Debug, Clone, PartialEq)]
    enum TestJson {
        Null,
        Bool(bool),
        Number(f64),
        Str(String),
        Array(Vec<TestJson>),
        Object(Vec<(String, TestJson)>),
    }

    struct Reader<'a> {
        bytes: &'a [u8],
        pos: usize,
    }

    impl<'a> Reader<'a> {
        fn new(input: &'a str) -> Self {
            Self {
                bytes: input.as_bytes(),
                pos: 0,
            }
        }

        fn peek(&self) -> Option<u8> {
            self.bytes.get(self.pos).copied()
        }

        fn bump(&mut self) -> Option<u8> {
            let b = self.peek()?;
            self.pos += 1;
            Some(b)
        }

        fn skip_ws(&mut self) {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.pos += 1;
            }
        }

        fn expect(&mut self, b: u8) {
            assert_eq!(
                self.bump(),
                Some(b),
                "expected {:?} at byte {}",
                b as char,
                self.pos
            );
        }

        fn parse_value(&mut self) -> TestJson {
            self.skip_ws();
            match self.peek().expect("unexpected end of input") {
                b'n' => {
                    self.expect_literal("null");
                    TestJson::Null
                }
                b't' => {
                    self.expect_literal("true");
                    TestJson::Bool(true)
                }
                b'f' => {
                    self.expect_literal("false");
                    TestJson::Bool(false)
                }
                b'"' => TestJson::Str(self.parse_string()),
                b'[' => self.parse_array(),
                b'{' => self.parse_object(),
                _ => TestJson::Number(self.parse_number()),
            }
        }

        fn expect_literal(&mut self, literal: &str) {
            for expected in literal.bytes() {
                self.expect(expected);
            }
        }

        fn parse_string(&mut self) -> String {
            self.expect(b'"');
            let mut out = String::new();
            loop {
                match self.bump().expect("unterminated string") {
                    b'"' => break,
                    b'\\' => match self.bump().expect("dangling escape") {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0C}'),
                        b'u' => {
                            let hex: String = (0..4)
                                .map(|_| self.bump().expect("truncated \\u escape") as char)
                                .collect();
                            let code = u32::from_str_radix(&hex, 16).expect("valid hex");
                            out.push(char::from_u32(code).expect("valid code point"));
                        }
                        other => panic!("unknown escape \\{}", other as char),
                    },
                    other_byte => {
                        // Reassemble UTF-8 multi-byte sequences: `other_byte`
                        // may be the lead byte of a multi-byte character,
                        // and the loop above walks raw bytes.
                        let start = self.pos - 1;
                        let extra = utf8_extra_bytes(other_byte);
                        self.pos += extra;
                        let slice = &self.bytes[start..self.pos];
                        out.push_str(std::str::from_utf8(slice).expect("valid utf-8"));
                    }
                }
            }
            out
        }

        fn parse_number(&mut self) -> f64 {
            let start = self.pos;
            if self.peek() == Some(b'-') {
                self.pos += 1;
            }
            while matches!(
                self.peek(),
                Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            ) {
                self.pos += 1;
            }
            let text = std::str::from_utf8(&self.bytes[start..self.pos]).expect("valid utf-8");
            text.parse().expect("valid JSON number")
        }

        fn parse_array(&mut self) -> TestJson {
            self.expect(b'[');
            let mut items = Vec::new();
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return TestJson::Array(items);
            }
            loop {
                items.push(self.parse_value());
                self.skip_ws();
                match self.bump().expect("unterminated array") {
                    b',' => continue,
                    b']' => break,
                    other => panic!("unexpected byte {} in array", other as char),
                }
            }
            TestJson::Array(items)
        }

        fn parse_object(&mut self) -> TestJson {
            self.expect(b'{');
            let mut fields = Vec::new();
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return TestJson::Object(fields);
            }
            loop {
                self.skip_ws();
                let key = self.parse_string();
                self.skip_ws();
                self.expect(b':');
                let value = self.parse_value();
                fields.push((key, value));
                self.skip_ws();
                match self.bump().expect("unterminated object") {
                    b',' => continue,
                    b'}' => break,
                    other => panic!("unexpected byte {} in object", other as char),
                }
            }
            TestJson::Object(fields)
        }
    }

    fn utf8_extra_bytes(lead: u8) -> usize {
        match lead {
            0x00..=0x7F => 0,
            0xC0..=0xDF => 1,
            0xE0..=0xEF => 2,
            0xF0..=0xF7 => 3,
            _ => panic!("invalid UTF-8 lead byte {lead:#x}"),
        }
    }

    fn parse(input: &str) -> TestJson {
        let mut reader = Reader::new(input);
        let value = reader.parse_value();
        reader.skip_ws();
        assert_eq!(
            reader.pos,
            reader.bytes.len(),
            "trailing bytes after JSON value"
        );
        value
    }

    fn object_field<'a>(value: &'a TestJson, key: &str) -> &'a TestJson {
        match value {
            TestJson::Object(fields) => {
                &fields
                    .iter()
                    .find(|(k, _)| k == key)
                    .unwrap_or_else(|| panic!("missing field {key:?} in {fields:?}"))
                    .1
            }
            other => panic!("expected an object, got {other:?}"),
        }
    }

    // --------------------------------------------------------- escaping tests

    #[test]
    fn plain_ascii_passes_through_unescaped() {
        assert_eq!(escape_str("hello world"), "hello world");
    }

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        assert_eq!(escape_str(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(escape_str(r"C:\path\to\file"), r"C:\\path\\to\\file");
    }

    #[test]
    fn named_control_escapes_match_the_json_grammar_exactly() {
        assert_eq!(escape_str("a\nb"), "a\\nb");
        assert_eq!(escape_str("a\rb"), "a\\rb");
        assert_eq!(escape_str("a\tb"), "a\\tb");
        assert_eq!(escape_str("a\u{08}b"), "a\\bb");
        assert_eq!(escape_str("a\u{0C}b"), "a\\fb");
    }

    #[test]
    fn other_control_characters_become_a_u_escape() {
        assert_eq!(escape_str("a\u{01}b"), "a\\u0001b");
        assert_eq!(escape_str("a\u{1F}b"), "a\\u001fb");
    }

    #[test]
    fn unicode_outside_the_control_range_passes_through_unescaped() {
        // JSON strings are UTF-8; there is nothing to escape here.
        assert_eq!(escape_str("héllo 🎉 日本語"), "héllo 🎉 日本語");
    }

    #[test]
    fn an_escaped_string_round_trips_exactly_through_the_test_parser() {
        for original in [
            "plain",
            "with \"quotes\" and \\backslashes\\",
            "line1\nline2\ttabbed",
            "emoji 🎉 and accents héllo",
            "\u{01}\u{02}control\u{1F}chars",
            "",
        ] {
            let literal = json_string(original);
            let parsed = parse(&literal);
            assert_eq!(
                parsed,
                TestJson::Str(original.to_owned()),
                "round trip of {original:?}"
            );
        }
    }

    #[test]
    fn json_finite_writes_null_for_non_finite_values() {
        assert_eq!(json_finite(f64::NAN), "null");
        assert_eq!(json_finite(f64::INFINITY), "null");
        assert_eq!(json_finite(f64::NEG_INFINITY), "null");
        assert_eq!(parse(&json_finite(1.5)), TestJson::Number(1.5));
    }

    // ------------------------------------------------------- structural tests

    #[test]
    fn json_object_builder_produces_a_parseable_object_with_exact_values() {
        let json = JsonObject::new()
            .field("a", &1u32)
            .field("b", "text")
            .field("c", &true)
            .build();
        let parsed = parse(&json);
        assert_eq!(object_field(&parsed, "a"), &TestJson::Number(1.0));
        assert_eq!(
            object_field(&parsed, "b"),
            &TestJson::Str("text".to_owned())
        );
        assert_eq!(object_field(&parsed, "c"), &TestJson::Bool(true));
    }

    #[test]
    fn option_none_serializes_as_null_and_some_as_the_inner_value() {
        let none: Option<u32> = None;
        let some: Option<u32> = Some(7);
        assert_eq!(parse(&none.to_json()), TestJson::Null);
        assert_eq!(parse(&some.to_json()), TestJson::Number(7.0));
    }

    #[test]
    fn duration_is_serialized_as_milliseconds() {
        let json = Duration::from_millis(1500).to_json();
        assert_eq!(parse(&json), TestJson::Number(1500.0));
    }

    #[test]
    fn vec_of_strings_round_trips_as_a_json_array() {
        let items = vec!["a".to_owned(), "b".to_owned()];
        let parsed = parse(&items.to_json());
        assert_eq!(
            parsed,
            TestJson::Array(vec![
                TestJson::Str("a".to_owned()),
                TestJson::Str("b".to_owned())
            ])
        );
    }

    #[test]
    fn inspector_node_round_trips_with_children_and_bounds() {
        use vieww_element::ElementId;

        let leaf = InspectorNode {
            id: ElementId::from_handle(0, 1),
            widget_name: "Text",
            builds: 3,
            recent_builds: 1,
            bounds: Some(vieww_foundation::Rect::new(1.0, 2.0, 3.0, 4.0)),
            children: Vec::new(),
        };
        let root = InspectorNode {
            id: ElementId::from_handle(0, 0),
            widget_name: "Flex",
            builds: 5,
            recent_builds: 0,
            bounds: None,
            children: vec![leaf],
        };

        let parsed = parse(&root.to_json());
        assert_eq!(
            object_field(&parsed, "widget_name"),
            &TestJson::Str("Flex".to_owned())
        );
        assert_eq!(object_field(&parsed, "builds"), &TestJson::Number(5.0));
        assert_eq!(object_field(&parsed, "bounds"), &TestJson::Null);

        let TestJson::Array(children) = object_field(&parsed, "children") else {
            panic!("children must be an array");
        };
        assert_eq!(children.len(), 1);
        assert_eq!(
            object_field(&children[0], "widget_name"),
            &TestJson::Str("Text".to_owned())
        );
        let bounds = object_field(&children[0], "bounds");
        assert_eq!(object_field(bounds, "left"), &TestJson::Number(1.0));
        assert_eq!(object_field(bounds, "bottom"), &TestJson::Number(4.0));
    }

    #[test]
    fn gpu_report_nests_driver_stats_as_all_null() {
        let report = crate::gpu_inspector::GpuReport {
            used_bytes: 10,
            budget_bytes: 100,
            budget_used_percent: 10.0,
            pending_uploads: 0,
            pending_readbacks: 0,
            driver: DriverStats::default(),
        };
        let parsed = parse(&report.to_json());
        let driver = object_field(&parsed, "driver");
        assert_eq!(
            object_field(driver, "driver_reported_vram_bytes"),
            &TestJson::Null
        );
        assert_eq!(
            object_field(driver, "gpu_frame_time_micros"),
            &TestJson::Null
        );
    }

    #[test]
    fn damage_report_fields_round_trip_exactly() {
        let report = DamageReport {
            region_count: 2,
            covered_area: 123.5,
            surface_area: 1000.0,
            coverage_percent: 12.35,
            is_everything: false,
            is_clean: false,
        };
        let parsed = parse(&report.to_json());
        assert_eq!(
            object_field(&parsed, "region_count"),
            &TestJson::Number(2.0)
        );
        assert_eq!(
            object_field(&parsed, "coverage_percent"),
            &TestJson::Number(12.35)
        );
        assert_eq!(object_field(&parsed, "is_clean"), &TestJson::Bool(false));
    }

    #[test]
    fn semantics_report_histogram_uses_role_debug_names_as_keys() {
        use vieww_foundation::FastMap;

        let mut role_histogram: FastMap<Role, usize> = FastMap::default();
        role_histogram.insert(Role::Button, 2);
        role_histogram.insert(Role::Custom("status"), 1);
        let report = SemanticsReport {
            total_nodes: 3,
            role_histogram,
            unlabeled_interactive_count: 1,
            focusable_count: 2,
            live_region_count: 1,
        };

        let parsed = parse(&report.to_json());
        let histogram = object_field(&parsed, "role_histogram");
        assert_eq!(object_field(histogram, "Button"), &TestJson::Number(2.0));
        assert_eq!(
            object_field(histogram, "Custom(\"status\")"),
            &TestJson::Number(1.0)
        );
    }

    #[test]
    fn render_graph_report_round_trips_pass_names_and_edges() {
        let report = RenderGraphReport {
            passes: vec![PassReport {
                id: sample_pass_id(),
                name: "composite",
                kind_name: "composite",
                reads: vec!["a", "b"],
                writes: vec!["screen"],
                culled: false,
                concurrency_batch: Some(1),
            }],
            declared_resource_count: 3,
            physical_slot_count: 2,
            culled_count: 0,
            concurrency_batch_count: 2,
        };

        let parsed = parse(&report.to_json());
        let TestJson::Array(passes) = object_field(&parsed, "passes") else {
            panic!("passes must be an array");
        };
        assert_eq!(
            object_field(&passes[0], "name"),
            &TestJson::Str("composite".to_owned())
        );
        let TestJson::Array(reads) = object_field(&passes[0], "reads") else {
            panic!("reads must be an array");
        };
        assert_eq!(
            reads,
            &vec![TestJson::Str("a".to_owned()), TestJson::Str("b".to_owned())]
        );
        assert_eq!(
            object_field(&passes[0], "concurrency_batch"),
            &TestJson::Number(1.0)
        );
    }

    /// A real `PassId` for the test above — obtained the only way this
    /// crate can, by compiling a one-pass graph, since `PassId`'s inner
    /// value is private to `vieww-render-graph`.
    fn sample_pass_id() -> vieww_render_graph::PassId {
        use vieww_render_graph::{Graph, PassDesc, PassKind, ResourceDesc};
        use vieww_scene::CostHint;

        let mut g = Graph::new();
        let r = g.add_resource(ResourceDesc::presentable(1, 1, "r"));
        g.add_pass(PassDesc::new("p", PassKind::Raster, CostHint::CONSERVATIVE).writing(r))
    }
}
