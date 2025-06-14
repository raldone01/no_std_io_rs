use core::convert::Infallible;

use crate::no_std_io::Read;

/// A reader that reads data from a byte slice.
pub struct SliceReader<'a> {
  source: &'a [u8],
}

impl<'a> SliceReader<'a> {
  #[must_use]
  pub fn new(source: &'a [u8]) -> Self {
    Self { source }
  }
}

impl<'a> Read for SliceReader<'a> {
  type ReadError = Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    let n = core::cmp::min(output_buffer.len(), self.source.len());
    output_buffer[..n].copy_from_slice(&self.source[..n]);
    self.source = &self.source[n..];
    Ok(n)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_slice_reader_reads_correctly() {
    let data = b"abcdef";
    let mut reader = SliceReader::new(data);

    let mut buf = [0u8; 3];

    // First read
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 3);
    assert_eq!(&buf, b"abc");

    // Second read
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 3);
    assert_eq!(&buf, b"def");

    // Third read (should be EOF)
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 0);
  }
}
