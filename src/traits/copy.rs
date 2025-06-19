use thiserror::Error;

use crate::{Read, Write, WriteAll as _, WriteAllError};

/// Reads all bytes from the `reader` and writes them to the `writer`.
///
/// This function continues until the reader returns 0 (EOF) or an error occurs.
/// Data is written using `write_all`, preserving sync hints for each write.
///
/// Note: When a reader or writer is buffered, consider using `[copy_to]` or `[copy_from]` or if both are `[copy_buffered]` instead for more efficiency.
///
/// Returns the total number of bytes copied.
pub fn copy<R: Read, W: Write>(
  reader: &mut R,
  writer: &mut W,
  transfer_buffer: &mut [u8],
  sync_hint: bool,
) -> Result<usize, CopyError<R::ReadError, W::WriteError>> {
  let mut total_bytes = 0;

  loop {
    let bytes_read = reader.read(transfer_buffer).map_err(CopyError::Read)?;
    if bytes_read == 0 {
      break; // EOF
    }

    writer
      .write_all(&transfer_buffer[..bytes_read], sync_hint)
      .map_err(CopyError::Write)?;

    total_bytes += bytes_read;
  }

  Ok(total_bytes)
}

/// Error type returned by the `pipe` function.
#[derive(Debug, Error)]
pub enum CopyError<RE, WE> {
  #[error("Read error: {0:?}")]
  Read(RE),
  #[error("Write error: {0:?}")]
  Write(WriteAllError<WE>),
}
