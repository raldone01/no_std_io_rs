use crate::{BufferedRead, Read, ReadExactError};

/// See [`BufferedRead`] for more details.
#[derive(Debug, PartialEq, Eq)]
pub struct ForkedBufferedReader<'a, R: BufferedRead + ?Sized> {
  buffered_reader: &'a mut R,
  position: usize,
}

impl<'a, R: BufferedRead + ?Sized> ForkedBufferedReader<'a, R> {
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

  fn read_internal(
    &mut self,
    byte_count: usize,
    peek: bool,
  ) -> Result<&[u8], ReadExactError<R::UnderlyingReadExactError>> {
    let full_buffer = self
      .buffered_reader
      .peek_exact(self.position + byte_count)?;
    let sliced_buffer = &full_buffer[self.position..];
    if !peek {
      self.position += byte_count;
    }
    Ok(sliced_buffer)
  }

  fn read_buffered_internal(&mut self, peek: bool) -> Result<&[u8], R::UnderlyingReadExactError> {
    let full_buffer = self.buffered_reader.peek_buffered()?;
    let sliced_buffer = &full_buffer[self.position..];
    if !peek {
      self.position += sliced_buffer.len();
    }
    Ok(sliced_buffer)
  }
}

impl<'a, R: BufferedRead + ?Sized> BufferedRead for ForkedBufferedReader<'a, R> {
  type UnderlyingReadExactError = R::UnderlyingReadExactError;
  type ForkedBufferedReaderImplementation<'b>
    = ForkedBufferedReader<'b, R>
  where
    Self: 'b;

  fn fork_reader(&mut self) -> Self::ForkedBufferedReaderImplementation<'_> {
    ForkedBufferedReader::new(self.buffered_reader, self.position)
  }

  fn skip(
    &mut self,
    byte_count: usize,
  ) -> Result<(), ReadExactError<Self::UnderlyingReadExactError>> {
    self.read_exact(byte_count).map(|_| ())
  }

  fn read_buffered(&mut self) -> Result<&[u8], Self::UnderlyingReadExactError> {
    self.read_buffered_internal(false)
  }

  fn peek_buffered(&mut self) -> Result<&[u8], Self::UnderlyingReadExactError> {
    self.read_buffered_internal(true)
  }

  fn read_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<Self::UnderlyingReadExactError>> {
    self.read_internal(byte_count, false)
  }

  fn peek_exact(
    &mut self,
    byte_count: usize,
  ) -> Result<&[u8], ReadExactError<Self::UnderlyingReadExactError>> {
    self.read_internal(byte_count, true)
  }
}

impl<R: BufferedRead + ?Sized> Read for ForkedBufferedReader<'_, R> {
  type ReadError = ReadExactError<R::UnderlyingReadExactError>;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    if output_buffer.is_empty() {
      return Ok(0);
    }

    let bytes = match self.read_exact(output_buffer.len()) {
      Ok(bytes) => bytes,
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
      Err(ReadExactError::Io(e)) => return Err(Self::ReadError::Io(e)),
    };

    output_buffer.copy_from_slice(bytes);
    Ok(output_buffer.len())
  }
}
