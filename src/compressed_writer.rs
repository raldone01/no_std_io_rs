use alloc::vec::Vec;

use miniz_oxide::{
  deflate::{
    core::{create_comp_flags_from_zip_params, CompressorOxide},
    stream::deflate,
  },
  MZError,
};

pub struct CompressedWriter {
  compressor: CompressorOxide,
  buffer: Vec<u8>,
  cursor: usize,
}

impl CompressedWriter {
  #[must_use]
  pub fn new(level: u8) -> Self {
    // use zlib wrapper (window bits == 1)
    let flags = create_comp_flags_from_zip_params(level.into(), 1, 0);
    Self {
      compressor: CompressorOxide::new(flags),
      buffer: Vec::new(),
      cursor: 0,
    }
  }

  fn write_internal(&mut self, data: &[u8], flush: miniz_oxide::MZFlush) -> Result<(), MZError> {
    let mut result = deflate(&mut self.compressor, data, &mut self.buffer, flush);
    self.cursor += result.bytes_written;
    while result.status == Err(MZError::Buf) {
      // Ensure we have enough space in the buffer
      let additional_space = self.buffer.len() / 2 + 1;
      self.buffer.resize(additional_space, 0);
      result = deflate(&mut self.compressor, data, &mut self.buffer, flush);
      self.cursor += result.bytes_written;
    }
    result.status.map(|_| ())
  }

  pub fn write(&mut self, data: &[u8], sync: bool) -> Result<(), MZError> {
    let flush = if sync {
      miniz_oxide::MZFlush::Sync
    } else {
      miniz_oxide::MZFlush::None
    };
    self.write_internal(data, flush)
  }

  pub fn finish(mut self) -> Result<Vec<u8>, MZError> {
    // Flush the compressor to ensure all data is written
    let flush_result = self.write_internal(&[], miniz_oxide::MZFlush::Finish);
    if let Err(e) = flush_result {
      return Err(e);
    }

    // Return the compressed data
    self.buffer.truncate(self.cursor);
    Ok(self.buffer)
  }
}
