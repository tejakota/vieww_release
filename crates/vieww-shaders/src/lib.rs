//! vieww's shader system: one WGSL library, cross-compiled through `naga`
//! to SPIR-V/MSL/HLSL, with reflection, pipeline cache keys, and hot-reload
//! hooks — the shared foundation every GPU backend compiles its shaders
//! through, so shader source lives in exactly one place instead of being
//! copy-pasted per backend (which is exactly what `vieww-hal::vulkan`'s
//! `CLEAR_SHADER_WGSL`/`MESH_SHADER_WGSL` constants were, before this crate
//! existed — see [`library::SOLID_WGSL`]'s doc for where they moved).
//!
//! # Where this sits in the render stack
//!
//! ```text
//! vieww-scene → vieww-render-graph → vieww-render-planner → CPU/GPU/Hybrid
//!                                                                    │
//!                                                        vieww-gpu, vieww-hal
//!                                                                    │
//!                                                            vieww-shaders (this crate)
//! ```
//!
//! This crate depends on nothing in this workspace except `naga` and
//! `blake3` — no `vieww-foundation`, no `vieww-paint`, nothing render-tree
//! shaped. A GPU backend (`vieww-hal`'s `vulkan`/`metal`/`d3d12` modules,
//! and eventually the `vieww-gpu-*` backend crates) depends on this one,
//! never the other way around — the same one-directional dependency rule
//! the rest of the render stack follows.
//!
//! # What's real here, and what a GPU backend still has to do
//!
//! Every module in this crate — parsing, validation, cross-compilation to
//! all three backend languages, reflection, cache keys, and the reload
//! mechanism — is pure-CPU `naga` and `blake3`, so all of it is genuinely
//! implemented and tested in this delivery, not deferred pending a GPU or
//! another OS's toolchain. What this crate does *not* do: turn the SPIR-V/
//! MSL/HLSL text it produces into an actual GPU pipeline object. That's a
//! backend's job (`vieww-hal::vulkan` already does it for SPIR-V; `metal`
//! and `d3d12`'s pipeline ports are the "still not here, and exactly why"
//! documented in their own modules) — this crate hands a backend correct
//! shader text and a description of its bindings, not a compiled pipeline.

pub mod compiler;
pub mod hot_reload;
pub mod library;
pub mod pipeline_cache;
pub mod reflection;

pub use compiler::{compile, CompileError, CompiledShader, ParsedShader, ShaderStage};
pub use hot_reload::{ReloadHook, ReloadReport, ReloadableLibrary};
pub use library::{ShaderLibrary, ShaderSource};
pub use pipeline_cache::{PipelineCache, PipelineCacheKey};
pub use reflection::{reflect, BindingKind, BindingReflection, Reflection, VertexAttribute};
