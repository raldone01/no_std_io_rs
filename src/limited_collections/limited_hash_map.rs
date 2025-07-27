use core::{
  convert::Infallible,
  hash::{BuildHasher, Hash},
  mem::MaybeUninit,
  ops::{Index, IndexMut, RangeBounds},
  slice::{self, SliceIndex},
};

use hashbrown::{
  hash_map::{
    Drain, Entry, EntryRef, ExtractIf, IntoKeys, IntoValues, Iter, IterMut, Keys, OccupiedError,
    Values, ValuesMut,
  },
  DefaultHashBuilder, Equivalent, HashMap, TryReserveError,
};

use crate::{BackingBuffer, LimitedBackingBufferError, ResizeError};

#[derive(Debug, Clone)]
pub struct LimitedHashMap<K, V, S = DefaultHashBuilder> {
  map: HashMap<K, V, S>,
  max_keys: usize,
}

impl<K, V> LimitedHashMap<K, V, DefaultHashBuilder> {
  #[must_use]
  pub fn new(max_keys: usize) -> Self {
    Self {
      map: HashMap::new(),
      max_keys,
    }
  }

  #[must_use]
  pub fn with_capacity(
    max_keys: usize,
    capacity: usize,
  ) -> Result<Self, LimitedBackingBufferError<TryReserveError>> {
    if capacity > max_keys {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(max_keys));
    }
    Ok(Self {
      map: HashMap::with_capacity_and_hasher(capacity, DefaultHashBuilder::default()),
      max_keys,
    })
  }
}

impl<K, V, S> LimitedHashMap<K, V, S> {
  #[must_use]
  pub fn from_hash_map(max_keys: usize, map: HashMap<K, V, S>) -> Self {
    Self { map, max_keys }
  }

  #[inline]
  #[must_use]
  pub fn max_keys(&self) -> usize {
    self.max_keys
  }

  #[inline]
  #[must_use]
  pub fn as_hash_map(&self) -> &HashMap<K, V, S> {
    &self.map
  }

  #[inline]
  #[must_use]
  pub fn to_hash_map(self) -> HashMap<K, V, S> {
    self.map
  }

  #[must_use]
  pub const fn with_hasher(max_keys: usize, hash_builder: S) -> Self {
    Self {
      map: HashMap::with_hasher(hash_builder),
      max_keys,
    }
  }

  #[must_use]
  pub fn with_capacity_and_hasher(
    max_keys: usize,
    capacity: usize,
    hash_builder: S,
  ) -> Result<Self, LimitedBackingBufferError<TryReserveError>> {
    if capacity > max_keys {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(max_keys));
    }
    Ok(Self {
      map: HashMap::with_capacity_and_hasher(capacity, hash_builder),
      max_keys,
    })
  }

  #[must_use]
  pub fn capacity(&self) -> usize {
    self.map.capacity()
  }

  #[must_use]
  pub fn keys(&self) -> Keys<'_, K, V> {
    self.map.keys()
  }

  #[must_use]
  pub fn values(&self) -> Values<'_, K, V> {
    self.map.values()
  }

  #[must_use]
  pub fn values_mut(&mut self) -> ValuesMut<'_, K, V> {
    self.map.values_mut()
  }

  #[must_use]
  pub fn iter(&self) -> Iter<'_, K, V> {
    self.map.iter()
  }

  #[must_use]
  pub fn iter_mut(&mut self) -> IterMut<'_, K, V> {
    self.map.iter_mut()
  }

  #[must_use]
  pub fn len(&self) -> usize {
    self.map.len()
  }

  #[must_use]
  pub fn is_empty(&self) -> bool {
    self.map.is_empty()
  }

  #[must_use]
  pub fn drain(&mut self) -> Drain<'_, K, V> {
    self.map.drain()
  }

  #[must_use]
  pub fn retain<F>(&mut self, mut f: F)
  where
    F: FnMut(&K, &mut V) -> bool,
  {
    self.map.retain(f);
  }

  #[must_use]
  pub fn extract_if<F>(&mut self, f: F) -> ExtractIf<'_, K, V, F>
  where
    F: FnMut(&K, &mut V) -> bool,
  {
    self.map.extract_if(f)
  }

  pub fn clear(&mut self) {
    self.map.clear();
  }

  #[must_use]
  pub fn into_keys(self) -> IntoKeys<K, V> {
    self.map.into_keys()
  }

  #[must_use]
  pub fn into_values(self) -> IntoValues<K, V> {
    self.map.into_values()
  }
}

impl<K, V, S> LimitedHashMap<K, V, S>
where
  K: Eq + Hash,
  S: BuildHasher,
{
  pub fn try_reserve(
    &mut self,
    additional: usize,
  ) -> Result<(), LimitedBackingBufferError<TryReserveError>> {
    if self.len() + additional > self.max_keys {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(
        self.max_keys,
      ));
    }
    self.map.try_reserve(additional)?;
    Ok(())
  }

  pub fn shrink_to_fit(&mut self) {
    self.map.shrink_to_fit();
  }

  pub fn shrink_to(&mut self, min_capacity: usize) {
    self.map.shrink_to(min_capacity);
  }

  pub fn entry(
    &mut self,
    key: K,
  ) -> Result<Entry<'_, K, V, S>, LimitedBackingBufferError<TryReserveError>> {
    let len = self.len();
    let entry = self.map.entry(key);
    if matches!(entry, Entry::Vacant(_)) && len >= self.max_keys {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(
        self.max_keys,
      ));
    }
    Ok(entry)
  }

  pub fn entry_ref<'a, 'b, Q>(
    &'a mut self,
    key: &'b Q,
  ) -> Result<EntryRef<'a, 'b, K, Q, V, S>, LimitedBackingBufferError<TryReserveError>>
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    let len = self.len();
    let entry = self.map.entry_ref(key);
    if matches!(entry, EntryRef::Vacant(_)) && len >= self.max_keys {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(
        self.max_keys,
      ));
    }
    Ok(entry)
  }

  #[inline]
  #[must_use]
  pub fn get<Q>(&self, k: &Q) -> Option<&V>
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.get(k)
  }

  #[inline]
  #[must_use]
  pub fn get_key_value<Q>(&self, k: &Q) -> Option<(&K, &V)>
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.get_key_value(k)
  }

  #[inline]
  #[must_use]
  pub fn get_key_value_mut<Q>(&mut self, k: &Q) -> Option<(&K, &mut V)>
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.get_key_value_mut(k)
  }

  #[must_use]
  pub fn contains_key<Q>(&self, k: &Q) -> bool
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.contains_key(k)
  }

  #[must_use]
  pub fn get_mut<Q>(&mut self, k: &Q) -> Option<&mut V>
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.get_mut(k)
  }

  #[must_use]
  pub fn get_many_mut<Q, const N: usize>(&mut self, ks: [&Q; N]) -> [Option<&'_ mut V>; N]
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.get_many_mut(ks)
  }

  #[must_use]
  pub unsafe fn get_many_unchecked_mut<Q, const N: usize>(
    &mut self,
    ks: [&Q; N],
  ) -> [Option<&'_ mut V>; N]
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    unsafe { self.map.get_many_unchecked_mut(ks) }
  }

  #[must_use]
  pub fn get_many_key_value_mut<Q, const N: usize>(
    &mut self,
    ks: [&Q; N],
  ) -> [Option<(&'_ K, &'_ mut V)>; N]
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.get_many_key_value_mut(ks)
  }

  #[must_use]
  pub unsafe fn get_many_key_value_unchecked_mut<Q, const N: usize>(
    &mut self,
    ks: [&Q; N],
  ) -> [Option<(&'_ K, &'_ mut V)>; N]
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    unsafe { self.map.get_many_key_value_unchecked_mut(ks) }
  }

  pub fn insert(
    &mut self,
    k: K,
    v: V,
  ) -> Result<Option<V>, LimitedBackingBufferError<TryReserveError>> {
    // To use the possibly faster underlying `hashbrown` insert function we would need to reimplement the whole HashMap based on a HashTable.
    // For now this is good enough.
    self.entry(k).map(|entry| match entry {
      Entry::Occupied(mut occupied) => Ok(Some(occupied.insert(v))),
      Entry::Vacant(vacant) => {
        vacant.insert(v);
        Ok(None)
      },
    })?
  }

  pub fn try_insert(
    &mut self,
    key: K,
    value: V,
  ) -> Result<Result<&mut V, OccupiedError<'_, K, V, S>>, LimitedBackingBufferError<TryReserveError>>
  {
    match self.entry(key)? {
      Entry::Occupied(entry) => Ok(Err(OccupiedError { entry, value })),
      Entry::Vacant(entry) => Ok(Ok(entry.insert(value))),
    }
  }

  pub fn remove<Q>(&mut self, k: &Q) -> Option<V>
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.remove(k)
  }

  pub fn remove_entry<Q>(&mut self, k: &Q) -> Option<(K, V)>
  where
    Q: Hash + Equivalent<K> + ?Sized,
  {
    self.map.remove_entry(k)
  }

  #[must_use]
  pub fn allocation_size(&self) -> usize {
    self.map.allocation_size()
  }
}

impl<K, V, S> PartialEq for LimitedHashMap<K, V, S>
where
  K: Eq + Hash,
  V: PartialEq,
  S: BuildHasher,
{
  fn eq(&self, other: &Self) -> bool {
    self.map == other.map
  }
}

impl<K, V, S> Eq for LimitedHashMap<K, V, S>
where
  K: Eq + Hash,
  V: Eq,
  S: BuildHasher,
{
}

impl<K, Q, V, S> Index<&Q> for LimitedHashMap<K, V, S>
where
  K: Eq + Hash,
  Q: Hash + Equivalent<K> + ?Sized,
  S: BuildHasher,
{
  type Output = V;

  fn index(&self, index: &Q) -> &Self::Output {
    self.map.index(index)
  }
}
