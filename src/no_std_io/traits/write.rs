use crate::no_std_io::LimitedWriter;

/// Trait for writing bytes.
pub trait Write {
  type WriteError;
  type FlushError;

  /// Write the contents of `input_buffer` to the underlying device.
  /// Providing an empty `input_buffer` is valid and will return 0 bytes written.
  ///
  /// Returns the number of bytes written.
  /// If `sync_hint` is true, it indicates that the write should be flushed to the actual device.
  fn write(&mut self, input_buffer: &[u8], sync_hint: bool) -> Result<usize, Self::WriteError>;

  /// Flush any buffered data to the underlying device.
  /// Must be called at the end to ensure all data is written.
  fn flush(&mut self) -> Result<(), Self::FlushError>;
}

pub trait WriteLimited: Write {
  /// Creates a new writer that limits the number of bytes written to `write_limit_bytes`.
  ///
  /// Returns a new [`LimitedWriter`] instance.
  #[must_use]
  fn put(&mut self, write_limit_bytes: usize) -> LimitedWriter<'_, Self>;
}

impl<W: Write + ?Sized> WriteLimited for W {
  fn put(&mut self, write_limit_bytes: usize) -> LimitedWriter<'_, Self> {
    LimitedWriter::new(self, write_limit_bytes)
  }
}
