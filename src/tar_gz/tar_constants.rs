use core::str::Utf8Error;

use thiserror::Error;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::tar_gz::tar_inode::{FilePermissions, Permission};

// --- Constants for the TAR Header Format ---
pub const BLOCK_SIZE: usize = 512;

/// A block of zeros for padding and end-of-archive markers.
pub const TAR_ZERO_HEADER: [u8; BLOCK_SIZE] = [0; BLOCK_SIZE];

/// https://www.gnu.org/software/tar/manual/html_node/Standard.html
/// # Typeflags:
///
/// ## STANDARD:
///
/// - `0` or `\0` for regular file (also called oldgnu)
/// - `1` for hard link
/// - `2` for symbolic link
/// - `3` for character device
/// - `4` for block device
/// - `5` for directory
/// - `6` for FIFO
/// - `7` for continuous file reserved (not used)
///
/// ## PAX:
///
/// - `x` for extended header (precedes the file it is associated with) also known as `pax`
/// - `g` for global extended header (applies to all following entries)
///
/// ## GNU:
///
/// - `L` for long name
/// - `K` for long link name
/// - `A` for GNU contiguous archive (obsolete)
/// - `D` for GNU dump dir contains a list of file names that were in the directory
/// - `M` for GNU multi-volume archive. This is a continuation of a file that began on another volume.
/// - `S` for sparse file (old format for sparse files)
/// - `V` for GNU volume header (metadata should be ignored)
/// - `N` for continuation of a sparse file
///
/// ## RARE:
///
/// - `X` for solaris extended header (pre-pax)
#[derive(Eq, Hash, PartialEq)]
pub enum TarTypeFlag {
  /// Regular file
  RegularFile,
  /// Hard link
  HardLink,
  /// Symbolic link
  SymbolicLink,
  /// Character device
  CharacterDevice,
  /// Block device
  BlockDevice,
  /// Directory
  Directory,
  /// FIFO (named pipe)
  Fifo,
  /// Extended header `pax`
  ExtendedHeaderPrePax,
  /// Global extended header `pax`
  GlobalExtendedHeaderPax,
  /// GNU extension - long file name
  LongNameGnu,
  /// GNU extension - long link name (link target)
  LongLinkNameGnu,
  /// GNU extension - sparse file
  SparseGnu,
  UnknownTypeFlag(u8),
}

impl From<u8> for TarTypeFlag {
  fn from(value: u8) -> Self {
    match value {
      b'\0' | b'0' => TarTypeFlag::RegularFile,
      b'1' => TarTypeFlag::HardLink,
      b'2' => TarTypeFlag::SymbolicLink,
      b'3' => TarTypeFlag::CharacterDevice,
      b'4' => TarTypeFlag::BlockDevice,
      b'5' => TarTypeFlag::Directory,
      b'6' => TarTypeFlag::Fifo,
      b'7' => TarTypeFlag::RegularFile,
      b'x' => TarTypeFlag::ExtendedHeaderPrePax,
      b'g' => TarTypeFlag::GlobalExtendedHeaderPax,
      b'L' => TarTypeFlag::LongNameGnu,
      b'K' => TarTypeFlag::LongLinkNameGnu,
      b'S' => TarTypeFlag::SparseGnu,
      _ => TarTypeFlag::UnknownTypeFlag(value),
    }
  }
}

impl From<TarTypeFlag> for u8 {
  fn from(value: TarTypeFlag) -> Self {
    match value {
      TarTypeFlag::RegularFile => b'\0',
      TarTypeFlag::HardLink => b'1',
      TarTypeFlag::SymbolicLink => b'2',
      TarTypeFlag::CharacterDevice => b'3',
      TarTypeFlag::BlockDevice => b'4',
      TarTypeFlag::Directory => b'5',
      TarTypeFlag::Fifo => b'6',
      TarTypeFlag::ExtendedHeaderPrePax => b'x',
      TarTypeFlag::GlobalExtendedHeaderPax => b'g',
      TarTypeFlag::LongNameGnu => b'L',
      TarTypeFlag::LongLinkNameGnu => b'K',
      TarTypeFlag::SparseGnu => b'S',
      TarTypeFlag::UnknownTypeFlag(value) => value,
    }
  }
}

fn null_terminated_end(bytes: &[u8]) -> usize {
  bytes
    .iter()
    .position(|&b| b == b'\0')
    .unwrap_or(bytes.len())
}

#[derive(Error, Debug)]
pub enum ParseOctalError {
  #[error("Invalid UTF-8 in octal string: {0}")]
  InvalidUtf8(#[from] Utf8Error),
  #[error("Failed to parse octal number: {0}")]
  ParseIntError(#[from] core::num::ParseIntError),
}

/// Parses a null-terminated, space-padded octal number from a byte slice.
fn parse_octal(bytes: &[u8]) -> Result<u64, ParseOctalError> {
  let end = null_terminated_end(bytes);
  let s = str::from_utf8(&bytes[..end]).map_err(|err| ParseOctalError::InvalidUtf8(err))?;
  u64::from_str_radix(s.trim(), 8).map_err(|err| ParseOctalError::ParseIntError(err))
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
/// Also known as `v7`
#[repr(C)]
pub struct OldHeader {
  /// File name, null-terminated
  pub name_bytes: [u8; 100],
  /// File mode (octal), stored as ASCII bytes
  pub mode: [u8; 8],
  /// User ID of file owner (octal), stored as ASCII bytes
  pub uid: [u8; 8],
  /// Group ID of file owner (octal), stored as ASCII bytes
  pub gid: [u8; 8],
  /// File size in bytes (octal), stored as ASCII bytes
  ///
  /// After the header block, not including the header itself.
  pub size: [u8; 12],
  /// Modification time (epoch seconds, octal), stored as ASCII bytes
  pub mtime: [u8; 12],
  /// Header checksum (space-padded), stored as ASCII bytes
  pub checksum: [u8; 8],
  /// File type flag (e.g., 0 = file, 5 = directory)
  pub typeflag: u8,
  /// Target name of a symbolic link, null-terminated
  pub linkname: [u8; 100],
  /// While technically this field is not part of the `v7` format,
  /// it is all zeros and used to distinguish it from other formats.
  ///
  /// Also it is usually made up of [u8; 6] for the magic string
  /// and [u8; 2] for the version string.
  /// However, we use [u8; 8] to simplify the structure.
  /// They are never used independently anyway.
  pub magic_version: [u8; 8],
  /// UstarHeaderAdditions or GnuHeaderAdditions or just zeros.
  pub padding: [u8; 247],
}

impl OldHeader {
  /// Used by the old `v7` format.
  pub const MAGIC_VERSION_V7: &[u8; 8] = b"\0\0\0\0\0\0\0\0";
  /// Shared by `ustar`, `pax` and `posix` formats.
  pub const MAGIC_VERSION_USTAR: &[u8; 8] = b"ustar\000";
  /// Used by the GNU format.
  pub const MAGIC_VERSION_GNU: &[u8; 8] = b"ustar  \0";

  pub fn parse_name(&self) -> Result<&str, Utf8Error> {
    let end = null_terminated_end(&self.name_bytes);
    str::from_utf8(&self.name_bytes[..end])
  }

  #[must_use]
  pub fn parse_mode(&self) -> Option<FilePermissions> {
    FilePermissions::parse_octal_ascii_unix_mode(&self.mode)
  }

  pub fn parse_uid(&self) -> Result<u32, ParseOctalError> {
    parse_octal(&self.uid).map(|uid| uid as u32)
  }

  pub fn parse_gid(&self) -> Result<u32, ParseOctalError> {
    parse_octal(&self.gid).map(|gid| gid as u32)
  }

  pub fn parse_size(&self) -> Result<u64, ParseOctalError> {
    parse_octal(&self.size)
  }

  pub fn parse_mtime(&self) -> Result<u64, ParseOctalError> {
    parse_octal(&self.mtime)
  }

  /// Computes the checksum of a TAR header according to the ustar spec.
  /// The checksum field (offsets 148..156) must be treated as if it were filled with ASCII spaces (0x20).
  pub fn compute_header_checksum(&self) -> u32 {
    let header = self.as_bytes();
    const CHECKSUM_START: usize = 148;
    const CHECKSUM_END: usize = 156;

    header
      .iter()
      .enumerate()
      .map(|(i, &byte)| {
        if i >= CHECKSUM_START && i < CHECKSUM_END {
          0x20_u32 // ASCII space
        } else {
          byte as u32
        }
      })
      .sum()
  }

  pub fn verify_checksum(&self) -> Result<u32, TarHeaderChecksumError> {
    let checksum = self.compute_header_checksum();
    let expected_checksum = parse_octal(&self.checksum)? as u32;

    if checksum == expected_checksum {
      Ok(checksum)
    } else {
      Err(TarHeaderChecksumError::WrongChecksum {
        expected: expected_checksum,
        actual: checksum,
      })
    }
  }

  #[must_use]
  pub fn parse_typeflag(&self) -> TarTypeFlag {
    self.typeflag.into()
  }

  pub fn parse_linkname(&self) -> Result<&str, Utf8Error> {
    let end = null_terminated_end(&self.linkname);
    str::from_utf8(&self.linkname[..end])
  }
}

#[derive(Error, Debug)]
pub enum TarHeaderChecksumError {
  #[error("Corrupt header: Invalid checksum expected {expected:?} but got {actual:?}")]
  WrongChecksum { expected: u32, actual: u32 },
  #[error("Failed to parse octal number from checksum field: {0}")]
  ParseOctalError(#[from] ParseOctalError),
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct UstarHeaderAdditions {
  // Fields following the version field of the OldHeader
  /// User name, null-terminated
  pub uname: [u8; 32],
  /// Group name, null-terminated
  pub gname: [u8; 32],
  /// Major device number (octal), stored as ASCII bytes
  pub dev_major: [u8; 8],
  /// Minor device number (octal), stored as ASCII bytes
  pub dev_minor: [u8; 8],
  /// Path prefix used if name exceeds 100 bytes, null-terminated
  pub prefix: [u8; 155],
  pub pad: [u8; 12],
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct GnuHeaderAdditions {
  // Fields following the version field of the OldHeader
  /// User name, null-terminated
  pub uname: [u8; 32],
  /// Group name, null-terminated
  pub gname: [u8; 32],
  /// Major device number (octal), stored as ASCII bytes
  pub dev_major: [u8; 8],
  /// Minor device number (octal), stored as ASCII bytes
  pub dev_minor: [u8; 8],
  pub atime: [u8; 12],
  pub ctime: [u8; 12],
  /// Only relevant for multi-volume archives.
  /// It is the offset of the start of this volume.
  pub offset: [u8; 12],
  pub longnames: [u8; 4],
  pub unused: [u8; 1],
  pub sparse: [GnuSparseHeader; 4],
  pub isextended: [u8; 1],
  pub realsize: [u8; 12],
  pub pad: [u8; 17],
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct GnuSparseHeader {
  /// Offset of the beginning of the chunk.
  pub offset: [u8; 12],
  /// Size of the chunk.
  pub num_bytes: [u8; 12],
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct GnuExtSparseHeader {
  pub sparse: [GnuSparseHeader; 21],
  pub isextended: [u8; 1],
  pub padding: [u8; 7],
}

/// https://www.open-std.org/jtc1/sc22/open/n4217.pdf
///
///
/// # PaxTime:
/// A dot is used for fractional seconds, e.g. `123456789.123456789`
/// Represented as decimal.
mod pax_keys_well_known {
  /// GNU sparse: https://www.gnu.org/software/tar/manual/html_section/Sparse-Formats.html
  ///
  /// For version 1.0 the sparse map is stored in the data section of the file.
  /// Series of decimal numbers delimited by '\n'.
  /// The first number gives the number of maps in the file.
  /// Each map is a pair of numbers: the offset in the file and the size of the data at that offset.
  /// The map is padded to the next 512 byte block boundary.
  mod gnu {
    /// Overrides the `name` field of the header. (0.0, 0.1, 1.0)
    const GNU_SPARSE_NAME: &str = "GNU.sparse.name";
    /// Overrides the real size of the file. (1.0)
    const GNU_SPARSE_REALSIZE: &str = "GNU.sparse.realsize";
    /// Version 1.0 this is the 1
    const GNU_SPARSE_MAJOR: &str = "GNU.sparse.major";
    /// Version 1.0 this is the 0
    const GNU_SPARSE_MINOR: &str = "GNU.sparse.minor";

    /// Overrides the real size of the file for old GNU sparse files. (0.0, 0.1)
    const GNU_SPARSE_REALSIZE_OLD: &str = "GNU.sparse.size";
    /// Number of blocks in the sparse map. (0.0, 0.1)
    /// After that the following fields are repeated numblocks times:
    /// * GNU_SPARSE_DATA_BLOCK_OFFSET
    /// * GNU_SPARSE_DATA_BLOCK_SIZE
    const GNU_SPARSE_MAP_NUM_BLOCKS: &str = "GNU.sparse.numblocks";
    /// Offset of the data block. (0.0, 0.1)
    const GNU_SPARSE_DATA_BLOCK_OFFSET: &str = "GNU.sparse.offset";
    /// Size of the data block. (0.0, 0.1)
    const GNU_SPARSE_DATA_BLOCK_SIZE: &str = "GNU.sparse.numbytes";
    /// The sparse map is a series of comma-separated values
    /// in the format `offset,size[,offset:size,...]`
    const GNU_SPARSE_MAP: &str = "GNU.sparse.map";
  }
  const ATIME: &str = "atime";
  const CHARSET: &str = "charset";
  const COMMENT: &str = "comment";
  /// Overrides the gid for files whose id is greater than `2 097 151 (octal 7 777 777)`.
  ///
  /// Stored in decimal format.
  const GID: &str = "gid";
  /// Overrides the `gname` field of the header.
  const GNAME: &str = "gname";
  /// Stores the charset used to encode `gname`, `linkname`, `path`, `uname` in the extended header.
  ///
  /// Standardized values: ISO-IR∆10646∆2000∆UTF-8
  ///
  /// BINARY might be anything?
  const HDRCHARSET: &str = "hdrcharset";
  /// Overrides the linkname of the header.
  const LINKPATH: &str = "linkpath";
  const MTIME: &str = "mtime";
  /// Overrides the `name` and `prefix` fields of the header.
  const PATH: &str = "path";
  /// Overrides the size of the header.
  /// Size of the file in bytes, decimal format.
  /// When size greater than `8 589 934 591 (octal 77 777 777 777)`.
  const SIZE: &str = "size";
  /// Overrides the uid for files whose id is greater than `2 097 151 (octal 7 777 777)`.
  ///
  /// Stored in decimal format.
  const UID: &str = "uid";
  /// Overrides the `uname` field of the header.
  const UNAME: &str = "uname";
}
