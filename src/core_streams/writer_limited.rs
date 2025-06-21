use thiserror::Error;

use crate::Write;

/// A writer that only writes up to a specified limit.
/// This is useful when handling user input to prevent resource exhaustion attacks.
pub struct LimitedWriter<W: Write> {
  source_writer: W,
  write_limit_bytes: usize,
  bytes_written: usize,
}

impl<W: Write> LimitedWriter<W> {
  /// Creates a new `LimitedWriter` with the specified limit.
  #[must_use]
  pub fn new(source_writer: W, write_limit_bytes: usize) -> Self {
    Self {
      source_writer,
      write_limit_bytes,
      bytes_written: 0,
    }
  }

  /// Returns the number of bytes written so far.
  #[must_use]
  pub fn bytes_written(&self) -> usize {
    self.bytes_written
  }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum LimitedWriterWriteError<U> {
  #[error("Write limit of {0} bytes exceeded")]
  WriteLimitExceeded(usize),
  #[error("Underlying write error: {0}")]
  UnderlyingWriteError(#[from] U),
}

impl<W: Write> Write for LimitedWriter<W> {
  type WriteError = LimitedWriterWriteError<W::WriteError>;
  type FlushError = W::FlushError;

  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    if self.bytes_written >= self.write_limit_bytes {
      return Err(LimitedWriterWriteError::WriteLimitExceeded(
        self.write_limit_bytes,
      ));
    }

    let remaining_limit = self.write_limit_bytes - self.bytes_written;
    let bytes_to_write = input_buffer.len().min(remaining_limit);

    let bytes_written = self
      .source_writer
      .write(&input_buffer[..bytes_to_write], sync_hint)?;

    self.bytes_written += bytes_written;
    Ok(bytes_written)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    self.source_writer.flush()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::{Cursor, WriteAll as _, WriteAllError};

  #[test]
  fn test_limited_writer() {
    let data = b"HelloWorld!";
    let mut buffer_writer = Cursor::new([0; 100]);
    let mut limited_writer = LimitedWriter::new(&mut buffer_writer, 10);

    let write_result = limited_writer.write_all(data, false);
    assert!(matches!(
      write_result,
      Err(WriteAllError::Io(
        LimitedWriterWriteError::WriteLimitExceeded(10)
      ))
    ));
    let written_data = buffer_writer.before();
    assert_eq!(written_data, b"HelloWorld");
  }
}
