use thiserror::Error;

use crate::Write;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum WriteAllError<U> {
  #[error("Underlying device wrote zero bytes after writing {bytes_written} bytes")]
  ZeroWrite { bytes_written: usize },
  #[error("Underlying write error: {0:?}")]
  Io(#[from] U),
}

/// Extension trait that provides a `write_all` method for any `Write` implementer.
pub trait WriteAll: Write {
  /// Writes the entire buffer, retrying partial writes.
  ///
  /// Does not flush, but passes the `sync_hint` to the underlying `write` method.
  fn write_all(
    &mut self,
    input_buffer: &[u8],
    sync_hint: bool,
  ) -> Result<(), WriteAllError<Self::WriteError>> {
    let mut buf = input_buffer;
    while !buf.is_empty() {
      match self.write(buf, sync_hint) {
        Ok(0) => {
          return Err(WriteAllError::ZeroWrite {
            bytes_written: input_buffer.len() - buf.len(),
          });
        },
        Ok(n) => buf = &buf[n..], // advance buffer
        Err(e) => return Err(WriteAllError::Io(e)),
      }
    }
    Ok(())
  }
}

/// Blanket implementation for all `Write` implementers.
impl<W: Write + ?Sized> WriteAll for W {}
