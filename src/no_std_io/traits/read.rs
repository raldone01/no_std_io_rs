use core::cell::{Cell, RefCell, UnsafeCell};

use alloc::boxed::Box;

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

impl<R: Read + ?Sized> Read for &mut R {
  type ReadError = R::ReadError;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    (**self).read(output_buffer)
  }
}

fn advance<T: AsRef<[u8]> + ?Sized>(slice: &mut T, n: usize) {
  let slice_ref = &mut slice.as_ref();
  *slice_ref = &core::mem::take(slice_ref)[n..];
}

fn read_slice<T: AsRef<[u8]> + ?Sized>(
  slice: &mut T,
  output_buffer: &mut [u8],
) -> Result<usize, core::convert::Infallible> {
  let n = core::cmp::min(output_buffer.len(), slice.as_ref().len());
  output_buffer[..n].copy_from_slice(&slice.as_ref()[..n]);
  advance(slice, n);
  Ok(n)
}

impl<R: Read + ?Sized> Read for Box<R> {
  type ReadError = R::ReadError;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    self.as_mut().read(output_buffer)
  }
}

impl<R: Read + ?Sized> Read for RefCell<R> {
  type ReadError = R::ReadError;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    self.get_mut().read(output_buffer)
  }
}

impl<R: Read + ?Sized> Read for Cell<R> {
  type ReadError = R::ReadError;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    self.get_mut().read(output_buffer)
  }
}

impl<R: Read + ?Sized> Read for UnsafeCell<R> {
  type ReadError = R::ReadError;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    self.get_mut().read(output_buffer)
  }
}

impl Read for &[u8] {
  type ReadError = core::convert::Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    read_slice(self, output_buffer)
  }
}

impl Read for &mut [u8] {
  type ReadError = core::convert::Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    read_slice(self, output_buffer)
  }
}

impl<const N: usize> Read for [u8; N] {
  type ReadError = core::convert::Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    read_slice(self, output_buffer)
  }
}

impl<const N: usize> Read for &[u8; N] {
  type ReadError = core::convert::Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    read_slice(self, output_buffer)
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
