use core::cell::{Cell, RefCell, UnsafeCell};

use alloc::{boxed::Box, collections::TryReserveError, vec::Vec};

use thiserror::Error;

use crate::{limited_collections::LimitedVec, LimitedBackingBufferError, LimitedWriter};

/// Trait for writing bytes.
pub trait Write {
  type WriteError;
  type FlushError;

  /// Write the contents of `input_buffer` to the underlying device.
  /// Providing an empty `input_buffer` is valid and will return 0 bytes written.
  ///
  /// Returns the number of bytes written.
  /// If `sync_hint` is true, it indicates that the write should be flushed to the actual device.
  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError>;

  /// Flush any buffered data to the underlying device.
  /// Must be called at the end to ensure all data is written.
  fn flush(&mut self) -> Result<(), Self::FlushError>;
}

impl<W: Write + ?Sized> Write for &mut W {
  type WriteError = W::WriteError;
  type FlushError = W::FlushError;

  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    (**self).write(input_buffer, sync_hint)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    (**self).flush()
  }
}

impl<W: Write + ?Sized> Write for Box<W> {
  type WriteError = W::WriteError;
  type FlushError = W::FlushError;

  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    self.as_mut().write(input_buffer, sync_hint)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    self.as_mut().flush()
  }
}

impl<W: Write + ?Sized> Write for RefCell<W> {
  type WriteError = W::WriteError;
  type FlushError = W::FlushError;

  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    self.get_mut().write(input_buffer, sync_hint)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    self.get_mut().flush()
  }
}

impl<W: Write + ?Sized> Write for Cell<W> {
  type WriteError = W::WriteError;
  type FlushError = W::FlushError;

  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    self.get_mut().write(input_buffer, sync_hint)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    self.get_mut().flush()
  }
}

impl<W: Write + ?Sized> Write for UnsafeCell<W> {
  type WriteError = W::WriteError;
  type FlushError = W::FlushError;

  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    self.get_mut().write(input_buffer, sync_hint)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    self.get_mut().flush()
  }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum SliceWriteError {
  #[error("Slice is not large enough to write the requested data of size {requested_size}")]
  SliceFull { requested_size: usize },
}

/// Write is implemented for `&mut [u8]` by copying into the slice, overwriting
/// its data.
///
/// Note that writing updates the slice to point to the yet unwritten part.
/// The slice will be empty when it has been completely overwritten.
///
/// If the number of bytes to be written exceeds the size of the slice, write operations will
/// return short writes: ultimately, `Ok(0)`; in this situation, `write_all` returns an error of
/// kind `WriteZero`.
impl Write for &mut [u8] {
  type WriteError = SliceWriteError;
  type FlushError = core::convert::Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    let amt = core::cmp::min(input_buffer.len(), self.len());
    let (a, b) = core::mem::take(self).split_at_mut(amt);

    a.copy_from_slice(&input_buffer[..amt]);

    *self = b;
    Ok(amt)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

impl Write for Vec<u8> {
  type WriteError = TryReserveError;
  type FlushError = core::convert::Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    if input_buffer.is_empty() {
      return Ok(0);
    }
    let bytes_to_write = input_buffer.len();
    self.try_reserve(bytes_to_write)?;
    let len = self.len();
    self.extend_from_slice(input_buffer);
    Ok(self.len() - len)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

impl Write for LimitedVec<u8> {
  type WriteError = LimitedBackingBufferError<TryReserveError>;
  type FlushError = core::convert::Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    if input_buffer.is_empty() {
      return Ok(0);
    }
    let bytes_to_write = input_buffer.len();
    self.try_reserve(bytes_to_write)?;
    let len = self.len();
    self.extend_from_slice(input_buffer);
    Ok(self.len() - len)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

// --- WriteLimited trait ---

pub trait WriteLimited: Write {
  /// Creates a new writer that limits the number of bytes written to `write_limit_bytes`.
  ///
  /// Returns a new [`LimitedWriter`] instance.
  #[must_use]
  fn put(&mut self, write_limit_bytes: usize) -> LimitedWriter<&mut Self>;
}

impl<W: Write + ?Sized> WriteLimited for W {
  fn put(&mut self, write_limit_bytes: usize) -> LimitedWriter<&mut Self> {
    LimitedWriter::new(self, write_limit_bytes)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_write_vec() {
    let mut buffer = Vec::new();
    let input = [1, 2, 3, 4, 5];
    let result = buffer.write(&input, false);
    assert_eq!(result, Ok(5));
    assert_eq!(buffer, input);
    let result = buffer.write(&[], false);
    assert_eq!(result, Ok(0));
  }
}
