use core::{marker::PhantomData, num::ParseIntError};

use alloc::string::{String, ToString};

use hashbrown::HashMap;
use thiserror::Error;

use crate::{
  extended_streams::tar::{
    corrupt_field_to_tar_err,
    gnu_sparse_1_0_parser::max_string_length_from_limit,
    limit_exceeded_to_tar_err,
    tar_constants::pax_keys_well_known::{
      gnu::{
        GNU_SPARSE_DATA_BLOCK_OFFSET_0_0, GNU_SPARSE_DATA_BLOCK_SIZE_0_0, GNU_SPARSE_MAJOR,
        GNU_SPARSE_MAP_0_1, GNU_SPARSE_MAP_NUM_BLOCKS_0_01, GNU_SPARSE_MINOR,
        GNU_SPARSE_NAME_01_01, GNU_SPARSE_REALSIZE_0_01, GNU_SPARSE_REALSIZE_1_0,
      },
      ATIME, CTIME, GID, GNAME, LINKPATH, MTIME, PATH, SIZE, UID, UNAME,
    },
    CorruptFieldContext, IgnoreTarViolationHandler, InodeBuilder, InodeConfidentValue,
    LimitExceededContext, SparseFileInstruction, SparseFormat, TarParserError, TarViolationHandler,
    TimeStamp, VHW,
  },
  BufferedRead, CopyBuffered as _, CopyUntilError, Cursor, FixedSizeBufferError, LimitedHashMap,
  LimitedVec, UnwrapInfallible, WriteAllError,
};

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PaxParserError {
  #[error("A PAX key-value pair is missing a newline at the end")]
  KeyValuePairMissingNewline,
  #[error("A gnu sparse map is malformed, expected an even number of parts found {0} parts")]
  GnuSparseMapMalformed(usize),
  #[error("A well-known PAX key '{key}' appeared in the wrong context. Expected: {expected_context:?}, Actual: {actual_context:?}")]
  WellKnownKeyAppearedInWrongPaxContext {
    key: &'static str,
    expected_context: PaxConfidence,
    actual_context: PaxConfidence,
  },
}

#[derive(Default, Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum PaxConfidence {
  GLOBAL = 1,
  #[default]
  LOCAL,
}

#[derive(Default)]
pub(crate) struct PaxConfidentValue<T> {
  global: Option<T>,
  local: Option<T>,
}

impl<T> PaxConfidentValue<T> {
  pub fn reset_local(&mut self) {
    self.local = None;
  }

  /// Returns the local value if it exists, otherwise returns the global value.
  #[must_use]
  pub fn get(&self) -> Option<&T> {
    if let Some(local_value) = &self.local {
      Some(local_value)
    } else {
      self.global.as_ref()
    }
  }

  pub fn get_mut(&mut self) -> Option<&mut T> {
    if let Some(local_value) = &mut self.local {
      Some(local_value)
    } else {
      self.global.as_mut()
    }
  }

  /// Returns the local value if it exists, otherwise returns the global value.
  #[must_use]
  pub fn get_with_confidence(&self) -> Option<(PaxConfidence, &T)> {
    if let Some(local_value) = &self.local {
      Some((PaxConfidence::LOCAL, local_value))
    } else if let Some(global_value) = &self.global {
      Some((PaxConfidence::GLOBAL, global_value))
    } else {
      None
    }
  }

  pub fn insert_with_confidence(&mut self, confidence: PaxConfidence, value: T) -> Option<&T> {
    match confidence {
      PaxConfidence::GLOBAL => {
        self.global = Some(value);
      },
      PaxConfidence::LOCAL => {
        self.local = Some(value);
      },
    }
    self.get()
  }
}

/// Maximum length of the length field in bytes
const MAX_KV_LENGTH_FIELD_LENGTH: usize = max_string_length_from_limit(usize::MAX, 10);

#[derive(Debug, PartialEq, Eq)]
struct StateParsingNewKV {
  kv_cursor: Cursor<[u8; MAX_KV_LENGTH_FIELD_LENGTH]>,
}

#[derive(Debug, PartialEq, Eq)]
struct StateParsingKey {
  /// The length of the key-value pair.
  length: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct StateParsingValue {
  key: String,
  length_after_equals: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum PaxParserState {
  ParsingNewKV(StateParsingNewKV),
  ParsingKey(StateParsingKey),
  ParsingValue(StateParsingValue),
  NoNextStateSet,
}

impl Default for PaxParserState {
  fn default() -> Self {
    PaxParserState::ParsingNewKV(StateParsingNewKV {
      kv_cursor: Cursor::new([0; MAX_KV_LENGTH_FIELD_LENGTH]),
    })
  }
}

#[derive(Default)]
pub struct SparseFileInstructionBuilder {
  offset_before: Option<u64>,
  data_size: Option<u64>,
}

/// "%d %s=%s\n", <length>, <keyword>, <value>
pub struct PaxParser<VH: TarViolationHandler = IgnoreTarViolationHandler> {
  global_attributes: LimitedHashMap<String, String>,
  // unknown/unparsed attributes
  unparsed_global_attributes: LimitedHashMap<String, String>,
  unparsed_local_attributes: LimitedHashMap<String, String>,

  // parsed attributes
  gnu_sparse_name_01_01: PaxConfidentValue<String>,
  gnu_sparse_realsize_1_0: PaxConfidentValue<usize>,
  gnu_sparse_major: PaxConfidentValue<u32>,
  gnu_sparse_minor: PaxConfidentValue<u32>,
  gnu_sparse_realsize_0_01: PaxConfidentValue<usize>,
  gnu_sparse_map_local: LimitedVec<SparseFileInstruction>,
  mtime: PaxConfidentValue<TimeStamp>,
  atime: PaxConfidentValue<TimeStamp>,
  ctime: PaxConfidentValue<TimeStamp>,
  gid: PaxConfidentValue<u32>,
  gname: PaxConfidentValue<String>,
  link_path: PaxConfidentValue<String>,
  path: PaxConfidentValue<String>,
  data_size: PaxConfidentValue<usize>,
  uid: PaxConfidentValue<u32>,
  uname: PaxConfidentValue<String>,

  // state
  state: PaxParserState,
  current_pax_mode: PaxConfidence,
  sparse_instruction_builder: SparseFileInstructionBuilder,
  pax_key_value_buffer: LimitedVec<u8>,

  _violation_handler: PhantomData<VH>,
}

impl<VH: TarViolationHandler> PaxParser<VH> {
  #[must_use]
  pub fn try_new(
    vh: &mut VHW<'_, VH>,
    initial_global_extended_attributes: HashMap<String, String>,
    max_global_attributes: usize,
    max_unparsed_global_attributes: usize,
    max_unparsed_local_attributes: usize,
    max_pax_key_value_length: usize,
    max_sparse_file_instructions: usize,
  ) -> Result<Self, TarParserError> {
    let mut selv = Self {
      global_attributes: LimitedHashMap::new(max_global_attributes),
      unparsed_global_attributes: LimitedHashMap::new(max_unparsed_global_attributes),
      unparsed_local_attributes: LimitedHashMap::new(max_unparsed_local_attributes),
      gnu_sparse_name_01_01: PaxConfidentValue::default(),
      gnu_sparse_realsize_1_0: PaxConfidentValue::default(),
      gnu_sparse_major: PaxConfidentValue::default(),
      gnu_sparse_minor: PaxConfidentValue::default(),
      gnu_sparse_realsize_0_01: PaxConfidentValue::default(),
      gnu_sparse_map_local: LimitedVec::new(max_sparse_file_instructions),
      mtime: PaxConfidentValue::default(),
      atime: PaxConfidentValue::default(),
      ctime: PaxConfidentValue::default(),
      gid: PaxConfidentValue::default(),
      gname: PaxConfidentValue::default(),
      link_path: PaxConfidentValue::default(),
      path: PaxConfidentValue::default(),
      data_size: PaxConfidentValue::default(),
      uid: PaxConfidentValue::default(),
      uname: PaxConfidentValue::default(),
      state: PaxParserState::default(),
      current_pax_mode: PaxConfidence::LOCAL,
      sparse_instruction_builder: SparseFileInstructionBuilder::default(),
      pax_key_value_buffer: LimitedVec::new(max_pax_key_value_length),
      _violation_handler: PhantomData,
    };
    for (key, value) in initial_global_extended_attributes {
      selv.ingest_attribute(vh, PaxConfidence::GLOBAL, key, value)?;
    }
    Ok(selv)
  }

  #[must_use]
  pub fn global_extended_attributes(&self) -> &HashMap<String, String> {
    self.global_attributes.as_hash_map()
  }

  #[must_use]
  pub fn get_sparse_format(&self) -> Option<SparseFormat> {
    SparseFormat::try_from_gnu_version(
      self.gnu_sparse_major.get().map(|v| *v),
      self.gnu_sparse_minor.get().map(|v| *v),
    )
  }

  fn to_confident_value<T>(value: Option<(PaxConfidence, T)>) -> InodeConfidentValue<T> {
    let mut inode_confident_value = InodeConfidentValue::default();
    if let Some((confidence, value)) = value {
      inode_confident_value.set(confidence.into(), value);
    }
    inode_confident_value
  }

  /// Parses a time value in the format "seconds.nanoseconds" or "seconds"
  fn parse_time(value: &str) -> Result<TimeStamp, ParseIntError> {
    let mut parts = value.split('.');

    let seconds = parts.next().unwrap_or("").parse::<u64>()?;
    // Default to 0 nanoseconds if not provided
    let nanoseconds = if let Some(nanosecond_part) = parts.next() {
      nanosecond_part.parse::<u32>()?
    } else {
      0
    };

    Ok(TimeStamp {
      seconds_since_epoch: seconds,
      nanoseconds,
    })
  }

  pub fn load_pax_attributes_into_inode_builder(&self, inode_builder: &mut InodeBuilder) {
    if let Some(sparse_format) = self.get_sparse_format() {
      if inode_builder.sparse_format.is_none() {
        inode_builder.sparse_format = Some(sparse_format);
        inode_builder
          .file_path
          .update_with(Self::to_confident_value(
            self.gnu_sparse_name_01_01.get_with_confidence(),
          ));
        inode_builder
          .sparse_real_size
          .update_with(Self::to_confident_value(
            self
              .gnu_sparse_realsize_1_0
              .get_with_confidence()
              .or(self.gnu_sparse_realsize_0_01.get_with_confidence()),
          ));
        if sparse_format == SparseFormat::Gnu0_0 || sparse_format == SparseFormat::Gnu0_1 {
          // TODO: avoid clone
          inode_builder.sparse_file_instructions = self.gnu_sparse_map_local.clone();
        }
      }
    }
    inode_builder
      .file_path
      .update_with(Self::to_confident_value(self.path.get_with_confidence()));
    inode_builder
      .mtime
      .update_with(Self::to_confident_value(self.mtime.get_with_confidence()));
    inode_builder
      .atime
      .update_with(Self::to_confident_value(self.atime.get_with_confidence()));
    inode_builder
      .ctime
      .update_with(Self::to_confident_value(self.ctime.get_with_confidence()));
    inode_builder
      .gid
      .update_with(Self::to_confident_value(self.gid.get_with_confidence()));
    inode_builder
      .gname
      .update_with(Self::to_confident_value(self.gname.get_with_confidence()));
    inode_builder
      .link_target
      .update_with(Self::to_confident_value(
        self.link_path.get_with_confidence(),
      ));
    inode_builder
      .data_after_header_size
      .update_with(Self::to_confident_value(
        self.data_size.get_with_confidence(),
      ));
    inode_builder
      .uid
      .update_with(Self::to_confident_value(self.uid.get_with_confidence()));
    inode_builder
      .uname
      .update_with(Self::to_confident_value(self.uname.get_with_confidence()));
  }

  pub fn set_current_pax_mode(&mut self, pax_confidence: PaxConfidence) {
    self.current_pax_mode = pax_confidence;
  }

  pub fn recover(&mut self) {
    // Reset the local unparsed attributes
    self.unparsed_local_attributes.clear();
    // Reset all parsed local attributes
    self.gnu_sparse_name_01_01.reset_local();
    self.gnu_sparse_realsize_1_0.reset_local();
    self.gnu_sparse_major.reset_local();
    self.gnu_sparse_minor.reset_local();
    self.gnu_sparse_realsize_0_01.reset_local();
    self.gnu_sparse_map_local.clear();
    self.mtime.reset_local();
    self.gid.reset_local();
    self.gname.reset_local();
    self.link_path.reset_local();
    self.path.reset_local();
    self.data_size.reset_local();
    self.uid.reset_local();
    self.uname.reset_local();

    // Reset the parser state to default
    self.state = PaxParserState::default();
    self.sparse_instruction_builder = Default::default();
  }

  fn try_finish_sparse_instruction(&mut self) -> Result<(), TarParserError> {
    if let (Some(offset_before), Some(data_size)) = (
      self.sparse_instruction_builder.offset_before,
      self.sparse_instruction_builder.data_size,
    ) {
      let sparse_instruction = SparseFileInstruction {
        offset_before,
        data_size,
      };

      self
        .gnu_sparse_map_local
        .push(sparse_instruction)
        .map_err(limit_exceeded_to_tar_err(
          self.gnu_sparse_map_local.max_len(),
          LimitExceededContext::TooManySparseFileInstructions,
        ))?;

      self.sparse_instruction_builder = Default::default();
    }
    Ok(())
  }

  /// The sparse map is a series of comma-separated decimal values
  /// in the format `offset,size[,offset,size,...]` (0.1)
  fn parse_gnu_sparse_map_0_1(
    &mut self,
    vh: &mut VHW<'_, VH>,
    value: String,
  ) -> Result<(), TarParserError> {
    let parts = value.split(',');
    let mut offset = None;
    let mut len_parts = 0;
    for (i, part) in parts.enumerate() {
      len_parts = i;
      if i % 2 == 0 {
        // This is an offset
        offset = vh.hpvr(part.parse::<u64>().map_err(corrupt_field_to_tar_err(
          CorruptFieldContext::GnuSparseMapOffsetValue(SparseFormat::Gnu0_1),
        )))?;
      } else {
        // This is a size
        if let Some(offset) = offset {
          if let Some(parsed_data_size) =
            vh.hpvr(part.parse::<u64>().map_err(corrupt_field_to_tar_err(
              CorruptFieldContext::GnuSparseMapSizeValue(SparseFormat::Gnu0_1),
            )))?
          {
            vh.hpvr(
              self
                .gnu_sparse_map_local
                .push(SparseFileInstruction {
                  offset_before: offset,
                  data_size: parsed_data_size,
                })
                .map_err(limit_exceeded_to_tar_err(
                  self.gnu_sparse_map_local.max_len(),
                  LimitExceededContext::TooManySparseFileInstructions,
                )),
            )?;
          }
        }
        offset = None; // Reset offset for the next pair
      }
    }
    if offset.is_some() {
      return Err(TarParserError::PaxParserError(
        PaxParserError::GnuSparseMapMalformed(len_parts),
      ));
    }
    Ok(())
  }

  pub fn drain_local_unparsed_attributes(&mut self) -> HashMap<String, String> {
    // TODO: reuse the allocation
    let mut combined_attributes = self.global_attributes.as_hash_map().clone();
    combined_attributes.extend(self.unparsed_local_attributes.drain());
    combined_attributes
  }

  fn ingest_attribute(
    &mut self,
    vh: &mut VHW<'_, VH>,
    confidence: PaxConfidence,
    key: String,
    value: String,
  ) -> Result<(), TarParserError> {
    if confidence == PaxConfidence::GLOBAL {
      vh.hpvr(
        self
          .global_attributes
          .insert(key.clone(), value.clone())
          .map_err(limit_exceeded_to_tar_err(
            self.global_attributes.max_keys(),
            LimitExceededContext::PaxTooManyGlobalAttributes,
          )),
      )?;
    }
    match key.as_str() {
      GNU_SPARSE_NAME_01_01 => {
        if confidence == PaxConfidence::LOCAL {
          self
            .gnu_sparse_name_01_01
            .insert_with_confidence(confidence, value);
        } else {
          vh.hpve(PaxParserError::WellKnownKeyAppearedInWrongPaxContext {
            key: GNU_SPARSE_NAME_01_01,
            expected_context: PaxConfidence::LOCAL,
            actual_context: confidence,
          })?;
        }
      },
      GNU_SPARSE_REALSIZE_1_0 => {
        if confidence == PaxConfidence::LOCAL {
          if let Some(parsed_value) =
            vh.hpvr(value.parse::<usize>().map_err(corrupt_field_to_tar_err(
              CorruptFieldContext::GnuSparseRealFileSize(SparseFormat::Gnu1_0),
            )))?
          {
            self
              .gnu_sparse_realsize_1_0
              .insert_with_confidence(confidence, parsed_value);
          }
        } else {
          vh.hpve(PaxParserError::WellKnownKeyAppearedInWrongPaxContext {
            key: GNU_SPARSE_REALSIZE_1_0,
            expected_context: PaxConfidence::LOCAL,
            actual_context: confidence,
          })?;
        }
      },
      GNU_SPARSE_MAJOR => {
        if let Some(parsed_value) = vh.hpvr(value.parse::<u32>().map_err(
          corrupt_field_to_tar_err(CorruptFieldContext::GnuSparseMajorVersion),
        ))? {
          self
            .gnu_sparse_major
            .insert_with_confidence(confidence, parsed_value);
        }
      },
      GNU_SPARSE_MINOR => {
        if let Some(parsed_value) = vh.hpvr(value.parse::<u32>().map_err(
          corrupt_field_to_tar_err(CorruptFieldContext::GnuSparseMinorVersion),
        ))? {
          self
            .gnu_sparse_minor
            .insert_with_confidence(confidence, parsed_value);
        }
      },
      GNU_SPARSE_REALSIZE_0_01 => {
        if confidence == PaxConfidence::LOCAL {
          if let Some(parsed_value) =
            vh.hpvr(value.parse::<usize>().map_err(corrupt_field_to_tar_err(
              CorruptFieldContext::GnuSparseRealFileSize(SparseFormat::Gnu0_1),
            )))?
          {
            self
              .gnu_sparse_realsize_0_01
              .insert_with_confidence(confidence, parsed_value);
          }
        } else {
          vh.hpve(PaxParserError::WellKnownKeyAppearedInWrongPaxContext {
            key: GNU_SPARSE_REALSIZE_0_01,
            expected_context: PaxConfidence::LOCAL,
            actual_context: confidence,
          })?;
        }
      },
      GNU_SPARSE_MAP_NUM_BLOCKS_0_01 => {
        // This is a user controlled value so we only try to reserve the space
        if let Some(new_len) = vh.hpvr(value.parse::<usize>().map_err(corrupt_field_to_tar_err(
          CorruptFieldContext::GnuSparseNumberOfMaps(SparseFormat::Gnu0_1),
        )))? {
          vh.hpvr(
            self
              .gnu_sparse_map_local
              .resize(new_len, SparseFileInstruction::default())
              .map_err(limit_exceeded_to_tar_err(
                self.gnu_sparse_map_local.max_len(),
                LimitExceededContext::TooManySparseFileInstructions,
              )),
          )?;
        }
      },
      GNU_SPARSE_DATA_BLOCK_OFFSET_0_0 => {
        if confidence == PaxConfidence::LOCAL {
          self
            .gnu_sparse_major
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          self
            .gnu_sparse_minor
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          if let Some(parsed_value) =
            vh.hpvr(value.parse::<u64>().map_err(corrupt_field_to_tar_err(
              CorruptFieldContext::GnuSparseMapOffsetValue(SparseFormat::Gnu0_0),
            )))?
          {
            self.sparse_instruction_builder.offset_before = Some(parsed_value);
          }
          vh.hpvr(self.try_finish_sparse_instruction())?;
        } else {
          vh.hpve(PaxParserError::WellKnownKeyAppearedInWrongPaxContext {
            key: GNU_SPARSE_DATA_BLOCK_OFFSET_0_0,
            expected_context: PaxConfidence::LOCAL,
            actual_context: confidence,
          })?;
        }
      },
      GNU_SPARSE_DATA_BLOCK_SIZE_0_0 => {
        if confidence == PaxConfidence::LOCAL {
          self
            .gnu_sparse_major
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          self
            .gnu_sparse_minor
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          if let Some(parsed_value) =
            vh.hpvr(value.parse::<u64>().map_err(corrupt_field_to_tar_err(
              CorruptFieldContext::GnuSparseMapSizeValue(SparseFormat::Gnu0_0),
            )))?
          {
            self.sparse_instruction_builder.data_size = Some(parsed_value);
          }
          vh.hpvr(self.try_finish_sparse_instruction())?;
        } else {
          vh.hpve(PaxParserError::WellKnownKeyAppearedInWrongPaxContext {
            key: GNU_SPARSE_DATA_BLOCK_SIZE_0_0,
            expected_context: PaxConfidence::LOCAL,
            actual_context: confidence,
          })?;
        }
      },
      GNU_SPARSE_MAP_0_1 => {
        if confidence == PaxConfidence::LOCAL {
          self
            .gnu_sparse_major
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          self
            .gnu_sparse_minor
            .insert_with_confidence(PaxConfidence::LOCAL, 1);
          self.parse_gnu_sparse_map_0_1(vh, value)?;
        } else {
          vh.hpve(PaxParserError::WellKnownKeyAppearedInWrongPaxContext {
            key: GNU_SPARSE_MAP_0_1,
            expected_context: PaxConfidence::LOCAL,
            actual_context: confidence,
          })?;
        }
      },
      ATIME => {
        if let Some(parsed_value) = vh.hpvr(Self::parse_time(value.as_str()).map_err(
          corrupt_field_to_tar_err(CorruptFieldContext::PaxWellKnownAtime),
        ))? {
          self.atime.insert_with_confidence(confidence, parsed_value);
        }
      },
      GID => {
        if let Some(parsed_value) = vh.hpvr(value.parse::<u32>().map_err(
          corrupt_field_to_tar_err(CorruptFieldContext::PaxWellKnownGid),
        ))? {
          self.gid.insert_with_confidence(confidence, parsed_value);
        }
      },
      GNAME => {
        self.gname.insert_with_confidence(confidence, value);
      },
      LINKPATH => {
        self.link_path.insert_with_confidence(confidence, value);
      },
      MTIME => {
        if let Some(parsed_value) = vh.hpvr(Self::parse_time(value.as_str()).map_err(
          corrupt_field_to_tar_err(CorruptFieldContext::PaxWellKnownMtime),
        ))? {
          self.mtime.insert_with_confidence(confidence, parsed_value);
        }
      },
      CTIME => {
        if let Some(parsed_value) = vh.hpvr(Self::parse_time(value.as_str()).map_err(
          corrupt_field_to_tar_err(CorruptFieldContext::PaxWellKnownCtime),
        ))? {
          self.ctime.insert_with_confidence(confidence, parsed_value);
        }
      },
      PATH => {
        self.path.insert_with_confidence(confidence, value);
      },
      SIZE => {
        if let Some(parsed_value) = vh.hpvr(value.parse::<usize>().map_err(
          corrupt_field_to_tar_err(CorruptFieldContext::PaxWellKnownSize),
        ))? {
          self
            .data_size
            .insert_with_confidence(confidence, parsed_value);
        }
      },
      UID => {
        if let Some(parsed_value) = vh.hpvr(value.parse::<u32>().map_err(
          corrupt_field_to_tar_err(CorruptFieldContext::PaxWellKnownUid),
        ))? {
          self.uid.insert_with_confidence(confidence, parsed_value);
        }
      },
      UNAME => {
        self.uname.insert_with_confidence(confidence, value);
      },
      _ => {
        // Unparsed attribute store it
        match confidence {
          PaxConfidence::GLOBAL => {
            vh.hpvr(self.unparsed_global_attributes.insert(key, value).map_err(
              limit_exceeded_to_tar_err(
                self.unparsed_global_attributes.max_keys(),
                LimitExceededContext::PaxTooManyUnparsedGlobalAttributes,
              ),
            ))?;
          },
          PaxConfidence::LOCAL => {
            vh.hpvr(self.unparsed_local_attributes.insert(key, value).map_err(
              limit_exceeded_to_tar_err(
                self.unparsed_local_attributes.max_keys(),
                LimitExceededContext::PaxTooManyUnparsedLocalAttributes,
              ),
            ))?;
          },
        }
      },
    }
    Ok(())
  }

  /// "%d %s=%s\n", <length>, <keyword>, <value>
  ///
  /// This function parses the length decimal and computes the values for the parsing key state.
  fn state_parsing_new_kv(
    &mut self,
    vh: &mut VHW<'_, VH>,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateParsingNewKV,
  ) -> Result<PaxParserState, TarParserError> {
    // Read the length until we hit a space or newline
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut state.kv_cursor,
      false,
      |byte: &u8| *byte == b' ' || *byte == b'\n',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) => {},
      Err(CopyUntilError::DelimiterNotFound { .. }) => {
        // Not enough data in the current `bytes` slice, preserve state and wait for more
        return Ok(PaxParserState::ParsingNewKV(state));
      },
      Err(CopyUntilError::IoRead(..)) => unreachable!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(FixedSizeBufferError { .. })),
      ) => {
        return Err(TarParserError::LimitExceeded {
          limit: MAX_KV_LENGTH_FIELD_LENGTH,
          context: LimitExceededContext::PaxLengthFieldDecimalStringTooLong,
        });
      },
    }

    // Convert the length bytes to a usize
    let length_str = vh.hfvr(
      core::str::from_utf8(state.kv_cursor.before())
        .map_err(corrupt_field_to_tar_err(CorruptFieldContext::PaxKvLength)),
    )?;
    let length = vh.hfvr(
      length_str
        .parse::<usize>()
        .map_err(corrupt_field_to_tar_err(CorruptFieldContext::PaxKvLength)),
    )?;

    let length = length.saturating_sub(state.kv_cursor.position() + 1);
    if length == 0 {
      // If the length is 0, we are done with this key-value pair
      return Ok(PaxParserState::default());
    }
    self.pax_key_value_buffer.clear();
    Ok(PaxParserState::ParsingKey(StateParsingKey { length }))
  }

  /// Parses the key from the cursor and returns the next state.
  fn state_parsing_key(
    &mut self,
    vh: &mut VHW<'_, VH>,
    cursor: &mut Cursor<&[u8]>,
    state: StateParsingKey,
  ) -> Result<PaxParserState, TarParserError> {
    // Read the length until we hit an equals sign
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut self.pax_key_value_buffer,
      false,
      |byte: &u8| *byte == b'=',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) => {},
      Err(CopyUntilError::DelimiterNotFound { .. }) => {
        // Not enough data in the current `bytes` slice, preserve state and wait for more.
        return Ok(PaxParserState::ParsingKey(state));
      },
      Err(CopyUntilError::IoRead(..)) => unreachable!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(..)),
      ) => {
        return Err(vh.hfve(TarParserError::LimitExceeded {
          limit: self.pax_key_value_buffer.max_len(),
          context: LimitExceededContext::PaxKvKeyTooLong,
        }));
      },
    }

    let length_after_equals = state
      .length
      .saturating_sub(self.pax_key_value_buffer.len() + 1);
    if length_after_equals == 0 {
      // If the length is 0, we are done with this key-value pair
      return Ok(PaxParserState::default());
    }
    let key = vh
      .hfvr(
        core::str::from_utf8(&self.pax_key_value_buffer)
          .map_err(corrupt_field_to_tar_err(CorruptFieldContext::PaxKvKey)),
      )?
      .to_string();
    self.pax_key_value_buffer.clear();
    return Ok(PaxParserState::ParsingValue(StateParsingValue {
      key,
      length_after_equals,
    }));
  }

  fn state_parsing_value(
    &mut self,
    vh: &mut VHW<'_, VH>,
    cursor: &mut Cursor<&[u8]>,
    state: StateParsingValue,
  ) -> Result<PaxParserState, TarParserError> {
    let value_len = state.length_after_equals.saturating_sub(1);
    let bytes_needed = value_len.saturating_sub(self.pax_key_value_buffer.len());

    let bytes_read = cursor.read_buffered(bytes_needed).unwrap_infallible();

    vh.hfvr(
      self
        .pax_key_value_buffer
        .extend_from_slice(bytes_read)
        .map_err(limit_exceeded_to_tar_err(
          self.pax_key_value_buffer.max_len(),
          LimitExceededContext::PaxKvValueTooLong,
        )),
    )?;

    // Check if we have the full value now
    if self.pax_key_value_buffer.len() < value_len {
      // Not enough data, preserve state
      return Ok(PaxParserState::ParsingValue(state));
    }

    // We have the value, now we need the trailing newline
    if cursor.position() >= cursor.full_buffer().len() {
      // Not enough data for the newline, preserve state
      return Ok(PaxParserState::ParsingValue(state));
    }

    let newline_char = cursor.full_buffer()[cursor.position()];
    if newline_char != b'\n' {
      // Record must end in a newline, so length of value part must be at least 1.
      vh.hpve(PaxParserError::KeyValuePairMissingNewline)?;
    } else {
      cursor.set_position(cursor.position() + 1);
    }

    // We have a full key-value pair. Ingest it.
    let value = vh
      .hfvr(
        core::str::from_utf8(&self.pax_key_value_buffer)
          .map_err(corrupt_field_to_tar_err(CorruptFieldContext::PaxKvValue)),
      )?
      .to_string();

    self.ingest_attribute(vh, self.current_pax_mode, state.key, value)?;

    // Ready for the next key-value pair
    Ok(PaxParserState::default())
  }

  pub fn parse(
    &mut self,
    vh: &mut VHW<'_, VH>,
    input_buffer: &[u8],
  ) -> Result<usize, TarParserError> {
    let mut bytes_read = 0;
    let mut cursor = Cursor::new(input_buffer);
    loop {
      let parser_state = core::mem::replace(&mut self.state, PaxParserState::NoNextStateSet);

      let initial_cursor_position = cursor.position();

      let next_state = match parser_state {
        PaxParserState::ParsingNewKV(state) => self.state_parsing_new_kv(vh, &mut cursor, state),
        PaxParserState::ParsingKey(state) => self.state_parsing_key(vh, &mut cursor, state),
        PaxParserState::ParsingValue(state) => self.state_parsing_value(vh, &mut cursor, state),
        PaxParserState::NoNextStateSet => {
          unreachable!("BUG: No next state set in PaxParser");
        },
      };

      let bytes_read_this_parse = cursor.position() - initial_cursor_position;
      bytes_read += bytes_read_this_parse;

      self.state = next_state?;

      if bytes_read_this_parse == 0 {
        return Ok(bytes_read);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use core::num::ParseIntError;

  use alloc::vec;

  use crate::extended_streams::tar::{GeneralParseError, StrictTarViolationHandler};

  use super::*;

  fn new_strict_parser() -> PaxParser<StrictTarViolationHandler> {
    PaxParser::try_new(
      &mut VHW(&mut StrictTarViolationHandler::default()),
      HashMap::new(),
      usize::MAX,
      usize::MAX,
      usize::MAX,
      usize::MAX,
      usize::MAX,
    )
    .expect("Failed to create PaxParser")
  }

  #[test]
  fn test_new_with_initial_global_attributes() {
    let mut globals = HashMap::new();
    globals.insert("gname".to_string(), "wheel".to_string());
    globals.insert("uid".to_string(), "0".to_string());

    let mut vh = IgnoreTarViolationHandler::default();
    let vh = &mut VHW(&mut vh);
    let parser = PaxParser::<IgnoreTarViolationHandler>::try_new(
      vh,
      globals,
      usize::MAX,
      usize::MAX,
      usize::MAX,
      usize::MAX,
      usize::MAX,
    )
    .expect("Failed to create PaxParser with initial global attributes");

    assert_eq!(
      parser.gname.get_with_confidence(),
      Some((PaxConfidence::GLOBAL, &"wheel".to_string()))
    );
    assert_eq!(
      parser.uid.get_with_confidence(),
      Some((PaxConfidence::GLOBAL, &0))
    );
    assert_eq!(parser.unparsed_global_attributes.len(), 0); // Parsed globals are not in the unparsed map.
  }

  fn drive_parser<VH: TarViolationHandler + Default>(
    parser: &mut PaxParser<VH>,
    input: &[u8],
    bytewise: bool,
  ) -> Result<(), TarParserError> {
    let mut vh = VH::default();
    let vh = &mut VHW(&mut vh);
    if bytewise {
      // If bytewise parsing is requested, we will parse one byte at a time.
      for &byte in input.iter() {
        let byte = *&[byte];
        parser.parse(vh, byte.as_slice())?;
      }
      return Ok(());
    }
    parser.parse(vh, input)?;
    Ok(())
  }

  #[test]
  fn test_simple_kv_parsing() {
    let mut parser = new_strict_parser();
    let data = b"18 path=some/file\n";
    drive_parser(&mut parser, data, false).unwrap();

    assert_eq!(parser.path.get(), Some(&"some/file".to_string()));
    assert_eq!(parser.state, PaxParserState::default());
  }

  #[test]
  fn test_multiple_kv_parsing() {
    let mut parser = new_strict_parser();
    let data = b"18 path=some/file\n12 size=123\n12 uid=1000\n";
    drive_parser(&mut parser, data, false).unwrap();

    assert_eq!(parser.path.get(), Some(&"some/file".to_string()));
    assert_eq!(parser.state, PaxParserState::default());
  }

  #[test]
  fn test_multiple_kv_parsing_from_archive() {
    let mut parser = new_strict_parser();
    let data =
      b"30 mtime=1749954382.774290089\n20 atime=1749803808\n30 ctime=1749954382.774290089\n";
    drive_parser(&mut parser, data, true).unwrap();

    assert_eq!(
      parser.mtime.get(),
      Some(&TimeStamp {
        seconds_since_epoch: 1749954382,
        nanoseconds: 774290089,
      })
    );
    assert_eq!(
      parser.atime.get(),
      Some(&TimeStamp {
        seconds_since_epoch: 1749803808,
        nanoseconds: 0,
      })
    );
    assert_eq!(
      parser.ctime.get(),
      Some(&TimeStamp {
        seconds_since_epoch: 1749954382,
        nanoseconds: 774290089,
      })
    );
    assert_eq!(parser.state, PaxParserState::default());
  }

  #[test]
  fn test_gnu_sparse_map_0_1() {
    let mut parser = new_strict_parser();
    let data = b"45 GNU.sparse.map=1024,512,8192,2048,16384,0\n";
    drive_parser(&mut parser, data, false).unwrap();

    let expected = LimitedVec::from_vec(
      usize::MAX,
      vec![
        SparseFileInstruction {
          offset_before: 1024,
          data_size: 512,
        },
        SparseFileInstruction {
          offset_before: 8192,
          data_size: 2048,
        },
        SparseFileInstruction {
          offset_before: 16384,
          data_size: 0,
        },
      ],
    );
    assert_eq!(parser.gnu_sparse_map_local, expected);
  }

  #[test]
  fn test_unparsed_attributes_and_drain() {
    let mut parser = new_strict_parser();
    let data = b"21 SCHILY.fflags=bar\n12 uid=1000\n";
    drive_parser(&mut parser, data, false).unwrap();

    assert_eq!(parser.unparsed_local_attributes.len(), 1);
    assert_eq!(
      parser.unparsed_local_attributes.get("SCHILY.fflags"),
      Some(&"bar".to_string())
    );

    let drained = parser.drain_local_unparsed_attributes();

    assert_eq!(drained.len(), 1);
    assert_eq!(drained.get("SCHILY.fflags"), Some(&"bar".to_string()));
    assert!(parser.unparsed_local_attributes.is_empty());
  }

  #[test]
  fn test_parser_error_bad_length() {
    let mut parser = new_strict_parser();
    let data = b"abc path=foo\n";
    assert!(matches!(
      drive_parser(&mut parser, data, false),
      Err(TarParserError::CorruptField {
        field: CorruptFieldContext::PaxKvLength,
        error: GeneralParseError::InvalidInteger(ParseIntError { .. }),
      })
    ));
  }

  #[test]
  fn test_parser_error_bad_value() {
    let mut parser = new_strict_parser();
    let data = b"12 path=foo ";
    assert_eq!(
      drive_parser(&mut parser, data, false),
      Err(TarParserError::PaxParserError(
        PaxParserError::KeyValuePairMissingNewline
      ))
    );
  }
}
