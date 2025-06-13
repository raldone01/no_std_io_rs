use alloc::{boxed::Box, format, string::String, vec, vec::Vec};

use miniz_oxide::{
  deflate::{
    core::{create_comp_flags_from_zip_params, CompressorOxide},
    stream::deflate,
  },
  MZFlush,
};

use crate::{
  dynamic_error::DynamicError,
  no_std_io::{IoError, Write},
};

pub struct CompressedWriter<W: Write> {
  compressor: CompressorOxide,
  target_writer: W,
  finished: bool,
  pending_flush: MZFlush,
  tmp_buffer: Vec<u8>,
}

impl<W: Write> CompressedWriter<W> {
  #[must_use]
  pub fn new(writer: W, level: u8, zlib_wrapped: bool, tmp_buffer_size: usize) -> Self {
    // use zlib wrapper (window bits == 1)
    let flags = create_comp_flags_from_zip_params(level.into(), zlib_wrapped as i32, 0);
    Self {
      compressor: CompressorOxide::new(flags),
      target_writer: writer,
      finished: false,
      pending_flush: MZFlush::None,
      tmp_buffer: vec![0_u8; tmp_buffer_size],
    }
  }

  #[must_use]
  fn strongest_flush(flush_a: MZFlush, flush_b: MZFlush) -> MZFlush {
    use MZFlush::*;
    // Order from weakest to strongest
    if matches!((flush_a, flush_b), (Finish, _) | (_, Finish)) {
      Finish
    } else if matches!((flush_a, flush_b), (Full, _) | (_, Full)) {
      Full
    } else if matches!((flush_a, flush_b), (Sync, _) | (_, Sync)) {
      Sync
    } else if matches!((flush_a, flush_b), (Partial, _) | (_, Partial)) {
      Partial
    } else if matches!((flush_a, flush_b), (Block, _) | (_, Block)) {
      Block
    } else {
      None
    }
  }

  #[must_use]
  pub fn wants_to_sync(&self) -> bool {
    self.pending_flush != MZFlush::None
  }

  fn write_internal(&mut self, input_buffer: &[u8], flush: MZFlush) -> Result<(), IoError> {
    self.pending_flush = Self::strongest_flush(self.pending_flush, flush);

    let result = deflate(
      &mut self.compressor,
      input_buffer,
      self.tmp_buffer.as_mut_slice(),
      self.pending_flush,
    );
    if result.bytes_consumed != input_buffer.len() {
      return Err(IoError::Io(Box::new(DynamicError(format!(
        "Deflate consumed {} bytes, expected {}: {:?}",
        result.bytes_consumed,
        input_buffer.len(),
        result,
      )))));
    }
    if result.bytes_written > 0 {
      // Write the compressed data to the target buffer
      self.target_writer.write(
        &self.tmp_buffer[..result.bytes_written],
        self.wants_to_sync(),
      )?;
    }
    result.status.map_err(|e| {
      IoError::Io(Box::new(DynamicError(format!(
        "Compression error: {:?}",
        e
      ))))
    })?;
    self.pending_flush = MZFlush::None;
    Ok(())
  }

  #[must_use]
  pub fn is_finished(&self) -> bool {
    self.finished
  }
}

impl<W: Write> Write for CompressedWriter<W> {
  fn write(&mut self, buffer_input: &[u8], sync_hint: bool) -> Result<(), IoError> {
    if self.finished {
      return Err(IoError::Io(Box::new(DynamicError(String::from(
        "Writer is already finished",
      )))));
    }
    let flush = if sync_hint {
      MZFlush::Sync
    } else {
      MZFlush::None
    };
    self.write_internal(buffer_input, flush)
  }

  fn flush(&mut self) -> Result<(), IoError> {
    let result = self.write_internal(&[], MZFlush::Finish);
    if result.is_err() {
      return result;
    }
    self.finished = true;
    result
  }
}

#[cfg(test)]
mod tests {
  use crate::writer_buffer::BufferWriter;

  use super::*;

  fn run_compressed_writer_test(use_zlib: bool) {
    let uncompressed_data = b"Hello, world! This is a test of the CompressedWriter.";

    let _reference_compressed_data = if use_zlib {
      miniz_oxide::deflate::compress_to_vec_zlib(uncompressed_data, 6)
    } else {
      miniz_oxide::deflate::compress_to_vec(uncompressed_data, 6)
    };

    let output_buffer = BufferWriter::new(128);
    let mut compressed_writer = CompressedWriter::new(output_buffer, 6, use_zlib, 128);
    compressed_writer
      .write(uncompressed_data, false)
      .expect("Failed to write compressed data");
    compressed_writer
      .flush()
      .expect("Failed to flush compressed data");
    let compressed_data = compressed_writer.target_writer.to_vec();
    let decompressed_data = if use_zlib {
      miniz_oxide::inflate::decompress_to_vec_zlib(&compressed_data)
    } else {
      miniz_oxide::inflate::decompress_to_vec(&compressed_data)
    }
    .expect("Failed to decompress data");
    assert_eq!(decompressed_data, uncompressed_data);
  }

  #[test]
  fn test_compressed_writer_raw() {
    run_compressed_writer_test(false);
  }

  #[test]
  fn test_compressed_writer_zlib() {
    run_compressed_writer_test(true);
  }
}
