//! Pipeline cache keys, and a small generic cache keyed by them.
//!
//! A GPU backend building a pipeline object (a genuinely expensive call —
//! it's the whole reason a *cache* is worth having) needs a key that is:
//! identical for the same (source, entry point, stage), and — this is the
//! part a fast-but-collidable hash gets wrong — different, with
//! overwhelming probability, for any two different ones. Silently sharing a
//! cache slot between two different shaders is a rendering-correctness bug
//! (the wrong pipeline gets used), not a slow one, so [`PipelineCacheKey`]
//! is a `blake3` content hash rather than `std`'s `DefaultHasher`.

use std::collections::HashMap;

/// A content-addressed key over one (WGSL source, entry point, stage)
/// triple. Two [`ParsedShader`](crate::compiler::ParsedShader)s built from
/// byte-identical source, requesting the same entry point and stage,
/// always produce equal keys — including across process runs, since
/// `blake3` (unlike `std`'s `RandomState`-seeded hashers) has no per-process
/// random seed.
///
/// # Why the raw source text, not the parsed IR
///
/// Hashing `naga::Module` would let two WGSL texts that differ only in
/// comments or whitespace share a cache entry. That is a real, deliberate
/// tradeoff this type does not make: hot reload
/// ([`crate::hot_reload`]) needs "did the source actually change" to be
/// answerable without a full parse, and source-hashing gives that for
/// free. The cost is the opposite case — two *different-looking* sources
/// that happen to produce identical IR (e.g. `2.0 + 2.0` vs `4.0`) get
/// different keys and thus separate, redundant pipeline objects. A
/// redundant pipeline is a memory/compile-time cost; a wrongly-shared one
/// is a correctness bug — this type is deliberately biased toward the
/// former.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PipelineCacheKey([u8; 32]);

impl PipelineCacheKey {
    #[must_use]
    pub fn new(source: &str, entry_point: &str, stage: naga::ShaderStage) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(source.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry_point.as_bytes());
        hasher.update(b"\0");
        hasher.update(stage_tag(stage));
        Self(*hasher.finalize().as_bytes())
    }

    #[must_use]
    pub fn to_hex(&self) -> String {
        blake3::Hash::from(self.0).to_hex().to_string()
    }
}

impl std::fmt::Display for PipelineCacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

fn stage_tag(stage: naga::ShaderStage) -> &'static [u8] {
    match stage {
        naga::ShaderStage::Vertex => b"vertex",
        naga::ShaderStage::Fragment => b"fragment",
        naga::ShaderStage::Compute => b"compute",
        naga::ShaderStage::Task => b"task",
        naga::ShaderStage::Mesh => b"mesh",
        // Ray tracing stages: this library's built-in shaders never use
        // them, but a caller-registered shader might one day, and a cache
        // key still needs *a* distinguishing tag for each rather than a
        // panic — `todo!()` here would turn "unused stage" into a runtime
        // crash the first time someone tries it.
        naga::ShaderStage::RayGeneration => b"ray_generation",
        naga::ShaderStage::Miss => b"miss",
        naga::ShaderStage::AnyHit => b"any_hit",
        naga::ShaderStage::ClosestHit => b"closest_hit",
    }
}

/// A trivial generic cache keyed by [`PipelineCacheKey`] — deliberately not
/// backend-specific. A real GPU backend stores its own pipeline object type
/// (`vk::Pipeline`, `MTLRenderPipelineState`, `ID3D12PipelineState`) as `V`;
/// this type owns none of the GPU-specific creation or destruction logic,
/// only "have I already built this one".
#[derive(Debug, Clone, Default)]
pub struct PipelineCache<V> {
    entries: HashMap<PipelineCacheKey, V>,
}

impl<V> PipelineCache<V> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    #[must_use]
    pub fn get(&self, key: &PipelineCacheKey) -> Option<&V> {
        self.entries.get(key)
    }

    /// Insert a freshly built pipeline object under `key`. Returns the
    /// previous value, if any — a caller that gets `Some` back built a
    /// pipeline it didn't need to (a cache-miss race, or a caller that
    /// didn't check [`Self::get`] first), which is worth knowing about
    /// even though it isn't wrong on its own.
    pub fn insert(&mut self, key: PipelineCacheKey, value: V) -> Option<V> {
        self.entries.insert(key, value)
    }

    /// Get-or-compute: return the cached value for `key`, or call `build`
    /// and cache its result. The common shape a real backend's "get me a
    /// pipeline for this shader" entry point wants.
    pub fn get_or_insert_with(
        &mut self,
        key: PipelineCacheKey,
        build: impl FnOnce() -> V,
    ) -> &mut V {
        self.entries.entry(key).or_insert_with(build)
    }

    /// Drop the entry for `key`, if present — what hot reload calls once a
    /// shader's source changes, so the next [`Self::get_or_insert_with`]
    /// rebuilds against the new source instead of reusing a pipeline built
    /// from the old one under what would otherwise be a stale key
    /// collision-free by construction (the key already changed), but the
    /// *old* entry is now simply dead weight without this.
    pub fn invalidate(&mut self, key: &PipelineCacheKey) -> Option<V> {
        self.entries.remove(key)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naga::ShaderStage;

    #[test]
    fn identical_inputs_produce_an_identical_key() {
        let a = PipelineCacheKey::new("// same", "vs_main", ShaderStage::Vertex);
        let b = PipelineCacheKey::new("// same", "vs_main", ShaderStage::Vertex);
        assert_eq!(a, b);
    }

    #[test]
    fn a_different_source_produces_a_different_key() {
        let a = PipelineCacheKey::new("// one", "vs_main", ShaderStage::Vertex);
        let b = PipelineCacheKey::new("// two", "vs_main", ShaderStage::Vertex);
        assert_ne!(a, b);
    }

    #[test]
    fn a_different_entry_point_produces_a_different_key_for_the_same_source() {
        let a = PipelineCacheKey::new("// x", "vs_main", ShaderStage::Vertex);
        let b = PipelineCacheKey::new("// x", "vs_other", ShaderStage::Vertex);
        assert_ne!(a, b);
    }

    #[test]
    fn a_different_stage_produces_a_different_key_for_the_same_source_and_name() {
        // Guards against a naive implementation that concatenates source
        // and entry point without a stage-distinguishing separator, which
        // would collide "vs_main" (vertex) with "vs_main" (fragment).
        let a = PipelineCacheKey::new("// x", "main", ShaderStage::Vertex);
        let b = PipelineCacheKey::new("// x", "main", ShaderStage::Fragment);
        assert_ne!(a, b);
    }

    #[test]
    fn get_or_insert_with_only_builds_once() {
        let mut cache: PipelineCache<u32> = PipelineCache::new();
        let key = PipelineCacheKey::new("// x", "vs_main", ShaderStage::Vertex);
        let mut build_calls = 0;
        {
            let value = cache.get_or_insert_with(key, || {
                build_calls += 1;
                42
            });
            assert_eq!(*value, 42);
        }
        cache.get_or_insert_with(key, || {
            build_calls += 1;
            99
        });
        assert_eq!(build_calls, 1);
        assert_eq!(*cache.get(&key).unwrap(), 42);
    }

    #[test]
    fn invalidate_removes_the_entry_so_the_next_build_runs() {
        let mut cache: PipelineCache<u32> = PipelineCache::new();
        let key = PipelineCacheKey::new("// x", "vs_main", ShaderStage::Vertex);
        cache.insert(key, 1);
        assert_eq!(cache.invalidate(&key), Some(1));
        assert!(cache.get(&key).is_none());
    }
}
