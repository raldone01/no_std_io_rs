use crate::no_std_io::LimitedReader;

/// Trait for reading bytes.
pub trait Read {
  type ReadError;

  /// Read up to `output_buffer.len()` bytes into `output_buffer`.
  /// Providing an empty `output_buffer` is valid and will return 0 bytes read.
  ///
  /// Returns number of bytes read.
  /// On EOF, it returns 0 bytes read.
  /// Any further reads after EOF return 0 bytes read.
  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError>;
}

fn advance_mut(slice: &mut &mut [u8], n: usize) {
  *slice = &mut core::mem::take(slice)[n..];
}

impl Read for &[u8] {
  type ReadError = core::convert::Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    let n = core::cmp::min(output_buffer.len(), self.len());
    output_buffer[..n].copy_from_slice(&self[..n]);
    *self = &self[n..];
    Ok(n)
  }
}

impl Read for &mut [u8] {
  type ReadError = core::convert::Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    let n = core::cmp::min(output_buffer.len(), self.len());
    output_buffer[..n].copy_from_slice(&self[..n]);
    advance_mut(self, n);
    Ok(n)
  }
}

impl<const N: usize> Read for [u8; N] {
  type ReadError = core::convert::Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    let n = core::cmp::min(output_buffer.len(), self.len());
    output_buffer[..n].copy_from_slice(&self[..n]);
    advance_mut(&mut self.as_mut_slice(), n);
    Ok(n)
  }
}

pub trait ReadLimited: Read {
  /// Creates a new reader that limits the number of bytes read to `read_limit_bytes`.
  ///
  /// Returns a new [`LimitedReader`] instance.
  #[must_use]
  fn take(&mut self, read_limit_bytes: usize) -> LimitedReader<'_, Self>;
}

impl<R: Read + ?Sized> ReadLimited for R {
  fn take(&mut self, read_limit_bytes: usize) -> LimitedReader<'_, Self> {
    LimitedReader::new(self, read_limit_bytes)
  }
}
