//! https://www.ietf.org/rfc/rfc1952.txt

use thiserror::Error;

use crate::no_std_io::{Write, WriteAll as _, WriteAllError};

const ID1: u8 = 0x1F;
const ID2: u8 = 0x8B;
const CM_DEFLATE: u8 = 0x08;
const FLG_FTEXT: u8 = 1 << 0;
const FLG_FHCRC: u8 = 1 << 1;
const FLG_FEXTRA: u8 = 1 << 2;
const FLG_FNAME: u8 = 1 << 3;
const FLG_FCOMMENT: u8 = 1 << 4;
// MTIME here
const XFL_MAXIMUM_COMPRESSION: u8 = 2;
const XFL_FASTEST_COMPRESSION: u8 = 4;
const OS_UNIX: u8 = 3;

// TODO: https://crates.io/crates/crc32fast writer/reader make them take &mut ref to an existing crc32fast::Hasher

/// GzHeader represents the gzip header with only the MTIME field parsed.
#[derive(Debug, Clone)]
pub struct GzHeader {
  pub mtime: u32,
}

#[derive(Error, Debug)]
pub enum GzHeaderError {
  #[error("Invalid gzip header: {0}")]
  InvalidHeader(&'static str),
  #[error("Buffer too short for gzip header")]
  BufferTooShort,
  #[error("Invalid magic numbers in gzip header expected 0x1F 0x8B got {0:x} {1:x}")]
  InvalidMagicNumbers(u8, u8),
  #[error("Invalid compression method in gzip header, expected deflate (0x08) got {0}")]
  InvalidCompressionMethod(u8),
  #[error("Optional field too short in gzip header")]
  OptionalFieldTooShort,
  #[error("Optional field out of bounds in gzip header")]
  OptionalFieldOutOfBounds,
}

impl GzHeader {
  // TODO: use reader
  /// Parse a GzHeader from a buffer slice.
  /// Returns `Ok((header_length, GzHeader))` if successful, otherwise `Err(GzHeaderError)`.
  pub fn parse(input_buffer: &[u8]) -> Result<(usize, GzHeader), GzHeaderError> {
    // Minimum gzip header length is 10 bytes
    if input_buffer.len() < 10 {
      return Err(GzHeaderError::BufferTooShort);
    }

    // Check magic numbers
    if input_buffer[0] != 0x1F || input_buffer[1] != 0x8B {
      return Err(GzHeaderError::InvalidMagicNumbers(
        input_buffer[0],
        input_buffer[1],
      ));
    }

    // Check compression method (must be deflate)
    if input_buffer[2] != 0x08 {
      return Err(GzHeaderError::InvalidCompressionMethod(input_buffer[2]));
    }

    let flg = input_buffer[3];
    let mtime = u32::from_le_bytes([
      input_buffer[4],
      input_buffer[5],
      input_buffer[6],
      input_buffer[7],
    ]);

    let mut offset = 10;

    // Skip optional fields according to flags
    if flg & 0x04 != 0 {
      if input_buffer.len() < offset + 2 {
        return Err(GzHeaderError::OptionalFieldTooShort);
      }
      let xlen = u16::from_le_bytes([input_buffer[offset], input_buffer[offset + 1]]) as usize;
      offset += 2 + xlen;
    }

    if flg & 0x08 != 0 {
      while offset < input_buffer.len() && input_buffer[offset] != 0 {
        offset += 1;
      }
      offset += 1;
    }

    if flg & 0x10 != 0 {
      while offset < input_buffer.len() && input_buffer[offset] != 0 {
        offset += 1;
      }
      offset += 1;
    }

    if flg & 0x02 != 0 {
      offset += 2;
    }

    if offset > input_buffer.len() {
      return Err(GzHeaderError::OptionalFieldOutOfBounds);
    }

    Ok((offset, GzHeader { mtime }))
  }

  /// Write a minimal gzip header to the given writer.
  /// Uses deflate compression method and Unix OS.
  pub fn write<W: Write + ?Sized>(&self, w: &mut W) -> Result<(), WriteAllError<W::WriteError>> {
    w.write_all(
      &[
        0x1F, 0x8B, // ID1, ID2
        0x08, // Compression method (deflate)
        0x00, // FLG (no optional fields)
      ],
      false,
    )?;

    // MTIME
    w.write_all(&self.mtime.to_le_bytes(), false)?;

    w.write_all(
      &[
        0x00, // XFL
        0x03, // OS (Unix)
      ],
      false,
    )?;

    Ok(())
  }
}
