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
    confident_value::ConfidentValue,
    tar_constants::{
      pax_keys_well_known::{
        gnu::{
          GNU_SPARSE_DATA_BLOCK_OFFSET, GNU_SPARSE_DATA_BLOCK_SIZE, GNU_SPARSE_NAME,
          GNU_SPARSE_REALSIZE, GNU_SPARSE_REALSIZE_OLD,
        },
        PATH,
      },
      CommonHeaderAdditions, GnuHeaderAdditions, GnuHeaderExtSparse, GnuSparseInstruction,
      ParseOctalError, TarHeaderChecksumError, TarTypeFlag, UstarHeaderAdditions, V7Header,
    },
    BlockDeviceEntry, CharacterDeviceEntry, FileEntry, FilePermissions, TarInode,
  },
  BufferedRead as _, ReadExactError, Write,
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
enum TarConfidence {
  V7 = 1,
  Ustar,
  Gnu,
  PaxGlobal,
  Pax,
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

#[derive(PartialEq, Eq, Clone, Debug)]
enum SparseFormat {
  GnuOld,
  Gnu0_0,
  Gnu0_1,
  Gnu1_0,
}

enum GnuLongNameType {
  FileName,
  LinkName,
}

pub struct StateExpectingOldGnuSparseExtendedHeader {
  /// The size of the data section following the old gnu sparse extended headers.
  data_after_header: usize,
  /// The amount of padding after the data section.
  padding_after_data: usize,
}

pub struct StateSkippingData {
  /// The amount of data that must be skipped.
  remaining_data: usize,
  /// The context for the skipped data, used for error messages and debugging.
  context: &'static str,
}

pub struct StateParsingGnuLongName {
  /// The amount of data that is still remaining to be read.
  remaining_data: usize,
  /// The amount of padding after the long name data.
  padding_after_data: usize,
  /// The type of the long name (file name or link name).
  long_name_type: GnuLongNameType,
  /// The collected long name bytes.
  collected_name: Vec<u8>,
}

struct StateReadingFileData {
  /// The amount of data that is still remaining to be read.
  remaining_data: usize,
  /// The amount of padding after the file data.
  padding_after: usize,
}

#[derive(Default)]
enum ParserState {
  #[default]
  ExpectingTarHeader,
  ParsingPaxData,
  ExpectingOldGnuSparseExtendedHeader(StateExpectingOldGnuSparseExtendedHeader),
  SkippingData(StateSkippingData),
  ParsingGnuLongName(StateParsingGnuLongName),
  ReadingFileData(StateReadingFileData),
  NoNextStateSet,
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

type InodeConfidentValue<T> = ConfidentValue<TarConfidence, T>;

#[derive(Default)]
struct InodeBuilder {
  file_path: InodeConfidentValue<RelativePathBuf>,
  mode: Option<FilePermissions>,
  uid: InodeConfidentValue<u32>,
  gid: InodeConfidentValue<u32>,
  mtime: InodeConfidentValue<u64>,
  uname: InodeConfidentValue<String>,
  gname: InodeConfidentValue<String>,
  link_target: InodeConfidentValue<String>,
  sparse_file_instructions: Vec<SparseFileInstruction>,
  /// The realsize if it is a sparse file.
  sparse_real_size: InodeConfidentValue<usize>,
  sparse_format: Option<SparseFormat>,
  dev_major: u32,
  dev_minor: u32,
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

  fn load_pax_into_parser(&mut self, parse_confidence: TarConfidence) {
    self
      .inode_state
      .file_path
      .get_or_set_with(parse_confidence, || {
        self
          .pax_state
          .get_attribute(GNU_SPARSE_NAME)
          .map(RelativePathBuf::from)
      });
    self
      .inode_state
      .file_path
      .get_or_set_with(parse_confidence, || {
        self
          .pax_state
          .get_attribute(PATH)
          .map(RelativePathBuf::from)
      });
    self
      .inode_state
      .sparse_real_size
      .get_or_set_with(parse_confidence, || {
        self
          .pax_state
          .get_attribute(GNU_SPARSE_REALSIZE)
          .and_then(|s| s.parse().ok())
      });
    self
      .inode_state
      .sparse_real_size
      .get_or_set_with(parse_confidence, || {
        self
          .pax_state
          .get_attribute(GNU_SPARSE_REALSIZE_OLD)
          .and_then(|s| s.parse().ok())
      });
  }

  pub fn recover(&mut self) {
    self.pax_state.reset_local();
    self.load_pax_into_parser(TarConfidence::PaxGlobal);
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
    debug_assert_eq!(self.inode_state.sparse_format, Some(SparseFormat::GnuOld));
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

  fn finish_inode(&mut self, file_entry: impl FnOnce(&mut Self) -> FileEntry) -> ParserState {
    self.load_pax_into_parser(TarConfidence::Pax);
    let file_entry = file_entry(self);
    //let mut inode = TarInode {};
    todo!()
  }

  fn state_expecting_tar_header(
    &mut self,
    reader: &mut Cursor<&[u8]>,
  ) -> Result<ParserState, TarParserError> {
    // header parsing variables
    let mut data_after_header = 0;
    let mut typeflag = TarTypeFlag::UnknownTypeFlag(255);
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
        .try_get_or_set_with(TarConfidence::V7, || {
          old_header.parse_name().map(RelativePathBuf::from)
        });
      self
        .inode_state
        .mode
        .get_or_insert_with_maybe(|| old_header.parse_mode());
      let _ = self
        .inode_state
        .uid
        .try_get_or_set_with(TarConfidence::V7, || old_header.parse_uid());
      let _ = self
        .inode_state
        .gid
        .try_get_or_set_with(TarConfidence::V7, || old_header.parse_gid());
      if let Ok(size) = old_header.parse_size() {
        data_after_header = size as usize;
      }

      let _ = self
        .inode_state
        .mtime
        .try_get_or_set_with(TarConfidence::V7, || old_header.parse_mtime());

      typeflag = old_header.parse_typeflag();
      if let Some(count) = self.found_type_flags.get_mut(&typeflag) {
        *count += 1;
      } else {
        self.found_type_flags.insert(typeflag.clone(), 1);
      }

      let _ = self
        .inode_state
        .link_target
        .try_get_or_set_with(TarConfidence::V7, || {
          old_header.parse_linkname().map(String::from)
        });

      Ok(())
    };

    let mut parse_common_header_additions =
      |common_header_additions: &CommonHeaderAdditions| -> Result<(), TarParserError> {
        let _ = self
          .inode_state
          .uname
          .try_get_or_set_with(TarConfidence::Ustar, || {
            common_header_additions.parse_uname().map(String::from)
          });
        let _ = self
          .inode_state
          .gname
          .try_get_or_set_with(TarConfidence::Ustar, || {
            common_header_additions.parse_gname().map(String::from)
          });
        self.inode_state.dev_major = common_header_additions.parse_dev_major().unwrap_or(0);
        self.inode_state.dev_minor = common_header_additions.parse_dev_minor().unwrap_or(0);
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
          .get_if_confidence_le(&TarConfidence::Ustar)
        {
          let prefix = ustar_additions
            .parse_prefix()
            .map(RelativePathBuf::from)
            .unwrap_or_else(|_| RelativePathBuf::from(""));
          self
            .inode_state
            .file_path
            .set(TarConfidence::Ustar, prefix.join(potential_path));
        } else {
          let _ = self
            .inode_state
            .file_path
            .try_get_or_set_with(TarConfidence::Ustar, || {
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
          .try_get_or_set_with(TarConfidence::Gnu, || {
            gnu_additions
              .parse_atime()
              .or_else(|_| gnu_additions.parse_ctime())
          });

        // Handle sparse entries (Old GNU Format)
        if typeflag == TarTypeFlag::SparseOldGnu {
          self.inode_state.sparse_format = Some(SparseFormat::GnuOld);
          self.parse_old_gnu_sparse_instructions(&gnu_additions.sparse);
          old_gnu_sparse_is_extended = gnu_additions.parse_is_extended();
        }

        let _ = self
          .inode_state
          .sparse_real_size
          .try_get_or_set_with(TarConfidence::Gnu, || {
            gnu_additions.parse_real_size().map(|s| s as usize)
          });

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
    let padding_after_data = data_after_header_block_aligned - data_after_header; // padding after header block

    /*let mut parse_pax_data = |global: bool| -> Result<(), TarExtractionError<R::ReadExactError>> {
      // We read the next block and parse the PAX data.
      let pax_data_bytes = &reader
        .read_exact(data_after_header_block_aligned)
        .map_err(TarExtractionError::Io)?[..data_after_header];
      todo!()
    };*/

    // now we match on the typeflag
    Ok(match typeflag {
      TarTypeFlag::CharacterDevice => self.finish_inode(|selv| {
        FileEntry::CharacterDevice(CharacterDeviceEntry {
          major: selv.inode_state.dev_major,
          minor: selv.inode_state.dev_minor,
        })
      }),
      TarTypeFlag::BlockDevice => self.finish_inode(|selv| {
        FileEntry::BlockDevice(BlockDeviceEntry {
          major: selv.inode_state.dev_major,
          minor: selv.inode_state.dev_minor,
        })
      }),
      TarTypeFlag::Fifo => self.finish_inode(|_| FileEntry::Fifo),
      /*TarTypeFlag::PaxExtendedHeader => {
        // We read the next block and parse the PAX data.
        parse_pax_data(false)?;
      },
      TarTypeFlag::PaxGlobalExtendedHeader => {
        // We read the next block and parse the PAX data.
        parse_pax_data(true)?;
      },*/
      TarTypeFlag::LongNameGnu => {
        ParserState::ParsingGnuLongName(StateParsingGnuLongName {
          remaining_data: data_after_header,
          padding_after_data,
          long_name_type: GnuLongNameType::FileName,
          collected_name: Vec::new(), // We don't use with_capacity here since this is a user controlled value and we don't want to exhaust resources.
        })
      },
      TarTypeFlag::LongLinkNameGnu => {
        ParserState::ParsingGnuLongName(StateParsingGnuLongName {
          remaining_data: data_after_header,
          padding_after_data,
          long_name_type: GnuLongNameType::LinkName,
          collected_name: Vec::new(), // We don't use with_capacity here since this is a user controlled value and we don't want to exhaust resources.
        })
      },
      TarTypeFlag::SparseOldGnu => {
        if old_gnu_sparse_is_extended {
          ParserState::ExpectingOldGnuSparseExtendedHeader(
            StateExpectingOldGnuSparseExtendedHeader {
              data_after_header,
              padding_after_data,
            },
          )
        } else {
          ParserState::ExpectingTarHeader
        }
      },
      TarTypeFlag::UnknownTypeFlag(_) => {
        // we just skip the data_after_header bytes if we don't know the typeflag
        ParserState::SkippingData(StateSkippingData {
          remaining_data: data_after_header_block_aligned,
          context: "Unknown typeflag",
        })
      },
      _ => todo!(),
    })
  }

  fn state_skipping_data(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    state_skipping_data: StateSkippingData,
  ) -> Result<ParserState, TarParserError> {
    let StateSkippingData {
      remaining_data,
      context,
    } = state_skipping_data;

    // incrementally skip the data
    let bytes_to_skip = remaining_data.min(reader.remaining());
    reader
      .skip(bytes_to_skip)
      .expect("BUG: Incremental unknown data skipping failed");
    let remaining_data = remaining_data - bytes_to_skip;
    Ok(if remaining_data == 0 {
      // We are done skipping unknown data, so we reset the parser state.
      ParserState::ExpectingTarHeader
    } else {
      // We still have some data to skip, so we keep the parser state.
      ParserState::SkippingData(StateSkippingData {
        remaining_data,
        context,
      })
    })
  }

  fn state_parsing_gnu_long_name(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    state_parsing_gnu_long_name: StateParsingGnuLongName,
  ) -> Result<ParserState, TarParserError> {
    let StateParsingGnuLongName {
      remaining_data,
      padding_after_data: padding_after,
      long_name_type,
      mut collected_name,
    } = state_parsing_gnu_long_name;

    // incrementally read the long name
    let bytes_to_read = remaining_data.min(reader.remaining());
    let long_name_bytes = reader
      .read_exact(bytes_to_read)
      .expect("BUG: Incremental long name reading failed");

    collected_name.extend_from_slice(long_name_bytes);
    let remaining_data = remaining_data - bytes_to_read;
    Ok(if remaining_data == 0 {
      // We are done reading the long name, so we parse it.
      let long_name = String::from_utf8(collected_name);

      if let Ok(long_name) = long_name {
        // Now we can insert the long name into the inode state.
        match long_name_type {
          GnuLongNameType::FileName => {
            self
              .inode_state
              .file_path
              .get_or_set_with(TarConfidence::Gnu, || {
                Some(RelativePathBuf::from(long_name))
              });
          },
          GnuLongNameType::LinkName => {
            self
              .inode_state
              .link_target
              .get_or_set_with(TarConfidence::Gnu, || Some(long_name));
          },
        }
      } else {
        // TODO: log this
      }

      if padding_after > 0 {
        // We have some padding after the long name, so we skip it.
        ParserState::SkippingData(StateSkippingData {
          remaining_data: padding_after,
          context: "Padding after long name",
        })
      } else {
        // We are done with the long name and there is no padding, so we reset the parser state.
        ParserState::ExpectingTarHeader
      }
    } else {
      // We still have some data to read, so we keep the parser state.
      ParserState::ParsingGnuLongName(StateParsingGnuLongName {
        remaining_data,
        padding_after_data: padding_after,
        long_name_type,
        collected_name,
      })
    })
  }

  fn state_expecting_old_gnu_sparse_extended_header(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    state: StateExpectingOldGnuSparseExtendedHeader,
  ) -> Result<ParserState, TarParserError> {
    let StateExpectingOldGnuSparseExtendedHeader {
      data_after_header,
      padding_after_data,
    } = state;

    // We must read the next block to get more sparse headers.

    let extended_header_buffer = reader.read_exact(512).map_err(|err| {
      Self::map_reader_error(
        err,
        "Only full old gnu sparse extended headers can be parsed",
      )
    })?;
    let extended_header = GnuHeaderExtSparse::ref_from_bytes(&extended_header_buffer)
      .expect("BUG: Not enough bytes for GnuHeaderExtSparse");
    self.parse_old_gnu_sparse_instructions(&extended_header.sparse);
    Ok(if extended_header.parse_is_extended() {
      // If the extended header is still extended, we need to read the next block.
      ParserState::ExpectingOldGnuSparseExtendedHeader(StateExpectingOldGnuSparseExtendedHeader {
        data_after_header,
        padding_after_data,
      })
    } else {
      ParserState::ReadingFileData(StateReadingFileData {
        remaining_data: data_after_header,
        padding_after: padding_after_data,
      })
    })
  }
}

impl Write for TarParser {
  type WriteError = TarParserError;
  type FlushError = Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    // TODO: add loop here?
    let mut reader = Cursor::new(input_buffer);

    let parser_state = core::mem::replace(&mut self.parser_state, ParserState::NoNextStateSet);

    let parse_write_result = match parser_state {
      ParserState::ExpectingTarHeader => self.state_expecting_tar_header(&mut reader),
      ParserState::SkippingData(state_skipping_data) => {
        self.state_skipping_data(&mut reader, state_skipping_data)
      },
      ParserState::ParsingGnuLongName(state_parsing_gnu_long_name) => {
        self.state_parsing_gnu_long_name(&mut reader, state_parsing_gnu_long_name)
      },
      ParserState::ExpectingOldGnuSparseExtendedHeader(state) => {
        self.state_expecting_old_gnu_sparse_extended_header(&mut reader, state)
      },
      ParserState::NoNextStateSet => {
        panic!("BUG: No next state set in TarParser");
      },
      _ => {
        todo!()
      },
    };
    parse_write_result.map(|next_parser_state| {
      self.parser_state = next_parser_state;
      reader.position()
    })
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}
