use core::{
  cell::{Cell, RefCell, UnsafeCell},
  convert::Infallible,
};

use alloc::boxed::Box;

use crate::LimitedReader;

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

/// Read is implemented for `&[u8]` by copying from the slice.
///
/// Note that reading updates the slice to point to the yet unread part.
/// The slice will be empty when EOF is reached.
impl Read for &[u8] {
  type ReadError = Infallible;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    let amt = core::cmp::min(output_buffer.len(), self.len());
    let (a, b) = self.split_at(amt);

    // First check if the amount of bytes we want to read is small:
    // `copy_from_slice` will generally expand to a call to `memcpy`, and
    // for a single byte the overhead is significant.

    if amt == 1 {
      output_buffer[0] = a[0];
    } else {
      output_buffer[..amt].copy_from_slice(a);
    }

    *self = b;
    Ok(amt)
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_read_slice() {
    let reader_data = [1, 2, 3, 4, 5];
    let mut reader = &reader_data[..];
    let mut output_buffer = [0; 3];
    let bytes_read = reader.read(&mut output_buffer).unwrap();
    assert_eq!(bytes_read, 3);
    assert_eq!(output_buffer, [1, 2, 3]);
    let bytes_read = reader.read(&mut output_buffer).unwrap();
    assert_eq!(bytes_read, 2);
    assert_eq!(output_buffer, [4, 5, 3]); // Remaining data
  }
}
