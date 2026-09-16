//! Instancing and batching: packing many repeated draws into one GPU
//! submission.
//!
//! `docs/RENDERER-V2-NOTES.md` flags GPU-visible instance buffers and
//! indirect draw generation as needing a real GPU execution pipeline this
//! workspace does not have yet — true. What *is* CPU-computable today, and
//! what this module does, is the packing step every one of those
//! eventually needs: given N draws that share a pipeline and a mesh, lay
//! out one instance buffer for all of them instead of N separate draw
//! calls. `vieww_paint::native::instancing`'s `InstancedMask` already
//! proved the CPU-rasterizer half of this idea (reuse one rasterized shape
//! across whole-pixel translations); this is the GPU-shaped generalization,
//! keyed on shared shape identity rather than exact-translation reuse.

use std::collections::HashMap;
use std::hash::Hash;

/// Per-instance data a batched draw call needs: where it goes and how it's
/// tinted. `[f32; 4]` transform columns rather than a named matrix type, so
/// this module has no dependency on any particular transform
/// representation — a caller lays its own `Transform` out into this shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    /// Column-major 2D affine transform: `[a, b, c, d, tx, ty]`.
    pub transform: [f32; 6],
    pub color: [f32; 4],
}

/// One batch: a shared mesh key, drawn once per instance in `instances`.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch<K> {
    pub mesh_key: K,
    pub instances: Vec<Instance>,
}

/// Group `draws` by mesh key, preserving each group's relative draw order
/// (instances within one batch still draw in the order they were
/// submitted, so z-ordering inside one batch is unaffected — only draws
/// that share a key are ever reordered relative to draws with a
/// *different* key, and only when [`group_by_mesh`]'s caller has already
/// established those different-key draws don't overlap, which is a
/// decision this function does not make).
#[must_use]
pub fn group_by_mesh<K: Eq + Hash + Clone>(draws: Vec<(K, Instance)>) -> Vec<Batch<K>> {
    let mut order: Vec<K> = Vec::new();
    let mut groups: HashMap<K, Vec<Instance>> = HashMap::new();
    for (key, instance) in draws {
        groups.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Vec::new()
        });
        groups.get_mut(&key).expect("just inserted").push(instance);
    }
    order
        .into_iter()
        .map(|key| {
            let instances = groups.remove(&key).expect("tracked in order");
            Batch {
                mesh_key: key,
                instances,
            }
        })
        .collect()
}

/// Whether batching a group of `count` same-mesh draws is worth the extra
/// instance-buffer upload versus just issuing `count` separate draw calls —
/// mirrors `hybrid::worth_splitting`'s reasoning: below some
/// count, per-draw dispatch overhead is cheaper than building and uploading
/// a buffer nobody amortizes the cost of.
#[must_use]
pub const fn worth_batching(count: usize, minimum: usize) -> bool {
    count >= minimum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(x: f32) -> Instance {
        Instance {
            transform: [1.0, 0.0, 0.0, 1.0, x, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn draws_sharing_a_mesh_key_land_in_one_batch() {
        let draws = vec![
            ("icon", instance(0.0)),
            ("label", instance(10.0)),
            ("icon", instance(20.0)),
        ];
        let batches = group_by_mesh(draws);
        assert_eq!(batches.len(), 2);
        let icon_batch = batches.iter().find(|b| b.mesh_key == "icon").unwrap();
        assert_eq!(icon_batch.instances.len(), 2);
    }

    #[test]
    fn batch_order_matches_first_appearance() {
        let draws = vec![
            ("b", instance(0.0)),
            ("a", instance(1.0)),
            ("b", instance(2.0)),
        ];
        let batches = group_by_mesh(draws);
        assert_eq!(batches[0].mesh_key, "b");
        assert_eq!(batches[1].mesh_key, "a");
    }

    #[test]
    fn a_single_repeated_draw_is_not_worth_batching() {
        assert!(!worth_batching(1, 4));
        assert!(worth_batching(4, 4));
    }
}
