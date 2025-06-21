use thiserror::Error;

use crate::Read;

/// A reader that only reads up to a specified limit.
/// This is useful when handling user input to prevent resource exhaustion attacks.
///
/// This is the equivalent of `std::io::Read::take`.
pub struct LimitedReader<R: Read> {
  source_reader: R,
  read_limit_bytes: usize,
  bytes_read: usize,
}

impl<R: Read> LimitedReader<R> {
  /// Creates a new `LimitedReader` with the specified limit.
  #[must_use]
  pub fn new(source_reader: R, read_limit_bytes: usize) -> Self {
    Self {
      source_reader,
      read_limit_bytes,
      bytes_read: 0,
    }
  }

  /// Returns the number of bytes read so far.
  #[must_use]
  pub fn bytes_read(&self) -> usize {
    self.bytes_read
  }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum LimitedReaderReadError<U> {
  #[error("Read limit of {0} bytes exceeded")]
  ReadLimitExceeded(usize),
  #[error("Underlying read error: {0}")]
  UnderlyingReadError(#[from] U),
}

impl<R: Read> Read for LimitedReader<R> {
  type ReadError = LimitedReaderReadError<R::ReadError>;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    if self.bytes_read >= self.read_limit_bytes {
      return Err(LimitedReaderReadError::ReadLimitExceeded(
        self.read_limit_bytes,
      ));
    }

    let remaining_limit = self.read_limit_bytes - self.bytes_read;
    let bytes_to_read = output_buffer.len().min(remaining_limit);

    let bytes_read = self
      .source_reader
      .read(&mut output_buffer[..bytes_to_read])?;

    self.bytes_read += bytes_read;
    Ok(bytes_read)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::Cursor;

  #[test]
  fn test_limited_reader() {
    let data = b"Rust programming language";
    let mut slice_reader = Cursor::new(data);
    let mut reader = LimitedReader::new(&mut slice_reader, 5);

    let mut buf = [0u8; 20];

    // First read
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 5);
    assert_eq!(&buf[..n], b"Rust ");

    // Second read should exceed the limit
    assert!(reader.read(&mut buf).is_err());
  }
}
