//! Resource and vertex-input reflection off a validated `naga::Module` — so
//! a GPU backend can build a bind group layout and a vertex buffer layout
//! from the shader itself, rather than a second, hand-maintained
//! description of the same bindings drifting out of sync with it.

use naga::{AddressSpace, Binding, ScalarKind, TypeInner};

/// What kind of resource one binding slot is, so far as this shader library
/// needs to distinguish — enough to build a `wgpu`/Vulkan-style bind group
/// layout entry, not a full re-statement of every `naga` type variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    UniformBuffer,
    StorageBuffer {
        read_only: bool,
    },
    SampledTexture,
    Sampler,
    /// Anything reflection didn't need a named case for (this library's
    /// built-ins never produce one, but a caller-registered shader might
    /// use a resource type not listed above — reported rather than
    /// silently dropped).
    Other,
}

#[derive(Debug, Clone)]
pub struct BindingReflection {
    pub group: u32,
    pub binding: u32,
    pub name: String,
    pub kind: BindingKind,
}

/// One scalar/vector vertex input attribute, in the shape a vertex buffer
/// layout needs: which shader `location` it feeds, and how many
/// float/int/uint components wide it is. This library's shaders never use
/// anything but `f32` components, so component count is what's reflected —
/// a caller building a `wgpu`-style `VertexFormat` maps `component_count`
/// (plus knowing it's always float here) onto `Float32`/`Float32x2`/etc.
#[derive(Debug, Clone)]
pub struct VertexAttribute {
    pub location: u32,
    pub name: String,
    pub component_count: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Reflection {
    pub bindings: Vec<BindingReflection>,
    pub vertex_attributes: Vec<VertexAttribute>,
}

fn classify(inner: &TypeInner, space: AddressSpace) -> BindingKind {
    match inner {
        // Every image variant (sampled, depth, storage, external) is
        // reported the same way — `SampledTexture` — since this library's
        // shaders only ever bind `texture_2d<f32>` and this crate does not
        // yet need to distinguish storage images from sampled ones at the
        // reflection layer; a caller that does can match on `naga`'s own
        // `ImageClass` directly off the module it already has.
        TypeInner::Image { .. } => BindingKind::SampledTexture,
        TypeInner::Sampler { .. } => BindingKind::Sampler,
        TypeInner::Struct { .. }
        | TypeInner::Scalar(_)
        | TypeInner::Vector { .. }
        | TypeInner::Matrix { .. } => match space {
            AddressSpace::Uniform => BindingKind::UniformBuffer,
            AddressSpace::Storage { access } => BindingKind::StorageBuffer {
                read_only: !access.contains(naga::StorageAccess::STORE),
            },
            _ => BindingKind::Other,
        },
        _ => BindingKind::Other,
    }
}

fn component_count(inner: &TypeInner) -> Option<u32> {
    match inner {
        TypeInner::Scalar(scalar) if scalar.kind == ScalarKind::Float => Some(1),
        TypeInner::Vector { size, scalar } if scalar.kind == ScalarKind::Float => {
            Some(*size as u32)
        }
        _ => None,
    }
}

/// Reflect every global resource binding in `module` (bindings are
/// module-wide in WGSL/naga, not per-entry-point, so this always reflects
/// the whole module) plus `entry_point`'s vertex input attributes, if it is
/// a vertex-stage entry point (an empty list otherwise — a fragment or
/// compute entry point has no per-vertex inputs to report).
#[must_use]
pub fn reflect(module: &naga::Module, entry_point: &naga::EntryPoint) -> Reflection {
    let mut bindings = Vec::new();
    for (_handle, global) in module.global_variables.iter() {
        let Some(resource_binding) = global.binding else {
            continue;
        };
        let ty = &module.types[global.ty].inner;
        bindings.push(BindingReflection {
            group: resource_binding.group,
            binding: resource_binding.binding,
            name: global.name.clone().unwrap_or_default(),
            kind: classify(ty, global.space),
        });
    }
    bindings.sort_by_key(|b| (b.group, b.binding));

    let mut vertex_attributes = Vec::new();
    if entry_point.stage == naga::ShaderStage::Vertex {
        for argument in &entry_point.function.arguments {
            let Some(Binding::Location { location, .. }) = argument.binding else {
                continue;
            };
            let ty = &module.types[argument.ty].inner;
            let Some(components) = component_count(ty) else {
                continue;
            };
            vertex_attributes.push(VertexAttribute {
                location,
                name: argument.name.clone().unwrap_or_default(),
                component_count: components,
            });
        }
        vertex_attributes.sort_by_key(|a| a.location);
    }

    Reflection {
        bindings,
        vertex_attributes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::ParsedShader;
    use crate::library;

    #[test]
    fn solid_reflects_one_uniform_buffer_and_one_vertex_attribute() {
        let parsed = ParsedShader::parse(library::SOLID_WGSL).unwrap();
        let reflection = parsed
            .reflect("vs_main", naga::ShaderStage::Vertex)
            .unwrap();
        assert_eq!(reflection.bindings.len(), 1);
        assert_eq!(reflection.bindings[0].kind, BindingKind::UniformBuffer);
        assert_eq!(reflection.bindings[0].group, 0);
        assert_eq!(reflection.bindings[0].binding, 0);
        assert_eq!(reflection.vertex_attributes.len(), 1);
        assert_eq!(reflection.vertex_attributes[0].location, 0);
        assert_eq!(reflection.vertex_attributes[0].component_count, 2);
    }

    #[test]
    fn image_reflects_a_uniform_a_texture_and_a_sampler() {
        let parsed = ParsedShader::parse(library::IMAGE_WGSL).unwrap();
        let reflection = parsed
            .reflect("fs_main", naga::ShaderStage::Fragment)
            .unwrap();
        assert_eq!(reflection.bindings.len(), 3);
        assert_eq!(reflection.bindings[0].kind, BindingKind::UniformBuffer);
        assert_eq!(reflection.bindings[1].kind, BindingKind::SampledTexture);
        assert_eq!(reflection.bindings[2].kind, BindingKind::Sampler);
    }

    #[test]
    fn a_fragment_entry_point_reflects_no_vertex_attributes() {
        let parsed = ParsedShader::parse(library::SOLID_WGSL).unwrap();
        let reflection = parsed
            .reflect("fs_main", naga::ShaderStage::Fragment)
            .unwrap();
        assert!(reflection.vertex_attributes.is_empty());
    }

    #[test]
    fn composite_reflects_two_textures_and_two_samplers_at_distinct_bindings() {
        let parsed = ParsedShader::parse(library::COMPOSITE_WGSL).unwrap();
        let reflection = parsed
            .reflect("fs_main", naga::ShaderStage::Fragment)
            .unwrap();
        let texture_count = reflection
            .bindings
            .iter()
            .filter(|b| b.kind == BindingKind::SampledTexture)
            .count();
        let sampler_count = reflection
            .bindings
            .iter()
            .filter(|b| b.kind == BindingKind::Sampler)
            .count();
        assert_eq!(texture_count, 2);
        assert_eq!(sampler_count, 2);
        let bindings: Vec<u32> = reflection.bindings.iter().map(|b| b.binding).collect();
        let mut sorted = bindings.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            bindings.len(),
            sorted.len(),
            "binding numbers must be distinct: {bindings:?}"
        );
    }
}
