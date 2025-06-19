use alloc::{vec, vec::Vec};

use miniz_oxide::{
  deflate::{
    core::{create_comp_flags_from_zip_params, CompressorOxide},
    stream::deflate,
  },
  MZError, MZFlush, MZStatus, StreamResult,
};
use thiserror::Error;

use crate::no_std_io::{Write, WriteAll as _, WriteAllError};

/// Don't forget to call `finish()` when done to finalize the compression and flush any remaining data.
pub struct CompressedWriter<'a, W: Write + ?Sized> {
  compressor: CompressorOxide,
  target_writer: &'a mut W,
  finished: bool,
  tmp_buffer: Vec<u8>,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CompressedWriteError<WWE, WFE> {
  #[error("Compressor did not consume all input bytes: {bytes_input} bytes read, {bytes_consumed} bytes consumed")]
  CompressorDidNotConsumeInput {
    bytes_input: usize,
    bytes_consumed: usize,
  },
  #[error("Compression error: {0:?}")]
  MZError(MZError),
  #[error("The writer is already finished and cannot accept more data")]
  Finished,
  #[error("Underlying write error: {0:?}")]
  IoWrite(WriteAllError<WWE>),
  #[error("Underlying flush error: {0:?}")]
  IoFlush(WFE),
}

impl<'a, W: Write + ?Sized> CompressedWriter<'a, W> {
  #[must_use]
  pub fn new(
    target_writer: &'a mut W,
    level: u8,
    zlib_wrapped: bool,
    tmp_buffer_size: usize,
  ) -> Self {
    // use zlib wrapper (window bits == 1)
    let flags = create_comp_flags_from_zip_params(level.into(), zlib_wrapped as i32, 0);
    Self {
      compressor: CompressorOxide::new(flags),
      target_writer,
      finished: false,
      tmp_buffer: vec![0_u8; tmp_buffer_size],
    }
  }

  fn write_internal(
    &mut self,
    input_buffer: &[u8],
    flush: MZFlush,
  ) -> Result<StreamResult, CompressedWriteError<W::WriteError, W::FlushError>> {
    let result = deflate(
      &mut self.compressor,
      input_buffer,
      self.tmp_buffer.as_mut_slice(),
      flush,
    );
    if result.bytes_consumed != input_buffer.len() {
      // The compressor did not consume all the bytes we read, which is unexpected.
      return Err(
        CompressedWriteError::<W::WriteError, W::FlushError>::CompressorDidNotConsumeInput {
          bytes_input: input_buffer.len(),
          bytes_consumed: result.bytes_consumed,
        },
      );
    }
    match result.status {
      Ok(MZStatus::Ok) | Err(MZError::Buf) => {},
      Ok(MZStatus::StreamEnd) => {
        self.finished = true;
      },
      Ok(MZStatus::NeedDict) => {
        panic!("Compressor returned NeedDict status, which is not supported in this context");
      },
      Err(e) => return Err(CompressedWriteError::<W::WriteError, W::FlushError>::MZError(e)),
    };
    let sync_hint = flush != MZFlush::None;
    self
      .target_writer
      .write_all(&self.tmp_buffer[..result.bytes_written], sync_hint)
      .map_err(CompressedWriteError::<W::WriteError, W::FlushError>::IoWrite)?;
    Ok(result)
  }

  #[must_use]
  pub fn is_finished(&self) -> bool {
    self.finished
  }

  pub fn finish(&mut self) -> Result<(), CompressedWriteError<W::WriteError, W::FlushError>> {
    while self.write_internal(&[], MZFlush::Finish)?.bytes_written != 0 {}
    self.finished = true;
    Ok(())
  }
}

impl<W: Write + ?Sized> Write for CompressedWriter<'_, W> {
  type WriteError = CompressedWriteError<W::WriteError, W::FlushError>;
  type FlushError = CompressedWriteError<W::WriteError, W::FlushError>;

  fn write(&mut self, buffer_input: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    if self.finished {
      return Err(CompressedWriteError::Finished);
    }
    let flush = if sync_hint {
      MZFlush::Sync
    } else {
      MZFlush::None
    };
    self
      .write_internal(buffer_input, flush)
      .map(|result| result.bytes_consumed)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    if self.finished {
      return Err(CompressedWriteError::Finished);
    }
    self.write_internal(&[], MZFlush::Sync)?;
    self
      .target_writer
      .flush()
      .map_err(CompressedWriteError::<W::WriteError, W::FlushError>::IoFlush)?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::no_std_io::{BytewiseWriter, Cursor};

  #[test]
  fn test_compressed_writer_buffer_size_dynamic_questionmark() {
    let input_string = "Hello, world! This is a test of the BufferedWriter.".repeat(50);
    let uncompressed_data = input_string.as_bytes();

    let _reference_compressed_data =
      miniz_oxide::deflate::compress_to_vec_zlib(uncompressed_data, 6);

    let mut buffer_writer = Cursor::new([0; 128]);
    // A buffered writer can counteract the overhead of bytewise writing
    let mut bytewise_writer_after = BytewiseWriter::new(&mut buffer_writer);
    let mut compressed_writer = CompressedWriter::new(&mut bytewise_writer_after, 6, true, 1);
    let mut bytewise_writer_before = BytewiseWriter::new(&mut compressed_writer);
    bytewise_writer_before
      .write_all(uncompressed_data, false)
      .expect("Failed to write uncompressed data to compressed writer");
    bytewise_writer_before
      .flush()
      .expect("Failed to flush compressed data");
    compressed_writer
      .finish()
      .expect("Failed to finish compressed writer");
    let compressed_data = buffer_writer.before();
    let decompressed_data = miniz_oxide::inflate::decompress_to_vec_zlib(&compressed_data)
      .expect("Failed to decompress data");
    assert_eq!(decompressed_data, uncompressed_data);
  }

  fn test_compressed_writer(use_zlib: bool) {
    let uncompressed_data = b"Hello, world! This is a test of the CompressedWriter.";

    let _reference_compressed_data = if use_zlib {
      miniz_oxide::deflate::compress_to_vec_zlib(uncompressed_data, 6)
    } else {
      miniz_oxide::deflate::compress_to_vec(uncompressed_data, 6)
    };

    let mut buffer_writer = Cursor::new([0; 128]);
    let mut compressed_writer = CompressedWriter::new(&mut buffer_writer, 6, use_zlib, 128);
    compressed_writer
      .write_all(uncompressed_data, false)
      .expect("Failed to write uncompressed data to compressed writer");
    // check if it can survive a flush
    compressed_writer
      .flush()
      .expect("Failed to flush compressed data");
    compressed_writer
      .finish()
      .expect("Failed to finish compressed writer");
    let compressed_data = buffer_writer.before();
    let decompressed_data = if use_zlib {
      miniz_oxide::inflate::decompress_to_vec_zlib(&compressed_data)
    } else {
      miniz_oxide::inflate::decompress_to_vec(&compressed_data)
    }
    .expect("Failed to decompress data");
    assert_eq!(decompressed_data, uncompressed_data);
  }

  #[test]
  fn test_compressed_writer_writes_correctly_raw() {
    test_compressed_writer(false);
  }

  #[test]
  fn test_compressed_writer_writes_correctly_zlib() {
    test_compressed_writer(true);
  }

  #[test]
  fn test_compressed_writer_writes_correctly_bytewise() {
    let uncompressed_data = b"Hello, world! This is a test of the CompressedWriter.";

    let _reference_compressed_data =
      miniz_oxide::deflate::compress_to_vec_zlib(uncompressed_data, 6);

    let mut buffer_writer = Cursor::new([0; 4096]);
    let mut bytewise_writer = BytewiseWriter::new(&mut buffer_writer);
    let mut compressed_writer = CompressedWriter::new(&mut bytewise_writer, 6, true, 128);
    compressed_writer
      .write_all(uncompressed_data, false)
      .expect("Failed to write uncompressed data to compressed writer");
    // check if it can survive a flush
    compressed_writer
      .flush()
      .unwrap_or_else(|e| panic!("Failed to flush compressed data: {}", e));
    compressed_writer
      .finish()
      .expect("Failed to finish compressed writer");
    let compressed_data = buffer_writer.before();
    let decompressed_data = miniz_oxide::inflate::decompress_to_vec_zlib(&compressed_data)
      .expect("Failed to decompress data");
    assert_eq!(decompressed_data, uncompressed_data);
  }
}
