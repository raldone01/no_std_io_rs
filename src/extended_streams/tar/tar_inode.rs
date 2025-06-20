use alloc::{string::String, vec::Vec};

use hashbrown::HashMap;
use relative_path::RelativePathBuf;

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct TimeStamp {
  pub seconds_since_epoch: u64,
  pub nanoseconds: u32,
}

pub struct TarInode {
  pub path: RelativePathBuf,
  pub entry: FileEntry,
  pub mode: FilePermissions,
  pub uid: u32,
  pub gid: u32,
  pub mtime: TimeStamp,
  pub uname: String,
  pub gname: String,
  pub unparsed_extended_attributes: HashMap<String, String>,
}

/// Represents permissions for a single user class (owner, group, or other)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permission {
  pub read: bool,
  pub write: bool,
  pub execute: bool,
}

/// Represents file permissions split into owner, group, and other
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePermissions {
  pub owner: Permission,
  pub group: Permission,
  pub other: Permission,
  pub set_uid: bool,
  pub set_gid: bool,
  pub sticky: bool,
}

impl Default for FilePermissions {
  fn default() -> Self {
    FilePermissions {
      owner: Permission {
        read: true,
        write: true,
        execute: false,
      },
      group: Permission {
        read: true,
        write: true,
        execute: false,
      },
      other: Permission {
        read: false,
        write: false,
        execute: false,
      },
      set_uid: false,
      set_gid: false,
      sticky: false,
    }
  }
}

impl FilePermissions {
  /// Parses an octal ASCII string representing Unix file permissions as found in the `mode` field of a tar header.
  /// The input is expected to be &[u8; 12].
  pub fn parse_octal_ascii_unix_mode(octal_bytes: &[u8]) -> Option<Self> {
    let mode_str = str::from_utf8(&octal_bytes).ok()?;
    let mode = u32::from_str_radix(mode_str, 8).ok()?;

    // Extract permission bits
    let owner = Permission {
      read: mode & 0o400 != 0,
      write: mode & 0o200 != 0,
      execute: mode & 0o100 != 0,
    };
    let group = Permission {
      read: mode & 0o040 != 0,
      write: mode & 0o020 != 0,
      execute: mode & 0o010 != 0,
    };
    let other = Permission {
      read: mode & 0o004 != 0,
      write: mode & 0o002 != 0,
      execute: mode & 0o001 != 0,
    };

    // Special permission bits
    let set_uid = mode & 0o4000 != 0;
    let set_gid = mode & 0o2000 != 0;
    let sticky = mode & 0o1000 != 0;

    Some(FilePermissions {
      owner,
      group,
      other,
      set_uid,
      set_gid,
      sticky,
    })
  }
}

#[derive(Debug)]
pub enum FileEntry {
  RegularFile(RegularFileEntry),
  HardLink(HardLinkEntry),
  SymbolicLink(SymbolicLinkEntry),
  CharacterDevice(CharacterDeviceEntry),
  BlockDevice(BlockDeviceEntry),
  Directory,
  Fifo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SparseFileInstruction {
  pub offset_before: u64,
  pub data_size: u64,
}

#[derive(Debug)]
pub enum FileData {
  Regular(Vec<u8>),
  Sparse {
    instructions: Vec<SparseFileInstruction>,
    data: Vec<u8>,
  },
}

#[derive(Debug)]
pub struct RegularFileEntry {
  pub continuous: bool,
  pub data: FileData,
}

#[derive(Debug)]
pub struct HardLinkEntry {
  pub link_target: RelativePathBuf,
}

#[derive(Debug)]
pub struct SymbolicLinkEntry {
  pub link_target: RelativePathBuf,
}

#[derive(Debug)]
pub struct CharacterDeviceEntry {
  pub major: u32,
  pub minor: u32,
}

#[derive(Debug)]
pub struct BlockDeviceEntry {
  pub major: u32,
  pub minor: u32,
}
