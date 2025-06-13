use alloc::{boxed::Box, format, string::String};
use miniz_oxide::{
  inflate::stream::{inflate, InflateState},
  DataFormat, MZError,
};

use crate::{
  dynamic_error::DynamicError,
  no_std_io::{IoError, Read},
};

pub struct CompressedReader<'a> {
  compressed_data: &'a [u8],
  decompressor: InflateState,
  current_position: usize,
}

impl<'a> CompressedReader<'a> {
  #[must_use]
  pub fn new(compressed_data: &'a [u8], zlib_wrapped: bool) -> Self {
    let data_format = if zlib_wrapped {
      DataFormat::Zlib
    } else {
      DataFormat::Raw
    };
    Self {
      compressed_data,
      decompressor: InflateState::new(data_format),
      current_position: 0,
    }
  }
}

impl Read for CompressedReader<'_> {
  fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
    let remaining_data = &self.compressed_data[self.current_position..];
    let result = inflate(
      &mut self.decompressor,
      remaining_data,
      buf,
      miniz_oxide::MZFlush::None,
    );
    self.current_position += result.bytes_consumed;
    match result.status {
      Ok(_) => Ok(result.bytes_written),
      Err(e) => {
        if e == MZError::Buf {
          Err(IoError::Io(Box::new(DynamicError(String::from(
            "Buffer error during decompression",
          )))))
        } else {
          Err(IoError::Io(Box::new(DynamicError(format!(
            "Decompression error: {:?}",
            e
          )))))
        }
      },
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::buffered_reader::{self};

  fn run_compressed_reader_test(use_zlib: bool) {
    let uncompressed_data = b"Hello, world! This is a test of the CompressedReader.";
    let compressed_data = if use_zlib {
      miniz_oxide::deflate::compress_to_vec_zlib(uncompressed_data, 6)
    } else {
      miniz_oxide::deflate::compress_to_vec(uncompressed_data, 6)
    };
    let reader = CompressedReader::new(&compressed_data, use_zlib);
    let mut buffered_reader = buffered_reader::BufferedReader::new(1024, reader);
    let bytes_read = buffered_reader
      .read_exact(uncompressed_data.len())
      .expect("Failed to read");
    assert_eq!(bytes_read, uncompressed_data);
  }

  #[test]
  fn test_compressed_reader_raw() {
    run_compressed_reader_test(false);
  }

  #[test]
  fn test_compressed_reader_zlib() {
    run_compressed_reader_test(true);
  }
}
