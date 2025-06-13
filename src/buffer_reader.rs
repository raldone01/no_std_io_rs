use crate::no_std_io::{IoError, Read};

pub struct BufferReader<'a> {
  source: &'a [u8],
  position: usize,
}

impl<'a> BufferReader<'a> {
  #[must_use]
  pub fn new(source: &'a [u8]) -> Self {
    Self {
      source,
      position: 0,
    }
  }
}

impl<'a> Read for BufferReader<'a> {
  fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
    if self.position >= self.source.len() {
      return Ok(0); // No more data to read
    }

    // Determine the number of bytes available in the source from the current position.
    let remaining_in_source = self.source.len() - self.position;

    // Determine the number of bytes to read. It's the minimum of the buffer's
    // capacity and the number of bytes remaining in our source.
    let bytes_to_read = core::cmp::min(buf.len(), remaining_in_source);

    // Get the part of the source slice we are going to copy from.
    let source_slice = &self.source[self.position..self.position + bytes_to_read];

    // Get the part of the destination buffer we are going to copy into.
    let dest_slice = &mut buf[..bytes_to_read];

    // Copy the bytes.
    dest_slice.copy_from_slice(source_slice);

    // Advance our position in the source.
    self.position += bytes_to_read;

    // Return the number of bytes that were read.
    Ok(bytes_to_read)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn buffer_reader_reads_correctly() {
    let data = b"abcdef";
    let mut reader = BufferReader::new(data);

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
