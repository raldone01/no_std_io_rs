use alloc::vec::Vec;

use thiserror::Error;

use crate::no_std_io::Read;

/// A buffered reader that allows pulling exact sized chunks from an underlying reader.
pub struct ExactReader<'a, R: Read> {
  source: &'a mut R,
  buffer: Vec<u8>,
  last_user_read: usize,
  bytes_in_buffer: usize,
  max_buffer_size: usize,
}

#[derive(Error, Debug)]
pub enum ReadExactError<U> {
  #[error("Unexpected EOF while reading")]
  UnexpectedEof,
  #[error("Expected EOF, but got more data")]
  UnexpectedData,
  #[error("Memory limit of {0} bytes exceeded for exact read")]
  MemoryLimitExceeded(usize),
  #[error("Underlying I/O error: {0:?}")]
  Io(#[from] U),
}

impl<'a, R: Read> ExactReader<'a, R> {
  #[must_use]
  pub fn new(max_buffer_size: usize, source: &'a mut R) -> Self {
    Self {
      source,
      buffer: Vec::new(),
      last_user_read: 0,
      bytes_in_buffer: 0,
      max_buffer_size,
    }
  }

  /// Reads exactly `byte_count` bytes from the underlying reader.
  pub fn read_exact(&mut self, byte_count: usize) -> Result<&[u8], ReadExactError<R::ReadError>> {
    if byte_count > self.max_buffer_size {
      return Err(ReadExactError::MemoryLimitExceeded(self.max_buffer_size));
    }
    if byte_count == 0 {
      let bytes_read = self.source.read(&mut [])?;
      if bytes_read != 0 {
        return Err(ReadExactError::UnexpectedData);
      }
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
        // If we read 0 bytes, it means the source is exhausted but the user requested more data.
        return Err(ReadExactError::UnexpectedEof);
      }
      self.bytes_in_buffer += bytes_read;
    }

    // Now we have enough data in the buffer, return the requested slice.
    self.last_user_read = byte_count;
    let result = &self.buffer[..byte_count];
    Ok(result)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::no_std_io::{BytewiseReader, SliceReader};

  #[test]
  fn test_buffered_reader_reads_correctly() {
    let source_data = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
    let mut slice_reader = SliceReader::new(&source_data);
    let max_buffer_size = 4;
    let mut reader = ExactReader::new(max_buffer_size, &mut slice_reader);

    // Read the first 3 bytes
    assert_eq!(reader.read_exact(3).unwrap(), &[0, 1, 2]);

    // Read the next 4 bytes. The buffer should handle the internal offset.
    assert_eq!(reader.read_exact(4).unwrap(), &[3, 4, 5, 6]);

    // The remaining data in the source should be copied and returned.
    assert_eq!(reader.read_exact(3).unwrap(), &[7, 8, 9]);

    // Test MemoryLimitExceeded error
    assert!(matches!(
      reader.read_exact(5).unwrap_err(),
      ReadExactError::MemoryLimitExceeded(max_buffer_size)
    ));

    // Test UnexpectedEof error
    assert!(matches!(
      reader.read_exact(1).unwrap_err(),
      ReadExactError::UnexpectedEof
    ));
  }

  #[test]
  fn test_buffered_reader_reads_correctly_bytewise() {
    let source_data = b"Hello, world!";
    let mut slice_reader = SliceReader::new(source_data);
    let mut bytewise_reader = BytewiseReader::new(&mut slice_reader);
    let mut buffered_reader = ExactReader::new(10, &mut bytewise_reader);
    // Read 5 bytes
    let bytes_read = buffered_reader.read_exact(5).unwrap();
    assert_eq!(bytes_read, b"Hello");
    // Read another 5 bytes
    let bytes_read = buffered_reader.read_exact(5).unwrap();
    assert_eq!(bytes_read, b", wor");
    // Read the remaining bytes
    let bytes_read = buffered_reader.read_exact(3).unwrap();
    assert_eq!(bytes_read, b"ld!");
  }
}
