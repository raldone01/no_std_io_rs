use core::panic;

use alloc::vec::Vec;

use thiserror::Error;

use crate::no_std_io::Read;

/// A buffered reader that allows pulling exact sized chunks from an underlying reader.
pub struct BufferedReader<'a, R: Read> {
  source: &'a mut R,
  buffer: Vec<u8>,
  last_user_read: usize,
  bytes_in_buffer: usize,
  max_buffer_size: usize,
  read_chunk_size: usize,
}

#[derive(Error, Debug)]
pub enum BufferedReaderReadError<U> {
  #[error("Unexpected EOF while reading")]
  UnexpectedEof,
  #[error("Memory limit of {0} bytes exceeded for exact read")]
  MemoryLimitExceeded(usize),
  #[error("Underlying read error: {0:?}")]
  Io(#[from] U),
}

impl<'a, R: Read> BufferedReader<'a, R> {
  #[must_use]
  pub fn new(source: &'a mut R, max_buffer_size: usize, read_chunk_size: usize) -> Self {
    Self {
      source,
      buffer: Vec::new(),
      last_user_read: 0,
      bytes_in_buffer: 0,
      max_buffer_size,
      read_chunk_size,
    }
  }

  fn read_exact_internal(
    &mut self,
    byte_count: usize,
    peek: bool,
  ) -> Result<&[u8], BufferedReaderReadError<R::ReadError>> {
    if byte_count == 0 {
      // If the user requests 0 bytes, we return an empty slice.
      return Ok(&[]);
    }
    if byte_count > self.max_buffer_size {
      return Err(BufferedReaderReadError::MemoryLimitExceeded(
        self.max_buffer_size,
      ));
    }

    if byte_count > self.buffer.len() {
      // If the buffer is smaller than the requested size, we need to grow it.
      // If we grow it, we grow it to at least the read_chunk_size.

      // Technically, the buffer could exceed the `max_buffer_size` here due to rounding to the nearest
      // `read_chunk_size`, but we allow that here.
      self.buffer.resize(
        byte_count.div_ceil(self.read_chunk_size).max(1) * self.read_chunk_size,
        0,
      );
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
        return Err(BufferedReaderReadError::UnexpectedEof);
      }
      self.bytes_in_buffer += bytes_read;
    }

    // Now we have enough data in the buffer, return the requested slice.
    if !peek {
      self.last_user_read = byte_count;
    }
    let result = &self.buffer[..byte_count];
    Ok(result)
  }

  /// Reads exactly `byte_count` bytes from the underlying reader consuming them.
  pub fn read_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], BufferedReaderReadError<R::ReadError>> {
    self.read_exact_internal(byte_count, false)
  }

  /// Peeks exactly `byte_count` bytes from the underlying reader without consuming them.
  pub fn peek_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], BufferedReaderReadError<R::ReadError>> {
    self.read_exact_internal(byte_count, true)
  }
}

impl<R: Read> Read for BufferedReader<'_, R> {
  type ReadError = R::ReadError;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    if output_buffer.is_empty() {
      return Ok(0);
    }

    // Read what is stored in the internal buffer first.
    let bytes_read_from_internal_buffer = output_buffer.len().min(self.bytes_in_buffer);
    let buffered_bytes = if bytes_read_from_internal_buffer != 0 {
      self
        .read_exact(bytes_read_from_internal_buffer)
        .unwrap_or_else(|_| panic!("Failed to read internal buffer. This is a bug!"))
    } else {
      // The unexpected data functionality is not wanted here
      &[]
    };
    output_buffer[..bytes_read_from_internal_buffer].copy_from_slice(buffered_bytes);

    // Check if the output_buffer is big enough to justify calling the source reader directly with it.
    let remaining_bytes = output_buffer.len() - bytes_read_from_internal_buffer;
    if remaining_bytes > self.read_chunk_size {
      let additional_bytes = self
        .source
        .read(&mut output_buffer[bytes_read_from_internal_buffer..])?;
      return Ok(bytes_read_from_internal_buffer + additional_bytes);
    }

    // To avoid tiny reads, we use the read_exact method to fill the rest of the buffer.
    let additional_bytes = match self.read_exact(remaining_bytes) {
      Ok(bytes) => bytes,
      Err(BufferedReaderReadError::MemoryLimitExceeded(max_buffer_size)) => {
        panic!(
          "Memory limit of {} bytes exceeded while using ExactReader as a Read. Is your max_buffer_size smaller than the read_chunk_size?",
          max_buffer_size
        );
      },
      Err(BufferedReaderReadError::UnexpectedEof) => {
        // If we reach here, it means we tried to read more data than was available.
        // This is an error condition for read_exact, but here we can return what we got.
        self
          .read_exact(self.bytes_in_buffer)
          .unwrap_or_else(|_| panic!("Failed to read internal buffer. This is a bug!"))
      },
      Err(BufferedReaderReadError::Io(e)) => return Err(e),
    };
    output_buffer[bytes_read_from_internal_buffer..].copy_from_slice(additional_bytes);
    Ok(bytes_read_from_internal_buffer + additional_bytes.len())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::no_std_io::{BytewiseReader, SliceReader};

  #[test]
  fn test_buffered_reader_exact_correct() {
    let source_data = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
    let mut slice_reader = SliceReader::new(&source_data);
    let max_buffer_size = 4;
    let mut reader = BufferedReader::new(&mut slice_reader, max_buffer_size, 1);

    // Read the first 3 bytes
    assert_eq!(reader.peek_exact(3).unwrap(), &[0, 1, 2]);
    assert_eq!(reader.read_exact(3).unwrap(), &[0, 1, 2]);

    // Read the next 4 bytes. The buffer should handle the internal offset.
    assert_eq!(reader.peek_exact(4).unwrap(), &[3, 4, 5, 6]);
    assert_eq!(reader.read_exact(4).unwrap(), &[3, 4, 5, 6]);

    // The remaining data in the source should be copied and returned.
    // don't peek here to test that too
    assert_eq!(reader.read_exact(3).unwrap(), &[7, 8, 9]);

    // Test MemoryLimitExceeded error
    assert!(matches!(
      reader.read_exact(5).unwrap_err(),
      BufferedReaderReadError::MemoryLimitExceeded(max_buffer_size)
    ));

    // Test UnexpectedEof error
    assert!(matches!(
      reader.read_exact(1).unwrap_err(),
      BufferedReaderReadError::UnexpectedEof
    ));
  }

  #[test]
  fn test_buffered_reader_exact_correct_bytewise() {
    let source_data = b"Hello, world!";
    let mut slice_reader = SliceReader::new(source_data);
    let mut bytewise_reader = BytewiseReader::new(&mut slice_reader);
    let mut buffered_reader = BufferedReader::new(&mut bytewise_reader, 10, 1);
    // Read 5 bytes
    let bytes_read = buffered_reader.peek_exact(5).unwrap();
    assert_eq!(bytes_read, b"Hello");
    let bytes_read = buffered_reader.read_exact(5).unwrap();
    assert_eq!(bytes_read, b"Hello");
    // Read another 5 bytes
    let bytes_read = buffered_reader.read_exact(5).unwrap();
    assert_eq!(bytes_read, b", wor");
    // Read the remaining bytes
    let bytes_read = buffered_reader.read_exact(3).unwrap();
    assert_eq!(bytes_read, b"ld!");

    // Check that eof is handled correctly
    assert!(matches!(
      buffered_reader.read_exact(1).unwrap_err(),
      BufferedReaderReadError::UnexpectedEof
    ));
  }

  #[test]
  fn test_buffered_reader_as_reader() {
    let source_data = b"Hello, world!";
    let mut slice_reader = SliceReader::new(source_data);
    let mut buffered_reader = BufferedReader::new(&mut slice_reader, 10, 1);

    let mut output_buffer = [0; 100];

    let read_buffer = &mut output_buffer[..5];
    let bytes_read = buffered_reader.read(read_buffer).unwrap();
    assert_eq!(&read_buffer, b"Hello");
    assert_eq!(bytes_read, 5);

    // Test peeking
    let peek = buffered_reader.peek_exact(8).unwrap();
    assert_eq!(peek, b", world!");

    // Read the next 8 bytes
    let read_buffer = &mut output_buffer[..8];
    let bytes_read = buffered_reader.read(read_buffer).unwrap();
    assert_eq!(&read_buffer, b", world!");
    assert_eq!(bytes_read, 8);
  }
}
