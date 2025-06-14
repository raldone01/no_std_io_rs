use alloc::{boxed::Box, format, string::String, vec::Vec};
use hashbrown::HashMap;

use crate::{dynamic_error::DynamicError, no_std_io::Read, tar_constants::ustar::*};

/// Represents a parsed TAR header containing file metadata.
#[derive(Debug)]
pub struct TarHeader {
  pub path: String,
  pub size: u64,
  /// The type of file entry (e.g., regular file, directory).
  pub typeflag: u8,
}

/// Parses a null-terminated string from a byte slice.
/// Maps parsing errors to `ReadError::Io`.
fn parse_name(bytes: &[u8]) -> Result<String, IoError> {
  let end = bytes
    .iter()
    .position(|&b| b == b'\0')
    .unwrap_or(bytes.len());
  core::str::from_utf8(&bytes[..end])
    .map(String::from)
    .map_err(|err| IoError::Io(Box::new(err)))
}

/// Parses a null-terminated, space-padded octal number from a byte slice.
/// Maps parsing errors to `ReadError::Io`.
fn parse_octal(bytes: &[u8]) -> Result<u64, IoError> {
  let end = bytes
    .iter()
    .position(|&b| b == b'\0')
    .unwrap_or(bytes.len());
  let s = core::str::from_utf8(&bytes[..end]).map_err(|err| IoError::Io(Box::new(err)))?;
  u64::from_str_radix(s.trim(), 8).map_err(|err| IoError::Io(Box::new(err)))
}

/// Parses a 512-byte buffer as a USTAR header.
fn parse_header(header_buf: &[u8]) -> Result<TarHeader, IoError> {
  // Verify the USTAR magic string: "ustar\0"
  if &header_buf[MAGIC_OFFSET..MAGIC_OFFSET + MAGIC_LEN] != MAGIC {
    // This indicates the archive is not in the expected USTAR format or is corrupted.
    return Err(IoError::Io(Box::new(DynamicError(String::from(
      "Invalid USTAR magic string",
    )))));
  }

  // NOTE: A more robust implementation would also verify the header checksum here.

  let path = parse_name(&header_buf[NAME_OFFSET..NAME_OFFSET + NAME_LEN])?;
  let size = parse_octal(&header_buf[SIZE_OFFSET..SIZE_OFFSET + SIZE_LEN])?;
  let typeflag = header_buf[TYPEFLAG_OFFSET];

  Ok(TarHeader {
    path,
    size,
    typeflag,
  })
}

/// A reader for decompressing and parsing a TAR archive on the fly.
///
/// It reads from a compressed data source, decompresses the data, and parses
/// the USTAR (TAR) format to extract files.
struct TarReader<R: Read> {
  // The BufferedReader is crucial for performance. It minimizes calls to the
  // underlying decompressor by reading larger chunks into its internal buffer.
  reader: BufferedReader<R>,
  /// A security limit on the total number of bytes that can be extracted
  /// from the archive to prevent decompression bomb attacks.
  max_extracted_bytes: usize,
  /// The number of bytes extracted so far.
  total_extracted: usize,
}

impl<R: Read> TarReader<R> {
  /// Creates a new `TarReader`.
  ///
  /// # Arguments
  /// * `reader` - A reader that provides compressed TAR data.
  /// * `max_extracted_bytes` - A security limit on the total number of bytes
  ///   that can be extracted from the archive.
  fn new(reader: R, max_extracted_bytes: usize) -> Self {
    let buffer_size = max_extracted_bytes / 8;
    let buffered_reader = BufferedReader::new(buffer_size, reader);

    Self {
      reader: buffered_reader,
      max_extracted_bytes,
      total_extracted: 0,
    }
  }

  /// Reads all files from the archive into a HashMap.
  ///
  /// This method iterates through the entire TAR archive, decompresses it on the fly,
  /// and collects all files into memory. It assumes the `BufferedReader` has a
  /// `read_exact` method to simplify reading fixed-size chunks.
  ///
  /// # Errors
  /// Returns `ReadError` if:
  /// - The archive is malformed or corrupted.
  /// - An I/O error occurs during decompression.
  /// - The total size of extracted files would exceed `max_extracted_bytes`.
  fn read_all_files(&mut self) -> Result<HashMap<String, Vec<u8>>, IoError> {
    let mut files = HashMap::new();

    loop {
      // Attempt to read a full header block.
      let header_buf = match self.reader.read_exact(BLOCK_SIZE) {
        Ok(header_buf) => header_buf,
        // A clean end of the underlying stream is a valid way to end the archive.
        Err(IoError::UnexpectedEof) => break,
        // Any other read error is fatal.
        Err(e) => return Err(e),
      };

      // Check for the end-of-archive marker, which is a block of all zeros.
      // A standard TAR archive ends with two such blocks, but encountering one
      // is a reliable signal to stop reading.
      if header_buf.iter().all(|&b| b == 0) {
        break;
      }

      // If it's not a zero block, parse it as a file header.
      let header = parse_header(&header_buf)?;
      let file_size = header.size as usize;

      // We only care about regular files ('0' or '\0').
      // Other types like directories ('5'), symlinks, etc., are skipped.
      if header.typeflag == TYPEFLAG_REGTYPE || header.typeflag == TYPEFLAG_AREGTYPE {
        // Security check: ensure we don't exceed the extraction limit.
        if self.total_extracted.saturating_add(file_size) > self.max_extracted_bytes {
          return Err(IoError::MemoryLimitExceeded);
        }

        // Read the file's data content.
        let file_data = self.reader.read_exact(file_size)?;
        self.total_extracted += file_size;

        // Store the extracted file.
        files.insert(header.path, file_data.to_vec());
      } else if file_size > 0 {
        // For non-regular files that might have data (e.g. symlinks), skip their data block.
        // For directories, file_size is 0, so this is a no-op.
        let _ = self.reader.read_exact(file_size)?;
      }

      // File data in a TAR archive is padded with null bytes to fill a 512-byte block.
      // We must consume these padding bytes to align the stream for the next header.
      let padding_size = (BLOCK_SIZE - (file_size % BLOCK_SIZE)) % BLOCK_SIZE;
      if padding_size > 0 {
        // Skip padding by reading it into a temporary, throwaway buffer.
        let _padding_buf = self.reader.read_exact(padding_size)?;
      }
    }

    Ok(files)
  }
}

pub fn extract_tar_file(
  tar_gz_data: &[u8],
  max_extracted_bytes: usize,
) -> Result<HashMap<String, Vec<u8>>, IoError> {
  let stripped_compressed_data = strip_gzip_header(tar_gz_data);
  // Try compressed raw DEFLATE reading as a fallback.
  let compressed_reader = CompressedReader::new(stripped_compressed_data, false);
  let mut tar_reader = TarReader::new(compressed_reader, max_extracted_bytes);
  let uncompressed_raw_error = match tar_reader.read_all_files() {
    Ok(files) => return Ok(files),
    Err(e) => e,
  };
  // If the compressed reading fails, try uncompressed reading.
  let buffer_reader = BufferReader::new(tar_gz_data);
  let mut tar_reader = TarReader::new(buffer_reader, max_extracted_bytes);
  let uncompressed_error = match tar_reader.read_all_files() {
    Ok(files) => return Ok(files),
    Err(e) => e,
  };

  Err(IoError::Io(Box::new(DynamicError(format!(
    "Failed to extract TAR file: Compressed Raw error: {:?}, Uncompressed error: {:?}",
    uncompressed_raw_error, uncompressed_error
  )))))
}
