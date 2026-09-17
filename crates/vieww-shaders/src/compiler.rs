//! Parse, validate, cross-compile and reflect one WGSL module.
//!
//! Every step here is pure-CPU `naga` — no GPU, no OS-specific toolchain,
//! no host target needed. That's the reason this crate's "cross-compile to
//! SPIR-V/MSL/HLSL" claim can be fully real and fully tested in this
//! delivery's sandbox, unlike the GPU backends themselves
//! (`vieww-hal::metal`/`d3d12`): naga's MSL and HLSL backends are text
//! generators, not compilers that need Metal.framework or `d3dcompiler.dll`
//! present to run. What they produce still needs a real Metal/D3D12
//! compiler downstream to turn into a pipeline object on the target OS —
//! this crate gets the shader *text* right, not the final GPU object.

use crate::pipeline_cache::PipelineCacheKey;
use naga::valid::{Capabilities, ValidationFlags, Validator};
pub use naga::ShaderStage;

#[derive(Debug)]
pub enum CompileError {
    Parse(String),
    Validate(String),
    Backend {
        target: &'static str,
        message: String,
    },
    /// The requested entry point does not exist in the module, or exists
    /// with a different stage than requested.
    MissingEntryPoint {
        name: String,
        stage: ShaderStage,
    },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "WGSL parse error: {message}"),
            Self::Validate(message) => write!(f, "WGSL validation error: {message}"),
            Self::Backend { target, message } => write!(f, "{target} codegen error: {message}"),
            Self::MissingEntryPoint { name, stage } => {
                write!(f, "no {stage:?} entry point named {name:?}")
            }
        }
    }
}

impl std::error::Error for CompileError {}

/// A parsed and validated WGSL module, ready to be cross-compiled or
/// reflected any number of times without re-parsing — the unit both
/// [`crate::pipeline_cache::PipelineCache`] and hot reload key off of.
pub struct ParsedShader {
    pub(crate) module: naga::Module,
    pub(crate) info: naga::valid::ModuleInfo,
    source: String,
}

impl std::fmt::Debug for ParsedShader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedShader")
            .field(
                "entry_points",
                &self
                    .module
                    .entry_points
                    .iter()
                    .map(|ep| &ep.name)
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl ParsedShader {
    /// Parse and validate `wgsl`. `Capabilities::all()` — not `::empty()`
    /// like `vieww-hal::vulkan`'s narrower clear/mesh shaders use — because
    /// this crate's built-ins (`image.wgsl`, `blur.wgsl`, …) use texture
    /// sampling and control flow the empty capability set doesn't need to
    /// forbid; naga's validator still rejects anything an actual target
    /// can't run, capabilities only gate *optional* hardware features
    /// (e.g. `PUSH_CONSTANT`) this library's shaders don't use anyway.
    pub fn parse(wgsl: &str) -> Result<Self, CompileError> {
        let module =
            naga::front::wgsl::parse_str(wgsl).map_err(|e| CompileError::Parse(e.to_string()))?;
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .map_err(|e| CompileError::Validate(e.to_string()))?;
        Ok(Self {
            module,
            info,
            source: wgsl.to_string(),
        })
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    fn find_entry_point(
        &self,
        name: &str,
        stage: ShaderStage,
    ) -> Result<&naga::EntryPoint, CompileError> {
        self.module
            .entry_points
            .iter()
            .find(|ep| ep.name == name && ep.stage == stage)
            .ok_or_else(|| CompileError::MissingEntryPoint {
                name: name.to_string(),
                stage,
            })
    }

    /// Translate to SPIR-V — the same `naga::back::spv` call
    /// `vieww-hal::vulkan::translate_wgsl_to_spirv` makes, generalized to
    /// any entry point rather than the two hardcoded ones there. A whole
    /// module is written at once (SPIR-V's binary carries every entry
    /// point; `entry_point`/`stage` select which one a *caller* means to
    /// bind when building a pipeline, they don't change what bytes come
    /// out), so `entry_point`/`stage` here only need to exist for the
    /// error to be caught early rather than at pipeline-creation time.
    pub fn to_spirv(
        &self,
        entry_point: &str,
        stage: ShaderStage,
    ) -> Result<Vec<u32>, CompileError> {
        self.find_entry_point(entry_point, stage)?;
        // Bounds-checked image loads: naga's default leaves an out-of-range
        // `textureLoad` unchecked, which is undefined behaviour that software
        // rasterisers happen to forgive and real drivers do not. Out of range
        // reads as transparent — what the CPU compositor does.
        let options = naga::back::spv::Options {
            bounds_check_policies: naga::proc::BoundsCheckPolicies {
                index: naga::proc::BoundsCheckPolicy::Restrict,
                buffer: naga::proc::BoundsCheckPolicy::Restrict,
                image_load: naga::proc::BoundsCheckPolicy::ReadZeroSkipWrite,
                binding_array: naga::proc::BoundsCheckPolicy::Restrict,
            },
            ..naga::back::spv::Options::default()
        };
        let mut writer =
            naga::back::spv::Writer::new(&options).map_err(|e| CompileError::Backend {
                target: "spir-v",
                message: e.to_string(),
            })?;
        let mut spirv = Vec::new();
        writer
            .write(&self.module, &self.info, None, &None, &mut spirv)
            .map_err(|e| CompileError::Backend {
                target: "spir-v",
                message: e.to_string(),
            })?;
        Ok(spirv)
    }

    /// Translate one entry point to Metal Shading Language, for
    /// `vieww-hal::metal`'s eventual pipeline port (see that module's own
    /// docs for exactly what's blocking the rest of that port — this
    /// function is not one of the blocked pieces; it's real and tested
    /// here).
    pub fn to_msl(&self, entry_point: &str, stage: ShaderStage) -> Result<String, CompileError> {
        self.find_entry_point(entry_point, stage)?;
        let options = naga::back::msl::Options::default();
        let pipeline_options = naga::back::msl::PipelineOptions {
            entry_point: Some((stage, entry_point.to_string())),
            ..Default::default()
        };
        let (source, _info) =
            naga::back::msl::write_string(&self.module, &self.info, &options, &pipeline_options)
                .map_err(|e| CompileError::Backend {
                    target: "msl",
                    message: e.to_string(),
                })?;
        Ok(source)
    }

    /// Translate one entry point to HLSL, for `vieww-hal::d3d12`'s eventual
    /// pipeline port — same relationship to that module as [`Self::to_msl`]
    /// has to `vieww-hal::metal`.
    pub fn to_hlsl(&self, entry_point: &str, stage: ShaderStage) -> Result<String, CompileError> {
        self.find_entry_point(entry_point, stage)?;
        // Push constants ("immediates") live in a constant buffer on D3D12, and
        // naga needs to be told which register. Space 0, register 15 — well
        // clear of the low registers the built-in shaders bind resources to.
        let options = naga::back::hlsl::Options {
            immediates_target: Some(naga::back::hlsl::BindTarget {
                space: 0,
                register: 15,
                binding_array_size: None,
                dynamic_storage_buffer_offsets_index: None,
                restrict_indexing: false,
            }),
            ..naga::back::hlsl::Options::default()
        };
        let pipeline_options = naga::back::hlsl::PipelineOptions {
            entry_point: Some((stage, entry_point.to_string())),
        };
        let mut buffer = String::new();
        let fragment_entry_point = if stage == ShaderStage::Fragment {
            naga::back::hlsl::FragmentEntryPoint::new(&self.module, entry_point)
        } else {
            None
        };
        let mut writer = naga::back::hlsl::Writer::new(&mut buffer, &options, &pipeline_options);
        writer
            .write(&self.module, &self.info, fragment_entry_point.as_ref())
            .map_err(|e| CompileError::Backend {
                target: "hlsl",
                message: e.to_string(),
            })?;
        Ok(buffer)
    }

    /// A cache key over this exact source text and requested entry
    /// point/stage — see [`crate::pipeline_cache`] for what it's for and
    /// why raw-source hashing (rather than hashing the parsed IR) is the
    /// documented, deliberate choice here.
    #[must_use]
    pub fn cache_key(&self, entry_point: &str, stage: ShaderStage) -> PipelineCacheKey {
        PipelineCacheKey::new(&self.source, entry_point, stage)
    }

    /// Reflect one entry point's resource bindings and (for a vertex stage)
    /// input attributes — see [`crate::reflection`].
    pub fn reflect(
        &self,
        entry_point: &str,
        stage: ShaderStage,
    ) -> Result<crate::reflection::Reflection, CompileError> {
        let ep = self.find_entry_point(entry_point, stage)?;
        Ok(crate::reflection::reflect(&self.module, ep))
    }
}

/// Convenience: parse and translate to all three backends plus reflection
/// in one call, for a caller (or test) that wants the full picture without
/// juggling a [`ParsedShader`] itself.
#[derive(Debug)]
pub struct CompiledShader {
    pub spirv: Vec<u32>,
    pub msl: String,
    pub hlsl: String,
    pub reflection: crate::reflection::Reflection,
    pub cache_key: PipelineCacheKey,
}

pub fn compile(
    wgsl: &str,
    entry_point: &str,
    stage: ShaderStage,
) -> Result<CompiledShader, CompileError> {
    let parsed = ParsedShader::parse(wgsl)?;
    Ok(CompiledShader {
        spirv: parsed.to_spirv(entry_point, stage)?,
        msl: parsed.to_msl(entry_point, stage)?,
        hlsl: parsed.to_hlsl(entry_point, stage)?,
        reflection: parsed.reflect(entry_point, stage)?,
        cache_key: parsed.cache_key(entry_point, stage),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ShaderLibrary;

    #[test]
    fn every_built_in_shader_parses_and_validates() {
        let library = ShaderLibrary::new();
        for source in library.iter() {
            ParsedShader::parse(&source.wgsl)
                .unwrap_or_else(|e| panic!("{} failed to parse/validate: {e}", source.name));
        }
    }

    #[test]
    fn every_built_in_shader_cross_compiles_to_all_three_backends() {
        let library = ShaderLibrary::new();
        for source in library.iter() {
            let parsed = ParsedShader::parse(&source.wgsl).unwrap();
            let spirv = parsed.to_spirv("vs_main", ShaderStage::Vertex).unwrap();
            assert!(!spirv.is_empty(), "{}: empty SPIR-V", source.name);
            let msl = parsed.to_msl("vs_main", ShaderStage::Vertex).unwrap();
            assert!(
                msl.contains("vertex"),
                "{}: MSL missing a vertex function: {msl}",
                source.name
            );
            let hlsl = parsed.to_hlsl("vs_main", ShaderStage::Vertex).unwrap();
            assert!(!hlsl.is_empty(), "{}: empty HLSL", source.name);

            let spirv_fs = parsed.to_spirv("fs_main", ShaderStage::Fragment).unwrap();
            assert!(
                !spirv_fs.is_empty(),
                "{}: empty fragment SPIR-V",
                source.name
            );
            let msl_fs = parsed.to_msl("fs_main", ShaderStage::Fragment).unwrap();
            assert!(
                msl_fs.contains("fragment"),
                "{}: MSL missing a fragment function",
                source.name
            );
            let hlsl_fs = parsed.to_hlsl("fs_main", ShaderStage::Fragment).unwrap();
            assert!(!hlsl_fs.is_empty(), "{}: empty fragment HLSL", source.name);
        }
    }

    #[test]
    fn a_missing_entry_point_is_a_compile_error_not_a_panic() {
        let parsed = ParsedShader::parse(crate::library::SOLID_WGSL).unwrap();
        let error = parsed
            .to_spirv("does_not_exist", ShaderStage::Vertex)
            .unwrap_err();
        assert!(matches!(error, CompileError::MissingEntryPoint { .. }));
    }

    #[test]
    fn a_syntax_error_is_a_parse_error_not_a_panic() {
        let error = ParsedShader::parse("this is not wgsl {{{").unwrap_err();
        assert!(matches!(error, CompileError::Parse(_)));
    }

    #[test]
    fn compile_returns_a_fully_populated_result_for_solid() {
        let compiled = compile(crate::library::SOLID_WGSL, "vs_main", ShaderStage::Vertex).unwrap();
        assert!(!compiled.spirv.is_empty());
        assert!(!compiled.msl.is_empty());
        assert!(!compiled.hlsl.is_empty());
        assert!(!compiled.reflection.vertex_attributes.is_empty());
    }
}
