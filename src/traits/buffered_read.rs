use core::{
  cell::{Cell, RefCell, UnsafeCell},
  convert::Infallible,
};

use alloc::boxed::Box;

use thiserror::Error;

use crate::{ForkedBufferedReader, Read};

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ReadExactError<U> {
  #[error(
    "Unexpected EOF after reading {min_readable_bytes} bytes, attempted to read {bytes_requested} bytes"
  )]
  UnexpectedEof {
    bytes_requested: usize,
    /// At least this many bytes can still be read from the underlying reader.
    min_readable_bytes: usize,
  },
  #[error("Underlying read error: {0:?}")]
  Io(#[from] U),
}

/// An interface for buffered readers.
///
/// It allows forking and reading/peeking exact sized chunks from an underlying reader.
///
/// This is the equivalent of `std::io::BufReader`.
pub trait BufferedRead: Read {
  type UnderlyingReadExactError;
  type ForkedBufferedReaderImplementation<'a>: BufferedRead + ?Sized
  where
    Self: 'a;

  /// Creates a forked reader that can read from the same underlying data without consuming it.
  #[must_use]
  fn fork_reader(&mut self) -> Self::ForkedBufferedReaderImplementation<'_>;

  /// Consumes at most `maximum_byte_count` bytes from the underlying reader potentially avoiding a copy to the internal buffer.
  fn skip_buffered(
    &mut self,
    maximum_byte_count: usize,
  ) -> Result<usize, Self::UnderlyingReadExactError>;

  /// Efficiently utilizes the internal buffer to read bytes from the underlying reader.
  /// It reads at most `maximum_byte_count` bytes, but may read fewer if the buffer is smaller.
  fn read_buffered(
    &mut self,
    maximum_byte_count: usize,
  ) -> Result<&[u8], Self::UnderlyingReadExactError>;

  /// Efficiently utilizes the internal buffer to peek bytes from the underlying reader.
  /// It reads at most `maximum_byte_count` bytes, but may read fewer if the buffer is smaller.
  fn peek_buffered(
    &mut self,
    maximum_byte_count: usize,
  ) -> Result<&[u8], Self::UnderlyingReadExactError>;

  /// Consumes `byte_count` bytes from the underlying reader potentially avoiding a copy to the internal buffer.
  fn skip_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<(), ReadExactError<Self::UnderlyingReadExactError>>;

  /// Reads exactly `byte_count` bytes from the underlying reader consuming them.
  fn read_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<Self::UnderlyingReadExactError>>;

  /// Peeks exactly `byte_count` bytes from the underlying reader without consuming them.
  fn peek_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<Self::UnderlyingReadExactError>>;
}

// --- BufferedRead implementations for common smart pointer types ---

macro_rules! impl_buffered_read_for_wrapper {
  ( $( ($wrapper:ty, $accessor:ident) ),* ) => {
      $(
          impl<R: BufferedRead + ?Sized> BufferedRead for $wrapper {
              type UnderlyingReadExactError = R::UnderlyingReadExactError;
              type ForkedBufferedReaderImplementation<'a>
                  = ForkedBufferedReader<'a, Self>
              where
                  Self: 'a;

              fn fork_reader(&mut self) -> Self::ForkedBufferedReaderImplementation<'_> {
                  ForkedBufferedReader::new(self, 0)
              }

              fn skip_buffered(
                &mut self,
                maximum_byte_count: usize,
              ) -> Result<usize, Self::UnderlyingReadExactError> {
                  self.$accessor().skip_buffered(maximum_byte_count)
              }

              fn read_buffered(
                &mut self,
                maximum_byte_count: usize,
              ) -> Result<&[u8], Self::UnderlyingReadExactError> {
                  self.$accessor().read_buffered(maximum_byte_count)
              }

              fn peek_buffered(
                &mut self,
                maximum_byte_count: usize,
              ) -> Result<&[u8], Self::UnderlyingReadExactError> {
                  self.$accessor().peek_buffered(maximum_byte_count)
              }

              fn skip_exact(
                &mut self,
                byte_count: usize,
              ) -> Result<(), ReadExactError<Self::UnderlyingReadExactError>> {
                  self.$accessor().skip_exact(byte_count)
              }

              fn read_exact(
                  &mut self,
                  byte_count: usize,
              ) -> Result<&[u8], ReadExactError<Self::UnderlyingReadExactError>> {
                  self.$accessor().read_exact(byte_count)
              }

              fn peek_exact(
                  &mut self,
                  byte_count: usize,
              ) -> Result<&[u8], ReadExactError<Self::UnderlyingReadExactError>> {
                  self.$accessor().peek_exact(byte_count)
              }
          }
      )*
  };
}

impl_buffered_read_for_wrapper!(
  (Box<R>, as_mut),
  (RefCell<R>, get_mut),
  (Cell<R>, get_mut),
  (UnsafeCell<R>, get_mut)
);

// --- BufferedRead implementations for slice types ---

impl BufferedRead for &[u8] {
  type UnderlyingReadExactError = Infallible;
  type ForkedBufferedReaderImplementation<'a>
    = ForkedBufferedReader<'a, Self>
  where
    Self: 'a;

  fn fork_reader(&mut self) -> Self::ForkedBufferedReaderImplementation<'_> {
    ForkedBufferedReader::new(self, 0)
  }

  fn skip_buffered(
    &mut self,
    maximum_byte_count: usize,
  ) -> Result<usize, Self::UnderlyingReadExactError> {
    let bytes_to_skip = self.len().min(maximum_byte_count);
    *self = &self[bytes_to_skip..];
    Ok(bytes_to_skip)
  }

  fn read_buffered(
    &mut self,
    maximum_byte_count: usize,
  ) -> Result<&[u8], Self::UnderlyingReadExactError> {
    let byte_count = self.len().min(maximum_byte_count);
    let bytes = &self[..byte_count];
    *self = &self[byte_count..];
    Ok(bytes)
  }

  fn peek_buffered(
    &mut self,
    maximum_byte_count: usize,
  ) -> Result<&[u8], Self::UnderlyingReadExactError> {
    let byte_count = self.len().min(maximum_byte_count);
    Ok(&self[..byte_count])
  }

  fn skip_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<(), ReadExactError<Self::UnderlyingReadExactError>> {
    self.read_exact(byte_count)?;
    Ok(())
  }

  fn read_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<Self::UnderlyingReadExactError>> {
    if byte_count > self.len() {
      return Err(ReadExactError::UnexpectedEof {
        bytes_requested: byte_count,
        min_readable_bytes: self.len(),
      });
    }
    let bytes = &self[..byte_count];
    *self = &self[byte_count..];
    Ok(bytes)
  }

  fn peek_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<Self::UnderlyingReadExactError>> {
    if byte_count > self.len() {
      return Err(ReadExactError::UnexpectedEof {
        bytes_requested: byte_count,
        min_readable_bytes: self.len(),
      });
    }
    Ok(&self[..byte_count])
  }
}

// --- BufferedReadExt trait ---

pub struct BufferedReadByteIterator<'a, R: BufferedRead + ?Sized> {
  buffered_read: &'a mut R,
}

impl<'a, R: BufferedRead + ?Sized> Iterator for BufferedReadByteIterator<'a, R> {
  type Item = Result<u8, ReadExactError<R::UnderlyingReadExactError>>;

  fn next(&mut self) -> Option<Self::Item> {
    match self.buffered_read.read_exact(1) {
      Ok(bytes) if !bytes.is_empty() => Some(Ok(bytes[0])),
      Ok(_) => None, // EOF reached
      Err(e) => Some(Err(e)),
    }
  }
}

pub trait BufferedReadExt: BufferedRead {
  fn bytes(&mut self) -> BufferedReadByteIterator<'_, Self> {
    BufferedReadByteIterator {
      buffered_read: self,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_buffered_read_slice() {
    let reader_data = [1, 2, 3, 4, 5];
    let mut reader = &reader_data[..];
    let bytes_read = reader.read_exact(1).unwrap();
    assert_eq!(bytes_read, [1]);
    let bytes_read = reader.read_exact(2).unwrap();
    assert_eq!(bytes_read, [2, 3]);
  }
}
