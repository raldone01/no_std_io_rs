use alloc::vec::Vec;

use crate::no_std_io::{IoError, Write};

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

impl Write for BufferWriter {
  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, IoError> {
    let available = self.max_buffer_size.saturating_sub(self.target.len());
    if available == 0 {
      return Err(IoError::MemoryLimitExceeded);
    }

    let n = core::cmp::min(input_buffer.len(), available);
    self.target.extend_from_slice(&input_buffer[..n]);
    Ok(n)
  }

  fn flush(&mut self) -> Result<(), IoError> {
    // No-op for in-memory buffer.
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn buffer_writer_simple_with_limit_test() {
    let mut writer = BufferWriter::new(10);

    // Write within limit
    let data1 = b"hello";
    writer.write(data1, false).expect("Write should succeed");
    assert_eq!(writer.as_slice(), b"hello");

    // Write more within remaining space
    let data2 = b"123";
    writer.write(data2, false).expect("Write should succeed");
    assert_eq!(writer.as_slice(), b"hello123");

    // Attempt to exceed limit
    let data3 = b"abcd"; // would exceed 10 bytes
    let result = writer.write(data3, false);
    assert!(matches!(result, Err(IoError::MemoryLimitExceeded)));
  }
}
