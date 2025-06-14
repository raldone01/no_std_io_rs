use core::convert::Infallible;

use crate::no_std_io::Write;

/// A `NullWriter` that discards all written data.
#[derive(Default)]
pub struct NullWriter;

impl NullWriter {
  /// Creates a new `NullWriter`.
  #[must_use]
  pub fn new() -> Self {
    Self
  }
}

impl Write for NullWriter {
  type WriteError = Infallible;
  type FlushError = Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    Ok(input_buffer.len())
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::no_std_io::WriteAll as _;

  #[test]
  fn test_null_writer() {
    let mut writer = NullWriter::new();
    let data = b"Hello, World!";
    writer.write_all(data, false).unwrap();
    assert_eq!(writer.flush(), Ok(()));
  }
}
