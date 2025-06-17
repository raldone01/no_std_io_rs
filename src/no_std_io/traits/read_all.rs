use thiserror::Error;

use crate::no_std_io::Read;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ReadAllError<U> {
  #[error("Unexpected EOF while reading {bytes_requested} bytes, only {bytes_read} bytes read")]
  UnexpectedEof {
    bytes_requested: usize,
    bytes_read: usize,
  },
  #[error("Underlying read error: {0:?}")]
  Io(#[from] U),
}

/// Extension trait that provides a `read_all` method for any `Read` implementer.
pub trait ReadAll: Read {
  /// Reads the entire buffer, retrying partial reads.
  fn read_all(&mut self, output_buffer: &mut [u8]) -> Result<(), ReadAllError<Self::ReadError>>;
}

/// Blanket implementation for all `Read` implementors.
impl<R: Read + ?Sized> ReadAll for R {
  fn read_all(&mut self, output_buffer: &mut [u8]) -> Result<(), ReadAllError<Self::ReadError>> {
    let requested_bytes = output_buffer.len();
    let mut buf = output_buffer;
    let mut total_read = 0;

    while !buf.is_empty() {
      match self.read(buf) {
        Ok(0) => {
          return Err(ReadAllError::UnexpectedEof {
            bytes_requested: requested_bytes,
            bytes_read: total_read,
          });
        },
        Ok(n) => {
          total_read += n;
          buf = &mut buf[n..]; // advance buffer
        },
        Err(e) => return Err(ReadAllError::Io(e)),
      }
    }
    Ok(())
  }
}
