use core::fmt::Display;

use alloc::{boxed::Box, collections::TryReserveError, vec::Vec};

use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub struct ResizeError<U> {
  pub size_after_resize: usize,
  pub resize_error: U,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub struct FixedSizeBufferError {
  pub fixed_buffer_size: usize,
  pub requested_size: usize,
}

impl Display for FixedSizeBufferError {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    write!(
      f,
      "Buffer has a fixed size of {}, but requested size is {}",
      self.fixed_buffer_size, self.requested_size
    )
  }
}

pub trait BackingBuffer {
  type ResizeError;

  /// Returns the new size of the buffer after resizing.
  ///
  /// If a larger size is requested but no new items could be allocated,
  /// an error must be returned.
  #[must_use]
  fn try_resize(&mut self, requested_size: usize) -> Result<usize, ResizeError<Self::ResizeError>>;

  #[must_use]
  fn len(&self) -> usize;
}

impl<B: BackingBuffer + ?Sized> BackingBuffer for &mut B {
  type ResizeError = B::ResizeError;

  fn try_resize(&mut self, requested_size: usize) -> Result<usize, ResizeError<Self::ResizeError>> {
    (**self).try_resize(requested_size)
  }

  fn len(&self) -> usize {
    (**self).len()
  }
}

impl<T: Clone + Default> BackingBuffer for Vec<T> {
  type ResizeError = TryReserveError;

  fn try_resize(&mut self, requested_size: usize) -> Result<usize, ResizeError<Self::ResizeError>> {
    let len = self.len();
    self
      .try_reserve(requested_size.saturating_sub(len))
      .map_err(|e| ResizeError {
        size_after_resize: len,
        resize_error: e,
      })?;
    self.resize(requested_size, Default::default());
    Ok(requested_size)
  }

  fn len(&self) -> usize {
    (**self).len()
  }
}

impl<T> BackingBuffer for &mut [T] {
  type ResizeError = FixedSizeBufferError;

  fn try_resize(&mut self, requested_size: usize) -> Result<usize, ResizeError<Self::ResizeError>> {
    let len = self.len();
    if requested_size > len {
      return Err(ResizeError {
        size_after_resize: len,
        resize_error: FixedSizeBufferError {
          fixed_buffer_size: len,
          requested_size: requested_size,
        },
      });
    }
    Ok(self.len())
  }

  fn len(&self) -> usize {
    (**self).len()
  }
}

impl<const N: usize, T> BackingBuffer for [T; N] {
  type ResizeError = FixedSizeBufferError;

  fn try_resize(&mut self, requested_size: usize) -> Result<usize, ResizeError<Self::ResizeError>> {
    if requested_size > N {
      return Err(ResizeError {
        size_after_resize: N,
        resize_error: FixedSizeBufferError {
          fixed_buffer_size: N,
          requested_size: requested_size,
        },
      });
    }
    Ok(N)
  }

  fn len(&self) -> usize {
    self.as_ref().len()
  }
}

impl<T> BackingBuffer for Box<[T]> {
  type ResizeError = FixedSizeBufferError;

  fn try_resize(&mut self, requested_size: usize) -> Result<usize, ResizeError<Self::ResizeError>> {
    // A Box<[T]> has a fixed size. Resizing would require a new allocation,
    // which is not supported by this implementation. For a resizable buffer, use Vec<T>.
    let len = self.len();

    if requested_size > len {
      return Err(ResizeError {
        size_after_resize: len,
        resize_error: FixedSizeBufferError {
          fixed_buffer_size: len,
          requested_size: requested_size,
        },
      });
    }
    Ok(len)
  }

  fn len(&self) -> usize {
    self.as_ref().len()
  }
}

/// Imposes a size limit on the resize function of a [`BackingBufferMut`].
#[derive(Clone, Debug)]
pub struct LimitedBackingBuffer<B: BackingBuffer> {
  backing_buffer: B,
  max_len: usize,
}

impl<B: BackingBuffer> LimitedBackingBuffer<B> {
  #[must_use]
  pub fn new(backing_buffer: B, max_size: usize) -> Self {
    Self {
      backing_buffer,
      max_len: max_size,
    }
  }

  #[must_use]
  pub fn backing_buffer(&self) -> &B {
    &self.backing_buffer
  }

  #[must_use]
  pub fn backing_buffer_mut(&mut self) -> &mut B {
    &mut self.backing_buffer
  }

  #[must_use]
  pub fn max_len(&self) -> usize {
    self.max_len
  }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum LimitedBackingBufferError<U> {
  #[error("Memory limit of {0} bytes exceeded for resize")]
  MemoryLimitExceeded(usize),
  #[error("Underlying resize error: {0:?}")]
  ResizeError(#[from] U),
}

impl<B: BackingBuffer> BackingBuffer for LimitedBackingBuffer<B> {
  type ResizeError = LimitedBackingBufferError<B::ResizeError>;

  fn try_resize(&mut self, requested_size: usize) -> Result<usize, ResizeError<Self::ResizeError>> {
    let resize_size = requested_size.min(self.max_len);
    let new_elements = resize_size.saturating_sub(self.backing_buffer.len());
    if new_elements == 0 {
      return Err(ResizeError {
        size_after_resize: self.backing_buffer.len(),
        resize_error: Self::ResizeError::MemoryLimitExceeded(self.max_len),
      });
    }
    let requested_size = self
      .backing_buffer
      .try_resize(resize_size)
      .map_err(|e| ResizeError {
        size_after_resize: e.size_after_resize,
        resize_error: Self::ResizeError::ResizeError(e.resize_error),
      })?;
    Ok(requested_size)
  }

  fn len(&self) -> usize {
    self.backing_buffer.len()
  }
}

impl<B: BackingBuffer + AsMut<[u8]>> AsMut<[u8]> for LimitedBackingBuffer<B> {
  fn as_mut(&mut self) -> &mut [u8] {
    self.backing_buffer.as_mut()
  }
}

impl<B: BackingBuffer + AsRef<[u8]>> AsRef<[u8]> for LimitedBackingBuffer<B> {
  fn as_ref(&self) -> &[u8] {
    self.backing_buffer.as_ref()
  }
}
