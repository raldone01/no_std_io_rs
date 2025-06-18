use core::convert::Infallible;

use thiserror::Error;

use crate::no_std_io::{
  BackingBuffer, BufferedRead, ForkedBufferedReader, Read, ReadExactError, ResizeError, Seek,
  SeekFrom, Write,
};

pub struct Cursor<B> {
  backing_buffer: B,
  position: usize,
}

impl<B> Cursor<B> {
  #[must_use]
  pub fn new(backing_buffer: B) -> Self {
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

impl<B: AsRef<[u8]>> Cursor<B> {
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

  #[must_use]
  pub fn full_buffer(&self) -> &[u8] {
    self.backing_buffer.as_ref()
  }
}

impl<B: BackingBuffer> Cursor<B> {
  #[must_use]
  pub fn backing_buffer(&self) -> &B {
    &self.backing_buffer
  }

  #[must_use]
  pub fn backing_buffer_mut(&mut self) -> &mut B {
    &mut self.backing_buffer
  }
}

impl<B: AsRef<[u8]>> Cursor<B> {
  #[must_use]
  pub fn split(&self) -> (&[u8], &[u8]) {
    let slice = self.backing_buffer.as_ref();
    let position = self.position.min(slice.len());
    slice.split_at(position)
  }

  #[must_use]
  pub fn before(&self) -> &[u8] {
    self.split().0
  }
  #[must_use]
  pub fn after(&self) -> &[u8] {
    self.split().1
  }
}

impl<B: AsMut<[u8]>> Cursor<B> {
  #[must_use]
  pub fn split_mut(&mut self) -> (&mut [u8], &mut [u8]) {
    let slice = self.backing_buffer.as_mut();
    let position = self.position.min(slice.len());
    slice.split_at_mut(position)
  }

  #[must_use]
  pub fn before_mut(&mut self) -> &mut [u8] {
    self.split_mut().0
  }
  #[must_use]
  pub fn after_mut(&mut self) -> &mut [u8] {
    self.split_mut().1
  }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CursorSeekError {
  #[error("Seek {offset:?} out of bounds: position {position}, length {length}")]
  OutOfBounds {
    position: usize,
    length: usize,
    offset: SeekFrom,
  },
}

impl<B: AsRef<[u8]>> Seek for Cursor<B> {
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

impl<B: AsRef<[u8]>> Read for Cursor<B> {
  type ReadError = Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    let n = Read::read(&mut Cursor::split(self).1, output_buffer)?;
    self.position += n;
    Ok(n)
  }
}

impl<B: AsRef<[u8]>> Cursor<B> {
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

impl<B: AsRef<[u8]>> BufferedRead for Cursor<B> {
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

impl<B: BackingBuffer> Write for Cursor<B> {
  type WriteError = B::ResizeError;
  type FlushError = Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    if input_buffer.is_empty() {
      return Ok(0);
    }

    let mut end_pos = self.position.saturating_add(input_buffer.len());

    // Resize if needed
    if end_pos > self.backing_buffer.as_mut().len() {
      let resize_result = self.backing_buffer.try_resize(end_pos);
      let backing_buffer_size = match resize_result {
        Ok(new_size) => new_size,
        Err(ResizeError {
          size_after_resize,
          resize_error,
        }) => {
          if size_after_resize.saturating_sub(end_pos) == 0 {
            return Err(resize_error);
          }
          size_after_resize
        },
      };

      end_pos = backing_buffer_size;
    };

    let buffer = self.backing_buffer.as_mut();
    buffer[self.position..end_pos].copy_from_slice(&input_buffer[..end_pos - self.position]);

    self.position = end_pos;
    Ok(input_buffer.len())
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    // No-op for in-memory buffer.
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use alloc::vec::Vec;

  use crate::no_std_io::FixedSizeBufferError;

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

  #[test]
  fn test_cursor_writes_correctly() {
    let mut cursor_mut = Cursor::new([0u8; 6]);

    // First write
    let n = cursor_mut.write(b"abc", false).unwrap();
    assert_eq!(n, 3);
    assert_eq!(&cursor_mut.before(), b"abc");

    // Second write
    let n = cursor_mut.write(b"def", false).unwrap();
    assert_eq!(n, 3);
    assert_eq!(&cursor_mut.before(), b"abcdef");

    // Third write (should not write anything)
    assert_eq!(
      cursor_mut.write(b"oof", false).unwrap_err(),
      FixedSizeBufferError {
        fixed_buffer_size: 6,
        requested_size: 9,
      }
    );
  }

  #[test]
  fn test_cursor_growing() {
    let mut cursor_mut = Cursor::new(Vec::new());
    let n = cursor_mut.write(b"abc", false).unwrap();
    assert_eq!(n, 3);
    assert_eq!(cursor_mut.before(), b"abc");
  }
}
