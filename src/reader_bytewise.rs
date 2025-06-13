use crate::no_std_io::Read;

/// A reader that reads data byte by byte, useful for testing.
pub struct BytewiseReader<R: Read> {
  source_reader: R,
}

impl<R: Read> BytewiseReader<R> {
  #[must_use]
  pub fn new(source_reader: R) -> Self {
    Self { source_reader }
  }
}

impl<R: Read> Read for BytewiseReader<R> {
  type Error = R::Error;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::Error> {
    if output_buffer.is_empty() {
      return Ok(0); // Nothing to read into
    }

    // Read a single byte from the source reader
    let mut single_byte = [0u8; 1];
    let bytes_read = self.source_reader.read(&mut single_byte)?;

    if bytes_read == 0 {
      return Ok(0); // EOF
    }

    // Copy the single byte into the output buffer
    output_buffer[0] = single_byte[0];

    Ok(1) // Return the number of bytes read
  }
}

#[cfg(test)]
mod tests {
  use crate::reader_slice::SliceReader;

  use super::*;

  #[test]
  fn bytewise_reader_reads_correctly() {
    let data = b"Rust";
    let mut reader = BytewiseReader::new(SliceReader::new(data));

    // Create a buffer that is larger than 1 to prove it only reads a single byte.
    let mut buf = [0u8; 5];

    // First read
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 1);
    assert_eq!(buf[0], b'R');

    // Second read
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 1);
    assert_eq!(buf[0], b'u');

    // Third read
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 1);
    assert_eq!(buf[0], b's');

    // Fourth read
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 1);
    assert_eq!(buf[0], b't');

    // Fifth read (should be EOF)
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 0);

    // Subsequent reads should also be EOF
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(n, 0);
  }
}
