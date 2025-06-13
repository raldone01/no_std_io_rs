use crate::no_std_io::{IoError, Write};

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
  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<(), IoError> {
    for &byte in input_buffer[..input_buffer.len() - 1].iter() {
      self.target_writer.write(&[byte], false)?;
    }
    // write the last byte with the sync hint
    if !input_buffer.is_empty() {
      self
        .target_writer
        .write(&[input_buffer[input_buffer.len() - 1]], sync_hint)?;
    }
    Ok(())
  }

  fn flush(&mut self) -> Result<(), IoError> {
    self.target_writer.flush()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn bytewise_writer_simple() {}
}
