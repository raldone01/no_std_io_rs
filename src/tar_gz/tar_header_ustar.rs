pub struct TarHeaderRaw {
  pub name: [u8; 100],
  pub mode: [u8; 8],
  pub uid: [u8; 8],
  pub gid: [u8; 8],
  pub size: [u8; 12],
  pub mtime: [u8; 12],
  pub chksum: [u8; 8],
  pub typeflag: u8,
  pub linkname: [u8; 100],
  // `v7` ends here and magic is 0
  // `ustar` starts here and has the magic string "ustar\0" or "ustar"
  // `gnu` starts here and has the magic string "ustar "
  // `ustar` has version "00"
  // `gnu` has version " \0"
  /// Combined field for magic[4] and version[2].
  pub magic_and_version: [u8; 6],
  pub version: [u8; 2],
  pub uname: [u8; 32],
  pub gname: [u8; 32],
  pub devmajor: [u8; 8],
  pub devminor: [u8; 8],
  pub prefix: [u8; 155],
  pub padding: [u8; 12],
}
