use core::{
  cell::{Cell, RefCell, UnsafeCell},
  convert::Infallible,
};

use alloc::boxed::Box;

use crate::{advance, LimitedReader};

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

fn read_slice<T: AsRef<[u8]> + ?Sized>(
  slice: &mut T,
  output_buffer: &mut [u8],
) -> Result<usize, Infallible> {
  let slice = &mut slice.as_ref();

  let n = core::cmp::min(output_buffer.len(), slice.len());
  output_buffer[..n].copy_from_slice(&slice.as_ref()[..n]);
  advance(slice, n);
  Ok(n)
}

// --- Read implementations for common smart pointer types ---

macro_rules! impl_read_for_wrapper {
  ( $( ($wrapper:ty, $accessor:ident) ),* ) => {
      $(
          impl<R: Read + ?Sized> Read for $wrapper {
              type ReadError = R::ReadError;

              fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
                  self.$accessor().read(output_buffer)
              }
          }
      )*
  };
}

// Now, use the macro to generate the implementations.
impl_read_for_wrapper!(
  (Box<R>, as_mut),
  (RefCell<R>, get_mut),
  (Cell<R>, get_mut),
  (UnsafeCell<R>, get_mut)
);

// --- Read implementations for slice types ---

impl Read for [u8] {
  type ReadError = Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    read_slice(self, output_buffer)
  }
}

impl Read for &[u8] {
  type ReadError = Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    read_slice(self, output_buffer)
  }
}

impl<const N: usize> Read for [u8; N] {
  type ReadError = Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    read_slice(self, output_buffer)
  }
}

impl<const N: usize> Read for &[u8; N] {
  type ReadError = Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    read_slice(self, output_buffer)
  }
}

// --- ReadLimited trait ---

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
