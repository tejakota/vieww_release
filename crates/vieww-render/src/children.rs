//! A node's child list, which does not allocate for the shape a tree actually
//! has.
//!
//! # The measurement
//!
//! `docs/PERFORMANCE.md` named this as the first of two things left after the
//! profiling round, and named it by its cost: `set_children` allocated a
//! `Vec<RenderId>` per node, and roughly 16% of a keystroke frame was
//! allocation. The distribution behind that number, counted over the studio's
//! own tree with the shell mounted:
//!
//! | children | nodes | share | cumulative |
//! |---:|---:|---:|---:|
//! | 0 | 518 | 32.4% | 32.4% |
//! | 1 | 956 | **59.7%** | 92.1% |
//! | 2 | 38 | 2.4% | 94.4% |
//! | 3 | 25 | 1.6% | 96.0% |
//! | 4 | 2 | 0.1% | 96.1% |
//! | 5 | 39 | 2.4% | 98.6% |
//! | 6 | 2 | 0.1% | 98.7% |
//! | 7+ | 21 | 1.3% | 100% |
//!
//! **A render tree is a chain, not a fan.** Nine nodes in ten have zero or one
//! child, because every `Padding`, `Align`, `Container`, `Opacity`,
//! `ColoredBox`, `Clip` and `Transform` in a widget tree is a single-child
//! wrapper, and a screen is mostly wrappers. The nodes that branch are the
//! handful of `Flex`es and `Stack`s holding them together.
//!
//! So of the 1083 nodes that allocate with a bare `Vec`, an inline capacity of
//! **4** removes 1021 of them — 94.3%. One removes 88.3%, two removes 91.8%,
//! six removes 98.1%. Four is the choice because the array is then 32 bytes and
//! the enum 40 against the bare `Vec`'s 24, and the step to six costs another
//! 16 bytes on **every** node — including the nine in ten that never use the
//! first four — to catch a cluster of 39. `Node` is walked by layout, paint,
//! hit testing and the boundary collector on every frame, and its size is a
//! cache question rather than a memory one.
//!
//! Run `cargo run --release -p viewwstudio --example bench -- --fanout` to
//! reprint the table above, so the constant can be re-argued against a real
//! tree — this one or a different application's — rather than against this
//! paragraph. It comes from
//! [`RenderTree::child_fanout`](crate::RenderTree::child_fanout).
//!
//! # Why this is not a general-purpose `SmallVec`
//!
//! It holds [`RenderId`], which is `Copy`, 8 bytes and has no `Drop`. That is
//! what makes the inline half a plain array with a `len` beside it and **no
//! `unsafe` anywhere** — no `MaybeUninit`, no manual drop, no aliasing
//! argument to get wrong. A generic version would need all three, and would be
//! a hundred lines of unsafe code in a crate that currently has none, to hold
//! one type.

use std::fmt;

use crate::RenderId;

/// How many children fit without touching the heap. See the module note.
const INLINE: usize = 4;

/// The value the unused inline slots hold.
///
/// They are never read — `len` is the only thing that says how much of the
/// array is real — so this exists to let the array be built in safe code
/// rather than to mean anything. A generation of `u32::MAX` is one the arena
/// does not issue, so if a bug ever *did* read past `len` the result is an id
/// that matches no node rather than one that silently addresses slot zero.
const PADDING: RenderId = RenderId::new(u32::MAX, u32::MAX);

/// A render node's children, inline up to four and on the heap beyond.
///
/// Derefs to `[RenderId]`, so everything that reads a child list — layout,
/// paint, hit testing, the boundary walk — is written against a slice and does
/// not know which half it is holding.
#[derive(Clone)]
pub struct ChildIds(Storage);

#[derive(Clone)]
enum Storage {
    /// `len` of `items` are real; the rest are `PADDING`.
    Inline { items: [RenderId; INLINE], len: u8 },
    /// Longer than the inline capacity. **Never shrinks back to `Inline`**: a list that
    /// grew past four is one a `Flex` produced, and it will be that long again
    /// next frame — dropping the allocation on a transient shrink only means
    /// making it again.
    Spilled(Vec<RenderId>),
}

impl ChildIds {
    /// An empty list. Allocates nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self(Storage::Inline {
            items: [PADDING; INLINE],
            len: 0,
        })
    }

    /// The single-child case, which is 60% of the tree.
    #[must_use]
    pub const fn one(id: RenderId) -> Self {
        Self(Storage::Inline {
            items: [id, PADDING, PADDING, PADDING],
            len: 1,
        })
    }

    /// The children, as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[RenderId] {
        match &self.0 {
            Storage::Inline { items, len } => &items[..*len as usize],
            Storage::Spilled(items) => items,
        }
    }

    /// Append `id`, moving to the heap if this is the fifth child.
    pub fn push(&mut self, id: RenderId) {
        match &mut self.0 {
            Storage::Inline { items, len } if (*len as usize) < INLINE => {
                items[*len as usize] = id;
                *len += 1;
            }
            Storage::Inline { items, len } => {
                let mut spilled = Vec::with_capacity(INLINE * 2);
                spilled.extend_from_slice(&items[..*len as usize]);
                spilled.push(id);
                self.0 = Storage::Spilled(spilled);
            }
            Storage::Spilled(items) => items.push(id),
        }
    }

    /// Append every id in `ids`.
    pub fn extend_from_slice(&mut self, ids: &[RenderId]) {
        // The common case by a wide margin — `sync_element` splices one child's
        // contribution into its parent's list, and a contribution is one id.
        // Reserving up front for the spilled case keeps a long list from
        // growing a element at a time.
        if let Storage::Spilled(items) = &mut self.0 {
            items.extend_from_slice(ids);
            return;
        }
        for &id in ids {
            self.push(id);
        }
    }

    /// Drop every id `keep` rejects, in place.
    ///
    /// Never moves a spilled list back inline: a list that grew past four is
    /// one a `Flex` produced, and it will be that long again next frame.
    pub fn retain(&mut self, mut keep: impl FnMut(RenderId) -> bool) {
        match &mut self.0 {
            Storage::Inline { items, len } => {
                let mut kept = 0usize;
                for index in 0..*len as usize {
                    let id = items[index];
                    if keep(id) {
                        items[kept] = id;
                        kept += 1;
                    }
                }
                for slot in items.iter_mut().take(*len as usize).skip(kept) {
                    *slot = PADDING;
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "kept <= len <= INLINE, which is 4"
                )]
                {
                    *len = kept as u8;
                }
            }
            Storage::Spilled(items) => items.retain(|&id| keep(id)),
        }
    }
}

impl Default for ChildIds {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for ChildIds {
    type Target = [RenderId];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl PartialEq for ChildIds {
    /// By **contents**, not by which half holds them. A list of two that spilled
    /// once and a list of two that never did are the same children, and
    /// `set_children`'s early return asks whether the children changed.
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for ChildIds {}

impl fmt::Debug for ChildIds {
    /// As a list. Which storage it is in is an implementation detail that would
    /// otherwise show up in every `RenderTree` dump.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

impl From<Vec<RenderId>> for ChildIds {
    /// Takes the allocation as-is when it is longer than the inline capacity,
    /// rather
    /// than copying into the array and dropping a `Vec` that is already the
    /// right shape.
    fn from(items: Vec<RenderId>) -> Self {
        if items.len() > INLINE {
            return Self(Storage::Spilled(items));
        }
        let mut out = Self::new();
        out.extend_from_slice(&items);
        out
    }
}

impl From<&[RenderId]> for ChildIds {
    fn from(items: &[RenderId]) -> Self {
        let mut out = Self::new();
        out.extend_from_slice(items);
        out
    }
}

impl<const N: usize> From<[RenderId; N]> for ChildIds {
    fn from(items: [RenderId; N]) -> Self {
        Self::from(&items[..])
    }
}

impl FromIterator<RenderId> for ChildIds {
    fn from_iter<I: IntoIterator<Item = RenderId>>(iter: I) -> Self {
        let mut out = Self::new();
        for id in iter {
            out.push(id);
        }
        out
    }
}

impl<'a> IntoIterator for &'a ChildIds {
    type Item = &'a RenderId;
    type IntoIter = std::slice::Iter<'a, RenderId>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl IntoIterator for ChildIds {
    type Item = RenderId;
    type IntoIter = std::vec::IntoIter<RenderId>;

    /// Owned iteration goes through a `Vec`, which is the one operation that
    /// *always* allocates. It has exactly one caller — `remove`, detaching a
    /// subtree — where the list is being torn down anyway and a walk that
    /// mutates the tree cannot borrow it.
    fn into_iter(self) -> Self::IntoIter {
        match self.0 {
            // `to_vec` rather than `iter().copied()`: the return type is one
            // iterator, and a borrowed one cannot outlive the array it is
            // reading out of a value this method consumed.
            #[expect(
                clippy::unnecessary_to_owned,
                reason = "the array is owned and about to drop"
            )]
            Storage::Inline { items, len } => items[..len as usize].to_vec().into_iter(),
            Storage::Spilled(items) => items.into_iter(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> RenderId {
        RenderId::new(n, 0)
    }

    #[test]
    fn a_new_list_is_empty() {
        let list = ChildIds::new();
        assert!(list.is_empty());
        assert_eq!(list.as_slice(), &[] as &[RenderId]);
    }

    /// The case the whole type exists for: 58% of the tree, and it must not
    /// reach the heap.
    #[test]
    fn the_first_four_children_stay_inline() {
        let mut list = ChildIds::new();
        for n in 0..4 {
            list.push(id(n));
        }
        assert!(matches!(list.0, Storage::Inline { .. }));
        assert_eq!(list.as_slice(), &[id(0), id(1), id(2), id(3)]);
    }

    #[test]
    fn the_fifth_child_spills_and_keeps_the_first_four() {
        let mut list = ChildIds::new();
        for n in 0..5 {
            list.push(id(n));
        }
        assert!(matches!(list.0, Storage::Spilled(_)));
        assert_eq!(list.as_slice(), &[id(0), id(1), id(2), id(3), id(4)]);
    }

    /// A spilled list stays spilled. Shrinking back would drop an allocation
    /// the next frame is about to make again.
    #[test]
    fn a_spilled_list_does_not_come_back_inline() {
        let mut list: ChildIds = (0..6).map(id).collect();
        list.retain(|held| held == id(0));
        assert!(matches!(list.0, Storage::Spilled(_)));
        assert_eq!(list.as_slice(), &[id(0)]);
    }

    #[test]
    fn retain_compacts_an_inline_list() {
        let mut list: ChildIds = (0..4).map(id).collect();
        list.retain(|held| held != id(1));
        assert_eq!(list.as_slice(), &[id(0), id(2), id(3)]);
        // And the vacated slot is the padding value rather than a stale id, so
        // a read past `len` cannot address a real node.
        let Storage::Inline { items, len } = list.0 else {
            panic!("still inline");
        };
        assert_eq!(len, 3);
        assert_eq!(items[3], PADDING);
    }

    #[test]
    fn extend_crosses_the_boundary_correctly() {
        let mut list = ChildIds::one(id(9));
        list.extend_from_slice(&[id(0), id(1), id(2), id(3), id(4)]);
        assert_eq!(list.as_slice(), &[id(9), id(0), id(1), id(2), id(3), id(4)]);
    }

    /// Equality is about children, not about storage — the early return in
    /// `set_children` depends on it.
    #[test]
    fn a_spilled_list_equals_an_inline_one_with_the_same_children() {
        let inline: ChildIds = (0..2).map(id).collect();
        let mut spilled: ChildIds = (0..6).map(id).collect();
        spilled.retain(|held| held == id(0) || held == id(1));
        assert!(matches!(spilled.0, Storage::Spilled(_)));
        assert_eq!(inline, spilled);
    }

    /// A `Vec` longer than the array is adopted rather than copied.
    #[test]
    fn a_long_vec_is_taken_by_move() {
        let items: Vec<RenderId> = (0..7).map(id).collect();
        let pointer = items.as_ptr();
        let list = ChildIds::from(items);
        let Storage::Spilled(held) = &list.0 else {
            panic!("a seven-child list belongs on the heap");
        };
        assert_eq!(held.as_ptr(), pointer, "the allocation was copied");
    }

    #[test]
    fn a_short_vec_comes_back_inline() {
        let list = ChildIds::from(vec![id(0), id(1)]);
        assert!(matches!(list.0, Storage::Inline { .. }));
        assert_eq!(list.as_slice(), &[id(0), id(1)]);
    }

    /// The type must not have grown past what it is worth: an inline list of
    /// four ids against the bare `Vec` it replaced.
    #[test]
    fn the_list_is_the_size_the_module_note_claims() {
        assert_eq!(std::mem::size_of::<ChildIds>(), 40);
        assert_eq!(std::mem::size_of::<Vec<RenderId>>(), 24);
    }
}
