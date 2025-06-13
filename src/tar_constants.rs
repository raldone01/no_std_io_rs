pub mod ustar {
  // --- Constants for the TAR Header Format (USTAR) ---
  pub const BLOCK_SIZE: usize = 512;
  pub const NAME_LEN: usize = 100;
  pub const MODE_LEN: usize = 8;
  pub const UID_LEN: usize = 8;
  pub const GID_LEN: usize = 8;
  pub const SIZE_LEN: usize = 12;
  pub const MTIME_LEN: usize = 12;
  pub const CHKSUM_LEN: usize = 8;
  pub const TYPEFLAG_LEN: usize = 1;
  pub const MAGIC_LEN: usize = 6;
  pub const VERSION_LEN: usize = 2;
  pub const UNAME_LEN: usize = 32;
  pub const GNAME_LEN: usize = 32;

  // Offsets
  pub const NAME_OFFSET: usize = 0;
  pub const MODE_OFFSET: usize = NAME_OFFSET + NAME_LEN;
  pub const UID_OFFSET: usize = MODE_OFFSET + MODE_LEN;
  pub const GID_OFFSET: usize = UID_OFFSET + UID_LEN;
  pub const SIZE_OFFSET: usize = GID_OFFSET + GID_LEN;
  pub const MTIME_OFFSET: usize = SIZE_OFFSET + SIZE_LEN;
  pub const CHKSUM_OFFSET: usize = MTIME_OFFSET + MTIME_LEN;
  pub const TYPEFLAG_OFFSET: usize = CHKSUM_OFFSET + CHKSUM_LEN;
  pub const MAGIC_OFFSET: usize = TYPEFLAG_OFFSET + TYPEFLAG_LEN + 100; // linkname is 100
  pub const VERSION_OFFSET: usize = MAGIC_OFFSET + MAGIC_LEN;
  pub const UNAME_OFFSET: usize = VERSION_OFFSET + VERSION_LEN;
  pub const GNAME_OFFSET: usize = UNAME_OFFSET + UNAME_LEN;

  // Typeflags
  /// Type flag for a regular file (standard).
  pub const TYPEFLAG_REGTYPE: u8 = b'0';
  /// Type flag for a regular file (legacy).
  pub const TYPEFLAG_AREGTYPE: u8 = b'\0';
  /// Type flag for a directory.
  pub const TYPEFLAG_DIRTYPE: u8 = b'5';

  // USTAR Magic values
  pub const MAGIC: &[u8; MAGIC_LEN] = b"ustar\0";
  pub const VERSION: &[u8; VERSION_LEN] = b"00";
  pub const TYPEFLAG_NORMAL_FILE: u8 = b'0';

  // A block of zeros for padding and end-of-archive markers.
  pub const ZERO_BLOCK: [u8; BLOCK_SIZE] = [0; BLOCK_SIZE];
}
