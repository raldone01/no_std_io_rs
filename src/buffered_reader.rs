use alloc::vec::Vec;

use crate::no_std_io::{IoError, Read};

/// A buffered reader that allows pulling exact sized chunks from a reader.
pub struct BufferedReader<R: Read> {
  source: R,
  buffer: Vec<u8>,
  last_user_read: usize,
  bytes_in_buffer: usize,
  max_buffer_size: usize,
}

impl<R: Read> BufferedReader<R> {
  #[must_use]
  pub fn new(max_buffer_size: usize, source: R) -> Self {
    Self {
      source,
      buffer: Vec::new(),
      last_user_read: 0,
      bytes_in_buffer: 0,
      max_buffer_size,
    }
  }

  /// Reads exactly `byte_count` bytes from the reader.
  pub fn read_exact(&mut self, byte_count: usize) -> Result<&[u8], IoError> {
    if byte_count > self.max_buffer_size {
      return Err(IoError::MemoryLimitExceeded);
    }
    if byte_count == 0 {
      return Ok(&[]);
    }

    if byte_count > self.buffer.len() {
      // If the buffer is smaller than the requested size, we need to grow it.
      self.buffer.resize(byte_count, 0);
    }

    // Move the remaining bytes in the buffer to the front.
    self
      .buffer
      .copy_within(self.last_user_read..self.bytes_in_buffer, 0);
    self.bytes_in_buffer -= self.last_user_read;
    self.last_user_read = 0;

    // If the buffer is smaller than the requested size, we need to fill it.
    while self.bytes_in_buffer < byte_count {
      // Read more data into the buffer.
      let bytes_read = self.source.read(&mut self.buffer[self.bytes_in_buffer..])?;
      if bytes_read == 0 {
        // If we read 0 bytes, it means the source is exhausted.
        return Err(IoError::UnexpectedEof);
      }
      self.bytes_in_buffer += bytes_read;
    }

    // Now we have enough data in the buffer, return the requested slice.
    self.last_user_read = byte_count;
    let result = &self.buffer[..byte_count];
    Ok(result)
  }
}
