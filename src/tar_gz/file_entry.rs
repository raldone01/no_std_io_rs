use alloc::string::String;

use relative_path::RelativePathBuf;

pub struct ExtractedFile {
  pub entry: FileEntry,
  pub mode: u32,
  pub uid: u32,
  pub gid: u32,
  pub mtime: u64,
  pub uname: String,
  pub gname: String,
}

pub enum FileEntry {
  Regular(RegularFileEntry),
  HardLink(HardLinkEntry),
  SymbolicLink(SymbolicLinkEntry),
  CharacterDevice(CharacterDeviceEntry),
  BlockDevice(BlockDeviceEntry),
  Directory(DirectoryEntry),
  Fifo(FifoEntry),
}

pub struct RegularFileEntry {
  name: RelativePathBuf,
}

pub struct HardLinkEntry {
  name: RelativePathBuf,
  link_target: RelativePathBuf,
}

pub struct SymbolicLinkEntry {
  name: RelativePathBuf,
  link_target: RelativePathBuf,
}

pub struct CharacterDeviceEntry {
  name: RelativePathBuf,
  major: u32,
  minor: u32,
}

pub struct BlockDeviceEntry {
  name: RelativePathBuf,
  major: u32,
  minor: u32,
}

pub struct DirectoryEntry {
  name: RelativePathBuf,
}

pub struct FifoEntry {
  name: RelativePathBuf,
}
