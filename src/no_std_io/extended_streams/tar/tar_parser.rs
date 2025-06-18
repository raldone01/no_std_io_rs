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
      pax_keys_well_known::{
        gnu::{GNU_SPARSE_NAME, GNU_SPARSE_REALSIZE, GNU_SPARSE_REALSIZE_OLD},
        PATH,
      },
      CommonHeaderAdditions, GnuHeaderAdditions, TarHeaderChecksumError, UstarHeaderAdditions,
      V7Header,
    },
    BlockDeviceEntry, CharacterDeviceEntry, FileEntry, FilePermissions,
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

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum ParseConfidence {
  V7 = 1,
  Ustar,
  Gnu,
  PaxGlobal,
  Pax,
}

impl<T> TryFrom<&ParsedValue<T>> for ParseConfidence {
  type Error = ();

  fn try_from(value: &ParsedValue<T>) -> Result<Self, Self::Error> {
    match value {
      ParsedValue::None => Err(()),
      ParsedValue::V7(_) => Ok(ParseConfidence::V7),
      ParsedValue::Ustar(_) => Ok(ParseConfidence::Ustar),
      ParsedValue::Gnu(_) => Ok(ParseConfidence::Gnu),
      ParsedValue::PaxGlobal(_) => Ok(ParseConfidence::PaxGlobal),
      ParsedValue::Pax(_) => Ok(ParseConfidence::Pax),
    }
  }
}

#[derive(Default, PartialEq, Eq, PartialOrd, Ord)]
enum ParsedValue<T> {
  // Lowest to highest
  #[default]
  None,
  V7(T),
  Ustar(T),
  Gnu(T),
  PaxGlobal(T),
  Pax(T),
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
      ParsedValue::V7(value)
      | ParsedValue::Ustar(value)
      | ParsedValue::Gnu(value)
      | ParsedValue::PaxGlobal(value)
      | ParsedValue::Pax(value) => Some(value),
    }
  }

  fn force_insert_confidence(&mut self, parse_confidence: ParseConfidence, value: T) {
    // Overwrite the current value with the new value and confidence
    *self = match parse_confidence {
      ParseConfidence::V7 => ParsedValue::V7(value),
      ParseConfidence::Ustar => ParsedValue::Ustar(value),
      ParseConfidence::Gnu => ParsedValue::Gnu(value),
      ParseConfidence::PaxGlobal => ParsedValue::PaxGlobal(value),
      ParseConfidence::Pax => ParsedValue::Pax(value),
    };
  }

  fn parse_with_confidence<F, E>(
    &mut self,
    parse_confidence: ParseConfidence,
    parse_value: F,
  ) -> Result<&T, E>
  where
    F: FnOnce() -> Result<T, E>,
  {
    // If the current value is more confident than the requested confidence, do nothing
    if let Ok(current_confidence) = ParseConfidence::try_from(&*self) {
      if current_confidence > parse_confidence {
        return Ok(self.as_ref().expect("BUG: ParsedValue should not be None"));
      }
    }
    // Otherwise, parse the value and overwrite the current value
    match parse_value() {
      Ok(parsed_value) => {
        self.force_insert_confidence(parse_confidence, parsed_value);
        Ok(self.as_ref().expect("BUG: ParsedValue should not be None"))
      },
      Err(err) => Err(err),
    }
  }

  fn parse_with_confidence_opt<F>(
    &mut self,
    parse_confidence: ParseConfidence,
    parse_value: F,
  ) -> Option<&T>
  where
    F: FnOnce() -> Option<T>,
  {
    // If the current value is more confident than the requested confidence, do nothing
    if let Ok(current_confidence) = ParseConfidence::try_from(&*self) {
      if current_confidence > parse_confidence {
        return self.as_ref();
      }
    }
    // Otherwise, parse the value and overwrite the current value
    if let Some(parsed_value) = parse_value() {
      self.force_insert_confidence(parse_confidence, parsed_value);
    }
    self.as_ref()
  }

  fn get_with_less_or_equal_confidence(&self, parse_confidence: ParseConfidence) -> Option<&T> {
    // If the current value is more confident than the requested confidence, return None
    if let Ok(current_confidence) = ParseConfidence::try_from(self) {
      if current_confidence > parse_confidence {
        return None;
      }
    }
    // Otherwise, return the current value
    self.as_ref()
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
  ExpectingOldGnuSparseExtendedHeader {
    data_after_header: usize,
  },
}
pub struct TarParser {
  /// The extracted files.
  extracted_files: Vec<TarInode>,

  /// The number of files found with each type flag.
  found_type_flags: HashMap<TarTypeFlag, usize>,
  /// Stores the index of each file in `extracted_files`.
  /// Used for keeping only the last version of each file.
  /// Only used if `keep_only_last` is true.
  seen_files: HashMap<RelativePathBuf, usize>,
  keep_only_last: bool,

  parser_state: ParserState,
  // Must be reset after each file:
  /// Contains both the global and local extended attributes.
  pax_state: PaxState,
  inode_state: InodeBuilder,
}

#[derive(Default)]
struct InodeBuilder {
  file_path: ParsedValue<RelativePathBuf>,
  mode: Option<FilePermissions>,
  uid: ParsedValue<u32>,
  gid: ParsedValue<u32>,
  mtime: ParsedValue<u64>,
  uname: ParsedValue<String>,
  gname: ParsedValue<String>,
  link_target: ParsedValue<String>,
  sparse_file_instructions: Vec<SparseFileInstruction>,
  /// The realsize if it is a sparse file.
  sparse_real_size: ParsedValue<usize>,
  sparse_gnu_major: ParsedValue<usize>,
  sparse_gnu_minor: ParsedValue<usize>,
}

impl TarParser {
  pub fn new(options: TarParserOptions) -> Self {
    Self {
      extracted_files: Default::default(),

      found_type_flags: Default::default(),
      seen_files: Default::default(),
      keep_only_last: options.keep_only_last,

      parser_state: Default::default(),
      pax_state: PaxState::new(options.initial_global_extended_attributes),
      inode_state: Default::default(),
    }
  }

  fn load_pax_into_parser(&mut self, parse_confidence: ParseConfidence) {
    self
      .inode_state
      .file_path
      .parse_with_confidence_opt(parse_confidence, || {
        self
          .pax_state
          .get_attribute(GNU_SPARSE_NAME)
          .map(RelativePathBuf::from)
      });
    self
      .inode_state
      .file_path
      .parse_with_confidence_opt(parse_confidence, || {
        self
          .pax_state
          .get_attribute(PATH)
          .map(RelativePathBuf::from)
      });
    self
      .inode_state
      .sparse_real_size
      .parse_with_confidence_opt(parse_confidence, || {
        self
          .pax_state
          .get_attribute(GNU_SPARSE_REALSIZE)
          .and_then(|s| s.parse().ok())
      });
    self
      .inode_state
      .sparse_real_size
      .parse_with_confidence_opt(parse_confidence, || {
        self
          .pax_state
          .get_attribute(GNU_SPARSE_REALSIZE_OLD)
          .and_then(|s| s.parse().ok())
      });
  }

  pub fn recover(&mut self) {
    self.pax_state.reset_local();
    self.load_pax_into_parser(ParseConfidence::PaxGlobal);
    self.inode_state = Default::default();
    self.parser_state = ParserState::ExpectingTarHeader;
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
        self.inode_state.sparse_file_instructions.push(instruction);
      } else {
        // If we can't parse the sparse header, we just ignore it.
        // This is a best-effort approach.
      }
    }
  }

  fn map_reader_error(err: ReadExactError<Infallible>, reason: &'static str) -> TarParserError {
    match err {
      ReadExactError::Io(_) => panic!("BUG: Infallible read error"),
      ReadExactError::UnexpectedEof {
        min_readable_bytes,
        bytes_requested,
      } => TarParserError::InputBufferTooSmall {
        actual_size: min_readable_bytes,
        required_size: bytes_requested,
        reason,
      },
    }
  }

  fn finish_inode(&mut self, file_entry: FileEntry) -> TarInode {
    self.load_pax_into_parser(ParseConfidence::Pax);
    //let mut inode = TarInode {};
    todo!()
  }

  fn parse_header(&mut self, reader: &mut Cursor<&[u8]>) -> Result<(), TarParserError> {
    // header parsing variables
    let mut data_after_header = 0;
    let mut typeflag = TarTypeFlag::UnknownTypeFlag(255);
    let mut potential_dev_major = None;
    let mut potential_dev_minor = None;
    let mut potential_sparse_real_size = None;
    let mut old_gnu_sparse_is_extended = false;

    let header_buffer = reader
      .read_exact(512)
      .map_err(|err| Self::map_reader_error(err, "Only full tar headers can be parsed"))?;
    let old_header =
      V7Header::ref_from_bytes(&header_buffer).expect("BUG: Not enough bytes for OldHeader");

    let mut parse_v7_header = || -> Result<(), TarParserError> {
      // verify checksum
      old_header
        .verify_checksum()
        .map_err(TarParserError::CorruptHeaderChecksum)?;

      // parse the information from the old header
      let _ = self
        .inode_state
        .file_path
        .parse_with_confidence(ParseConfidence::V7, || {
          old_header.parse_name().map(RelativePathBuf::from)
        });
      self
        .inode_state
        .mode
        .get_or_insert_with_maybe(|| old_header.parse_mode());
      let _ = self
        .inode_state
        .uid
        .parse_with_confidence(ParseConfidence::V7, || old_header.parse_uid());
      let _ = self
        .inode_state
        .gid
        .parse_with_confidence(ParseConfidence::V7, || old_header.parse_gid());
      if let Ok(size) = old_header.parse_size() {
        data_after_header = size as usize;
      }

      let _ = self
        .inode_state
        .mtime
        .parse_with_confidence(ParseConfidence::V7, || old_header.parse_mtime());

      typeflag = old_header.parse_typeflag();
      if let Some(count) = self.found_type_flags.get_mut(&typeflag) {
        *count += 1;
      } else {
        self.found_type_flags.insert(typeflag.clone(), 1);
      }

      let _ = self
        .inode_state
        .link_target
        .parse_with_confidence(ParseConfidence::V7, || {
          old_header.parse_linkname().map(String::from)
        });

      Ok(())
    };

    let mut parse_common_header_additions =
      |common_header_additions: &CommonHeaderAdditions| -> Result<(), TarParserError> {
        let _ = self
          .inode_state
          .uname
          .parse_with_confidence(ParseConfidence::Ustar, || {
            common_header_additions.parse_uname().map(String::from)
          });
        let _ = self
          .inode_state
          .gname
          .parse_with_confidence(ParseConfidence::Ustar, || {
            common_header_additions.parse_gname().map(String::from)
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

        // If there is already a path with a confidence of USTAR or less, we want to prefix the path with the ustar prefix.
        // If there is no path, we want to use the ustar prefix as the path.
        if let Some(potential_path) = self
          .inode_state
          .file_path
          .get_with_less_or_equal_confidence(ParseConfidence::Ustar)
        {
          let prefix = ustar_additions
            .parse_prefix()
            .map(RelativePathBuf::from)
            .unwrap_or_else(|_| RelativePathBuf::from(""));
          self
            .inode_state
            .file_path
            .force_insert_confidence(ParseConfidence::Ustar, prefix.join(potential_path));
        } else {
          let _ = self
            .inode_state
            .file_path
            .parse_with_confidence(ParseConfidence::Ustar, || {
              ustar_additions
                .parse_prefix()
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
        let _ = self
          .inode_state
          .mtime
          .parse_with_confidence(ParseConfidence::Gnu, || {
            gnu_additions
              .parse_atime()
              .or_else(|_| gnu_additions.parse_ctime())
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
    // We parsed everything from the header block and released the buffer.

    let data_after_header_block_aligned = (data_after_header + 511) & !511; // align to next 512 byte block

    /*let mut gnu_parse_long_name = |output: &mut String,
                                   context: &'static str|
     -> Result<(), TarExtractionError<R::ReadExactError>> {
      let long_file_name_bytes = &reader
        .read_exact(data_after_header_block_aligned)
        .map_err(TarExtractionError::Io)?[..data_after_header];
      let long_file_name = str::from_utf8(long_file_name_bytes)
        .map_err(|e| TarExtractionError::InvalidUtf8InFileName(context, e))?;
      output.clear();
      output.push_str(long_file_name);
      Ok(())
    };*/

    /*let mut parse_pax_data = |global: bool| -> Result<(), TarExtractionError<R::ReadExactError>> {
      // We read the next block and parse the PAX data.
      let pax_data_bytes = &reader
        .read_exact(data_after_header_block_aligned)
        .map_err(TarExtractionError::Io)?[..data_after_header];
      todo!()
    };*/

    // now we match on the typeflag
    match typeflag {
      TarTypeFlag::CharacterDevice => {
        self.finish_inode(FileEntry::CharacterDevice(CharacterDeviceEntry {
          major: potential_dev_major.unwrap_or(0),
          minor: potential_dev_minor.unwrap_or(0),
        }));
      },
      TarTypeFlag::BlockDevice => {
        self.finish_inode(FileEntry::BlockDevice(BlockDeviceEntry {
          major: potential_dev_major.unwrap_or(0),
          minor: potential_dev_minor.unwrap_or(0),
        }));
      },
      TarTypeFlag::Fifo => {
        self.finish_inode(FileEntry::Fifo);
      },
      /*TarTypeFlag::PaxExtendedHeader => {
        // We read the next block and parse the PAX data.
        parse_pax_data(false)?;
      },
      TarTypeFlag::PaxGlobalExtendedHeader => {
        // We read the next block and parse the PAX data.
        parse_pax_data(true)?;
      },*/
      /*TarTypeFlag::LongNameGnu => {
        gnu_parse_long_name(&mut gnu_long_file_name, "GNU long file name")?;
      },
      TarTypeFlag::LongLinkNameGnu => {
        gnu_parse_long_name(&mut gnu_long_link_name, "GNU long link name")?;
      },*/
      /*TarTypeFlag::SparseOldGnu => {
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
      },*/
      TarTypeFlag::UnknownTypeFlag(_) => {
        // we just skip the data_after_header bytes if we don't know the typeflag
        // TODO: make this a parser enum state and skip incrementally
        reader
          .skip(data_after_header_block_aligned)
          .map_err(|err| Self::map_reader_error(err, "Unknown typeflag, skipping data"))?;
      },
      _ => todo!(),
    }

    // reset state here

    // todo: prefill next inode builder with pax global state

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
