//! Dense, method-local identities and the tables indexed by them.
//!
//! An identity is not a position in an arbitrary vector: values, local storage,
//! and control-flow blocks have independent namespaces. Tables expose their own
//! identities so algorithms do not need to reconstruct them from integer ranges.

use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

pub trait EntityId: Copy {
    fn from_index(index: usize) -> Self;
    fn index(self) -> usize;
}

macro_rules! entity_id {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(usize);

        impl $name {
            pub const fn from_index(index: usize) -> Self { Self(index) }
            pub const fn index(self) -> usize { self.0 }
        }

        impl EntityId for $name {
            fn from_index(index: usize) -> Self { Self(index) }
            fn index(self) -> usize { self.0 }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    )+};
}

entity_id!(ValueId, LocalId, BlockId);

/// A dense table whose positions belong to one identity namespace.
///
/// ```compile_fail
/// use sporeflower::index::{BlockId, IdVec, ValueId};
/// let values: IdVec<ValueId, u8> = vec![7].into();
/// let _ = values[BlockId::from_index(0)];
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdVec<I, T> {
    values: Vec<T>,
    id: PhantomData<fn(I) -> I>,
}

impl<I: EntityId, T> IdVec<I, T> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            id: PhantomData,
        }
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn next_id(&self) -> I {
        I::from_index(self.len())
    }
    pub fn push(&mut self, value: T) -> I {
        let id = self.next_id();
        self.values.push(value);
        id
    }
    pub fn ids(&self) -> impl DoubleEndedIterator<Item = I> + ExactSizeIterator + use<I, T> {
        (0..self.len()).map(I::from_index)
    }
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.values.iter()
    }
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.values.iter_mut()
    }
    pub fn iter_enumerated(&self) -> impl DoubleEndedIterator<Item = (I, &T)> + ExactSizeIterator {
        self.values
            .iter()
            .enumerate()
            .map(|(i, value)| (I::from_index(i), value))
    }
    pub fn iter_enumerated_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = (I, &mut T)> + ExactSizeIterator {
        self.values
            .iter_mut()
            .enumerate()
            .map(|(i, value)| (I::from_index(i), value))
    }
    pub fn into_iter_enumerated(
        self,
    ) -> impl DoubleEndedIterator<Item = (I, T)> + ExactSizeIterator {
        self.values
            .into_iter()
            .enumerate()
            .map(|(i, value)| (I::from_index(i), value))
    }
    pub fn get(&self, id: I) -> Option<&T> {
        self.values.get(id.index())
    }
    pub fn get_mut(&mut self, id: I) -> Option<&mut T> {
        self.values.get_mut(id.index())
    }
    pub fn contains_id(&self, id: I) -> bool {
        id.index() < self.len()
    }
    pub fn fill(&mut self, value: T)
    where
        T: Clone,
    {
        self.values.fill(value);
    }
    pub fn resize(&mut self, len: usize, value: T)
    where
        T: Clone,
    {
        self.values.resize(len, value);
    }
    pub fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub fn extend(&mut self, values: impl IntoIterator<Item = T>) {
        self.values.extend(values);
    }
}

impl<I, T> Default for IdVec<I, T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            id: PhantomData,
        }
    }
}
impl<I, T> From<Vec<T>> for IdVec<I, T> {
    fn from(values: Vec<T>) -> Self {
        Self {
            values,
            id: PhantomData,
        }
    }
}
impl<I, T> FromIterator<T> for IdVec<I, T> {
    fn from_iter<It: IntoIterator<Item = T>>(iter: It) -> Self {
        Self::from(Vec::from_iter(iter))
    }
}
impl<I: EntityId, T> Index<I> for IdVec<I, T> {
    type Output = T;
    fn index(&self, id: I) -> &T {
        &self.values[id.index()]
    }
}
impl<I: EntityId, T> IndexMut<I> for IdVec<I, T> {
    fn index_mut(&mut self, id: I) -> &mut T {
        &mut self.values[id.index()]
    }
}
impl<I, T> IntoIterator for IdVec<I, T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}
impl<'a, I, T> IntoIterator for &'a IdVec<I, T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}
impl<'a, I, T> IntoIterator for &'a mut IdVec<I, T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}

#[cfg(test)]
pub(crate) mod test_ids {
    use super::{BlockId, LocalId, ValueId};
    pub const B0: BlockId = BlockId::from_index(0);
    pub const B1: BlockId = BlockId::from_index(1);
    pub const B2: BlockId = BlockId::from_index(2);
    pub const B3: BlockId = BlockId::from_index(3);
    pub const B4: BlockId = BlockId::from_index(4);
    pub const B5: BlockId = BlockId::from_index(5);
    pub const B10: BlockId = BlockId::from_index(10);
    pub const B20: BlockId = BlockId::from_index(20);
    pub const L0: LocalId = LocalId::from_index(0);
    pub const L1: LocalId = LocalId::from_index(1);
    pub const L2: LocalId = LocalId::from_index(2);
    pub const L3: LocalId = LocalId::from_index(3);
    pub const L4: LocalId = LocalId::from_index(4);
    pub const L5: LocalId = LocalId::from_index(5);
    pub const V0: ValueId = ValueId::from_index(0);
    pub const V1: ValueId = ValueId::from_index(1);
    pub const V2: ValueId = ValueId::from_index(2);
    pub const V3: ValueId = ValueId::from_index(3);
    pub const V4: ValueId = ValueId::from_index(4);
    pub const V5: ValueId = ValueId::from_index(5);
    pub const V6: ValueId = ValueId::from_index(6);
    pub const V7: ValueId = ValueId::from_index(7);
    pub const V8: ValueId = ValueId::from_index(8);
    pub const V9: ValueId = ValueId::from_index(9);
}
