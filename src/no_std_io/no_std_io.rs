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

/// Extension trait that provides a `write_all` method for any `Write` implementer.
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
impl<W: Write + ?Sized> WriteAll for W {
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

#[derive(Error, Debug)]
pub enum ReadAllError<U> {
  #[error("Unexpected EOF while reading {bytes_requested} bytes, only {bytes_read} bytes read")]
  UnexpectedEof {
    bytes_requested: usize,
    bytes_read: usize,
  },
  #[error("Underlying read error: {0:?}")]
  Io(#[from] U),
}

/// Extension trait that provides a `read_all` method for any `Read` implementer.
pub trait ReadAll: Read {
  /// Reads the entire buffer, retrying partial reads.
  fn read_all(&mut self, output_buffer: &mut [u8]) -> Result<(), ReadAllError<Self::ReadError>>;
}

/// Blanket implementation for all `Read` implementors.
impl<R: Read + ?Sized> ReadAll for R {
  fn read_all(&mut self, output_buffer: &mut [u8]) -> Result<(), ReadAllError<Self::ReadError>> {
    let requested_bytes = output_buffer.len();
    let mut buf = output_buffer;
    let mut total_read = 0;

    while !buf.is_empty() {
      match self.read(buf) {
        Ok(0) => {
          return Err(ReadAllError::UnexpectedEof {
            bytes_requested: requested_bytes,
            bytes_read: total_read,
          });
        },
        Ok(n) => {
          total_read += n;
          buf = &mut buf[n..]; // advance buffer
        },
        Err(e) => return Err(ReadAllError::Io(e)),
      }
    }
    Ok(())
  }
}

/// Reads all bytes from the `reader` and writes them to the `writer`.
///
/// This function continues until the reader returns 0 (EOF) or an error occurs.
/// Data is written using `write_all`, preserving sync hints for each write.
///
/// Note: When a reader or writer is buffered, consider using `[copy_to]` or `[copy_from]` or if both are `[copy_buffered]` instead for more efficiency.
///
/// Returns the total number of bytes copied.
pub fn copy<R: Read, W: Write>(
  reader: &mut R,
  writer: &mut W,
  transfer_buffer: &mut [u8],
  sync_hint: bool,
) -> Result<usize, CopyError<R::ReadError, W::WriteError>> {
  let mut total_bytes = 0;

  loop {
    let bytes_read = reader.read(transfer_buffer).map_err(CopyError::Read)?;
    if bytes_read == 0 {
      break; // EOF
    }

    writer
      .write_all(&transfer_buffer[..bytes_read], sync_hint)
      .map_err(CopyError::Write)?;

    total_bytes += bytes_read;
  }

  Ok(total_bytes)
}

/// Error type returned by the `pipe` function.
#[derive(Debug, Error)]
pub enum CopyError<RE, WE> {
  #[error("Read error: {0:?}")]
  Read(RE),
  #[error("Write error: {0:?}")]
  Write(WriteAllError<WE>),
}
