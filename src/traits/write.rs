use core::cell::{Cell, RefCell, UnsafeCell};

use alloc::boxed::Box;
use thiserror::Error;

use crate::{advance, LimitedWriter};

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

fn write_slice<T: AsMut<[u8]> + ?Sized>(
  slice: &mut T,
  input_buffer: &[u8],
) -> Result<usize, SliceWriteError> {
  if input_buffer.is_empty() {
    return Ok(0);
  }
  let slice = &mut slice.as_mut();

  let bytes_to_write = core::cmp::min(input_buffer.len(), slice.len());
  if bytes_to_write == 0 {
    return Err(SliceWriteError::SliceFull {
      requested_size: input_buffer.len(),
    });
  }
  slice[..bytes_to_write].copy_from_slice(&input_buffer[..bytes_to_write]);
  advance(slice, bytes_to_write);
  Ok(bytes_to_write)
}

impl Write for [u8] {
  type WriteError = SliceWriteError;
  type FlushError = core::convert::Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    write_slice(self, input_buffer)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

impl<const N: usize> Write for [u8; N] {
  type WriteError = SliceWriteError;
  type FlushError = core::convert::Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    write_slice(self, input_buffer)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

pub trait WriteLimited: Write {
  /// Creates a new writer that limits the number of bytes written to `write_limit_bytes`.
  ///
  /// Returns a new [`LimitedWriter`] instance.
  #[must_use]
  fn put(&mut self, write_limit_bytes: usize) -> LimitedWriter<'_, Self>;
}

impl<W: Write + ?Sized> WriteLimited for W {
  fn put(&mut self, write_limit_bytes: usize) -> LimitedWriter<'_, Self> {
    LimitedWriter::new(self, write_limit_bytes)
  }
}
