/// A container that holds a value of type `T` associated with a confidence `C`.
///
/// A `ConfidentValue` will only update its stored value if a new value is provided
/// with a confidence that is greater than or equal to the existing confidence.
/// This is useful for scenarios where a value is derived from multiple sources of
/// varying reliability, and you only want to keep the result from the most reliable source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentValue<C: Ord, T> {
  value: Option<(C, T)>,
}

impl<C: Ord, T> ConfidentValue<C, T> {
  /// Creates a new, empty `ConfidentValue`.
  pub fn new() -> Self {
    Default::default()
  }

  /// Returns `true` if the currently stored value has a strictly greater
  /// confidence than the new one being considered.
  fn has_superior_confidence(&self, new_confidence: &C) -> bool {
    self
      .value
      .as_ref()
      .map_or(false, |(current_confidence, _)| {
        *current_confidence > *new_confidence
      })
  }

  /// Unconditionally sets or replaces the stored value and its confidence.
  ///
  /// This bypasses the usual confidence check.
  pub fn set(&mut self, confidence: C, value: T) {
    self.value = Some((confidence, value));
  }

  /// Returns a reference to the stored value, if any.
  pub fn get(&self) -> Option<&T> {
    self.value.as_ref().map(|(_, v)| v)
  }

  /// Returns a reference to the confidence and value, if any.
  pub fn get_with_confidence(&self) -> Option<(&C, &T)> {
    self.value.as_ref().map(|(c, v)| (c, v))
  }

  /// Returns a reference to the value only if its confidence is less than or
  /// equal to the provided `max_confidence`.
  pub fn get_if_confidence_le(&self, max_confidence: &C) -> Option<&T> {
    self
      .value
      .as_ref()
      .filter(|(current_confidence, _)| *current_confidence <= *max_confidence)
      .map(|(_, value)| value)
  }

  /// Ensures a value is set, using a closure if the current confidence is insufficient.
  ///
  /// If the current value has a higher confidence than `new_confidence`, this function
  /// does nothing and returns a reference to the existing value.
  ///
  /// Otherwise, it calls the closure `f`. If `f` returns `Ok(value)`, the `ConfidentValue`
  /// is updated with the new value and confidence, and a reference to the new value is returned.
  /// If `f` returns an `Err`, the error is propagated and the `ConfidentValue` remains unchanged.
  ///
  /// # Returns
  /// - `Ok(&T)`: A reference to the value, which could be the old or new one.
  /// - `Err(E)`: The error returned by the closure.
  pub fn try_get_or_set_with<F, E>(&mut self, new_confidence: C, f: F) -> Result<&T, E>
  where
    F: FnOnce() -> Result<T, E>,
    C: Clone, // Required to re-insert confidence after a successful parse
  {
    if self.has_superior_confidence(&new_confidence) {
      // The unwrap is safe because has_superior_confidence is only true if value is Some.
      return Ok(&self.value.as_ref().unwrap().1);
    }

    match f() {
      Ok(parsed_value) => {
        self.set(new_confidence, parsed_value);
        // The unwrap is safe because we just set the value.
        Ok(&self.value.as_ref().unwrap().1)
      },
      Err(err) => Err(err),
    }
  }

  /// Ensures a value is set, using a closure that returns an `Option`.
  ///
  /// If the current value has a higher confidence than `new_confidence`, this function
  /// does nothing. Otherwise, it calls the closure `f`. If `f` returns `Some(value)`,
  /// the `ConfidentValue` is updated.
  ///
  /// In all cases, it returns a reference to the value that is present at the end of the
  /// operation, or `None` if no value was set.
  pub fn get_or_set_with<F>(&mut self, new_confidence: C, f: F) -> Option<&T>
  where
    F: FnOnce() -> Option<T>,
  {
    if !self.has_superior_confidence(&new_confidence) {
      if let Some(parsed_value) = f() {
        self.set(new_confidence, parsed_value);
      }
    }
    self.get()
  }

  pub fn update_with(&mut self, other: ConfidentValue<C, &T>) -> Option<&T>
  where
    T: Clone,
  {
    if let Some((other_confidence, other_value)) = other.value {
      if !self.has_superior_confidence(&other_confidence) {
        self.set(other_confidence, other_value.clone());
      }
    }
    self.get()
  }
}

impl<C: Ord, T> Default for ConfidentValue<C, T> {
  fn default() -> Self {
    Self { value: None }
  }
}

impl<C: Ord, T> AsRef<Option<(C, T)>> for ConfidentValue<C, T> {
  fn as_ref(&self) -> &Option<(C, T)> {
    &self.value
  }
}
