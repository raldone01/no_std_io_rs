use alloc::vec::Vec;

use crate::no_std_io::{IoError, Write};

pub struct BufferWriter {
  target: Vec<u8>,
  max_buffer_size: usize,
  position: usize,
}

impl BufferWriter {
  #[must_use]
  pub fn new(max_buffer_size: usize) -> Self {
    Self {
      target: Vec::new(),
      max_buffer_size,
      position: 0,
    }
  }

  /// Get a reference to the internal buffer.
  #[must_use]
  pub fn as_slice(&self) -> &[u8] {
    &self.target[..self.position]
  }

  /// Get the current internal buffer.
  #[must_use]
  pub fn to_vec(self) -> Vec<u8> {
    self.target
  }
}

impl Write for BufferWriter {
  fn write(&mut self, buf: &[u8], _sync_hint: bool) -> Result<(), IoError> {
    // Calculate new length after writing
    let new_len = self
      .position
      .checked_add(buf.len())
      .ok_or(IoError::MemoryLimitExceeded)?;

    // Check if new length exceeds max buffer size
    if new_len > self.max_buffer_size {
      return Err(IoError::MemoryLimitExceeded);
    }

    // Ensure the internal Vec has enough capacity
    if self.target.len() < new_len {
      let new_target_buffer_size = new_len.max(self.target.len() * 2).min(self.max_buffer_size);
      self.target.resize(new_target_buffer_size, 0);
    }

    // Copy buf into target at current position
    self.target[self.position..new_len].copy_from_slice(buf);

    // Update position to reflect bytes written
    self.position = new_len;

    Ok(())
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
  fn buffer_writer_writes_and_limits_buffer() {
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
