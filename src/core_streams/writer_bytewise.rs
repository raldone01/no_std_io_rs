use crate::Write;

/// A writer that writes data byte by byte, useful for testing.
pub struct BytewiseWriter<W: Write> {
  target_writer: W,
}

impl<W: Write> BytewiseWriter<W> {
  #[must_use]
  pub fn new(target_writer: W) -> Self {
    Self { target_writer }
  }
}

impl<W: Write> Write for BytewiseWriter<W> {
  type WriteError = W::WriteError;
  type FlushError = W::FlushError;

  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    let mut bytes_written = 0;
    for &byte in input_buffer[..input_buffer.len().saturating_sub(1)].iter() {
      bytes_written += self.target_writer.write(&[byte], false)?;
    }
    // write the last byte with the sync hint
    if !input_buffer.is_empty() {
      bytes_written += self
        .target_writer
        .write(&[input_buffer[input_buffer.len() - 1]], sync_hint)?;
    }
    Ok(bytes_written)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    self.target_writer.flush()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use alloc::vec::Vec;

  use crate::Cursor;

  #[test]
  fn test_bytewise_writer_writes_correctly() {
    let mut buffer_writer = Cursor::new(Vec::new());
    let mut writer = BytewiseWriter::new(&mut buffer_writer);

    // Input data to write
    let input = b"Rust";

    // Write the full buffer with sync_hint = true
    let bytes_written = writer.write(input, true).unwrap();
    assert_eq!(bytes_written, 4);

    // Flush should succeed
    assert!(writer.flush().is_ok());

    // Ensure all bytes were written correctly
    assert_eq!(buffer_writer.before(), b"Rust");
  }

  #[test]
  fn test_bytewise_writer_empty_input() {
    let mut buffer_writer = Cursor::new(Vec::new());
    let mut writer = BytewiseWriter::new(&mut buffer_writer);

    // Write empty buffer
    let bytes_written = writer.write(&[], true).unwrap();
    assert_eq!(bytes_written, 0);

    // Ensure nothing was written
    assert!(buffer_writer.before().is_empty());
  }
}
