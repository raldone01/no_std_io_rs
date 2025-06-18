use core::convert::Infallible;

use alloc::{
  string::{String, ToString},
  vec::Vec,
};

use hashbrown::HashMap;
use relative_path::RelativePathBuf;
use thiserror::Error;
use zerocopy::FromBytes as _;

use crate::no_std_io::{
  core_streams::Cursor,
  extended_streams::tar::{
    tar_constants::{
      CommonHeaderAdditions, GnuHeaderAdditions, TarHeaderChecksumError, UstarHeaderAdditions,
      V7Header,
    },
    TarInodeBuilder,
  },
  BufferedRead as _, ReadExactError,
};
use crate::no_std_io::{
  extended_streams::tar::{
    tar_constants::{
      pax_keys_well_known::gnu::{GNU_SPARSE_DATA_BLOCK_OFFSET, GNU_SPARSE_DATA_BLOCK_SIZE},
      GnuSparseInstruction, ParseOctalError, TarTypeFlag,
    },
    TarInode,
  },
  Write,
};

#[derive(Error, Debug, PartialEq, Eq)]
pub enum TarParserError {
  #[error("The input buffer is only {actual_size} bytes, but at least {required_size} bytes are required for parsing. Reason: {reason}")]
  InputBufferTooSmall {
    actual_size: usize,
    required_size: usize,
    reason: &'static str,
  },
  #[error("Corrupt header: Checksum error: {0}")]
  CorruptHeaderChecksum(#[from] TarHeaderChecksumError),
  #[error("Corrupt header: Unknown magic or version: {magic:?} {version:?}")]
  CorruptHeaderMagicVersion { magic: [u8; 6], version: [u8; 2] },
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ParseConfidence {
  V7,
  GNU,
  PAX,
}

impl<T> TryFrom<&ParsedValue<T>> for ParseConfidence {
  type Error = ();

  fn try_from(value: &ParsedValue<T>) -> Result<Self, Self::Error> {
    match value {
      ParsedValue::None => Err(()),
      ParsedValue::V7(_) => Ok(ParseConfidence::V7),
      ParsedValue::GNU(_) => Ok(ParseConfidence::GNU),
      ParsedValue::PAX(_) => Ok(ParseConfidence::PAX),
    }
  }
}

#[derive(Default, PartialEq, Eq, PartialOrd, Ord)]
enum ParsedValue<T> {
  // Lowest to highest
  #[default]
  None,
  V7(T),
  GNU(T),
  PAX(T),
}

impl<T> ParsedValue<T> {
  #[must_use]
  fn is_none(&self) -> bool {
    matches!(self, ParsedValue::None)
  }

  #[must_use]
  fn is_some(&self) -> bool {
    !self.is_none()
  }

  #[must_use]
  fn as_ref(&self) -> Option<&T> {
    match self {
      ParsedValue::None => None,
      ParsedValue::V7(value) | ParsedValue::GNU(value) | ParsedValue::PAX(value) => Some(value),
    }
  }

  fn set(&mut self, value: T, parse_confidence: ParseConfidence) {
    // Overwrite the current value with the new value and confidence
    *self = match parse_confidence {
      ParseConfidence::V7 => ParsedValue::V7(value),
      ParseConfidence::GNU => ParsedValue::GNU(value),
      ParseConfidence::PAX => ParsedValue::PAX(value),
    };
  }

  fn parse_with_confidence<F, E>(
    &mut self,
    parse_confidence: ParseConfidence,
    parse_value: F,
  ) -> Result<(), E>
  where
    F: FnOnce() -> Result<T, E>,
  {
    // overwrite the current value if the new value is more confident
    let current_confidence = ParseConfidence::try_from(&*self);
    match current_confidence {
      Ok(current_confidence) if current_confidence < parse_confidence => {
        let parsed_value = parse_value()?;
        // We just parsed a value with higher confidence, so we overwrite the current value
        self.set(parsed_value, parse_confidence);
        Ok(())
      },
      Ok(_) => Ok(()), // The current value is more confident, so we don't overwrite it
      Err(_) => {
        // The current value is None, so we can safely overwrite it
        let parsed_value = parse_value()?;
        self.set(parsed_value, parse_confidence);
        Ok(())
      },
    }
  }
}

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

pub struct TarParserOptions {
  /// Tar can contain previous versions of the same file.
  ///
  /// If true, only the last version of each file will be kept.
  /// If false, all versions of each file will be kept.
  keep_only_last: bool,
  initial_global_extended_attributes: HashMap<String, String>,
}

impl Default for TarParserOptions {
  fn default() -> Self {
    Self {
      keep_only_last: true,
      initial_global_extended_attributes: HashMap::new(),
    }
  }
}

/// Extension trait for Option to conditionally insert a value using a closure that returns an Option,
/// only when `self` is None.
trait GetOrInsertWithMaybe<T> {
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

#[derive(Default)]
enum ParserState {
  #[default]
  ExpectingTarHeader,
  ParsingPaxData,
  ExpectingOldGnuSparseExtendedHeader,
}
pub struct TarParser {
  parser_state: ParserState,
  /// The extracted files.
  extracted_files: Vec<TarInode>,
  /// The number of files found with each type flag.
  found_type_flags: HashMap<TarTypeFlag, usize>,
  /// Stores the index of each file in `extracted_files`.
  /// Used for keeping only the last version of each file.
  /// Only used if `keep_only_last` is true.
  seen_files: HashMap<RelativePathBuf, usize>,
  keep_only_last: bool,

  // Must be reset after each file:
  /// Contains both the global and local extended attributes.
  pax_state: PaxState,
  inode_builder: TarInodeBuilder,
  gnu_long_file_name: Option<String>,
  gnu_long_link_name: Option<String>,
  sparse_file_instructions: Vec<SparseFileInstruction>,
}

impl TarParser {
  pub fn new(options: TarParserOptions) -> Self {
    Self {
      parser_state: Default::default(),
      extracted_files: Default::default(),
      found_type_flags: Default::default(),
      pax_state: PaxState::new(options.initial_global_extended_attributes),
      seen_files: Default::default(),
      keep_only_last: options.keep_only_last,
      inode_builder: TarInodeBuilder::default(),
      gnu_long_file_name: None,
      gnu_long_link_name: None,
      sparse_file_instructions: Vec::new(),
    }
  }

  /// Returns the currently active global extended pax attributes.
  pub fn get_global_extended_attributes(&self) -> &HashMap<String, String> {
    &self.pax_state.global_extended_attributes
  }

  /// Returns the files that have been extracted so far.
  pub fn get_extracted_files(&self) -> &[TarInode] {
    &self.extracted_files
  }

  /// Returns the number of files found with each type flag.
  pub fn get_found_type_flags(&self) -> &HashMap<TarTypeFlag, usize> {
    &self.found_type_flags
  }

  fn parse_old_gnu_sparse_instructions(&mut self, sparse_headers: &[GnuSparseInstruction]) {
    for sparse_header in sparse_headers {
      if sparse_header.is_empty() {
        continue;
      }
      if let Ok(instruction) = SparseFileInstruction::try_from(sparse_header) {
        self.sparse_file_instructions.push(instruction);
      } else {
        // If we can't parse the sparse header, we just ignore it.
        // This is a best-effort approach.
      }
    }
  }

  fn read_exact<'a>(
    reader: &'a mut Cursor<&[u8]>,
    byte_count: usize,
    reason: &'static str,
  ) -> Result<&'a [u8], TarParserError> {
    reader.read_exact(byte_count).map_err(|err| match err {
      ReadExactError::Io(_) => panic!("BUG: Infallible read error"),
      ReadExactError::UnexpectedEof {
        min_readable_bytes,
        bytes_requested: _,
      } => TarParserError::InputBufferTooSmall {
        actual_size: min_readable_bytes,
        required_size: byte_count,
        reason,
      },
    })
  }

  fn parse_header(&mut self, reader: &mut Cursor<&[u8]>) -> Result<(), TarParserError> {
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

    let header_buffer = Self::read_exact(reader, 512, "Only full tar headers can be parsed")?;
    let old_header =
      V7Header::ref_from_bytes(&header_buffer).expect("BUG: Not enough bytes for OldHeader");

    let mut parse_v7_header = || -> Result<(), TarParserError> {
      // verify checksum
      old_header
        .verify_checksum()
        .map_err(TarParserError::CorruptHeaderChecksum)?;

      // parse the information from the old header
      potential_path = old_header.parse_name().map(RelativePathBuf::from).ok();
      self
        .inode_builder
        .mode
        .get_or_insert_with_maybe(|| old_header.parse_mode());
      self
        .inode_builder
        .uid
        .get_or_insert_with_maybe(|| old_header.parse_uid().ok());
      self
        .inode_builder
        .gid
        .get_or_insert_with_maybe(|| old_header.parse_gid().ok());
      if let Ok(size) = old_header.parse_size() {
        data_after_header = size as usize;
      }

      self
        .inode_builder
        .mtime
        .get_or_insert_with_maybe(|| old_header.parse_mtime().ok());

      typeflag = old_header.parse_typeflag();
      if let Some(count) = self.found_type_flags.get_mut(&typeflag) {
        *count += 1;
      } else {
        self.found_type_flags.insert(typeflag.clone(), 1);
      }

      potential_linkname.get_or_insert_with_maybe(|| old_header.parse_linkname().ok());

      Ok(())
    };

    let mut parse_common_header_additions =
      |common_header_additions: &CommonHeaderAdditions| -> Result<(), TarParserError> {
        self.inode_builder.uname.get_or_insert_with_maybe(|| {
          common_header_additions.parse_uname().ok().map(String::from)
        });
        self.inode_builder.gname.get_or_insert_with_maybe(|| {
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
            self
              .inode_builder
              .path
              .get_or_insert_with(|| RelativePathBuf::from(prefix).join(path));
          } else {
            self.inode_builder.path.get_or_insert(path);
          }
        } else {
          self.inode_builder.path.get_or_insert_with_maybe(|| {
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
        self.inode_builder.mtime.get_or_insert_with_maybe(|| {
          gnu_additions
            .parse_atime()
            .ok()
            .or_else(|| gnu_additions.parse_ctime().ok())
        });

        // Handle sparse entries (Old GNU Format)
        if typeflag == TarTypeFlag::SparseOldGnu {
          self.parse_old_gnu_sparse_instructions(&gnu_additions.sparse);
          old_gnu_sparse_is_extended = gnu_additions.parse_is_extended();
        }

        potential_sparse_real_size
          .get_or_insert_with_maybe(|| gnu_additions.parse_real_size().ok());

        // Done GNU header parsing.
      },
      unknown_version_magic => {
        return Err(TarParserError::CorruptHeaderMagicVersion {
          magic: unknown_version_magic[..6].try_into().unwrap(),
          version: unknown_version_magic[6..].try_into().unwrap(),
        });
      },
    }

    todo!()
  }
}

impl Write for TarParser {
  type WriteError = TarParserError;
  type FlushError = Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    let mut reader = Cursor::new(input_buffer);

    let parse_write_result = match self.parser_state {
      ParserState::ExpectingTarHeader => self.parse_header(&mut reader),
      _ => {
        todo!()
      },
    };
    parse_write_result.map(|_| reader.position())
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}
