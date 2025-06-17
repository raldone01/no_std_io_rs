use thiserror::Error;

use crate::no_std_io::{BufferedRead, Read, ReadExactError};

/// See [`BufferedRead`] for more details.
pub struct ForkedBufferedReader<'a, R: BufferedRead + ?Sized> {
  buffered_reader: &'a mut R,
  position: usize,
}

impl<'a, R: BufferedRead<BackingImplementation = R> + ?Sized> ForkedBufferedReader<'a, R> {
  #[must_use]
  pub fn new(buffered_reader: &'a mut R, start_position: usize) -> Self {
    Self {
      buffered_reader,
      position: start_position,
    }
  }

  pub fn reset(&mut self) {
    self.position = 0;
  }

  pub fn bytes_read(&self) -> usize {
    self.position
  }
}

impl<'a, R: BufferedRead<BackingImplementation = R> + ?Sized> BufferedRead
  for ForkedBufferedReader<'a, R>
{
  type BackingImplementation = R::BackingImplementation;

  fn fork_reader(&mut self) -> ForkedBufferedReader<'_, Self::BackingImplementation> {
    ForkedBufferedReader::new(self.buffered_reader, self.position)
  }

  fn skip(
    &mut self,
    byte_count: usize,
  ) -> Result<(), ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    self.read_exact(byte_count).map(|_| ())
  }

  fn buffer_size_hint(&self) -> usize {
    self.buffered_reader.buffer_size_hint()
  }

  fn read_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    let full_buffer = self
      .buffered_reader
      .peek_exact(self.position + byte_count)?;
    let sliced_buffer = &full_buffer[self.position..];
    self.position += byte_count;
    Ok(sliced_buffer)
  }

  fn peek_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<<Self::BackingImplementation as Read>::ReadError>> {
    let full_buffer = self
      .buffered_reader
      .peek_exact(self.position + byte_count)?;
    Ok(&full_buffer[self.position..])
  }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ForkedBufferedReaderReadError<U> {
  #[error("Memory limit of {0} bytes exceeded for exact read")]
  MemoryLimitExceeded(usize),
  #[error("Underlying read error: {0:?}")]
  Io(#[from] U),
}

impl<R: BufferedRead<BackingImplementation = R> + ?Sized> Read for ForkedBufferedReader<'_, R> {
  type ReadError = ForkedBufferedReaderReadError<R::ReadError>;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    if output_buffer.is_empty() {
      return Ok(0);
    }

    let bytes = match self.read_exact(output_buffer.len()) {
      Ok(bytes) => bytes,
      Err(ReadExactError::MemoryLimitExceeded(max_buffer_size)) => {
        return Err(ForkedBufferedReaderReadError::MemoryLimitExceeded(
          max_buffer_size,
        ));
      },
      Err(ReadExactError::UnexpectedEof {
        min_readable_bytes: bytes_read,
        bytes_requested: _,
      }) => {
        let position = self.position;
        // If we reach here, it means we tried to read more data than was available.
        // This is an error condition for read_exact, but here we can return what we got.
        &self
          .read_exact(bytes_read)
          .unwrap_or_else(|_| panic!("Failed to read internal buffer. This is a bug!"))[position..]
      },
      Err(ReadExactError::Io(e)) => return Err(ForkedBufferedReaderReadError::Io(e)),
    };

    output_buffer.copy_from_slice(bytes);
    Ok(output_buffer.len())
  }
}
