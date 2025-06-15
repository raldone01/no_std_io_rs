use alloc::{borrow::Cow, boxed::Box, string::String, vec::Vec};

use hashbrown::HashMap;
use relative_path::RelativePathBuf;
use thiserror::Error;
use zerocopy::FromBytes;

use crate::{
  no_std_io::{BufferedReader, IBufferedReader, Read},
  tar_gz::{
    tar_constants::{
      GnuHeaderAdditions, OldHeader, TarHeaderChecksumError, TarTypeFlag, UstarHeaderAdditions,
    },
    tar_inode::{FilePermissions, Permission, TarInode, TarInodeBuilder},
  },
};

struct SparseFileInstruction {
  offset_before: u64,
  data_size: u64,
}

/// "%d %s=%s\n", <length>, <keyword>, <value>
struct PaxState {
  global_extended_attributes: HashMap<String, String>,
  /// GNU tar violated the POSIX standard by using repeated keywords.
  /// So we don't use a `HashMap` here.
  attributes: Vec<(String, String)>,
}

impl PaxState {
  #[must_use]
  fn new(initial_global_extended_attributes: Option<HashMap<String, String>>) -> Self {
    Self {
      global_extended_attributes: initial_global_extended_attributes.unwrap_or_else(HashMap::new),
      attributes: Vec::new(),
    }
  }

  fn reset(&mut self) {
    self.attributes.clear();
  }

  fn get_attribute(&self, key: &str) -> Option<&String> {
    let local_attr = self
      .attributes
      .iter()
      .find_map(|(k, v)| if k == key { Some(v) } else { None });
    local_attr.or_else(|| self.global_extended_attributes.get(key))
  }

  fn get_local_attributes(&self) -> &[(String, String)] {
    &self.attributes
  }
}

trait FileNamePredicate: for<'a> Fn(&'a str) + Clone {}

pub struct TarParseOptions {
  /// Tar can contain previous versions of the same file.
  ///
  /// If true, only the last version of each file will be kept.
  /// If false, all versions of each file will be kept.
  keep_only_last: bool,
  initial_global_extended_attributes: HashMap<String, String>,
}

impl Default for TarParseOptions {
  fn default() -> Self {
    Self {
      keep_only_last: true,
      initial_global_extended_attributes: HashMap::new(),
    }
  }
}

pub struct ParseSuccess {
  /// The global extended attributes that were active at the end of parsing.
  global_extended_attributes: HashMap<String, String>,
  /// The extracted files.
  extracted_files: Vec<TarInode>,
  /// The number of files found with each type flag.
  found_type_flags: HashMap<TarTypeFlag, usize>,
}

pub fn parse_tar_file<'a, R: IBufferedReader<'a>>(
  reader: &mut R,
  parse_options: &TarParseOptions,
) -> Result<Vec<TarInode>, TarExtractionError<R::ReadExactError>> {
  // TODO: detect and handle gzip header also handle the case where the input is not gzipped

  let mut extracted_files = Vec::<TarInode>::new();
  // Stores the index of each file in `extracted_files`.
  // Used for keeping only the last version of each file.
  let mut seen_files = HashMap::<RelativePathBuf, usize>::new();
  let mut found_type_flags = HashMap::<TarTypeFlag, usize>::new();

  let mut pax_state = PaxState::new(Some(
    parse_options.initial_global_extended_attributes.clone(),
  ));

  let mut current_tar_inode = TarInodeBuilder::default();

  loop {
    let header_buffer = reader.read_exact(512).map_err(TarExtractionError::Io)?;
    let old_header =
      OldHeader::ref_from_bytes(&header_buffer).expect("BUG: Not enough bytes for OldHeader");

    let mut data_after_header = 0;
    let mut typeflag = TarTypeFlag::UnknownTypeFlag(255);
    let mut potential_linkname = None;

    let parse_v7_header = || -> Result<(), TarExtractionError<R::ReadExactError>> {
      // verify checksum
      old_header
        .verify_checksum()
        .map_err(TarExtractionError::CorruptHeaderChecksum)?;

      // parse the information from the old header
      if current_tar_inode.path.is_none() {
        let file_name = old_header.parse_name()?;
        current_tar_inode.path = Some(RelativePathBuf::from(file_name));
      }
      if current_tar_inode.mode.is_none() {
        current_tar_inode.mode = old_header.parse_mode();
      }
      if current_tar_inode.uid.is_none() {
        current_tar_inode.uid = current_tar_inode.uid.or(old_header.parse_uid().ok());
      }
      if current_tar_inode.gid.is_none() {
        current_tar_inode.gid = current_tar_inode.gid.or(old_header.parse_gid().ok());
      }
      if let Ok(size) = old_header.parse_size() {
        data_after_header = size;
      }
      if current_tar_inode.mtime.is_none() {
        current_tar_inode.mtime = current_tar_inode.mtime.or(old_header.parse_mtime().ok());
      }
      typeflag = old_header.parse_typeflag();
      if let Some(count) = found_type_flags.get_mut(&typeflag) {
        *count += 1;
      } else {
        found_type_flags.insert(typeflag, 1);
      }

      potential_linkname = old_header.parse_linkname().ok();

      Ok(())
    };

    match &old_header.magic_version {
      OldHeader::MAGIC_VERSION_V7 => {
        parse_v7_header()?;
      },
      OldHeader::MAGIC_VERSION_USTAR => {
        parse_v7_header()?;
        let ustar_additions = UstarHeaderAdditions::ref_from_bytes(&old_header.padding)
          .expect("BUG: Not enough bytes for UstarHeaderAdditions");
        todo!()
      },
      OldHeader::MAGIC_VERSION_GNU => {
        parse_v7_header()?;
        let gnu_additions = GnuHeaderAdditions::ref_from_bytes(&old_header.padding)
          .expect("BUG: Not enough bytes for GnuHeaderAdditions");
        todo!()
      },
      unknown_version_magic => {
        return Err(TarExtractionError::CorruptHeaderMagicVersion {
          magic: unknown_version_magic[..6].try_into().unwrap(),
          version: unknown_version_magic[6..].try_into().unwrap(),
        });
      },
    }

    // NOW WE MATCH ON THE TYPEFLAG

    // todo: prefill next inode builder with pax global state
  }

  todo!()
}

#[derive(Error, Debug)]
pub enum TarExtractionError<U> {
  #[error("Invalid UTF-8 in file name: {0}")]
  InvalidUtf8InFileName(#[from] core::str::Utf8Error),
  #[error("Corrupt header: {0}")]
  CorruptHeaderChecksum(#[from] TarHeaderChecksumError),
  #[error("Corrupt header: Unknown magic or version {magic:?} {version:?}")]
  CorruptHeaderMagicVersion { magic: [u8; 6], version: [u8; 2] },
  #[error("Underlying read error: {0:?}")]
  Io(U),
}
