use core::convert::Infallible;

use crate::{Read, Write};

/// [`EmptyStream`] ignores any data written via [`Write`], and will always be empty (returning zero bytes) when read via [`Read`].
///
/// This is the equivalent of `std::io::sink()`, `std::io::empty()`, `std::io::Empty` and `/dev/null`.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct EmptyStream;

impl EmptyStream {
  #[must_use]
  pub fn new() -> Self {
    Self
  }
}

impl Write for EmptyStream {
  type WriteError = Infallible;
  type FlushError = Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    Ok(input_buffer.len())
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

impl Read for EmptyStream {
  type ReadError = Infallible;

  fn read(&mut self, _output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    Ok(0)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::WriteAll as _;

  #[test]
  fn test_empty_write() {
    let mut writer = EmptyStream::new();
    let data = b"Hello, World!";
    writer.write_all(data, false).unwrap();
    assert_eq!(writer.flush(), Ok(()));
  }

  #[test]
  fn test_empty_read() {
    let mut reader = EmptyStream::new();
    let mut buffer = [0u8; 10];
    let bytes_read = reader.read(&mut buffer).unwrap();
    assert_eq!(bytes_read, 0);
    assert_eq!(&buffer, &[0; 10]);
  }
}
