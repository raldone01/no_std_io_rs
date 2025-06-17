use core::convert::Infallible;

use crate::no_std_io::{
  BackingBufferMut, BufferedRead, CursorSeekError, ForkedBufferedReader, Read, ReadExactError,
  Seek, SeekFrom,
};

pub struct Cursor<'a, B: ?Sized> {
  backing_buffer: &'a B,
  position: usize,
}

impl<'a, B: ?Sized> Cursor<'a, B> {
  #[must_use]
  pub fn new(backing_buffer: &'a B) -> Self {
    Self {
      backing_buffer,
      position: 0,
    }
  }

  #[must_use]
  pub fn position(&self) -> usize {
    self.position
  }

  pub fn set_position(&mut self, position: usize) {
    self.position = position;
  }
}

impl<B: AsRef<[u8]> + ?Sized> Cursor<'_, B> {
  #[must_use]
  pub fn len(&self) -> usize {
    self.backing_buffer.as_ref().len()
  }

  #[must_use]
  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }

  #[must_use]
  pub fn remaining(&self) -> usize {
    self.len().saturating_sub(self.position)
  }
}

impl<B: BackingBufferMut + ?Sized> Cursor<'_, B> {
  #[must_use]
  pub fn backing_buffer(&self) -> &B {
    self.backing_buffer
  }
}

impl<B: AsRef<[u8]> + ?Sized> Cursor<'_, B> {
  pub fn split(&self) -> (&[u8], &[u8]) {
    let slice = self.backing_buffer.as_ref();
    let position = self.position.min(slice.len());
    slice.split_at(position)
  }
}

impl<B: AsRef<[u8]>> Seek for Cursor<'_, B> {
  type SeekError = CursorSeekError;

  fn seek(&mut self, style: SeekFrom) -> Result<usize, Self::SeekError> {
    let (base_pos, offset) = match style {
      SeekFrom::Start(n) => {
        self.position = n;

        return Ok(n);
      },

      SeekFrom::End(n) => (self.backing_buffer.as_ref().len() as usize, n),

      SeekFrom::Current(n) => (self.position, n),
    };

    match base_pos.checked_add_signed(offset) {
      Some(n) => {
        self.position = n;

        Ok(self.position)
      },

      None => Err(CursorSeekError::OutOfBounds {
        position: base_pos,
        length: self.backing_buffer.as_ref().len(),
        offset: style,
      }),
    }
  }
}

impl<B: AsRef<[u8]> + ?Sized> Read for Cursor<'_, B> {
  type ReadError = Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    let n = Read::read(&mut Cursor::split(self).1, output_buffer)?;
    self.position += n;
    Ok(n)
  }
}

impl<B: AsRef<[u8]> + ?Sized> Cursor<'_, B> {
  fn read_exact_internal(
    &mut self,
    byte_count: usize,
    peek: bool,
  ) -> Result<
    &[u8],
    ReadExactError<<<Self as BufferedRead>::BackingImplementation as Read>::ReadError>,
  > {
    let remaining_buffer =
      self
        .backing_buffer
        .as_ref()
        .get(self.position..)
        .ok_or(ReadExactError::UnexpectedEof {
          min_readable_bytes: self.remaining(),
          bytes_requested: byte_count,
        })?;

    if remaining_buffer.len() < byte_count {
      return Err(ReadExactError::UnexpectedEof {
        min_readable_bytes: self.remaining(),
        bytes_requested: byte_count,
      });
    }

    let sliced_buffer = &remaining_buffer[..byte_count];
    if !peek {
      self.position += byte_count;
    }
    Ok(sliced_buffer)
  }
}

impl<B: AsRef<[u8]> + ?Sized> BufferedRead for Cursor<'_, B> {
  type BackingImplementation = Self;

  fn fork_reader(&mut self) -> ForkedBufferedReader<'_, Self::BackingImplementation> {
    ForkedBufferedReader::new(self, 0)
  }

  fn skip(
    &mut self,
    byte_count: usize,
  ) -> Result<(), ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    self.position += byte_count;
    Ok(())
  }

  fn buffer_size_hint(&self) -> usize {
    self.split().1.len()
  }

  fn read_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    self.read_exact_internal(byte_count, false)
  }

  fn peek_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    self.read_exact_internal(byte_count, true)
  }
}

impl<B: AsRef<[u8]> + ?Sized> AsRef<[u8]> for Cursor<'_, B> {
  fn as_ref(&self) -> &[u8] {
    self.backing_buffer.as_ref()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_cursor_reads_correctly() {
    let data = b"abcdef";

    let mut buf = [0u8; 3];

    let mut reader = Cursor::new(data);

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
