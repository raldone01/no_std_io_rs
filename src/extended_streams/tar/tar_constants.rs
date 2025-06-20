use core::str::Utf8Error;

use thiserror::Error;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::extended_streams::tar::{FilePermissions, SparseFileInstruction, TimeStamp};

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
#[derive(Debug, Eq, Hash, PartialEq, Clone)]
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
  /// Indicates that this is a continuous file,
  ContinuousFile,
  /// Extended header `pax`
  PaxExtendedHeader,
  /// Global extended header `pax`
  PaxGlobalExtendedHeader,
  /// GNU extension - long file name
  LongNameGnu,
  /// GNU extension - long link name (link target)
  LongLinkNameGnu,
  /// GNU extension - sparse file
  SparseOldGnu,
  UnknownTypeFlag(u8),
}

impl TarTypeFlag {
  #[must_use]
  pub fn is_file_like(&self) -> bool {
    matches!(
      self,
      TarTypeFlag::RegularFile
        | TarTypeFlag::HardLink
        | TarTypeFlag::SymbolicLink
        | TarTypeFlag::CharacterDevice
        | TarTypeFlag::BlockDevice
        | TarTypeFlag::Directory
        | TarTypeFlag::Fifo
        | TarTypeFlag::ContinuousFile
        | TarTypeFlag::SparseOldGnu
    )
  }

  #[must_use]
  pub fn is_link_like(&self) -> bool {
    matches!(self, TarTypeFlag::HardLink | TarTypeFlag::SymbolicLink)
  }
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
      b'7' => TarTypeFlag::ContinuousFile,
      b'x' => TarTypeFlag::PaxExtendedHeader,
      b'g' => TarTypeFlag::PaxGlobalExtendedHeader,
      b'L' => TarTypeFlag::LongNameGnu,
      b'K' => TarTypeFlag::LongLinkNameGnu,
      b'S' => TarTypeFlag::SparseOldGnu,
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
      TarTypeFlag::ContinuousFile => b'7',
      TarTypeFlag::PaxExtendedHeader => b'x',
      TarTypeFlag::PaxGlobalExtendedHeader => b'g',
      TarTypeFlag::LongNameGnu => b'L',
      TarTypeFlag::LongLinkNameGnu => b'K',
      TarTypeFlag::SparseOldGnu => b'S',
      TarTypeFlag::UnknownTypeFlag(value) => value,
    }
  }
}

pub(crate) fn find_null_terminator_index(bytes: &[u8]) -> usize {
  bytes
    .iter()
    .position(|&b| b == b'\0')
    .unwrap_or(bytes.len())
}

pub fn parse_null_terminated_string(bytes: &[u8]) -> Result<&str, Utf8Error> {
  let end = find_null_terminator_index(bytes);
  str::from_utf8(&bytes[..end])
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ParseOctalError {
  #[error("Invalid UTF-8 in octal string: {0}")]
  InvalidUtf8(#[from] Utf8Error),
  #[error("Failed to parse octal number: {0}")]
  ParseIntError(#[from] core::num::ParseIntError),
}

/// Parses a null-terminated, space-padded octal number from a byte slice.
fn parse_octal(bytes: &[u8]) -> Result<u64, ParseOctalError> {
  let s = parse_null_terminated_string(&bytes).map_err(|err| ParseOctalError::InvalidUtf8(err))?;
  u64::from_str_radix(s.trim(), 8).map_err(|err| ParseOctalError::ParseIntError(err))
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
/// Also known as `v7`
#[repr(C)]
pub struct V7Header {
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
  /// [`CommonHeaderAdditions`] if `magic_version` matches or just zeros.
  pub padding: [u8; 247],
}

impl V7Header {
  /// Used by the old `v7` format.
  pub const MAGIC_VERSION_V7: &[u8; 8] = b"\0\0\0\0\0\0\0\0";
  /// Shared by `ustar`, `pax` and `posix` formats.
  pub const MAGIC_VERSION_USTAR: &[u8; 8] = b"ustar\000";
  /// Used by the GNU format.
  pub const MAGIC_VERSION_GNU: &[u8; 8] = b"ustar  \0";

  pub fn parse_name(&self) -> Result<&str, Utf8Error> {
    parse_null_terminated_string(&self.name_bytes)
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

  pub fn parse_size(&self) -> Result<u32, ParseOctalError> {
    parse_octal(&self.size).map(|size| size as u32)
  }

  pub fn parse_mtime(&self) -> Result<TimeStamp, ParseOctalError> {
    parse_octal(&self.mtime).map(|mtime| TimeStamp {
      seconds_since_epoch: mtime,
      nanoseconds: 0,
    })
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
    parse_null_terminated_string(&self.linkname)
  }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum TarHeaderChecksumError {
  #[error("Corrupt header: Invalid checksum expected {expected:?} but got {actual:?}")]
  WrongChecksum { expected: u32, actual: u32 },
  #[error("Failed to parse octal number from checksum field: {0}")]
  ParseOctalError(#[from] ParseOctalError),
}

/// Fields contained in the padding of the [`V7Header`].
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct CommonHeaderAdditions {
  /// User name, null-terminated
  pub uname: [u8; 32],
  /// Group name, null-terminated
  pub gname: [u8; 32],
  /// Major device number (octal), stored as ASCII bytes
  pub dev_major: [u8; 8],
  /// Minor device number (octal), stored as ASCII bytes
  pub dev_minor: [u8; 8],
  /// [`UstarHeaderAdditions`] or [`GnuHeaderAdditions`].
  pub padding: [u8; 167],
}

impl CommonHeaderAdditions {
  pub fn parse_uname(&self) -> Result<&str, Utf8Error> {
    parse_null_terminated_string(&self.uname)
  }
  pub fn parse_gname(&self) -> Result<&str, Utf8Error> {
    parse_null_terminated_string(&self.gname)
  }
  pub fn parse_dev_major(&self) -> Result<u32, ParseOctalError> {
    parse_octal(&self.dev_major).map(|v| v as u32)
  }
  pub fn parse_dev_minor(&self) -> Result<u32, ParseOctalError> {
    parse_octal(&self.dev_minor).map(|v| v as u32)
  }
}

/// Fields contained in the padding of the [`CommonHeaderAdditions`].
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct UstarHeaderAdditions {
  /// Path prefix used if name exceeds 100 bytes, null-terminated
  pub prefix: [u8; 155],
  pub pad: [u8; 12],
}

impl UstarHeaderAdditions {
  pub fn parse_prefix(&self) -> Result<&str, Utf8Error> {
    parse_null_terminated_string(&self.prefix)
  }
}

/// Fields contained in the padding of the [`CommonHeaderAdditions`].
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub(crate) struct GnuHeaderAdditions {
  /// Access time in octal ASCII, null-terminated (12 bytes)
  pub atime: [u8; 12],
  /// Creation time in octal ASCII, null-terminated (12 bytes)
  pub ctime: [u8; 12],
  /// Offset of the start of this volume in octal ASCII, null-terminated (12 bytes)
  /// Used only for multi-volume tar archives
  pub offset: [u8; 12],
  /// Offset to long name data, or 0 if not used (4 bytes)
  pub longnames: [u8; 4],
  /// Reserved byte; always set to zero (1 byte)
  pub unused: [u8; 1],
  /// List of up to 4 sparse file entries
  pub sparse: [GnuSparseInstruction; 4],
  /// Flag indicating if there are more sparse entries in an extended header (1 byte)
  /// '1' means more headers follow
  pub is_extended: [u8; 1],
  /// Actual size of the file before compression, in octal ASCII (12 bytes)
  pub real_size: [u8; 12],
  /// Unused padding bytes to fill the structure (17 bytes)
  pub padding: [u8; 17],
}

impl GnuHeaderAdditions {
  pub fn parse_atime(&self) -> Result<TimeStamp, ParseOctalError> {
    parse_octal(&self.atime).map(|atime| TimeStamp {
      seconds_since_epoch: atime,
      nanoseconds: 0,
    })
  }

  pub fn parse_ctime(&self) -> Result<TimeStamp, ParseOctalError> {
    parse_octal(&self.ctime).map(|ctime| TimeStamp {
      seconds_since_epoch: ctime,
      nanoseconds: 0,
    })
  }

  pub fn parse_offset(&self) -> Result<u64, ParseOctalError> {
    parse_octal(&self.offset)
  }

  pub fn parse_longnames(&self) -> Result<u32, ParseOctalError> {
    parse_octal(&self.longnames).map(|v| v as u32)
  }

  #[must_use]
  pub fn parse_is_extended(&self) -> bool {
    self.is_extended[0] != 0
  }

  pub fn parse_real_size(&self) -> Result<u64, ParseOctalError> {
    parse_octal(&self.real_size)
  }
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, PartialEq, Eq)]
#[repr(C)]
pub(crate) struct GnuSparseInstruction {
  /// Offset of the beginning of the chunk.
  pub offset: [u8; 12],
  /// Size of the chunk.
  pub num_bytes: [u8; 12],
}

impl GnuSparseInstruction {
  const ZERO_INSTRUCTION: GnuSparseInstruction = GnuSparseInstruction {
    offset: [0; 12],
    num_bytes: [0; 12],
  };

  pub fn parse_offset(&self) -> Result<u64, ParseOctalError> {
    parse_octal(&self.offset)
  }

  pub fn parse_num_bytes(&self) -> Result<u64, ParseOctalError> {
    parse_octal(&self.num_bytes)
  }

  #[must_use]
  pub fn is_empty(&self) -> bool {
    self == &Self::ZERO_INSTRUCTION
  }

  pub fn convert_to_sparse_instruction(&self) -> Result<SparseFileInstruction, ParseOctalError> {
    let offset_before = self.parse_offset()?;
    let data_size = self.parse_num_bytes()?;
    Ok(SparseFileInstruction {
      offset_before,
      data_size,
    })
  }
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub(crate) struct GnuHeaderExtSparse {
  pub sparse: [GnuSparseInstruction; 21],
  pub is_extended: [u8; 1],
  pub padding: [u8; 7],
}

impl GnuHeaderExtSparse {
  #[must_use]
  pub fn parse_is_extended(&self) -> bool {
    self.is_extended[0] != 0
  }
}

/// https://www.open-std.org/jtc1/sc22/open/n4217.pdf
///
///
/// # PaxTime:
/// A dot is used for fractional seconds, e.g. `123456789.123456789`
/// Represented as decimal.
pub mod pax_keys_well_known {
  /// GNU sparse: https://www.gnu.org/software/tar/manual/html_section/Sparse-Formats.html
  ///
  /// For version 1.0 the sparse map is stored in the data section of the file.
  /// Series of decimal numbers delimited by '\n'.
  /// The first number gives the number of maps in the file.
  /// Each map is a pair of numbers: the offset in the file and the size of the data at that offset.
  /// The map is padded to the next 512 byte block boundary.
  pub mod gnu {
    /// Overrides the `name` field of the header. (0.0, 0.1, 1.0)
    pub const GNU_SPARSE_NAME_01_01: &str = "GNU.sparse.name";
    /// Overrides the real size of the file. (1.0)
    pub const GNU_SPARSE_REALSIZE_1_0: &str = "GNU.sparse.realsize";
    /// Version 1.0 this is the 1
    pub const GNU_SPARSE_MAJOR: &str = "GNU.sparse.major";
    /// Version 1.0 this is the 0
    pub const GNU_SPARSE_MINOR: &str = "GNU.sparse.minor";

    /// Overrides the real size of the file for old GNU sparse files. (0.0, 0.1)
    pub const GNU_SPARSE_REALSIZE_0_01: &str = "GNU.sparse.size";
    /// Number of blocks in the sparse map. (0.0, 0.1)
    /// After that the following fields are repeated numblocks times:
    /// * GNU_SPARSE_DATA_BLOCK_OFFSET
    /// * GNU_SPARSE_DATA_BLOCK_SIZE
    pub const GNU_SPARSE_MAP_NUM_BLOCKS_0_01: &str = "GNU.sparse.numblocks";
    /// Offset of the data block. (0.0)
    pub const GNU_SPARSE_DATA_BLOCK_OFFSET_0_0: &str = "GNU.sparse.offset";
    /// Size of the data block. (0.0)
    pub const GNU_SPARSE_DATA_BLOCK_SIZE_0_0: &str = "GNU.sparse.numbytes";
    /// The sparse map is a series of comma-separated decimal values
    /// in the format `offset,size[,offset,size,...]` (0.1)
    pub const GNU_SPARSE_MAP_0_1: &str = "GNU.sparse.map";
  }
  pub const ATIME: &str = "atime";
  /// The character set used to encode the file.
  /// We don't care about this field.
  pub const CHARSET: &str = "charset";
  pub const COMMENT: &str = "comment";
  /// Overrides the gid for files whose id is greater than `2 097 151 (octal 7 777 777)`.
  ///
  /// Stored in decimal format.
  pub const GID: &str = "gid";
  /// Overrides the `gname` field of the header.
  pub const GNAME: &str = "gname";
  /// Stores the charset used to encode `gname`, `linkname`, `path`, `uname` in the extended header.
  ///
  /// Standardized values: ISO-IR∆10646∆2000∆UTF-8
  ///
  /// BINARY might be anything?
  pub const HDRCHARSET: &str = "hdrcharset";
  /// Overrides the linkname of the header.
  pub const LINKPATH: &str = "linkpath";
  pub const MTIME: &str = "mtime";
  /// Non-standard GNU extension.
  pub const CTIME: &str = "ctime";
  /// Overrides the `name` and `prefix` fields of the header.
  pub const PATH: &str = "path";
  /// Overrides the size of the header.
  /// Size of the file in bytes, decimal format.
  /// When size greater than `8 589 934 591 (octal 77 777 777 777)`.
  pub const SIZE: &str = "size";
  /// Overrides the uid for files whose id is greater than `2 097 151 (octal 7 777 777)`.
  ///
  /// Stored in decimal format.
  pub const UID: &str = "uid";
  /// Overrides the `uname` field of the header.
  pub const UNAME: &str = "uname";
}
