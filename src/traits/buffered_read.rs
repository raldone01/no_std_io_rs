use thiserror::Error;

use crate::Read;

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

  /// Consumes `byte_count` bytes from the underlying reader potentially avoiding a copy to the internal buffer.
  fn skip(
    &mut self,
    byte_count: usize,
  ) -> Result<(), ReadExactError<Self::UnderlyingReadExactError>>;

  /// Returns the size of the internal buffer.
  #[must_use]
  fn buffer_size_hint(&self) -> usize;

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
