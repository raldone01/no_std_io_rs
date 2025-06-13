use alloc::boxed::Box;
use thiserror::Error;

/// Minimal I/O error type for reading operations.
#[derive(Error, Debug)]
pub enum IoError {
  #[error("Underlying I/O error")]
  Io(#[from] Box<dyn core::error::Error + Send + Sync>),
  #[error("Unexpected end of file while reading")]
  UnexpectedEof,
  #[error("Memory limit exceeded for buffered read")]
  MemoryLimitExceeded,
}

/// Trait for reading bytes
pub trait Read {
  /// Read up to `buf.len()` bytes into `buf`.
  ///
  /// Returns number of bytes read, or `Error::Io`.
  fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError>;
}

/// Trait for writing bytes
pub trait Write {
  /// Write the contents of `buf` to the underlying storage.
  ///
  /// Returns number of bytes written, or `Error::Io`.
  fn write(&mut self, buf: &[u8]) -> Result<usize, IoError>;

  /// Flush any buffered data to the underlying storage.
  /// Must be called at the end to ensure all data is written.
  fn flush(&mut self) -> Result<(), IoError>;
}
