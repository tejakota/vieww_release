//! Development-time shader reload hooks.
//!
//! # What this is, and what it deliberately is not
//!
//! This module is the *mechanism* — safely swapping one shader's source at
//! runtime, validating the replacement before it takes effect, and
//! notifying observers when it does — not a live filesystem watcher. There
//! is no `notify`-crate dependency here, on purpose: watching a directory
//! for changes is a dev-server/tooling concern (where to watch, how to
//! debounce a save-triggered double-write, how to expose it to a running
//! app process) that belongs in whatever development harness embeds this
//! crate, not in the shader system itself. What this module guarantees is
//! the part that actually needs to be correct: given new source text from
//! *anywhere* — a file watcher's callback, a devtools UI's text box, a
//! network message from a connected editor — [`ReloadableLibrary::reload`]
//! either takes effect cleanly or is rejected with the old shader left
//! completely intact, never a partially-applied or corrupted state.
//!
//! A `notify`-based file watcher that calls [`ReloadableLibrary::reload`]
//! from its callback is real, separate, addable work; this module is what
//! it would call.

use crate::compiler::{CompileError, ParsedShader};
use crate::library::{ShaderLibrary, ShaderSource};
use crate::pipeline_cache::PipelineCacheKey;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Observes reloads so a caller (typically a [`crate::pipeline_cache::PipelineCache`]
/// owner) can invalidate whatever it cached under the shader's old
/// [`PipelineCacheKey`]s.
pub trait ReloadHook: Send + Sync {
    fn on_shader_reloaded(&self, report: &ReloadReport);
}

/// What changed in one [`ReloadableLibrary::reload`] call. `keys_before` is
/// always empty for a shader that didn't exist before this reload (a
/// brand-new registration rather than a replacement).
#[derive(Debug, Clone)]
pub struct ReloadReport {
    pub name: String,
}

/// A [`ShaderLibrary`] wrapped with parse-before-swap semantics and a
/// notification list — the "hot" half of hot reload. Held behind an
/// `Arc<RwLock<_>>` internally so a reader (a render thread pulling a
/// [`ParsedShader`] to build a pipeline from) and a writer (a reload
/// callback) can't corrupt each other; a reload that fails to parse never
/// takes the write lock's contents to an invalid state because the new
/// source is fully parsed *before* it replaces the old one.
#[derive(Clone)]
pub struct ReloadableLibrary {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    library: ShaderLibrary,
    parsed: BTreeMap<String, ParsedShader>,
    hooks: Vec<Arc<dyn ReloadHook>>,
}

impl std::fmt::Debug for ReloadableLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("ReloadableLibrary")
            .field("shader_count", &inner.library.len())
            .finish_non_exhaustive()
    }
}

impl Default for ReloadableLibrary {
    fn default() -> Self {
        Self::new(ShaderLibrary::new())
    }
}

impl ReloadableLibrary {
    #[must_use]
    pub fn new(library: ShaderLibrary) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                library,
                parsed: BTreeMap::new(),
                hooks: Vec::new(),
            })),
        }
    }

    pub fn add_hook(&self, hook: Arc<dyn ReloadHook>) {
        self.inner
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .hooks
            .push(hook);
    }

    /// Replace (or add) the shader named `name` with `wgsl`. The new source
    /// is parsed and validated *before* anything about the library changes
    /// — a syntax error or a validation failure leaves the previously
    /// registered shader (if any) completely untouched, and this returns
    /// `Err` describing exactly what was wrong, the same
    /// [`CompileError`] a caller would get from calling
    /// [`ParsedShader::parse`] directly.
    ///
    /// On success, every registered [`ReloadHook`] is called with a
    /// [`ReloadReport`] naming the shader, so a pipeline cache can drop
    /// whatever it built under this shader's old
    /// [`crate::pipeline_cache::PipelineCacheKey`]s
    /// (the key itself already changed, since it's a hash of the source —
    /// the hook's job is evicting the *old* key, which nothing else knows
    /// to do on its own).
    pub fn reload(
        &self,
        name: &str,
        wgsl: impl Into<String>,
    ) -> Result<ReloadReport, CompileError> {
        let wgsl = wgsl.into();
        // Parse and validate *first* — see this method's doc for why this
        // ordering is the whole point.
        let parsed = ParsedShader::parse(&wgsl)?;

        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner
            .library
            .register(ShaderSource::new(name.to_string(), wgsl));
        inner.parsed.insert(name.to_string(), parsed);
        let report = ReloadReport {
            name: name.to_string(),
        };
        for hook in inner.hooks.clone() {
            hook.on_shader_reloaded(&report);
        }
        Ok(report)
    }

    /// The current cache key for `name`'s given entry point/stage, if the
    /// shader has been parsed (either at construction via
    /// [`Self::warm`] or through a prior [`Self::reload`]) — a caller
    /// checks this against whatever key it last built a pipeline for to
    /// decide whether a rebuild is needed, without holding this library's
    /// lock across the (potentially slow) pipeline build itself.
    #[must_use]
    pub fn current_key(
        &self,
        name: &str,
        entry_point: &str,
        stage: naga::ShaderStage,
    ) -> Option<PipelineCacheKey> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner
            .parsed
            .get(name)
            .map(|parsed| parsed.cache_key(entry_point, stage))
    }

    /// Parse and cache every shader currently in the underlying library
    /// that isn't already parsed — called once at startup so
    /// [`Self::current_key`] has something to report for the built-ins
    /// without waiting for a first [`Self::reload`].
    pub fn warm(&self) -> Result<(), CompileError> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let to_parse: Vec<ShaderSource> = inner
            .library
            .iter()
            .filter(|source| !inner.parsed.contains_key(source.name.as_ref()))
            .cloned()
            .collect();
        for source in to_parse {
            let parsed = ParsedShader::parse(&source.wgsl)?;
            inner.parsed.insert(source.name.into_owned(), parsed);
        }
        Ok(())
    }

    #[must_use]
    pub fn shader_names(&self) -> Vec<String> {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .library
            .names()
            .map(str::to_string)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naga::ShaderStage;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingHook(Arc<AtomicUsize>);
    impl ReloadHook for CountingHook {
        fn on_shader_reloaded(&self, _report: &ReloadReport) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn reloading_a_valid_replacement_changes_the_cache_key() {
        let library = ReloadableLibrary::new(ShaderLibrary::empty());
        library.reload("a", "// v1").unwrap();
        let key1 = library.current_key("a", "vs_main", ShaderStage::Vertex);
        library.reload("a", "// v2").unwrap();
        let key2 = library.current_key("a", "vs_main", ShaderStage::Vertex);
        assert_ne!(key1, key2);
    }

    #[test]
    fn an_invalid_replacement_leaves_the_previous_shader_intact() {
        let library = ReloadableLibrary::new(ShaderLibrary::empty());
        library.reload("a", crate::library::SOLID_WGSL).unwrap();
        let key_before = library.current_key("a", "vs_main", ShaderStage::Vertex);

        let result = library.reload("a", "this is not valid wgsl {{{");
        assert!(result.is_err());

        let key_after = library.current_key("a", "vs_main", ShaderStage::Vertex);
        assert_eq!(
            key_before, key_after,
            "a failed reload must not change the live shader"
        );
    }

    #[test]
    fn hooks_are_notified_exactly_once_per_successful_reload() {
        let library = ReloadableLibrary::new(ShaderLibrary::empty());
        let count = Arc::new(AtomicUsize::new(0));
        library.add_hook(Arc::new(CountingHook(count.clone())));

        library.reload("a", "// v1").unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);

        assert!(library.reload("a", "not wgsl {{{").is_err());
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "a failed reload must not notify hooks"
        );

        library.reload("a", "// v2").unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn warm_parses_every_built_in_so_current_key_reports_immediately() {
        let library = ReloadableLibrary::default();
        assert!(library
            .current_key("solid", "vs_main", ShaderStage::Vertex)
            .is_none());
        library.warm().unwrap();
        assert!(library
            .current_key("solid", "vs_main", ShaderStage::Vertex)
            .is_some());
    }
}
