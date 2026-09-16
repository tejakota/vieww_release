//! The one WGSL shader library — spec's "one shader source" (already
//! `vieww-hal::vulkan`'s own stated rule for its two shaders; this module
//! is what makes that rule hold across every backend rather than per-file).
//!
//! Every built-in shader's WGSL text lives on disk under `shaders/` next to
//! this crate's `Cargo.toml`, embedded at compile time with `include_str!`
//! — real source files a human (or an IDE with WGSL syntax highlighting)
//! can open directly, not string literals buried in Rust. `vieww-hal`'s
//! own `CLEAR_SHADER_WGSL`/`MESH_SHADER_WGSL` constants predate this crate;
//! `SOLID` is the same shader, moved here so it has exactly one home
//! instead of two copies drifting apart — see `vulkan/mod.rs`'s own
//! comment at its (now-deleted) copy.

use std::borrow::Cow;
use std::collections::BTreeMap;

/// One named WGSL module: a vertex and fragment entry point (`vs_main`/
/// `fs_main` by convention, matched exactly by every built-in below) that
/// [`crate::compiler`] parses, validates and cross-compiles as a unit.
#[derive(Debug, Clone)]
pub struct ShaderSource {
    pub name: Cow<'static, str>,
    pub wgsl: Cow<'static, str>,
}

impl ShaderSource {
    #[must_use]
    pub fn new_static(name: &'static str, wgsl: &'static str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            wgsl: Cow::Borrowed(wgsl),
        }
    }

    #[must_use]
    pub fn new(name: impl Into<String>, wgsl: impl Into<String>) -> Self {
        Self {
            name: Cow::Owned(name.into()),
            wgsl: Cow::Owned(wgsl.into()),
        }
    }
}

/// Whole-viewport flat-colour clear — moved from `vieww-hal::vulkan`'s
/// `CLEAR_SHADER_WGSL`.
pub const CLEAR_WGSL: &str = include_str!("../shaders/clear.wgsl");
/// Solid-colour mesh fill — moved from `vieww-hal::vulkan`'s
/// `MESH_SHADER_WGSL`.
pub const SOLID_WGSL: &str = include_str!("../shaders/solid.wgsl");
/// A whole batched scene: fills, strokes, glyphs, images and gradients in one
/// vertex buffer and one pipeline, writing premultiplied colour.
///
/// What `vieww_gpu::scene`'s planner targets for its `Draw` steps. See the
/// file itself for the four materials and why every texture is fetched rather
/// than sampled.
pub const SCENE_WGSL: &str = include_str!("../shaders/scene.wgsl");
/// Offscreen compositing for `vieww_gpu::Step`: copy, three-box blur passes,
/// colour matrix, mask multiply, all 28 blend modes, and the shadow
/// seed/inset/tint operations. See the file's header for the mode table.
pub const POST_WGSL: &str = include_str!("../shaders/post.wgsl");
/// Two-stop linear gradient.
pub const GRADIENT_WGSL: &str = include_str!("../shaders/gradient.wgsl");
/// Textured mesh, optionally tinted.
pub const IMAGE_WGSL: &str = include_str!("../shaders/image.wgsl");
/// Coverage-mask glyph rendering, for a text-only pass.
///
/// **Not what draws text today.** `vieww_gpu::scene` batches glyphs into the
/// same vertex buffer as everything else and [`SCENE_WGSL`] samples the atlas
/// for both, so a single-purpose text shader would cost a pipeline bind
/// between a panel and the label on it. This is kept for a future pass that
/// genuinely wants one — and its Y-flip was fixed only because the text work
/// went looking; nothing had ever drawn through it, so nothing could disagree
/// with it. See the file's own comment.
pub const TEXT_WGSL: &str = include_str!("../shaders/text.wgsl");
/// Separable Gaussian blur, one direction per draw.
pub const BLUR_WGSL: &str = include_str!("../shaders/blur.wgsl");
/// Offset, tinted drop shadow from a coverage mask.
pub const SHADOW_WGSL: &str = include_str!("../shaders/shadow.wgsl");
/// Alpha masking of a content texture by a coverage texture.
pub const MASK_WGSL: &str = include_str!("../shaders/mask.wgsl");
/// Blend-mode-selectable two-layer compositing.
pub const COMPOSITE_WGSL: &str = include_str!("../shaders/composite.wgsl");

/// A registry of named [`ShaderSource`]s: the built-ins above, plus whatever
/// an application or effect crate registers alongside them. Every built-in
/// is registered under its file's base name (`"solid"`, `"gradient"`, …),
/// so `library.get("solid")` and reading `shaders/solid.wgsl` refer to
/// exactly the same text.
#[derive(Debug, Clone)]
pub struct ShaderLibrary {
    sources: BTreeMap<String, ShaderSource>,
}

impl Default for ShaderLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl ShaderLibrary {
    /// A library pre-populated with every built-in shader above.
    #[must_use]
    pub fn new() -> Self {
        let mut sources = BTreeMap::new();
        for source in [
            ShaderSource::new_static("clear", CLEAR_WGSL),
            ShaderSource::new_static("solid", SOLID_WGSL),
            ShaderSource::new_static("scene", SCENE_WGSL),
            ShaderSource::new_static("post", POST_WGSL),
            ShaderSource::new_static("gradient", GRADIENT_WGSL),
            ShaderSource::new_static("image", IMAGE_WGSL),
            ShaderSource::new_static("text", TEXT_WGSL),
            ShaderSource::new_static("blur", BLUR_WGSL),
            ShaderSource::new_static("shadow", SHADOW_WGSL),
            ShaderSource::new_static("mask", MASK_WGSL),
            ShaderSource::new_static("composite", COMPOSITE_WGSL),
        ] {
            sources.insert(source.name.clone().into_owned(), source);
        }
        Self { sources }
    }

    /// An empty library with none of the built-ins — for a caller that
    /// wants only its own shaders, or a test that wants to control exactly
    /// what's registered.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            sources: BTreeMap::new(),
        }
    }

    /// Register (or replace) a shader source under `source.name`. Returns
    /// the previous source under that name, if any — the same shape
    /// `hot_reload::ShaderLibrary::reload_from_str` builds on to
    /// detect "this is a reload" versus "this is new".
    pub fn register(&mut self, source: ShaderSource) -> Option<ShaderSource> {
        self.sources
            .insert(source.name.clone().into_owned(), source)
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ShaderSource> {
        self.sources.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.sources.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ShaderSource> {
        self.sources.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_library_has_exactly_the_eleven_built_ins() {
        let library = ShaderLibrary::new();
        assert_eq!(library.len(), 11);
        for name in [
            "clear",
            "solid",
            "scene",
            "post",
            "gradient",
            "image",
            "text",
            "blur",
            "shadow",
            "mask",
            "composite",
        ] {
            assert!(
                library.get(name).is_some(),
                "missing built-in shader {name:?}"
            );
        }
    }

    #[test]
    fn an_empty_library_has_none() {
        assert!(ShaderLibrary::empty().is_empty());
    }

    #[test]
    fn registering_a_new_name_does_not_replace_an_existing_one() {
        let mut library = ShaderLibrary::empty();
        assert!(library.register(ShaderSource::new("a", "// a")).is_none());
        assert!(library.register(ShaderSource::new("b", "// b")).is_none());
        assert_eq!(library.len(), 2);
    }

    #[test]
    fn registering_over_an_existing_name_returns_the_old_source() {
        let mut library = ShaderLibrary::empty();
        library.register(ShaderSource::new("a", "// v1"));
        let old = library.register(ShaderSource::new("a", "// v2"));
        assert_eq!(old.unwrap().wgsl.as_ref(), "// v1");
        assert_eq!(library.get("a").unwrap().wgsl.as_ref(), "// v2");
        assert_eq!(library.len(), 1);
    }
}
