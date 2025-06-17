use alloc::{boxed::Box, collections::TryReserveError, vec::Vec};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum FixedSizeBufferError {
  #[error("Buffer has a fixed size of {size}, but requested size is {requested_size}")]
  FixedSize { size: usize, requested_size: usize },
}

pub trait BackingBufferMut: AsMut<[u8]> + AsRef<[u8]> {
  type ResizeError;

  /// Returns the new size of the buffer after resizing.
  ///
  /// When shrinking is requested, but the buffer has a fixed size, returning a larger size than `new_size` is allowed.
  ///
  /// It is an error to return less than the requested size.
  #[must_use]
  fn try_resize(&mut self, new_size: usize) -> Result<usize, Self::ResizeError>;
}

impl BackingBufferMut for Vec<u8> {
  type ResizeError = TryReserveError;

  fn try_resize(&mut self, new_size: usize) -> Result<usize, Self::ResizeError> {
    self.try_reserve(new_size.saturating_sub(self.len()))?;
    self.resize(new_size, 0);
    Ok(self.len())
  }
}

impl<'a> BackingBufferMut for &'a mut [u8] {
  type ResizeError = FixedSizeBufferError;

  fn try_resize(&mut self, new_size: usize) -> Result<usize, Self::ResizeError> {
    if new_size > self.len() {
      return Err(FixedSizeBufferError::FixedSize {
        size: self.len(),
        requested_size: new_size,
      });
    }
    Ok(self.len())
  }
}

impl<const N: usize> BackingBufferMut for [u8; N] {
  type ResizeError = FixedSizeBufferError;

  fn try_resize(&mut self, new_size: usize) -> Result<usize, Self::ResizeError> {
    if new_size > N {
      return Err(FixedSizeBufferError::FixedSize {
        size: N,
        requested_size: new_size,
      });
    }
    Ok(N)
  }
}

impl BackingBufferMut for Box<[u8]> {
  type ResizeError = FixedSizeBufferError;

  fn try_resize(&mut self, new_size: usize) -> Result<usize, Self::ResizeError> {
    // A Box<[u8]> has a fixed size. Resizing would require a new allocation,
    // which is not supported by this implementation. For a resizable buffer, use Vec<u8>.
    /*
    let mut vec = self.to_vec();
    vec.try_reserve(new_size.saturating_sub(vec.len()))?;
    vec.resize(new_size, 0);
    *self = vec.into_boxed_slice();
    Ok(self.len())
     */
    if new_size > self.len() {
      return Err(FixedSizeBufferError::FixedSize {
        size: self.len(),
        requested_size: new_size,
      });
    }
    Ok(self.len())
  }
}

/// Imposes a size limit on the resize function of a [`BackingBufferMut`].
pub struct LimitedBackingBuffer<'a, B: BackingBufferMut + ?Sized> {
  backing_buffer: &'a mut B,
  max_size: usize,
}

impl<'a, B: BackingBufferMut + ?Sized> LimitedBackingBuffer<'a, B> {
  #[must_use]
  pub fn new(backing_buffer: &'a mut B, max_size: usize) -> Self {
    Self {
      backing_buffer,
      max_size,
    }
  }

  pub fn backing_buffer(&self) -> &B {
    &self.backing_buffer
  }

  pub fn backing_buffer_mut(&mut self) -> &mut B {
    &mut self.backing_buffer
  }
}

#[derive(Error, Debug)]
pub enum LimitedBackingBufferError<U> {
  #[error("Memory limit of {0} bytes exceeded for resize")]
  MemoryLimitExceeded(usize),
  #[error("Underlying resize error: {0:?}")]
  ResizeError(#[from] U),
}

impl<B: BackingBufferMut + ?Sized> AsMut<[u8]> for LimitedBackingBuffer<'_, B> {
  fn as_mut(&mut self) -> &mut [u8] {
    self.backing_buffer.as_mut()
  }
}

impl<B: BackingBufferMut + ?Sized> BackingBufferMut for LimitedBackingBuffer<'_, B> {
  type ResizeError = LimitedBackingBufferError<B::ResizeError>;

  fn try_resize(&mut self, new_size: usize) -> Result<usize, Self::ResizeError> {
    if new_size > self.max_size {
      return Err(LimitedBackingBufferError::MemoryLimitExceeded(
        self.max_size,
      ));
    }
    let new_size = self.backing_buffer.try_resize(new_size)?;
    Ok(new_size)
  }
}

impl<B: BackingBufferMut + ?Sized> AsRef<[u8]> for LimitedBackingBuffer<'_, B> {
  fn as_ref(&self) -> &[u8] {
    self.backing_buffer.as_ref()
  }
}
