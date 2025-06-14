//! This module provides traits and utilities for reading and writing bytes in a no-std environment.
use thiserror::Error;

/// Trait for reading bytes.
pub trait Read {
  type ReadError;

  /// Read up to `output_buffer.len()` bytes into `output_buffer`.
  /// Providing an empty `output_buffer` is valid and will return 0 bytes read.
  ///
  /// Returns number of bytes read.
  /// On EOF, it returns 0 bytes read.
  /// Any further reads after EOF return 0 bytes read.
  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError>;
}

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

#[derive(Error, Debug)]
pub enum WriteAllError<U> {
  #[error("Underlying device wrote zero bytes after writing {bytes_written} bytes")]
  ZeroWrite { bytes_written: usize },
  #[error("Underlying write error: {0:?}")]
  Io(#[from] U),
}

/// Extension trait that provides a `write_all` method for any `Write` implementor.
pub trait WriteAll: Write {
  /// Writes the entire buffer, retrying partial writes.
  ///
  /// Does not flush, but passes the `sync_hint` to the underlying `write` method.
  fn write_all(
    &mut self,
    input_buffer: &[u8],
    sync_hint: bool,
  ) -> Result<(), WriteAllError<Self::WriteError>>;
}

/// Blanket implementation for all `Write` implementors.
impl<T: Write + ?Sized> WriteAll for T {
  fn write_all(
    &mut self,
    input_buffer: &[u8],
    sync_hint: bool,
  ) -> Result<(), WriteAllError<Self::WriteError>> {
    let mut buf = input_buffer;
    while !buf.is_empty() {
      match self.write(buf, sync_hint) {
        Ok(0) => {
          return Err(WriteAllError::ZeroWrite {
            bytes_written: input_buffer.len() - buf.len(),
          });
        },
        Ok(n) => buf = &buf[n..], // advance buffer
        Err(e) => return Err(WriteAllError::Io(e)),
      }
    }
    Ok(())
  }
}
