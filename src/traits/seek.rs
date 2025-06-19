/// Enumeration of possible methods to seek within an I/O object.
///
/// It is used by the [`Seek`] trait.
#[derive(Copy, PartialEq, Eq, Clone, Debug)]
pub enum SeekFrom {
  Start(usize),
  End(isize),
  Current(isize),
}

/// The `Seek` trait provides a cursor which can be moved within a stream of bytes.
pub trait Seek {
  type SeekError;

  /// Seek to an offset, in bytes, in a stream.
  ///
  /// A seek beyond the end of a stream is allowed, but behavior is defined
  /// by the implementation.
  ///
  /// If the seek operation completed successfully,
  /// this method returns the new position from the start of the stream.
  /// That position can be used later with [`SeekFrom::Start`].
  ///
  /// # Errors
  ///
  /// Seeking can fail, for example because it might involve flushing a buffer.
  fn seek(&mut self, offset: SeekFrom) -> Result<usize, Self::SeekError>;
}
