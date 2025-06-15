use core::convert::Infallible;

use crate::no_std_io::Write;

/// A writer that writes data to a byte slice.
pub struct SliceWriter<'a> {
  target: &'a mut [u8],
  position: usize,
}

impl<'a> SliceWriter<'a> {
  #[must_use]
  pub fn new(target: &'a mut [u8]) -> Self {
    Self {
      target,
      position: 0,
    }
  }

  /// Returns the remaining capacity of the slice.
  #[must_use]
  pub fn remaining_capacity(&self) -> usize {
    self.target.len() - self.position
  }

  #[must_use]
  pub fn as_mut_slice(&mut self) -> &mut [u8] {
    self.target
  }
  #[must_use]
  pub fn as_slice(&self) -> &[u8] {
    self.target
  }
}

impl Write for SliceWriter<'_> {
  type WriteError = Infallible;
  type FlushError = Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    if input_buffer.is_empty() {
      return Ok(0);
    }

    let bytes_to_write = core::cmp::min(input_buffer.len(), self.remaining_capacity());
    self.target[self.position..self.position + bytes_to_write]
      .copy_from_slice(&input_buffer[..bytes_to_write]);
    self.position += bytes_to_write;
    Ok(bytes_to_write)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_slice_writer_writes_correctly() {
    let mut data = [0u8; 6];
    let mut writer = SliceWriter::new(&mut data);

    // First write
    let n = writer.write(b"abc", false).unwrap();
    assert_eq!(n, 3);
    assert_eq!(&writer.as_slice()[..n], b"abc");

    // Second write
    let n = writer.write(b"def", false).unwrap();
    assert_eq!(n, 3);
    assert_eq!(&writer.as_slice(), b"abcdef");

    // Third write (should not write anything)
    let n = writer.write(b"oof", false).unwrap();
    assert_eq!(n, 0);
  }
}
