use alloc::{vec, vec::Vec};
use thiserror::Error;

use crate::no_std_io::{Write, WriteAll as _, WriteAllError};

/// A buffered writer accumulates data until it reaches a certain size before writing it to the target writer.
pub struct BufferedWriter<'a, W: Write + ?Sized> {
  target_writer: &'a mut W,
  buffer: Vec<u8>,
  pos: usize,
  always_chunk: bool,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum BufferedWriterWriteError<WWE, WFE> {
  #[error("Underlying write error: {0:?}")]
  IoWrite(WriteAllError<WWE>),
  #[error("Underlying flush error: {0:?}")]
  IoFlush(WFE),
}

impl<'a, W: Write + ?Sized> BufferedWriter<'a, W> {
  /// Creates a new `BufferedWriter` with the specified chunk buffer size.
  #[must_use]
  pub fn new(target_writer: &'a mut W, chunk_buffer_size: usize, always_chunk: bool) -> Self {
    Self {
      target_writer,
      buffer: vec![0; chunk_buffer_size],
      pos: 0,
      always_chunk,
    }
  }

  /// Flushes the internal buffer to the target writer.
  fn flush_buffer(&mut self, sync_hint: bool) -> Result<(), WriteAllError<W::WriteError>> {
    if self.pos == 0 {
      return Ok(());
    }
    self
      .target_writer
      .write_all(&self.buffer[..self.pos], sync_hint)?;
    self.pos = 0;
    Ok(())
  }
}

impl<'a, W: Write + ?Sized> Write for BufferedWriter<'a, W> {
  type WriteError = BufferedWriterWriteError<W::WriteError, W::FlushError>;
  type FlushError = BufferedWriterWriteError<W::WriteError, W::FlushError>;

  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError> {
    if input_buffer.is_empty() {
      return Ok(0);
    }

    if !self.always_chunk && (input_buffer.len() + self.pos > self.buffer.len()) {
      // Flush the current buffer
      self
        .flush_buffer(sync_hint)
        .map_err(BufferedWriterWriteError::IoWrite)?;
      // Write the input buffer directly to the target writer
      return self
        .target_writer
        .write_all(input_buffer, sync_hint)
        .map(|_| input_buffer.len())
        .map_err(BufferedWriterWriteError::IoWrite);
    }

    // Copy the input buffer into the internal buffer
    let bytes_to_write = core::cmp::min(input_buffer.len(), self.buffer.len() - self.pos);
    self.buffer[self.pos..self.pos + bytes_to_write]
      .copy_from_slice(&input_buffer[..bytes_to_write]);
    self.pos += bytes_to_write;
    if self.pos == self.buffer.len() {
      // If the buffer is full, flush it
      self
        .flush_buffer(sync_hint)
        .map_err(BufferedWriterWriteError::IoWrite)?;
    }
    Ok(bytes_to_write)
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    self
      .flush_buffer(true)
      .map_err(BufferedWriterWriteError::IoWrite)?;
    self
      .target_writer
      .flush()
      .map_err(BufferedWriterWriteError::IoFlush)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::no_std_io::{BytewiseWriter, Cursor};

  #[test]
  fn test_buffered_writer_chunks_correctly_always_chunk() {
    let input_data = b"Hello, world! This is a test of the BufferedWriter.";
    let mut buffer_writer = Cursor::new([0; 128]);
    let mut bytewise_writer = BytewiseWriter::new(&mut buffer_writer);
    let mut buffered_writer = BufferedWriter::new(&mut bytewise_writer, 20, true);
    buffered_writer
      .write_all(input_data, false)
      .unwrap_or_else(|e| panic!("Failed to write data: {}", e));
    buffered_writer
      .flush()
      .expect("Failed to flush buffered writer");
    let written_data = buffer_writer.before();
    assert_eq!(written_data, input_data);
  }

  #[test]
  fn test_buffered_writer_chunks_correctly_chunk_when_necessary() {
    let input_data = b"Hello, world! This is a test of the BufferedWriter.";
    let mut buffer_writer = Cursor::new([0; 128]);
    let mut buffered_writer = BufferedWriter::new(&mut buffer_writer, 20, false);
    buffered_writer
      .write_all(&input_data[..30], false)
      .unwrap_or_else(|e| panic!("Failed to write data: {}", e));
    buffered_writer
      .write_all(&input_data[30..], false)
      .unwrap_or_else(|e| panic!("Failed to write data: {}", e));
    buffered_writer
      .flush()
      .expect("Failed to flush buffered writer");
    let written_data = buffer_writer.before();
    assert_eq!(written_data, input_data);
  }
}
