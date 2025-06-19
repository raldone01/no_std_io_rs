use core::panic;

use alloc::{vec, vec::Vec};

use miniz_oxide::{
  inflate::stream::{inflate, InflateState},
  DataFormat, MZError, MZStatus,
};
use thiserror::Error;

use crate::Read;

pub struct CompressedReader<'a, R: Read + ?Sized> {
  source_reader: &'a mut R,
  decompressor: InflateState,
  tmp_buffer: Vec<u8>,
}

impl<'a, R: Read + ?Sized> CompressedReader<'a, R> {
  #[must_use]
  pub fn new(reader: &'a mut R, zlib_wrapped: bool, tmp_buffer_size: usize) -> Self {
    let data_format = if zlib_wrapped {
      DataFormat::Zlib
    } else {
      DataFormat::Raw
    };
    Self {
      source_reader: reader,
      decompressor: InflateState::new(data_format),
      tmp_buffer: vec![0_u8; tmp_buffer_size],
    }
  }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CompressedReadError<U> {
  #[error("Decompressor did not consume all input bytes: {bytes_input} bytes read, {bytes_consumed} bytes consumed")]
  DecompressorDidNotConsumeInput {
    bytes_input: usize,
    bytes_consumed: usize,
  },
  #[error("Unexpected EOF while reading compressed data")]
  UnexpectedEof,
  #[error("Decompression error: {0:?}")]
  MZError(MZError),
  #[error("Underlying read error: {0:?}")]
  Io(#[from] U),
}

impl<R: Read + ?Sized> Read for CompressedReader<'_, R> {
  type ReadError = CompressedReadError<R::ReadError>;

  fn read(&mut self, output_buffer: &mut [u8]) -> Result<usize, Self::ReadError> {
    if output_buffer.is_empty() {
      return Ok(0); // Nothing to read into
    }

    loop {
      // Read some data from the source reader into the temporary buffer.
      let bytes_read_count = self.source_reader.read(&mut self.tmp_buffer)?;
      let bytes_read = &self.tmp_buffer[..bytes_read_count];

      // Pass the read bytes to the decompressor.
      let result = inflate(
        &mut self.decompressor,
        &bytes_read,
        output_buffer,
        miniz_oxide::MZFlush::None,
      );
      if result.bytes_consumed != bytes_read_count {
        // The decompressor did not consume all the bytes we read, which is unexpected.
        return Err(Self::ReadError::DecompressorDidNotConsumeInput {
          bytes_input: bytes_read_count,
          bytes_consumed: result.bytes_consumed,
        });
      }
      match result.status {
        Ok(MZStatus::Ok) => {
          if result.bytes_written != 0 {
            return Ok(result.bytes_written);
          }
        },
        Ok(MZStatus::StreamEnd) => return Ok(result.bytes_written),
        Ok(MZStatus::NeedDict) => {
          panic!("Decompressor returned NeedDict status, which is not supported in this context");
        },
        Err(MZError::Buf) => {
          if bytes_read_count == 0 {
            return Err(Self::ReadError::UnexpectedEof);
          }
          // Not enough input data so we try again.
        },
        Err(e) => return Err(Self::ReadError::MZError(e)),
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::{BufferedRead as _, BufferedReader, BytewiseReader, Cursor};

  fn test_compressed_reader_simple_read(use_zlib: bool) {
    let uncompressed_data = b"Hello, world! This is a test of the CompressedReader.";
    let compressed_data = if use_zlib {
      miniz_oxide::deflate::compress_to_vec_zlib(uncompressed_data, 6)
    } else {
      miniz_oxide::deflate::compress_to_vec(uncompressed_data, 6)
    };

    let mut slice_reader = Cursor::new(&compressed_data);
    let mut compressed_reader = CompressedReader::new(&mut slice_reader, use_zlib, 4096);
    let mut buffered_reader = BufferedReader::new(&mut compressed_reader, [0; 1024], 1);
    let bytes_read = buffered_reader
      .read_exact(uncompressed_data.len())
      .expect("Failed to read");
    assert_eq!(bytes_read, uncompressed_data);
  }

  #[test]
  fn test_compressed_reader_reads_raw_correctly() {
    test_compressed_reader_simple_read(false);
  }

  #[test]
  fn test_compressed_reader_reads_zlib_correctly() {
    test_compressed_reader_simple_read(true);
  }

  #[test]
  fn test_compressed_reader_reads_correctly_bytewise() {
    let uncompressed_data = b"Hello, world! This is a test of the CompressedReader.";
    let compressed_data = miniz_oxide::deflate::compress_to_vec(uncompressed_data, 6);

    let mut slice_reader = Cursor::new(&compressed_data);
    let mut bytewise_reader = BytewiseReader::new(&mut slice_reader);
    let mut compressed_reader = CompressedReader::new(&mut bytewise_reader, false, 4096);
    let mut buffered_reader = BufferedReader::new(&mut compressed_reader, [0; 1024], 1);
    let bytes_read = buffered_reader
      .read_exact(uncompressed_data.len())
      .unwrap_or_else(|e| panic!("Failed to read: {}", e));
    assert_eq!(bytes_read, uncompressed_data);
  }
}
