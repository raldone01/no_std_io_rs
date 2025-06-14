use alloc::string::String;

use hashbrown::HashMap;
use relative_path::RelativePathBuf;

pub struct ExtractedFile {
  pub path: RelativePathBuf,
  pub entry: FileEntry,
  pub mode: u32,
  pub uid: u32,
  pub gid: u32,
  pub mtime: u64,
  pub uname: String,
  pub gname: String,
  pub unparsed_extended_attributes: HashMap<String, String>,
}

pub enum FileEntry {
  Regular(RegularFileEntry),
  HardLink(HardLinkEntry),
  SymbolicLink(SymbolicLinkEntry),
  CharacterDevice(CharacterDeviceEntry),
  BlockDevice(BlockDeviceEntry),
  Directory,
  Fifo,
}

pub struct RegularFileEntry {}

pub struct HardLinkEntry {
  link_target: RelativePathBuf,
}

pub struct SymbolicLinkEntry {
  link_target: RelativePathBuf,
}

pub struct CharacterDeviceEntry {
  major: u32,
  minor: u32,
}

pub struct BlockDeviceEntry {
  major: u32,
  minor: u32,
}
