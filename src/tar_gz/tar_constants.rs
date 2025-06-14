// --- Constants for the TAR Header Format ---
pub const BLOCK_SIZE: usize = 512;

// --- Header Field Lengths ---
pub const NAME_LEN: usize = 100;
pub const MODE_LEN: usize = 8;
pub const UID_LEN: usize = 8;
pub const GID_LEN: usize = 8;
pub const SIZE_LEN: usize = 12;
pub const MTIME_LEN: usize = 12;
pub const CHKSUM_LEN: usize = 8;
pub const TYPEFLAG_LEN: usize = 1;
pub const LINKNAME_LEN: usize = 100;
pub const MAGIC_LEN: usize = 6;
pub const VERSION_LEN: usize = 2;
pub const UNAME_LEN: usize = 32;
pub const GNAME_LEN: usize = 32;

// --- Offsets for Header Fields ---
pub const NAME_OFFSET: usize = 0;
pub const MODE_OFFSET: usize = NAME_OFFSET + NAME_LEN;
pub const UID_OFFSET: usize = MODE_OFFSET + MODE_LEN;
pub const GID_OFFSET: usize = UID_OFFSET + UID_LEN;
pub const SIZE_OFFSET: usize = GID_OFFSET + GID_LEN;
pub const MTIME_OFFSET: usize = SIZE_OFFSET + SIZE_LEN;
pub const CHKSUM_OFFSET: usize = MTIME_OFFSET + MTIME_LEN;
pub const TYPEFLAG_OFFSET: usize = CHKSUM_OFFSET + CHKSUM_LEN;
pub const LINKNAME_OFFSET: usize = TYPEFLAG_OFFSET + TYPEFLAG_LEN;
pub const MAGIC_OFFSET: usize = LINKNAME_OFFSET + LINKNAME_LEN;
pub const VERSION_OFFSET: usize = MAGIC_OFFSET + MAGIC_LEN;
pub const UNAME_OFFSET: usize = VERSION_OFFSET + VERSION_LEN;
pub const GNAME_OFFSET: usize = UNAME_OFFSET + UNAME_LEN;

// --- Combined Magic and Version Values ---
pub const MAGIC_VERSION_V7: &[u8; MAGIC_LEN + VERSION_LEN] = b"\0\0\0\0\0\0\0\0";
/// Shared by `ustar`, `pax/posix` formats.
pub const MAGIC_VERSION_USTAR: &[u8; MAGIC_LEN + VERSION_LEN] = b"ustar\000";
pub const MAGIC_VERSION_GNU: &[u8; MAGIC_LEN + VERSION_LEN] = b"ustar  \0";

/// A block of zeros for padding and end-of-archive markers.
pub const TAR_ZERO_HEADER: [u8; BLOCK_SIZE] = [0; BLOCK_SIZE];

/// # Typeflags:
///
/// ## STANDARD:
///
/// - `0` or `\0` for regular file
/// - `1` for hard link
/// - `2` for symbolic link
/// - `3` for character device
/// - `4` for block device
/// - `5` for directory
/// - `6` for FIFO
/// - `7` for reserved (not used)
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
/// - `D` for GNU dump dir
/// - `M` for GNU multi-volume archive (metadata should be ignored)
/// - `S` for sparse file (old format for sparse files)
/// - `V` for GNU volume header (metadata should be ignored)
/// - `N` for continuation of a sparse file
///
/// ## RARE:
///
/// - `X` for extended header (pre-pax)
#[repr(u8)]
pub enum TarTypeFlag {
  /// Regular file (legacy)
  RegularFileLegacy = b'\0',
  /// Regular file
  RegularFile = b'0',
  /// Hard link
  HardLink = b'1',
  /// Symbolic link
  SymbolicLink = b'2',
  /// Character device
  CharacterDevice = b'3',
  /// Block device
  BlockDevice = b'4',
  /// Directory
  Directory = b'5',
  /// FIFO (named pipe)
  Fifo = b'6',
  /// Extended header `pax`
  ExtendedHeaderPrePax = b'x',
  /// Global extended header `pax`
  GlobalExtendedHeaderPax = b'g',
  /// Long name (GNU)
  LongNameGnu = b'L',
  /// Long link name (GNU)
  LongLinkNameGnu = b'K',
}

pub struct TarHeaderRaw {
  /// File name, null-terminated
  pub name: [u8; 100],
  /// File mode (octal), stored as ASCII bytes
  pub mode: [u8; 8],
  /// User ID of file owner (octal), stored as ASCII bytes
  pub uid: [u8; 8],
  /// Group ID of file owner (octal), stored as ASCII bytes
  pub gid: [u8; 8],
  /// File size in bytes (octal), stored as ASCII bytes
  pub size: [u8; 12],
  /// Modification time (epoch seconds, octal), stored as ASCII bytes
  pub mtime: [u8; 12],
  /// Header checksum (space-padded), stored as ASCII bytes
  pub chksum: [u8; 8],
  /// File type flag (e.g., 0 = file, 5 = directory)
  pub typeflag: u8,
  /// Target name of a symbolic link, null-terminated
  pub linkname: [u8; 100],
  // `v7` ends here and magic is 0
  // `ustar` starts here and has the magic string "ustar\0" or "ustar"
  // `gnu` starts here and has the magic string "ustar "
  // `ustar` has version "00"
  // `gnu` has version " \0"
  /// Combined field for magic[4] and version[2].
  pub magic_and_version: [u8; 8],
  /// User name, null-terminated
  pub uname: [u8; 32],
  /// Group name, null-terminated
  pub gname: [u8; 32],
  /// Major device number (octal), stored as ASCII bytes
  pub devmajor: [u8; 8],
  /// Minor device number (octal), stored as ASCII bytes
  pub devminor: [u8; 8],
  /// Path prefix used if name exceeds 100 bytes, null-terminated
  pub prefix: [u8; 155],
  /// Unused padding to fill the 512-byte header block
  pub padding: [u8; 12],
}
