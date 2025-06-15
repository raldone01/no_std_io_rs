use alloc::{
  borrow::Cow,
  boxed::Box,
  string::{String, ToString},
  vec::Vec,
};

use hashbrown::HashMap;
use relative_path::RelativePathBuf;
use thiserror::Error;
use zerocopy::FromBytes;

use crate::{
  no_std_io::{BufferedReader, IBufferedReader, Read},
  tar_gz::{
    tar_constants::{
      pax_keys_well_known::gnu::{GNU_SPARSE_DATA_BLOCK_OFFSET, GNU_SPARSE_DATA_BLOCK_SIZE},
      CommonHeaderAdditions, GnuHeaderAdditions, TarHeaderChecksumError, TarTypeFlag,
      UstarHeaderAdditions, V7Header,
    },
    tar_inode::{FileEntry, FilePermissions, Permission, TarInode, TarInodeBuilder},
  },
};

struct SparseFileInstruction {
  offset_before: u64,
  data_size: u64,
}

/// "%d %s=%s\n", <length>, <keyword>, <value>
struct PaxState {
  /// We don't support GNU sparse file attributes in global pax attributes.
  global_extended_attributes: HashMap<String, String>,
  /// GNU tar violated the POSIX standard by using repeated keywords.
  /// So we don't use a `HashMap` here.
  attributes: Vec<(String, String)>,
  parsed_attributes: HashMap<String, ()>,
}

impl PaxState {
  fn preload_parsed_attributes(&mut self) {
    // Since these attributes are completely broken anyway, we don't want the user to ever see them.
    const ALWAYS_PARSED_ATTRIBUTES: &[&str] =
      &[GNU_SPARSE_DATA_BLOCK_OFFSET, GNU_SPARSE_DATA_BLOCK_SIZE];
    for key in ALWAYS_PARSED_ATTRIBUTES {
      self.parsed_attributes.insert(key.to_string(), ());
    }
  }

  #[must_use]
  fn new(initial_global_extended_attributes: HashMap<String, String>) -> Self {
    let mut selv = Self {
      global_extended_attributes: initial_global_extended_attributes,
      attributes: Vec::new(),
      parsed_attributes: HashMap::new(),
    };
    selv.preload_parsed_attributes();
    selv
  }

  fn reset_local(&mut self) {
    self.attributes.clear();
    self.parsed_attributes.clear();
    self.preload_parsed_attributes();
  }

  fn get_attribute(&mut self, key: &str) -> Option<&String> {
    if self.parsed_attributes.get(key).is_none() {
      // TODO: interning the key would be more efficient
      self.parsed_attributes.insert(key.to_string(), ());
    }
    let local_attr = self
      .attributes
      .iter()
      .rev()
      .find_map(|(k, v)| if k == key { Some(v) } else { None });
    local_attr.or_else(|| self.global_extended_attributes.get(key))
  }

  fn get_unparsed_extended_attributes(&mut self) -> HashMap<String, String> {
    let mut unparsed = HashMap::new();
    for (key, value) in &self.attributes {
      if !self.parsed_attributes.contains_key(key) {
        unparsed.insert(key.clone(), value.clone());
      }
    }
    unparsed
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

/// Extension trait for Option to conditionally insert a value using a closure that returns an Option,
/// only when `self` is None.
pub trait GetOrInsertWithMaybe<T> {
  /// If `self` is Some, returns a mutable reference to the value.
  /// Otherwise, runs the closure. If the closure returns Some, inserts it and returns a mutable reference.
  /// If the closure returns None, leaves `self` as None and returns None.
  fn get_or_insert_with_maybe<F>(&mut self, f: F) -> Option<&mut T>
  where
    F: FnOnce() -> Option<T>;
}

impl<T> GetOrInsertWithMaybe<T> for Option<T> {
  fn get_or_insert_with_maybe<F>(&mut self, f: F) -> Option<&mut T>
  where
    F: FnOnce() -> Option<T>,
  {
    if self.is_none() {
      *self = f();
    }
    self.as_mut()
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

  let mut pax_state = PaxState::new(parse_options.initial_global_extended_attributes.clone());

  let mut current_tar_inode = TarInodeBuilder::default();

  loop {
    let header_buffer = reader.read_exact(512).map_err(TarExtractionError::Io)?;
    let old_header =
      V7Header::ref_from_bytes(&header_buffer).expect("BUG: Not enough bytes for OldHeader");

    let mut potential_path_postfix = None;
    let mut data_after_header = 0;
    let mut typeflag = TarTypeFlag::UnknownTypeFlag(255);
    let mut potential_linkname = None;
    let mut potential_dev_major = None;
    let mut potential_dev_minor = None;

    let mut parse_v7_header = || -> Result<(), TarExtractionError<R::ReadExactError>> {
      // verify checksum
      old_header
        .verify_checksum()
        .map_err(TarExtractionError::CorruptHeaderChecksum)?;

      // parse the information from the old header
      potential_path_postfix = old_header.parse_name().map(RelativePathBuf::from).ok();
      current_tar_inode
        .mode
        .get_or_insert_with_maybe(|| old_header.parse_mode());
      current_tar_inode
        .uid
        .get_or_insert_with_maybe(|| old_header.parse_uid().ok());
      current_tar_inode
        .gid
        .get_or_insert_with_maybe(|| old_header.parse_gid().ok());
      if let Ok(size) = old_header.parse_size() {
        data_after_header = size;
      }

      current_tar_inode
        .mtime
        .get_or_insert_with_maybe(|| old_header.parse_mtime().ok());

      typeflag = old_header.parse_typeflag();
      if let Some(count) = found_type_flags.get_mut(&typeflag) {
        *count += 1;
      } else {
        found_type_flags.insert(typeflag.clone(), 1);
      }

      potential_linkname.get_or_insert_with_maybe(|| old_header.parse_linkname().ok());

      Ok(())
    };

    let mut parse_common_header_additions = |common_header_additions: &CommonHeaderAdditions| -> Result<
      (),
      TarExtractionError<R::ReadExactError>,
    > {
      current_tar_inode
        .uname
        .get_or_insert_with_maybe(|| common_header_additions.parse_uname().ok().map(String::from));
      current_tar_inode
        .gname
        .get_or_insert_with_maybe(|| common_header_additions.parse_gname().ok().map(String::from));
      potential_dev_major
        .get_or_insert_with_maybe(|| common_header_additions.parse_dev_major().ok());
      potential_dev_minor
        .get_or_insert_with_maybe(|| common_header_additions.parse_dev_minor().ok());
      Ok(())
    };

    match &old_header.magic_version {
      V7Header::MAGIC_VERSION_V7 => {
        parse_v7_header()?;
      },
      V7Header::MAGIC_VERSION_USTAR => {
        parse_v7_header()?;
        let common_header_additions = CommonHeaderAdditions::ref_from_bytes(&old_header.padding)
          .expect("BUG: Not enough bytes for CommonHeaderAdditions in USTAR");
        parse_common_header_additions(common_header_additions)?;
        let ustar_additions =
          UstarHeaderAdditions::ref_from_bytes(&common_header_additions.padding)
            .expect("BUG: Not enough bytes for UstarHeaderAdditions");
        todo!()
      },
      V7Header::MAGIC_VERSION_GNU => {
        parse_v7_header()?;
        let common_header_additions = CommonHeaderAdditions::ref_from_bytes(&old_header.padding)
          .expect("BUG: Not enough bytes for CommonHeaderAdditions in GNU");
        parse_common_header_additions(common_header_additions)?;
        let gnu_additions = GnuHeaderAdditions::ref_from_bytes(&common_header_additions.padding)
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

    // now we match on the typeflag
    match typeflag {
      TarTypeFlag::Fifo => {
        current_tar_inode.unparsed_extended_attributes =
          pax_state.get_unparsed_extended_attributes();
        current_tar_inode.entry = Some(FileEntry::Fifo);
      },
      TarTypeFlag::UnknownTypeFlag(_) => {
        // we just skip the data_after_header bytes if we don't know the typeflag
      },
      _ => todo!(),
    }

    // move reader ahead

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
