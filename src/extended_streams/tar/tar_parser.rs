use core::{convert::Infallible, num::ParseIntError};

use alloc::{
  string::{String, ToString as _},
  vec::Vec,
};

use hashbrown::HashMap;
use relative_path::RelativePathBuf;
use thiserror::Error;
use zerocopy::FromBytes as _;

use crate::{
  core_streams::Cursor,
  extended_streams::tar::{
    confident_value::ConfidentValue,
    gnu_sparse_1_0_parser::GnuSparse1_0Parser,
    pax_parser::{PaxConfidence, PaxConfidentValue, PaxParser},
    tar_constants::{
      find_null_terminator_index, CommonHeaderAdditions, GnuHeaderAdditions, GnuHeaderExtSparse,
      GnuSparseInstruction, ParseOctalError, TarHeaderChecksumError, TarTypeFlag,
      UstarHeaderAdditions, V7Header, BLOCK_SIZE, TAR_ZERO_HEADER,
    },
    BlockDeviceEntry, CharacterDeviceEntry, FileData, FileEntry, FilePermissions, HardLinkEntry,
    RegularFileEntry, SparseFileInstruction, SymbolicLinkEntry, TarInode, TimeStamp,
  },
  BufferedRead as _, Write, WriteAll as _,
};

/// TODO: unify and cleanup
#[derive(Error, Debug, PartialEq, Eq)]
pub enum TarParserError {
  #[error("Corrupt header: Checksum error: {0}")]
  CorruptHeaderChecksum(#[from] TarHeaderChecksumError),
  #[error("Corrupt header: Unknown magic or version: {magic:?} {version:?}")]
  CorruptHeaderMagicVersion { magic: [u8; 6], version: [u8; 2] },
  #[error(
    "Corrupt pax length field: The length field is longer than {max_length_field_length} bytes"
  )]
  CorruptPaxLength { max_length_field_length: usize },
  #[error("Corrupt pax length field: {0}")]
  CorruptPaxLengthInteger(ParseIntError),
  #[error("Corrupt pax key")]
  CorruptPaxKey,
  #[error("Corrupt pax value")]
  CorruptPaxValue,
  #[error("Corrupt gnu sparse 1.0 maps: The number of maps field is longer than {max_number_of_maps_field_length} bytes")]
  CorruptGnuSparse1_0NumberOfMaps {
    max_number_of_maps_field_length: usize,
  },
  #[error("Corrupt gnu sparse 1.0 maps: The number of maps field is invalid: {0}")]
  CorruptGnuSparse1_0NumberOfMapsInteger(ParseIntError),
  #[error("Corrupt gnu sparse 1.0 map entry: The entry length field is longer than {max_value_field_length} bytes")]
  CorruptGnuSparse1_0MapEntryLength { max_value_field_length: usize },
  #[error("Corrupt gnu sparse 1.0 map entry: The entry length field is invalid: {0}")]
  CorruptGnuSparse1_0MapEntryInteger(ParseIntError),
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum TarConfidence {
  V7 = 1,
  Ustar,
  Gnu,
  PaxGlobal,
  PaxLocal,
}

impl From<PaxConfidence> for TarConfidence {
  fn from(value: PaxConfidence) -> Self {
    match value {
      PaxConfidence::LOCAL => TarConfidence::PaxLocal,
      PaxConfidence::GLOBAL => TarConfidence::PaxGlobal,
    }
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
pub(crate) trait GetOrInsertWithOption<T> {
  /// If `self` is Some, returns a mutable reference to the value.
  /// Otherwise, runs the closure. If the closure returns Some, inserts it and returns a mutable reference.
  /// If the closure returns None, leaves `self` as None and returns None.
  fn get_or_insert_with_option<F>(&mut self, f: F) -> Option<&mut T>
  where
    F: FnOnce() -> Option<T>;
}

impl<T> GetOrInsertWithOption<T> for Option<T> {
  fn get_or_insert_with_option<F>(&mut self, f: F) -> Option<&mut T>
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
pub(crate) enum SparseFormat {
  GnuOld,
  Gnu0_0,
  Gnu0_1,
  Gnu1_0,
  GnuUnknownSparseFormat { major: u32, minor: u32 },
}

impl SparseFormat {
  /// Returns the major and minor version of the GNU sparse format.
  #[must_use]
  pub fn get_major_minor(&self) -> (u32, u32) {
    match self {
      SparseFormat::GnuOld => (0, 0),
      SparseFormat::Gnu0_0 => (0, 0),
      SparseFormat::Gnu0_1 => (0, 1),
      SparseFormat::Gnu1_0 => (1, 0),
      SparseFormat::GnuUnknownSparseFormat { major, minor } => (*major, *minor),
    }
  }

  /// Creates a new `SparseFormat` from the major and minor version.
  #[must_use]
  pub fn try_from_gnu_version(major: Option<u32>, minor: Option<u32>) -> Option<Self> {
    Some(match (major, minor) {
      (Some(0), Some(0) | None) => SparseFormat::Gnu0_0,
      (Some(0) | None, Some(1)) => SparseFormat::Gnu0_1,
      (Some(1), Some(0)) => SparseFormat::Gnu1_0,
      (None, None) => return None,
      (major, minor) => SparseFormat::GnuUnknownSparseFormat {
        major: major.unwrap_or(0),
        minor: minor.unwrap_or(0),
      },
    })
  }
}

enum GnuLongNameType {
  FileName,
  LinkName,
}

struct StateReadingTarHeader {
  /// The temporary buffer used for reading the tar header.
  temp_tar_header_buffer: Cursor<[u8; BLOCK_SIZE]>,
}

pub struct StateReadingOldGnuSparseExtendedHeader {
  /// The size of the data section following the old gnu sparse extended headers.
  data_after_header: usize,
  /// The amount of padding after the data section.
  padding_after_data: usize,
  /// The temporary buffer used for reading the tar header.
  temp_old_gnu_sparse_header_buffer: Cursor<[u8; BLOCK_SIZE]>,
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

struct StateParsingPaxData {
  /// The amount of data that is still remaining to be read.
  remaining_data: usize,
  /// The amount of padding after the PAX data.
  padding_after: usize,
  pax_mode: PaxConfidence,
}

struct StateParsingGnuSparse1_0 {
  /// The amount of data that is still remaining to be read.
  data_after_header: usize,
  /// The amount of padding after the file data.
  padding_after: usize,
  sparse_parser: GnuSparse1_0Parser,
}

enum TarParserState {
  ReadingTarHeader(StateReadingTarHeader),
  ReadingOldGnuSparseExtendedHeader(StateReadingOldGnuSparseExtendedHeader),
  SkippingData(StateSkippingData),
  ParsingGnuLongName(StateParsingGnuLongName),
  ReadingFileData(StateReadingFileData),
  ParsingPaxData(StateParsingPaxData),
  ParsingGnuSparse1_0(StateParsingGnuSparse1_0),
  NoNextStateSet,
}

impl Default for TarParserState {
  fn default() -> Self {
    Self::ReadingTarHeader(StateReadingTarHeader {
      temp_tar_header_buffer: Cursor::new([0; BLOCK_SIZE]),
    })
  }
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

  parser_state: TarParserState,
  /// Contains both the global and local extended attributes.
  pax_parser: PaxParser,
  // Must be reset after each file:
  inode_state: InodeBuilder,
}

pub(crate) fn buffer_array<'a, const BUFFER_SIZE: usize>(
  reader: &'a mut Cursor<&[u8]>,
  temp_buffer: &'a mut Cursor<[u8; BUFFER_SIZE]>,
) -> Result<Option<&'a [u8]>, TarParserError> {
  // perform an incremental read into the tar header buffer
  let bytes_to_read = temp_buffer.remaining().min(reader.remaining());

  if bytes_to_read == BUFFER_SIZE {
    // We can directly pass through the buffer so we don't have to copy it to the intermediate buffer.
    return Ok(Some(
      reader
        .read_exact(BUFFER_SIZE)
        .expect("BUG: buffer_array direct patch through failed"),
    ));
  }

  // read bytes into the tar header buffer
  let read_bytes = reader
    .read_exact(bytes_to_read)
    .expect("BUG: buffer_array incremental read failed");
  temp_buffer
    .write_all(read_bytes, false)
    .expect("BUG: buffer_array incremental write failed");
  if temp_buffer.remaining() == 0 {
    // We have a complete tar header block, so we can return it.
    temp_buffer.set_position(0); // reset the cursor for the next read
    let header_buffer = temp_buffer.after();
    Ok(Some(header_buffer))
  } else {
    // We don't have a complete tar header block yet, so we return None.
    Ok(None)
  }
}

pub(crate) type InodeConfidentValue<T> = ConfidentValue<TarConfidence, T>;

impl<T: Clone> From<PaxConfidentValue<T>> for InodeConfidentValue<T> {
  fn from(value: PaxConfidentValue<T>) -> Self {
    let mut confident_value = ConfidentValue::default();
    if let Some((pax_confidence, value)) = value.get_with_confidence() {
      confident_value.set(TarConfidence::from(pax_confidence), value.clone());
    }
    confident_value
  }
}

#[derive(Default)]
pub(crate) struct InodeBuilder {
  pub(crate) file_path: InodeConfidentValue<RelativePathBuf>,
  pub(crate) mode: Option<FilePermissions>,
  pub(crate) uid: InodeConfidentValue<u32>,
  pub(crate) gid: InodeConfidentValue<u32>,
  pub(crate) mtime: InodeConfidentValue<TimeStamp>,
  pub(crate) uname: InodeConfidentValue<String>,
  pub(crate) gname: InodeConfidentValue<String>,
  pub(crate) link_target: InodeConfidentValue<String>,
  pub(crate) sparse_file_instructions: Vec<SparseFileInstruction>,
  /// The realsize if it is a sparse file.
  pub(crate) sparse_real_size: InodeConfidentValue<usize>,
  pub(crate) sparse_format: Option<SparseFormat>,
  pub(crate) dev_major: u32,
  pub(crate) dev_minor: u32,
  pub(crate) data_after_header_size: InodeConfidentValue<usize>,
  pub(crate) continuous_file: bool,
  pub(crate) data: Vec<u8>,
}

impl From<InodeBuilder> for RegularFileEntry {
  fn from(inode_builder: InodeBuilder) -> Self {
    let continuous = inode_builder.continuous_file;
    let data = if inode_builder.sparse_file_instructions.is_empty() {
      FileData::Regular(inode_builder.data)
    } else {
      FileData::Sparse {
        instructions: inode_builder.sparse_file_instructions,
        data: inode_builder.data,
      }
    };

    Self { continuous, data }
  }
}

impl TarParser {
  pub fn new(options: TarParserOptions) -> Self {
    Self {
      extracted_files: Default::default(),

      found_type_flags: Default::default(),
      seen_files: Default::default(),
      keep_only_last: options.keep_only_last,

      parser_state: Default::default(),
      pax_parser: PaxParser::new(options.initial_global_extended_attributes),
      inode_state: Default::default(),
    }
  }

  fn recover_internal(&mut self) -> InodeBuilder {
    self.pax_parser.recover();
    self
      .pax_parser
      .load_pax_attributes_into_inode_builder(&mut self.inode_state);
    self.parser_state = Default::default();
    core::mem::replace(&mut self.inode_state, Default::default())
  }

  pub fn recover(&mut self) {
    self.recover_internal();
  }

  /// Returns the currently active global extended pax attributes.
  pub fn get_global_extended_attributes(&self) -> &HashMap<String, String> {
    &self.pax_parser.global_extended_attributes()
  }

  /// Returns the files that have been extracted so far.
  pub fn get_extracted_files(&self) -> &[TarInode] {
    &self.extracted_files
  }

  /// Returns the number of files found with each type flag.
  pub fn get_found_type_flags(&self) -> &HashMap<TarTypeFlag, usize> {
    &self.found_type_flags
  }

  fn parse_old_gnu_sparse_instructions(
    inode_state: &mut InodeBuilder,
    sparse_headers: &[GnuSparseInstruction],
  ) {
    debug_assert_eq!(inode_state.sparse_format, Some(SparseFormat::GnuOld));
    for sparse_header in sparse_headers {
      if sparse_header.is_empty() {
        continue;
      }
      if let Ok(instruction) = sparse_header.convert_to_sparse_instruction() {
        inode_state.sparse_file_instructions.push(instruction);
      } else {
        // If we can't parse the sparse header, we just ignore it.
        // This is a best-effort approach.
      }
    }
  }

  fn finish_inode(&mut self, file_entry: impl FnOnce(&mut Self, InodeBuilder) -> FileEntry) {
    self
      .pax_parser
      .load_pax_attributes_into_inode_builder(&mut self.inode_state);
    let inode_builder = self.recover_internal();

    // These clones can definitely be optimized.
    // Splitting the Inode builder into two parts would be a good start.
    let tar_inode = TarInode {
      path: inode_builder
        .file_path
        .get()
        .cloned()
        .unwrap_or_else(|| RelativePathBuf::from("")),
      entry: FileEntry::Fifo,
      mode: inode_builder
        .mode
        .clone()
        .unwrap_or_else(|| FilePermissions::default()),
      uid: inode_builder.uid.get().cloned().unwrap_or(0),
      gid: inode_builder.gid.get().cloned().unwrap_or(0),
      mtime: inode_builder.mtime.get().cloned().unwrap_or_default(),
      uname: inode_builder.uname.get().cloned().unwrap_or_default(),
      gname: inode_builder.gname.get().cloned().unwrap_or_default(),
      unparsed_extended_attributes: self.pax_parser.drain_local_unparsed_attributes(),
    };

    let file_entry = file_entry(self, inode_builder);

    // If we are keeping only the last version of each file, we check if we have seen this file before.
    if self.keep_only_last {
      if let Some(index) = self.seen_files.get(&tar_inode.path) {
        // We have seen this file before, so we replace the old entry.
        self.extracted_files[*index] = TarInode {
          entry: file_entry,
          ..tar_inode
        };
      } else {
        // We haven't seen this file before, so we add it to the list.
        self
          .seen_files
          .insert(tar_inode.path.clone(), self.extracted_files.len());
        self.extracted_files.push(TarInode {
          entry: file_entry,
          ..tar_inode
        });
      }
    } else {
      // We just add the new file to the list.
      self.extracted_files.push(TarInode {
        entry: file_entry,
        ..tar_inode
      });
    }
  }

  fn compute_file_parsing_state(
    &mut self,
    data_after_header: usize,
    padding_after_data: usize,
  ) -> TarParserState {
    if self.inode_state.sparse_format == Some(SparseFormat::Gnu1_0) {
      TarParserState::ParsingGnuSparse1_0(StateParsingGnuSparse1_0 {
        data_after_header,
        padding_after: padding_after_data,
        sparse_parser: GnuSparse1_0Parser::default(),
      })
    } else {
      TarParserState::ReadingFileData(StateReadingFileData {
        remaining_data: data_after_header,
        padding_after: padding_after_data,
      })
    }
  }

  fn compute_opt_skip_state(
    &mut self,
    data_after_header: usize,
    context: &'static str,
  ) -> TarParserState {
    if data_after_header > 0 {
      TarParserState::SkippingData(StateSkippingData {
        remaining_data: data_after_header,
        context,
      })
    } else {
      TarParserState::default()
    }
  }

  fn state_reading_tar_header(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateReadingTarHeader,
  ) -> Result<TarParserState, TarParserError> {
    // header parsing variables
    let mut typeflag = TarTypeFlag::UnknownTypeFlag(255);
    let mut old_gnu_sparse_is_extended = false;

    let header_buffer = match buffer_array(reader, &mut state.temp_tar_header_buffer)? {
      Some(buffer) => buffer,
      None => {
        // We don't have a complete buffer yet, so we need to wait for more data.
        return Ok(TarParserState::ReadingTarHeader(state));
      },
    };

    if header_buffer == TAR_ZERO_HEADER {
      // We have reached the end of the tar archive.
      // However we remain ready to read the next header.
      return Ok(TarParserState::default());
    }

    let old_header =
      V7Header::ref_from_bytes(&header_buffer).expect("BUG: Not enough bytes for OldHeader");

    let mut parse_v7_header = || -> Result<(), TarParserError> {
      // verify checksum
      old_header
        .verify_checksum()
        .map_err(TarParserError::CorruptHeaderChecksum)?;

      typeflag = old_header.parse_typeflag();
      if let Some(count) = self.found_type_flags.get_mut(&typeflag) {
        *count += 1;
      } else {
        self.found_type_flags.insert(typeflag.clone(), 1);
      }

      // parse the information from the old header
      let _ = self
        .inode_state
        .data_after_header_size
        .try_get_or_set_with(TarConfidence::V7, || {
          old_header.parse_size().map(|s| s as usize)
        });

      if typeflag.is_file_like() {
        let _ = self
          .inode_state
          .file_path
          .try_get_or_set_with(TarConfidence::V7, || {
            old_header.parse_name().map(RelativePathBuf::from)
          });
        self
          .inode_state
          .mode
          .get_or_insert_with_option(|| old_header.parse_mode());
        let _ = self
          .inode_state
          .uid
          .try_get_or_set_with(TarConfidence::V7, || old_header.parse_uid());
        let _ = self
          .inode_state
          .gid
          .try_get_or_set_with(TarConfidence::V7, || old_header.parse_gid());

        let _ = self
          .inode_state
          .mtime
          .try_get_or_set_with(TarConfidence::V7, || old_header.parse_mtime());
      }

      if typeflag.is_link_like() {
        let _ = self
          .inode_state
          .link_target
          .try_get_or_set_with(TarConfidence::V7, || {
            old_header.parse_linkname().map(String::from)
          });
      }

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

        if typeflag.is_file_like() {
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
        }

        // Done ustar header parsing.
      },
      V7Header::MAGIC_VERSION_GNU => {
        parse_v7_header()?;

        if typeflag.is_file_like() {
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
            Self::parse_old_gnu_sparse_instructions(&mut self.inode_state, &gnu_additions.sparse);
            old_gnu_sparse_is_extended = gnu_additions.parse_is_extended();
          }

          let _ = self
            .inode_state
            .sparse_real_size
            .try_get_or_set_with(TarConfidence::Gnu, || {
              gnu_additions.parse_real_size().map(|s| s as usize)
            });
        }

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

    let data_after_header = *self.inode_state.data_after_header_size.get().unwrap_or(&0);
    let data_after_header_block_aligned = (data_after_header + 511) & !511; // align to next 512 byte block
    let padding_after_data = data_after_header_block_aligned - data_after_header; // padding after header block

    // now we match on the typeflag
    Ok(match typeflag {
      TarTypeFlag::RegularFile => {
        self.inode_state.continuous_file = false;
        self.compute_file_parsing_state(data_after_header, padding_after_data)
      },
      TarTypeFlag::HardLink => {
        self.finish_inode(|selv, inode_state| {
          FileEntry::HardLink(HardLinkEntry {
            link_target: inode_state
              .link_target
              .get()
              .map(|s| RelativePathBuf::from(s))
              .unwrap_or_default(),
          })
        });
        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after HardLink")
      },
      TarTypeFlag::SymbolicLink => {
        self.finish_inode(|selv, inode_state| {
          FileEntry::SymbolicLink(SymbolicLinkEntry {
            link_target: inode_state
              .link_target
              .get()
              .map(|s| RelativePathBuf::from(s))
              .unwrap_or_default(),
          })
        });

        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after SymbolicLink")
      },
      TarTypeFlag::CharacterDevice => {
        self.finish_inode(|selv, inode_state| {
          FileEntry::CharacterDevice(CharacterDeviceEntry {
            major: inode_state.dev_major,
            minor: inode_state.dev_minor,
          })
        });

        self.compute_opt_skip_state(
          data_after_header_block_aligned,
          "Data after CharacterDevice",
        )
      },
      TarTypeFlag::BlockDevice => {
        self.finish_inode(|selv, inode_state| {
          FileEntry::BlockDevice(BlockDeviceEntry {
            major: inode_state.dev_major,
            minor: inode_state.dev_minor,
          })
        });
        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after BlockDevice")
      },
      TarTypeFlag::Directory => {
        self.finish_inode(|_, _| FileEntry::Directory);
        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after Directory")
      },
      TarTypeFlag::Fifo => {
        self.finish_inode(|_, _| FileEntry::Fifo);
        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after Fifo")
      },
      TarTypeFlag::ContinuousFile => {
        self.inode_state.continuous_file = true;
        self.compute_file_parsing_state(data_after_header, padding_after_data)
      },
      TarTypeFlag::PaxExtendedHeader => {
        self.pax_parser.set_current_pax_mode(PaxConfidence::LOCAL);
        TarParserState::ParsingPaxData(StateParsingPaxData {
          remaining_data: data_after_header,
          padding_after: padding_after_data,
          pax_mode: PaxConfidence::LOCAL, // We are parsing a local PAX header.
        })
      },
      TarTypeFlag::PaxGlobalExtendedHeader => {
        self.pax_parser.set_current_pax_mode(PaxConfidence::GLOBAL);
        TarParserState::ParsingPaxData(StateParsingPaxData {
          remaining_data: data_after_header,
          padding_after: padding_after_data,
          pax_mode: PaxConfidence::GLOBAL, // We are parsing a local PAX header.
        })
      },
      TarTypeFlag::LongNameGnu => {
        TarParserState::ParsingGnuLongName(StateParsingGnuLongName {
          remaining_data: data_after_header,
          padding_after_data,
          long_name_type: GnuLongNameType::FileName,
          collected_name: Vec::new(), // We don't use with_capacity here since this is a user controlled value and we don't want to exhaust resources.
        })
      },
      TarTypeFlag::LongLinkNameGnu => {
        TarParserState::ParsingGnuLongName(StateParsingGnuLongName {
          remaining_data: data_after_header,
          padding_after_data,
          long_name_type: GnuLongNameType::LinkName,
          collected_name: Vec::new(), // We don't use with_capacity here since this is a user controlled value and we don't want to exhaust resources.
        })
      },
      TarTypeFlag::SparseOldGnu => {
        if old_gnu_sparse_is_extended {
          TarParserState::ReadingOldGnuSparseExtendedHeader(
            StateReadingOldGnuSparseExtendedHeader {
              data_after_header,
              padding_after_data,
              temp_old_gnu_sparse_header_buffer: Cursor::new([0; BLOCK_SIZE]),
            },
          )
        } else {
          TarParserState::default()
        }
      },
      TarTypeFlag::UnknownTypeFlag(_) => {
        // we just skip the data_after_header bytes if we don't know the typeflag
        self.compute_opt_skip_state(data_after_header_block_aligned, "Unknown typeflag")
      },
    })
  }

  fn state_skipping_data(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    state: StateSkippingData,
  ) -> Result<TarParserState, TarParserError> {
    // incrementally skip the data
    let bytes_to_skip = state.remaining_data.min(reader.remaining());
    reader
      .skip(bytes_to_skip)
      .expect("BUG: Incremental unknown data skipping failed");
    let remaining_data = state.remaining_data - bytes_to_skip;
    Ok(if remaining_data == 0 {
      // We are done skipping unknown data, so we reset the parser state.
      TarParserState::default()
    } else {
      // We still have some data to skip, so we keep the parser state.
      TarParserState::SkippingData(state)
    })
  }

  fn state_parsing_gnu_long_name(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateParsingGnuLongName,
  ) -> Result<TarParserState, TarParserError> {
    // incrementally read the long name
    let bytes_to_read = state.remaining_data.min(reader.remaining());
    let long_name_bytes = reader
      .read_exact(bytes_to_read)
      .expect("BUG: Incremental long name reading failed");

    state.collected_name.extend_from_slice(long_name_bytes);
    let remaining_data = state.remaining_data - bytes_to_read;
    Ok(if remaining_data == 0 {
      // We are done reading the long name, so we parse it.
      let null_term = find_null_terminator_index(&state.collected_name);
      state.collected_name.truncate(null_term);
      let long_name = String::from_utf8(state.collected_name);

      if let Ok(long_name) = long_name {
        // Now we can insert the long name into the inode state.
        match state.long_name_type {
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

      if state.padding_after_data > 0 {
        // We have some padding after the long name, so we skip it.
        TarParserState::SkippingData(StateSkippingData {
          remaining_data: state.padding_after_data,
          context: "Padding after long name",
        })
      } else {
        // We are done with the long name and there is no padding, so we reset the parser state.
        TarParserState::default()
      }
    } else {
      // We still have some data to read, so we keep the parser state.
      TarParserState::ParsingGnuLongName(state)
    })
  }

  fn state_expecting_old_gnu_sparse_extended_header(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateReadingOldGnuSparseExtendedHeader,
  ) -> Result<TarParserState, TarParserError> {
    // We must read the next block to get more sparse headers.

    let extended_header_buffer =
      match buffer_array(reader, &mut state.temp_old_gnu_sparse_header_buffer)? {
        Some(buffer) => buffer,
        None => {
          // We don't have a complete buffer yet, so we need to wait for more data.
          return Ok(TarParserState::ReadingOldGnuSparseExtendedHeader(state));
        },
      };

    let extended_header = GnuHeaderExtSparse::ref_from_bytes(&extended_header_buffer)
      .expect("BUG: Not enough bytes for GnuHeaderExtSparse");
    Self::parse_old_gnu_sparse_instructions(&mut self.inode_state, &extended_header.sparse);
    Ok(if extended_header.parse_is_extended() {
      // If the extended header is still extended, we need to read the next block.
      TarParserState::ReadingOldGnuSparseExtendedHeader(state)
    } else {
      TarParserState::ReadingFileData(StateReadingFileData {
        remaining_data: state.data_after_header,
        padding_after: state.padding_after_data,
      })
    })
  }

  fn state_parsing_pax_data(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateParsingPaxData,
  ) -> Result<TarParserState, TarParserError> {
    // incrementally read the PAX data
    let bytes_to_read = state.remaining_data.min(reader.remaining());
    let debug_str = str::from_utf8(reader.peek_exact(80).unwrap()).unwrap();
    let pax_bytes = reader
      .peek_exact(bytes_to_read)
      .expect("BUG: Incremental PAX data reading failed");

    // make this non-fatal just audit log it
    let bytes_read = self.pax_parser.write(pax_bytes, false)?;
    reader
      .skip(bytes_read)
      .expect("BUG: Incremental PAX data reading failed");

    state.remaining_data = state.remaining_data - bytes_read;
    Ok(if state.remaining_data == 0 {
      // We are done reading the PAX data, so we reset the parser state.
      if state.padding_after > 0 {
        // We have some padding after the PAX data, so we skip it.
        TarParserState::SkippingData(StateSkippingData {
          remaining_data: state.padding_after,
          context: "Padding after PAX data",
        })
      } else {
        TarParserState::default()
      }
    } else {
      // We still have some data to read, so we keep the parser state.
      TarParserState::ParsingPaxData(state)
    })
  }

  fn state_parsing_gnu_sparse_1_0(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateParsingGnuSparse1_0,
  ) -> Result<TarParserState, TarParserError> {
    let done = state
      .sparse_parser
      .parse(reader, &mut self.inode_state.sparse_file_instructions)?;

    if !done {
      // We still have some data to read, so we keep the parser state.
      return Ok(TarParserState::ParsingGnuSparse1_0(state));
    }

    // We are done reading the sparse data
    Ok(TarParserState::ReadingFileData(StateReadingFileData {
      remaining_data: state.data_after_header - state.sparse_parser.bytes_read,
      padding_after: state.padding_after,
    }))
  }

  fn state_reading_file_data(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateReadingFileData,
  ) -> Result<TarParserState, TarParserError> {
    // incrementally read the file data
    let bytes_to_read = state.remaining_data.min(reader.remaining());
    let file_data_bytes = reader
      .read_exact(bytes_to_read)
      .expect("BUG: Incremental file data reading failed");

    self.inode_state.data.extend_from_slice(file_data_bytes);
    state.remaining_data -= bytes_to_read;

    if state.remaining_data != 0 {
      // We still have some data to read, so we keep the parser state.
      return Ok(TarParserState::ReadingFileData(state));
    }

    // We are done reading the file data, so we can finish the inode.
    self.finish_inode(|selv, inode_state| FileEntry::RegularFile(inode_state.into()));

    Ok(self.compute_opt_skip_state(state.padding_after, "Padding after file data"))
  }
}

impl Write for TarParser {
  type WriteError = TarParserError;
  type FlushError = Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    // TODO: add loop here?
    // TODO: update this and pax parser to avoid moving the state through the parsing process
    let mut reader = Cursor::new(input_buffer);

    let parser_state = core::mem::replace(&mut self.parser_state, TarParserState::NoNextStateSet);

    let next_state: TarParserState = match parser_state {
      TarParserState::ReadingTarHeader(state) => {
        self.state_reading_tar_header(&mut reader, state)?
      },
      TarParserState::SkippingData(state) => self.state_skipping_data(&mut reader, state)?,
      TarParserState::ParsingGnuLongName(state) => {
        self.state_parsing_gnu_long_name(&mut reader, state)?
      },
      TarParserState::ReadingOldGnuSparseExtendedHeader(state) => {
        self.state_expecting_old_gnu_sparse_extended_header(&mut reader, state)?
      },
      TarParserState::ParsingPaxData(state) => self.state_parsing_pax_data(&mut reader, state)?,
      TarParserState::ParsingGnuSparse1_0(state) => {
        self.state_parsing_gnu_sparse_1_0(&mut reader, state)?
      },
      TarParserState::ReadingFileData(state) => self.state_reading_file_data(&mut reader, state)?,
      TarParserState::NoNextStateSet => {
        panic!("BUG: No next state set in TarParser");
      },
    };
    self.parser_state = next_state;
    Ok(reader.position())
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}
