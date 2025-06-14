use alloc::{string::String, vec::Vec};
use core::fmt::Write;

use miniz_oxide::MZError;

use crate::{
  compressed_writer::CompressedWriter,
  no_std_io::{IoError, Write as _},
  tar_constants::ustar::*,
};

/// Helper function to write an octal number into a fixed-size field in the TAR header.
/// The field is zero-padded on the left and null-terminated.
fn write_octal_field(field: &mut [u8], value: u64) {
  let mut buffer = String::with_capacity(field.len());
  // Format as octal, ensuring it fits, and add a null terminator.
  // Example: for a size-8 field, we format to 7 chars and add '\0'.
  write!(buffer, "{:0>width$o}\0", value, width = field.len() - 1)
    .expect("Formatting to a pre-allocated string should not fail");

  let bytes = buffer.as_bytes();
  field[..bytes.len()].copy_from_slice(bytes);
}

/// Creates a USTAR-formatted TAR header.
fn create_tar_header(path: &str, file_size: u64) -> [u8; BLOCK_SIZE] {
  let mut header = [0u8; BLOCK_SIZE];

  // --- Filename (positions 0-99) ---
  let path_bytes = path.as_bytes();
  // Truncate if path is too long
  let name_len = path_bytes.len().min(NAME_LEN);
  header[NAME_OFFSET..NAME_OFFSET + name_len].copy_from_slice(&path_bytes[..name_len]);

  // --- File Mode (positions 100-107) ---
  // `0o100644` -> regular file with rw-r--r-- permissions
  write_octal_field(&mut header[MODE_OFFSET..MODE_OFFSET + MODE_LEN], 0o644);

  // --- UID & GID (positions 108-115, 116-123) ---
  // Set to 0 (root) as a sensible default
  write_octal_field(&mut header[UID_OFFSET..UID_OFFSET + UID_LEN], 0);
  write_octal_field(&mut header[GID_OFFSET..GID_OFFSET + GID_LEN], 0);

  // --- File Size (positions 124-135) ---
  write_octal_field(&mut header[SIZE_OFFSET..SIZE_OFFSET + SIZE_LEN], file_size);

  // --- Modification Time (positions 136-147) ---
  // Set to 0 as we don't have access to a system clock in `no_std` easily.
  write_octal_field(&mut header[MTIME_OFFSET..MTIME_OFFSET + MTIME_LEN], 0);

  // --- Type Flag (position 156) ---
  header[TYPEFLAG_OFFSET] = TYPEFLAG_NORMAL_FILE;

  // --- USTAR Magic & Version (positions 257-262, 263-264) ---
  header[MAGIC_OFFSET..MAGIC_OFFSET + MAGIC_LEN].copy_from_slice(MAGIC);
  header[VERSION_OFFSET..VERSION_OFFSET + VERSION_LEN].copy_from_slice(VERSION);

  // --- Owner/Group Names (positions 265-296, 297-328) ---
  // Leave as null-terminated empty strings.

  // --- Checksum (positions 148-155) ---
  // This is calculated last. First, we fill the checksum field with spaces.
  let chksum_slice = &mut header[CHKSUM_OFFSET..CHKSUM_OFFSET + CHKSUM_LEN];
  chksum_slice.fill(b' ');

  // Then, calculate the sum of all bytes in the header.
  let checksum_val: u64 = header.iter().map(|&byte| u64::from(byte)).sum();

  // Finally, write the checksum value back into its field as an octal string.
  write_octal_field(
    &mut header[CHKSUM_OFFSET..CHKSUM_OFFSET + CHKSUM_LEN],
    checksum_val,
  );

  header
}

pub struct TarBuilder {
  compressed_writer: CompressedWriter,
}

impl TarBuilder {
  #[must_use]
  pub fn new() -> Self {
    Self {
      compressed_writer: CompressedWriter::new(6), // Default compression level
    }
  }

  /// Adds a file to the TAR archive.
  ///
  /// # Arguments
  /// * `path` - The full path of the file within the archive. Must be ASCII.
  /// * `data` - The raw byte content of the file.
  ///
  /// # Errors
  /// Returns `IoError` if compression fails.
  pub fn add_file(&mut self, path: &str, data: &[u8]) -> Result<(), IoError> {
    // 1. Create the TAR header for this file.
    let header = create_tar_header(path, data.len() as u64);

    // 2. Write the header to the compressed stream.
    self.compressed_writer.write(&header, false)?;

    // 3. Write the file data to the compressed stream.
    self.compressed_writer.write(data, false)?;

    // 4. Write padding to align the end of the file data to a 512-byte boundary.
    let data_len = data.len();
    let padding_size = (BLOCK_SIZE - (data_len % BLOCK_SIZE)) % BLOCK_SIZE;
    if padding_size > 0 {
      self
        .compressed_writer
        .write(&ZERO_BLOCK[..padding_size], true)?;
    }

    Ok(())
  }

  /// Finalizes the archive and returns the compressed bytes.
  /// This writes the mandatory end-of-archive marker (two zero blocks).
  pub fn finish(mut self) -> Result<Vec<u8>, IoError> {
    // A TAR archive is terminated by two 512-byte zero blocks.
    self.compressed_writer.write(&ZERO_BLOCK, false)?;
    self.compressed_writer.write(&ZERO_BLOCK, false)?;

    self.compressed_writer.flush()?;
    self.compressed_writer.get_compressed_data()
  }
}
