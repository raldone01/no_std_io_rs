use core::{
  mem::MaybeUninit,
  ops::{Index, IndexMut, RangeBounds},
  slice::{self, SliceIndex},
};

use alloc::{
  boxed::Box,
  collections::TryReserveError,
  vec::{Drain, ExtractIf, IntoIter, Splice, Vec},
};

use crate::{BackingBuffer, LimitedBackingBufferError, ResizeError};

#[derive(Debug, Hash, Clone, Eq, Ord)]
pub struct LimitedVec<T> {
  vec: Vec<T>,
  max_len: usize,
}

impl<T> LimitedVec<T> {
  #[inline]
  #[must_use]
  pub const fn new(max_len: usize) -> Self {
    Self {
      vec: Vec::new(),
      max_len,
    }
  }

  /// This function does not check the length of the vec since the vec exists already anyway.
  #[inline]
  #[must_use]
  pub fn from_vec(max_len: usize, vec: Vec<T>) -> Self {
    Self { vec, max_len }
  }

  #[inline]
  #[must_use]
  pub fn max_len(&self) -> usize {
    self.max_len
  }

  #[inline]
  #[must_use]
  pub fn as_vec(&self) -> &Vec<T> {
    &self.vec
  }

  #[inline]
  #[must_use]
  pub fn to_vec(self) -> Vec<T> {
    self.vec
  }

  #[inline]
  #[must_use]
  pub fn with_capacity(
    max_len: usize,
    capacity: usize,
  ) -> Result<Self, LimitedBackingBufferError<TryReserveError>> {
    if capacity > max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(max_len));
    }
    let vec = Vec::with_capacity(capacity); // TODO: use try_with_capacity once it is stable
    Ok(Self {
      vec,
      max_len: max_len,
    })
  }

  #[inline]
  #[must_use]
  pub const fn capacity(&self) -> usize {
    self.vec.capacity()
  }

  pub fn try_reserve(
    &mut self,
    additional: usize,
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>> {
    if self.len() + additional > self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.try_reserve(additional)?;
    Ok(())
  }

  pub fn try_reserve_exact(
    &mut self,
    additional: usize,
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>> {
    if self.len() + additional > self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.try_reserve_exact(additional)?;
    Ok(())
  }

  #[inline]
  pub fn shrink_to_fit(&mut self) {
    self.vec.shrink_to_fit();
  }

  pub fn shrink_to(&mut self, min_capacity: usize) {
    self.vec.shrink_to(min_capacity);
  }

  #[must_use]
  pub fn into_boxed_slice(self) -> Box<[T]> {
    self.vec.into_boxed_slice()
  }

  #[inline]
  pub fn truncate(&mut self, len: usize) {
    self.vec.truncate(len);
  }

  #[inline]
  #[must_use]
  pub const fn as_slice(&self) -> &[T] {
    self.vec.as_slice()
  }

  #[inline]
  #[must_use]
  pub const fn as_mut_slice(&mut self) -> &mut [T] {
    self.vec.as_mut_slice()
  }

  #[inline]
  #[must_use]
  pub const fn as_ptr(&self) -> *const T {
    self.vec.as_ptr()
  }

  #[inline]
  pub const fn as_mut_ptr(&mut self) -> *mut T {
    self.vec.as_mut_ptr()
  }

  #[inline]
  pub fn swap_remove(&mut self, index: usize) -> T {
    self.vec.swap_remove(index)
  }

  pub fn insert(
    &mut self,
    index: usize,
    element: T,
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>> {
    if self.vec.len() >= self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.insert(index, element);
    Ok(())
  }

  pub fn remove(&mut self, index: usize) -> T {
    self.vec.remove(index)
  }

  pub fn retain<F>(&mut self, mut f: F)
  where
    F: FnMut(&T) -> bool,
  {
    self.vec.retain(&mut f);
  }

  pub fn retain_mut<F>(&mut self, mut f: F)
  where
    F: FnMut(&mut T) -> bool,
  {
    self.vec.retain_mut(&mut f);
  }

  #[inline]
  pub fn dedup_by_key<F, K>(&mut self, mut key: F)
  where
    F: FnMut(&mut T) -> K,

    K: PartialEq,
  {
    self.vec.dedup_by_key(&mut key);
  }

  pub fn dedup_by<F>(&mut self, mut same_bucket: F)
  where
    F: FnMut(&mut T, &mut T) -> bool,
  {
    self.vec.dedup_by(&mut same_bucket);
  }

  #[inline]
  pub fn push(&mut self, value: T) -> Result<(), LimitedBackingBufferError<TryReserveError>> {
    if self.vec.len() >= self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.push(value);
    Ok(())
  }

  #[inline]
  pub fn push_within_capacity(&mut self, value: T) -> Result<(), T> {
    // TODO: use push_within_capacity once it is stable
    if self.vec.len() >= self.vec.capacity() {
      return Err(value);
    }
    self.vec.push(value);
    Ok(())
  }

  #[inline]
  pub fn pop(&mut self) -> Option<T> {
    self.vec.pop()
  }

  pub fn pop_if(&mut self, predicate: impl FnOnce(&mut T) -> bool) -> Option<T> {
    self.vec.pop_if(predicate)
  }

  #[inline]
  pub fn append(
    &mut self,
    other: &mut Self,
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>> {
    if self.vec.len() + other.vec.len() > self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.append(&mut other.vec);
    Ok(())
  }

  pub fn drain<R>(&mut self, range: R) -> Drain<'_, T>
  where
    R: RangeBounds<usize>,
  {
    self.vec.drain(range)
  }

  #[inline]
  pub fn clear(&mut self) {
    self.vec.clear();
  }

  #[inline]
  pub const fn len(&self) -> usize {
    self.vec.len()
  }

  pub const fn is_empty(&self) -> bool {
    self.vec.is_empty()
  }

  #[inline]
  pub fn split_off(&mut self, at: usize) -> Self {
    let split_vec = self.vec.split_off(at);
    Self {
      vec: split_vec,
      max_len: self.max_len,
    }
  }

  pub fn resize_with<F>(
    &mut self,
    new_len: usize,
    f: F,
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>>
  where
    F: FnMut() -> T,
  {
    if new_len > self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.resize_with(new_len, f);
    Ok(())
  }

  #[inline]

  pub fn leak(self) -> &'static mut [T] {
    self.vec.leak()
  }

  #[inline]
  pub fn spare_capacity_mut(&mut self) -> &mut [MaybeUninit<T>] {
    self.vec.spare_capacity_mut()
  }
}

impl<T: Clone> LimitedVec<T> {
  pub fn resize(
    &mut self,
    new_len: usize,
    value: T,
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>> {
    if new_len > self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.resize(new_len, value);
    Ok(())
  }

  pub fn extend_from_slice(
    &mut self,
    other: &[T],
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>> {
    if self.vec.len() + other.len() > self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.extend_from_slice(other);
    Ok(())
  }

  pub fn extend_from_within<R>(
    &mut self,
    src: R,
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>>
  where
    R: RangeBounds<usize>,
  {
    let left_bound = match src.start_bound() {
      core::ops::Bound::Included(&start) => start,
      core::ops::Bound::Excluded(&start) => start + 1,
      core::ops::Bound::Unbounded => 0,
    };
    let right_bound = match src.end_bound() {
      core::ops::Bound::Included(&end) => end + 1,
      core::ops::Bound::Excluded(&end) => end,
      core::ops::Bound::Unbounded => self.vec.len(),
    };
    let range_len = right_bound.saturating_sub(left_bound);
    if self.vec.len() + range_len > self.max_len {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(self.max_len));
    }
    self.vec.extend_from_within(src);
    Ok(())
  }
}

impl<T, const N: usize> LimitedVec<[T; N]> {
  pub fn into_flattened(self) -> Vec<T> {
    self.vec.into_flattened()
  }
}

impl<T: PartialEq> LimitedVec<T> {
  #[inline]
  pub fn dedup(&mut self) {
    self.dedup_by(|a, b| a == b)
  }
}

impl<T> core::ops::Deref for LimitedVec<T> {
  type Target = [T];

  #[inline]
  fn deref(&self) -> &[T] {
    self.as_slice()
  }
}

impl<T> core::ops::DerefMut for LimitedVec<T> {
  #[inline]
  fn deref_mut(&mut self) -> &mut [T] {
    self.as_mut_slice()
  }
}

impl<T, I: SliceIndex<[T]>> Index<I> for LimitedVec<T> {
  type Output = I::Output;

  #[inline]
  fn index(&self, index: I) -> &Self::Output {
    Index::index(&**self, index)
  }
}

impl<T, I: SliceIndex<[T]>> IndexMut<I> for LimitedVec<T> {
  #[inline]
  fn index_mut(&mut self, index: I) -> &mut Self::Output {
    IndexMut::index_mut(&mut **self, index)
  }
}

impl<T> LimitedVec<T> {
  #[inline]
  pub fn try_from_iter<I: IntoIterator<Item = T>>(
    max_len: usize,
    iter: I,
  ) -> Result<Self, LimitedBackingBufferError<TryReserveError>> {
    let mut vec = LimitedVec::new(max_len);
    for item in iter {
      vec.push(item)?;
    }
    Ok(vec)
  }
}

impl<T> IntoIterator for LimitedVec<T> {
  type Item = T;

  type IntoIter = IntoIter<T>;

  #[inline]
  fn into_iter(self) -> Self::IntoIter {
    self.vec.into_iter()
  }
}

impl<'a, T> IntoIterator for &'a LimitedVec<T> {
  type Item = &'a T;

  type IntoIter = slice::Iter<'a, T>;

  fn into_iter(self) -> Self::IntoIter {
    self.iter()
  }
}

impl<'a, T> IntoIterator for &'a mut LimitedVec<T> {
  type Item = &'a mut T;

  type IntoIter = slice::IterMut<'a, T>;

  fn into_iter(self) -> Self::IntoIter {
    self.iter_mut()
  }
}

impl<T> LimitedVec<T> {
  #[inline]
  pub fn splice<R, I>(&mut self, range: R, replace_with: I) -> Splice<'_, I::IntoIter>
  where
    R: RangeBounds<usize>,
    I: IntoIterator<Item = T>,
  {
    self.vec.splice(range, replace_with)
  }

  pub fn extract_if<F, R>(&mut self, range: R, filter: F) -> ExtractIf<'_, T, F>
  where
    F: FnMut(&mut T) -> bool,
    R: RangeBounds<usize>,
  {
    self.vec.extract_if(range, filter)
  }
}

impl<T> AsRef<LimitedVec<T>> for LimitedVec<T> {
  fn as_ref(&self) -> &LimitedVec<T> {
    self
  }
}

impl<T> AsMut<LimitedVec<T>> for LimitedVec<T> {
  fn as_mut(&mut self) -> &mut LimitedVec<T> {
    self
  }
}

impl<T> AsRef<[T]> for LimitedVec<T> {
  fn as_ref(&self) -> &[T] {
    self
  }
}

impl<T> AsMut<[T]> for LimitedVec<T> {
  fn as_mut(&mut self) -> &mut [T] {
    self
  }
}

impl<T: PartialEq> PartialEq for LimitedVec<T> {
  fn eq(&self, other: &Self) -> bool {
    self.vec == other.vec
  }
}

impl<T: PartialOrd> PartialOrd for LimitedVec<T> {
  fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
    self.vec.partial_cmp(&other.vec)
  }
}

impl<T: Clone + Default> BackingBuffer for LimitedVec<T> {
  type ResizeError = LimitedBackingBufferError<TryReserveError>;

  fn try_resize(&mut self, requested_size: usize) -> Result<usize, ResizeError<Self::ResizeError>> {
    let resize_size = requested_size.min(self.max_len);
    let new_elements = resize_size.saturating_sub(self.vec.len());
    if new_elements == 0 {
      return Err(ResizeError {
        size_after_resize: self.vec.len(),
        resize_error: Self::ResizeError::MemoryLimitExceeded(self.max_len),
      });
    }
    let requested_size = self.vec.try_resize(resize_size).map_err(|e| ResizeError {
      size_after_resize: e.size_after_resize,
      resize_error: Self::ResizeError::ResizeError(e.resize_error),
    })?;
    Ok(requested_size)
  }

  fn len(&self) -> usize {
    self.vec.len()
  }
}
