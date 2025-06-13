use std::io::{self, Read, Write};

/// GzHeader represents the gzip header with only the MTIME field parsed.
#[derive(Debug, Clone)]
pub struct GzHeader {
  pub mtime: u32,
}

impl GzHeader {
  /// Parse a GzHeader from a buffer slice.
  /// Returns `Some((header_length, GzHeader))` if successful, otherwise `None`.
  pub fn parse(buf: &[u8]) -> Option<(usize, GzHeader)> {
    // TODO: rename buf -> input_buffer
    // Minimum gzip header length is 10 bytes
    if buf.len() < 10 {
      return None;
    }

    // Check magic numbers
    if buf[0] != 0x1F || buf[1] != 0x8B {
      return None;
    }

    // Check compression method (must be deflate)
    if buf[2] != 0x08 {
      return None;
    }

    let flg = buf[3];
    let mtime = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);

    let mut offset = 10;

    // Skip optional fields according to flags
    if flg & 0x04 != 0 {
      if buf.len() < offset + 2 {
        return None;
      }
      let xlen = u16::from_le_bytes([buf[offset], buf[offset + 1]]) as usize;
      offset += 2 + xlen;
    }

    if flg & 0x08 != 0 {
      while offset < buf.len() && buf[offset] != 0 {
        offset += 1;
      }
      offset += 1;
    }

    if flg & 0x10 != 0 {
      while offset < buf.len() && buf[offset] != 0 {
        offset += 1;
      }
      offset += 1;
    }

    if flg & 0x02 != 0 {
      offset += 2;
    }

    if offset > buf.len() {
      return None;
    }

    Some((offset, GzHeader { mtime }))
  }

  /// Write a minimal gzip header to the given writer.
  /// Uses deflate compression method and Unix OS.
  pub fn write<W: Write>(&self, mut w: W) -> io::Result<()> {
    w.write_all(&[
      0x1F, 0x8B, // ID1, ID2
      0x08, // Compression method (deflate)
      0x00, // FLG (no optional fields)
    ])?;

    w.write_all(&self.mtime.to_le_bytes())?; // MTIME
    w.write_all(&[
      0x00, // XFL
      0x03, // OS (Unix)
    ])?;

    Ok(())
  }
}
