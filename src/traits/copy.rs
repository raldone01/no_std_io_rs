use thiserror::Error;

use crate::{BufferedRead, Read, ReadExactError, Write, WriteAll as _, WriteAllError};

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CopyError<RE, WE> {
  #[error("Underlying read error: {0:?}")]
  IoRead(RE),
  #[error("Underlying write error: {0:?}")]
  IoWrite(WriteAllError<WE>),
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CopyUntilError<RE, WE> {
  #[error("Delimiter byte {delimiter_byte:?} not found after reading {bytes_read} bytes")]
  DelimiterNotFound {
    delimiter_byte: u8,
    bytes_read: usize,
  },
  #[error("Underlying read error: {0:?}")]
  IoRead(RE),
  #[error("Underlying write error: {0:?}")]
  IoWrite(WriteAllError<WE>),
}

pub trait Copy: Read {
  /// Streams all bytes from the reader to the writer using a transfer buffer.
  ///
  /// This function continues until the reader returns 0 (EOF) or an error occurs.
  ///
  /// Note: If the reader supports buffered reading, consider using `copy_buffered` instead for better performance.
  ///
  /// Returns the total number of bytes copied.
  fn copy<W: Write + ?Sized>(
    &mut self,
    writer: &mut W,
    transfer_buffer: &mut [u8],
    sync_hint: bool,
  ) -> Result<usize, CopyError<Self::ReadError, W::WriteError>> {
    let mut total_bytes = 0;

    loop {
      let bytes_read = self.read(transfer_buffer).map_err(CopyError::IoRead)?;
      if bytes_read == 0 {
        break; // EOF
      }

      writer
        .write_all(&transfer_buffer[..bytes_read], sync_hint)
        .map_err(CopyError::IoWrite)?;

      total_bytes += bytes_read;
    }

    Ok(total_bytes)
  }

  /// Streams bytes from the reader to the writer until a specific delimiter byte is encountered.
  ///
  /// Note: If the reader supports buffered reading, consider using `copy_buffered_until` instead for better performance.
  ///
  /// Returns the total number of bytes copied.
  fn copy_until<W: Write + ?Sized>(
    &mut self,
    delimiter_byte: u8,
    writer: &mut W,
    sync_hint: bool,
    write_delimiter: bool,
  ) -> Result<usize, CopyUntilError<Self::ReadError, W::WriteError>> {
    let mut total_bytes = 0;
    let mut transfer_byte = [0];

    loop {
      let bytes_read = self
        .read(&mut transfer_byte)
        .map_err(CopyUntilError::IoRead)?;
      if bytes_read == 0 {
        return Err(CopyUntilError::DelimiterNotFound {
          delimiter_byte,
          bytes_read: total_bytes,
        });
      }

      if !write_delimiter && transfer_byte[0] == delimiter_byte {
        break; // Delimiter found
      }

      writer
        .write_all(&transfer_byte, sync_hint)
        .map_err(CopyUntilError::IoWrite)?;

      total_bytes += bytes_read;

      if transfer_byte[0] == delimiter_byte {
        break; // Delimiter found
      }
    }
    Ok(total_bytes)
  }
}

/// Blanket implementation for all `Read` implementers.
impl<R: Read + ?Sized> Copy for R {}

pub trait CopyBuffered: BufferedRead {
  /// Streams all bytes from the reader to the writer using a transfer buffer.
  ///
  /// This function continues until the reader returns 0 (EOF) or an error occurs.
  ///
  /// Returns the total number of bytes copied.
  fn copy_buffered<W: Write + ?Sized>(
    &mut self,
    writer: &mut W,
    sync_hint: bool,
  ) -> Result<usize, CopyError<Self::UnderlyingReadExactError, W::WriteError>> {
    let mut total_bytes = 0;

    loop {
      let bytes_read = self.read_buffered().map_err(CopyError::IoRead)?;
      if bytes_read.is_empty() {
        break; // EOF
      }
      writer
        .write_all(bytes_read, sync_hint)
        .map_err(CopyError::IoWrite)?;
      total_bytes += bytes_read.len();
    }

    Ok(total_bytes)
  }

  /// Streams bytes from the reader to the writer until a specific delimiter byte is encountered.
  ///
  /// Returns the total number of bytes copied.
  fn copy_buffered_until<W: Write + ?Sized>(
    &mut self,
    delimiter_byte: u8,
    writer: &mut W,
    sync_hint: bool,
    write_delimiter: bool,
  ) -> Result<usize, CopyUntilError<Self::UnderlyingReadExactError, W::WriteError>> {
    let mut total_bytes = 0;

    loop {
      let bytes_read = self.peek_buffered().map_err(CopyUntilError::IoRead)?;
      if bytes_read.is_empty() {
        return Err(CopyUntilError::DelimiterNotFound {
          delimiter_byte,
          bytes_read: total_bytes,
        });
      }

      // find the position of the delimiter byte
      let mut delimiter_found = false;
      let bytes_read = if let Some(pos) = bytes_read.iter().position(|&b| b == delimiter_byte) {
        delimiter_found = true;
        if write_delimiter {
          &bytes_read[..=pos] // include the delimiter byte in the write
        } else {
          &bytes_read[..pos] // exclude the delimiter byte from the write
        }
      } else {
        bytes_read // no delimiter found, write all bytes
      };
      let bytes_read_count = bytes_read.len();

      writer
        .write_all(bytes_read, sync_hint)
        .map_err(CopyUntilError::IoWrite)?;
      total_bytes += bytes_read_count;
      self.skip(bytes_read_count).map_err(|e| match e {
        ReadExactError::UnexpectedEof { .. } => {
          panic!("BUG: We are only skipping bytes that are in the buffer.")
        },
        ReadExactError::Io(e) => CopyUntilError::IoRead(e),
      })?;

      if delimiter_found {
        break; // Delimiter found
      }
    }

    Ok(total_bytes)
  }
}

/// Blanked implementation for all `BufferedRead` implementers.
impl<R: BufferedRead + ?Sized> CopyBuffered for R {}
