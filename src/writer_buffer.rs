use alloc::vec::Vec;
use thiserror::Error;

use crate::no_std_io::Write;

/// A writer that writes data to an in-memory buffer with a maximum size limit.
pub struct BufferWriter {
  target: Vec<u8>,
  max_buffer_size: usize,
}

impl BufferWriter {
  #[must_use]
  pub fn new(max_buffer_size: usize) -> Self {
    Self {
      target: Vec::new(),
      max_buffer_size,
    }
  }

  /// Get a reference to the internal buffer.
  #[must_use]
  pub fn as_slice(&self) -> &[u8] {
    &self.target
  }

  /// Get the current internal buffer.
  #[must_use]
  pub fn to_vec(self) -> Vec<u8> {
    self.target
  }

  #[must_use]
  pub fn len(&self) -> usize {
    self.target.len()
  }
}

#[derive(Error, Debug)]
pub enum BufferWriterWriteError {
  #[error("Memory limit exceeded for writer buffer")]
  MemoryLimitExceeded,
}

impl Write for BufferWriter {
  type WriteError = BufferWriterWriteError;
  type FlushError = core::convert::Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    let available = self.max_buffer_size.saturating_sub(self.target.len());
    if available == 0 {
      return Err(Self::WriteError::MemoryLimitExceeded);
    }

    let n = core::cmp::min(input_buffer.len(), available);
    self.target.extend_from_slice(&input_buffer[..n]);
    Ok(n)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    // No-op for in-memory buffer.
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_buffer_writer_respects_limit() {
    let mut writer = BufferWriter::new(5);
    let data = b"hello world";

    // Should write only up to the limit
    let written = writer.write(data, false).unwrap();
    assert_eq!(written, 5);
    assert_eq!(writer.as_slice(), b"hello");

    // Further writes should return MemoryLimitExceeded
    let err = writer.write(b"!", false).unwrap_err();
    assert!(matches!(err, BufferWriterWriteError::MemoryLimitExceeded));
  }

  #[test]
  fn test_buffer_writer_to_vec_and_len() {
    let data = b"abc";
    let mut writer = BufferWriter::new(10);
    writer.write(data, false).unwrap();
    assert_eq!(writer.len(), 3);
    assert_eq!(writer.to_vec(), data);
  }
}
