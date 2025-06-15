use core::str::Utf8Error;

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
      CommonHeaderAdditions, GnuHeaderAdditions, GnuHeaderExtSparse, GnuSparseInstruction,
      ParseOctalError, TarHeaderChecksumError, TarTypeFlag, UstarHeaderAdditions, V7Header,
    },
    tar_inode::{
      BlockDeviceEntry, CharacterDeviceEntry, FileEntry, FilePermissions, Permission, TarInode,
      TarInodeBuilder,
    },
  },
};

struct SparseFileInstruction {
  offset_before: u64,
  data_size: u64,
}

impl TryFrom<&GnuSparseInstruction> for SparseFileInstruction {
  type Error = ParseOctalError;

  fn try_from(value: &GnuSparseInstruction) -> Result<Self, Self::Error> {
    Ok(Self {
      offset_before: value.parse_offset()?,
      data_size: value.parse_num_bytes()?,
    })
  }
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

/// todo make this into a read where the user can push bytes into it.
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

  let mut gnu_long_file_name = String::new();
  let mut gnu_long_link_name = String::new();

  loop {
    // header parsing variables
    let mut potential_path = None;
    let mut data_after_header = 0;
    let mut typeflag = TarTypeFlag::UnknownTypeFlag(255);
    let mut potential_linkname = None;
    let mut potential_dev_major = None;
    let mut potential_dev_minor = None;
    let mut potential_sparse_instructions = Vec::<SparseFileInstruction>::new();
    let mut potential_sparse_real_size = None;
    let mut old_gnu_sparse_is_extended = false;

    let mut old_gnu_sparse_parse_sparse_instructions =
      |sparse_headers: &[GnuSparseInstruction]| {
        for sparse_header in sparse_headers {
          if sparse_header.is_empty() {
            continue;
          }
          if let Ok(instruction) = SparseFileInstruction::try_from(sparse_header) {
            potential_sparse_instructions.push(instruction);
          } else {
            // If we can't parse the sparse header, we just ignore it.
            // This is a best-effort approach.
          }
        }
      };

    {
      let header_buffer = reader.read_exact(512).map_err(TarExtractionError::Io)?;
      let old_header =
        V7Header::ref_from_bytes(&header_buffer).expect("BUG: Not enough bytes for OldHeader");

      let mut parse_v7_header = || -> Result<(), TarExtractionError<R::ReadExactError>> {
        // verify checksum
        old_header
          .verify_checksum()
          .map_err(TarExtractionError::CorruptHeaderChecksum)?;

        // parse the information from the old header
        potential_path = old_header.parse_name().map(RelativePathBuf::from).ok();
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
          data_after_header = size as usize;
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
        current_tar_inode.uname.get_or_insert_with_maybe(|| {
          common_header_additions.parse_uname().ok().map(String::from)
        });
        current_tar_inode.gname.get_or_insert_with_maybe(|| {
          common_header_additions.parse_gname().ok().map(String::from)
        });
        potential_dev_major
          .get_or_insert_with_maybe(|| common_header_additions.parse_dev_major().ok());
        potential_dev_minor
          .get_or_insert_with_maybe(|| common_header_additions.parse_dev_minor().ok());
        Ok(())
      };

      // This parses all fields in a header block regardless of the typeflag.
      // There is some room for improving allocations/parsing based on the typeflag.
      match &old_header.magic_version {
        V7Header::MAGIC_VERSION_V7 => {
          parse_v7_header()?;
          // Done v7 header parsing.
        },
        V7Header::MAGIC_VERSION_USTAR => {
          parse_v7_header()?;
          let common_header_additions = CommonHeaderAdditions::ref_from_bytes(&old_header.padding)
            .expect("BUG: Not enough bytes for CommonHeaderAdditions in USTAR");
          parse_common_header_additions(common_header_additions)?;
          let ustar_additions =
            UstarHeaderAdditions::ref_from_bytes(&common_header_additions.padding)
              .expect("BUG: Not enough bytes for UstarHeaderAdditions");

          // if there is already a path we want to prefix it with the ustar additions
          // if there is no path, we want to use the ustar prefix as the path
          if let Some(path) = potential_path {
            let prefix = ustar_additions.parse_prefix().ok().map(String::from);
            if let Some(prefix) = prefix {
              current_tar_inode
                .path
                .get_or_insert_with(|| RelativePathBuf::from(prefix).join(path));
            } else {
              current_tar_inode.path.get_or_insert(path);
            }
          } else {
            current_tar_inode.path.get_or_insert_with_maybe(|| {
              ustar_additions
                .parse_prefix()
                .ok()
                .map(|prefix| RelativePathBuf::from(prefix))
            });
          }
          // Done ustar header parsing.
        },
        V7Header::MAGIC_VERSION_GNU => {
          parse_v7_header()?;
          let common_header_additions = CommonHeaderAdditions::ref_from_bytes(&old_header.padding)
            .expect("BUG: Not enough bytes for CommonHeaderAdditions in GNU");
          parse_common_header_additions(common_header_additions)?;
          let gnu_additions = GnuHeaderAdditions::ref_from_bytes(&common_header_additions.padding)
            .expect("BUG: Not enough bytes for GnuHeaderAdditions");

          // We don't care about atime or ctime so we just use them if we could not parse mtime.
          current_tar_inode.mtime.get_or_insert_with_maybe(|| {
            gnu_additions
              .parse_atime()
              .ok()
              .or_else(|| gnu_additions.parse_ctime().ok())
          });

          // Handle sparse entries (Old GNU Format)
          if typeflag == TarTypeFlag::SparseOldGnu {
            old_gnu_sparse_parse_sparse_instructions(&gnu_additions.sparse);
            old_gnu_sparse_is_extended = gnu_additions.parse_is_extended();
          }

          potential_sparse_real_size
            .get_or_insert_with_maybe(|| gnu_additions.parse_real_size().ok());

          // Done GNU header parsing.
        },
        unknown_version_magic => {
          return Err(TarExtractionError::CorruptHeaderMagicVersion {
            magic: unknown_version_magic[..6].try_into().unwrap(),
            version: unknown_version_magic[6..].try_into().unwrap(),
          });
        },
      }
    }
    // We parsed everything from the header block and released the buffer.

    let mut gnu_parse_long_name = |output: &mut String,
                                   context: &'static str|
     -> Result<(), TarExtractionError<R::ReadExactError>> {
      let long_file_name_bytes = reader
        .read_exact(data_after_header)
        .map_err(TarExtractionError::Io)?;
      let long_file_name = str::from_utf8(long_file_name_bytes)
        .map_err(|e| TarExtractionError::InvalidUtf8InFileName(context, e))?;
      output.clear();
      output.push_str(long_file_name);
      Ok(())
    };

    // now we match on the typeflag
    match typeflag {
      TarTypeFlag::CharacterDevice => {
        current_tar_inode.unparsed_extended_attributes =
          pax_state.get_unparsed_extended_attributes();
        current_tar_inode.entry = Some(FileEntry::CharacterDevice(CharacterDeviceEntry {
          major: potential_dev_major.unwrap_or(0),
          minor: potential_dev_minor.unwrap_or(0),
        }));
      },
      TarTypeFlag::BlockDevice => {
        current_tar_inode.unparsed_extended_attributes =
          pax_state.get_unparsed_extended_attributes();
        current_tar_inode.entry = Some(FileEntry::BlockDevice(BlockDeviceEntry {
          major: potential_dev_major.unwrap_or(0),
          minor: potential_dev_minor.unwrap_or(0),
        }));
      },
      TarTypeFlag::Fifo => {
        current_tar_inode.unparsed_extended_attributes =
          pax_state.get_unparsed_extended_attributes();
        current_tar_inode.entry = Some(FileEntry::Fifo);
      },
      TarTypeFlag::LongNameGnu => {
        gnu_parse_long_name(&mut gnu_long_file_name, "GNU long file name")?;
      },
      TarTypeFlag::LongLinkNameGnu => {
        gnu_parse_long_name(&mut gnu_long_link_name, "GNU long link name")?;
      },
      TarTypeFlag::SparseOldGnu => {
        if old_gnu_sparse_is_extended {
          // We must read the next block to get more sparse headers.
          loop {
            let extended_header_buffer = reader.read_exact(512).map_err(TarExtractionError::Io)?;
            let extended_header = GnuHeaderExtSparse::ref_from_bytes(&extended_header_buffer)
              .expect("BUG: Not enough bytes for GnuHeaderExtSparse");
            old_gnu_sparse_parse_sparse_instructions(&extended_header.sparse);
            if !extended_header.parse_is_extended() {
              break;
            }
          }
        }
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
  #[error("Invalid UTF-8 in {0}: {1}")]
  InvalidUtf8InFileName(&'static str, Utf8Error),
  #[error("Corrupt header: {0}")]
  CorruptHeaderChecksum(#[from] TarHeaderChecksumError),
  #[error("Corrupt header: Unknown magic or version {magic:?} {version:?}")]
  CorruptHeaderMagicVersion { magic: [u8; 6], version: [u8; 2] },
  #[error("Underlying read error: {0:?}")]
  Io(U),
}
