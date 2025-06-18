use thiserror::Error;

use crate::no_std_io::{
  BackingBuffer, BufferedRead, ForkedBufferedReader, Read, ReadExactError, ResizeError,
};

/// A buffered reader can be used to add buffering to any reader.
///
/// To be generic over any buffered reader implementation, consider being generic over the [`BufferedRead`](crate::no_std_io::BufferedRead) trait instead.
pub struct BufferedReader<R: Read, B: BackingBuffer> {
  source_reader: R,
  buffer: B,
  last_user_read: usize,
  bytes_in_buffer: usize,
  read_chunk_size: usize,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum BufferedReaderReadError<U, RU> {
  #[error("Failed to resize the internal buffer to fit the requested exact read size: {0}")]
  ResizeError(ResizeError<RU>),
  #[error("Underlying read error: {0:?}")]
  Io(#[from] U),
}

impl<R: Read, B: BackingBuffer> BufferedReader<R, B> {
  /// Creates a new buffered reader with the given source and buffer.
  #[must_use]
  pub fn new(source: R, internal_buffer: B, read_chunk_size: usize) -> Self {
    Self {
      source_reader: source,
      buffer: internal_buffer,
      last_user_read: 0,
      bytes_in_buffer: 0,
      read_chunk_size,
    }
  }

  fn read_exact_internal(
    &mut self,
    byte_count: usize,
    skip: bool, // TODO: Use this parameter to skip copying data to the internal buffer.
    peek: bool,
  ) -> Result<&[u8], ReadExactError<BufferedReaderReadError<R::ReadError, B::ResizeError>>> {
    if byte_count == 0 {
      // If the user requests 0 bytes, we return an empty slice.
      return Ok(&[]);
    }

    if byte_count > self.buffer.as_ref().len() {
      // If the buffer is smaller than the requested size, we need to grow it.
      // If we grow it, we grow it to at least the read_chunk_size.
      let grow_target = byte_count.div_ceil(self.read_chunk_size).max(1) * self.read_chunk_size;
      let resize_result = self.buffer.try_resize(grow_target);
      if let Err(ResizeError {
        size_after_resize,
        resize_error,
      }) = resize_result
      {
        if size_after_resize < byte_count {
          return Err(ReadExactError::Io(BufferedReaderReadError::ResizeError(
            ResizeError {
              size_after_resize,
              resize_error,
            },
          )));
        }
      }
    }

    // Move the remaining bytes in the buffer to the front.
    self
      .buffer
      .as_mut()
      .copy_within(self.last_user_read..self.bytes_in_buffer, 0);
    self.bytes_in_buffer -= self.last_user_read;
    self.last_user_read = 0;

    // If the buffer is smaller than the requested size, we need to fill it.
    while self.bytes_in_buffer < byte_count {
      // Read more data into the buffer.
      let bytes_read = self
        .source_reader
        .read(&mut self.buffer.as_mut()[self.bytes_in_buffer..])
        .map_err(|e| ReadExactError::Io(BufferedReaderReadError::Io(e)))?;
      self.bytes_in_buffer += bytes_read;
      if bytes_read == 0 {
        // If we read 0 bytes, it means the source is exhausted but the user requested more data.
        return Err(ReadExactError::UnexpectedEof {
          bytes_requested: byte_count,
          min_readable_bytes: self.bytes_in_buffer,
        });
      }
    }

    // Now we have enough data in the buffer, return the requested slice.
    if !peek {
      self.last_user_read = byte_count;
    }
    let result = &self.buffer.as_mut()[..byte_count];
    Ok(result)
  }
}

impl<R: Read, B: BackingBuffer> Read for BufferedReader<R, B> {
  type ReadError = BufferedReaderReadError<R::ReadError, B::ResizeError>;

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
        .source_reader
        .read(&mut output_buffer[bytes_read_from_internal_buffer..])?;
      return Ok(bytes_read_from_internal_buffer + additional_bytes);
    }

    // To avoid tiny reads, we use the read_exact method to fill the rest of the buffer.
    let additional_bytes = match self.read_exact(remaining_bytes) {
      Ok(bytes) => bytes,
      Err(ReadExactError::UnexpectedEof {
        bytes_requested: _,
        min_readable_bytes: _,
      }) => {
        // If we reach here, it means we tried to read more data than was available.
        // This is an error condition for read_exact, but here we can return what we got.
        self
          .read_exact(self.bytes_in_buffer)
          .unwrap_or_else(|_| panic!("Failed to read internal buffer. This is a bug!"))
      },
      Err(ReadExactError::Io(e)) => return Err(e),
    };
    output_buffer[bytes_read_from_internal_buffer..].copy_from_slice(additional_bytes);
    Ok(bytes_read_from_internal_buffer + additional_bytes.len())
  }
}

impl<R: Read, B: BackingBuffer> BufferedRead for BufferedReader<R, B> {
  type BackingImplementation = Self;

  fn fork_reader(&mut self) -> ForkedBufferedReader<'_, Self::BackingImplementation> {
    ForkedBufferedReader::new(self, 0)
  }

  fn skip(
    &mut self,
    byte_count: usize,
  ) -> Result<(), ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    self
      .read_exact_internal(byte_count, true, false)
      .map(|_| ())
  }

  fn buffer_size_hint(&self) -> usize {
    self.buffer.as_ref().len()
  }

  fn read_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    self.read_exact_internal(byte_count, false, false)
  }

  fn peek_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    self.read_exact_internal(byte_count, false, true)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::no_std_io::{BytewiseReader, Cursor, FixedSizeBufferError};

  #[test]
  fn test_buffered_reader_exact_correct() {
    let source_data = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
    let mut slice_reader = Cursor::new(&source_data);
    const MAX_BUFFER_SIZE: usize = 4;
    let mut backing_buffer = [0; MAX_BUFFER_SIZE];
    let mut reader = BufferedReader::new(&mut slice_reader, &mut backing_buffer, 1);

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
    assert_eq!(
      reader.read_exact(5).unwrap_err(),
      ReadExactError::Io(BufferedReaderReadError::ResizeError(ResizeError {
        size_after_resize: MAX_BUFFER_SIZE,
        resize_error: FixedSizeBufferError {
          fixed_buffer_size: MAX_BUFFER_SIZE,
          requested_size: 5,
        }
      }))
    );

    // Test UnexpectedEof error
    assert_eq!(
      reader.read_exact(1).unwrap_err(),
      ReadExactError::UnexpectedEof {
        bytes_requested: 1,
        min_readable_bytes: 0,
      }
    );
  }

  #[test]
  fn test_buffered_reader_exact_correct_bytewise() {
    let source_data = b"Hello, world!";
    let mut slice_reader = Cursor::new(source_data);
    let mut bytewise_reader = BytewiseReader::new(&mut slice_reader);
    const MAX_BUFFER_SIZE: usize = 10;
    let mut backing_buffer = [0; MAX_BUFFER_SIZE];
    let mut buffered_reader = BufferedReader::new(&mut bytewise_reader, &mut backing_buffer, 1);
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
      ReadExactError::UnexpectedEof {
        bytes_requested: 1,
        min_readable_bytes: 0
      }
    ));
  }

  #[test]
  fn test_buffered_reader_as_reader() {
    let source_data = b"Hello, world!";
    let mut slice_reader = Cursor::new(source_data);
    const MAX_BUFFER_SIZE: usize = 10;
    let mut backing_buffer = [0; MAX_BUFFER_SIZE];
    let mut buffered_reader = BufferedReader::new(&mut slice_reader, &mut backing_buffer, 1);

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

  #[test]
  fn test_forked_buffered_reader() {
    let source_data = b"Hello, world!";
    let mut slice_reader = Cursor::new(source_data);
    const MAX_BUFFER_SIZE: usize = 12;
    let mut backing_buffer = [0; MAX_BUFFER_SIZE];
    let mut buffered_reader = BufferedReader::new(&mut slice_reader, &mut backing_buffer, 1);

    let mut forked_reader = buffered_reader.fork_reader();

    // Read the first 5 bytes
    assert_eq!(forked_reader.read_exact(5).unwrap(), b"Hello");

    let mut forked_forked_reader = forked_reader.fork_reader();
    // Peek the next 7 bytes without consuming them
    assert_eq!(forked_forked_reader.peek_exact(7).unwrap(), b", world");

    // Check that a forked reader works as a regular reader
    let mut output_buffer = [0; 7];
    let bytes_read = forked_forked_reader.read(&mut output_buffer).unwrap();
    assert_eq!(&output_buffer[..bytes_read], b", world");
    assert_eq!(bytes_read, 7);

    // Peek the next 7 bytes without consuming them
    assert_eq!(forked_reader.peek_exact(7).unwrap(), b", world");

    // Read the next 7 bytes
    assert_eq!(forked_reader.read_exact(7).unwrap(), b", world");

    // Check that out of memory error is handled correctly
    assert_eq!(
      forked_reader.read_exact(1).unwrap_err(),
      ReadExactError::Io(BufferedReaderReadError::ResizeError(ResizeError {
        size_after_resize: MAX_BUFFER_SIZE,
        resize_error: FixedSizeBufferError {
          fixed_buffer_size: MAX_BUFFER_SIZE,
          requested_size: MAX_BUFFER_SIZE + 1,
        },
      }))
    );

    // Check that we can still read from the original buffered reader
    assert_eq!(buffered_reader.read_exact(2).unwrap(), b"He");
  }
}
