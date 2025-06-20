use core::panic;

use alloc::{collections::TryReserveError, string::String, vec::Vec};

use hashbrown::HashMap;

use crate::{
  extended_streams::tar::{
    tar_constants::pax_keys_well_known::{
      gnu::{
        GNU_SPARSE_DATA_BLOCK_OFFSET_0_0, GNU_SPARSE_DATA_BLOCK_SIZE_0_0, GNU_SPARSE_MAJOR,
        GNU_SPARSE_MAP_0_1, GNU_SPARSE_MAP_NUM_BLOCKS_0_01, GNU_SPARSE_MINOR,
        GNU_SPARSE_NAME_01_01, GNU_SPARSE_REALSIZE_0_01, GNU_SPARSE_REALSIZE_1_0,
      },
      ATIME, CTIME, GID, GNAME, LINKPATH, MTIME, PATH, SIZE, UID, UNAME,
    },
    InodeBuilder, InodeConfidentValue, SparseFileInstruction, SparseFormat, TarParserError,
    TimeStamp,
  },
  CopyBuffered as _, CopyUntilError, Cursor, FixedSizeBufferError, Write, WriteAllError,
};

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
const MAX_KV_LENGTH_FIELD_LENGTH: usize = 32;

#[derive(Debug, PartialEq, Eq)]
struct StateParsingNewKV {
  kv_cursor: Cursor<[u8; MAX_KV_LENGTH_FIELD_LENGTH]>,
}

#[derive(Debug, PartialEq, Eq)]
struct StateParsingKey {
  length: usize,
  keyword: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
struct StateParsingValue {
  key: String,
  length_after_equals: usize,
  value: Vec<u8>,
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
#[derive(Default)]
pub struct PaxParser {
  global_attributes: HashMap<String, String>,
  // unknown/unparsed attributes
  unparsed_global_attributes: HashMap<String, String>,
  unparsed_attributes: HashMap<String, String>,

  // parsed attributes
  gnu_sparse_name_01_01: PaxConfidentValue<String>,
  gnu_sparse_realsize_1_0: PaxConfidentValue<usize>,
  gnu_sprase_major: PaxConfidentValue<u32>,
  gnu_sparse_minor: PaxConfidentValue<u32>,
  gnu_sparse_realsize_0_01: PaxConfidentValue<usize>,
  gnu_sparse_map_local: Vec<SparseFileInstruction>,
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
}

impl PaxParser {
  #[must_use]
  pub fn new(initial_global_extended_attributes: HashMap<String, String>) -> Self {
    let mut selv = Self {
      ..Default::default()
    };
    for (key, value) in initial_global_extended_attributes {
      selv.ingest_attribute(PaxConfidence::GLOBAL, key, value);
    }
    selv
  }

  #[must_use]
  pub fn global_extended_attributes(&self) -> &HashMap<String, String> {
    &self.global_attributes
  }

  #[must_use]
  pub fn get_sparse_format(&self) -> Option<SparseFormat> {
    SparseFormat::try_from_gnu_version(
      self.gnu_sprase_major.get().map(|v| *v),
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
  fn parse_time(value: &str) -> Option<TimeStamp> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.is_empty() || parts.len() > 2 {
      return None; // Invalid format
    }

    let seconds = parts[0].parse::<u64>().ok()?;
    let nanoseconds = if parts.len() == 2 {
      parts[1].parse::<u32>().ok()?
    } else {
      0 // Default to 0 nanoseconds if not provided
    };

    Some(TimeStamp {
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
          inode_builder.sparse_file_instructions = self.gnu_sparse_map_local.clone();
        }
      }
    }
    inode_builder
      .file_path
      .update_with(Self::to_confident_value(self.path.get_with_confidence()));
    inode_builder.mtime.update_with(Self::to_confident_value(
      self
        .mtime
        .get_with_confidence()
        .or(self.mtime.get_with_confidence()),
    ));
    inode_builder.atime.update_with(Self::to_confident_value(
      self
        .mtime
        .get_with_confidence()
        .or(self.atime.get_with_confidence()),
    ));
    inode_builder.ctime.update_with(Self::to_confident_value(
      self
        .mtime
        .get_with_confidence()
        .or(self.ctime.get_with_confidence()),
    ));
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
    self.unparsed_attributes.clear();
    // Reset all parsed local attributes
    self.gnu_sparse_name_01_01.reset_local();
    self.gnu_sparse_realsize_1_0.reset_local();
    self.gnu_sprase_major.reset_local();
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

  fn try_finish_sparse_instruction(&mut self) {
    match (
      self.sparse_instruction_builder.offset_before,
      self.sparse_instruction_builder.data_size,
    ) {
      (Some(offset_before), Some(data_size)) => {
        let sparse_instruction = SparseFileInstruction {
          offset_before,
          data_size,
        };

        self.gnu_sparse_map_local.push(sparse_instruction);

        self.sparse_instruction_builder = Default::default();
      },
      _ => {},
    }
  }

  /// The sparse map is a series of comma-separated decimal values
  /// in the format `offset,size[,offset,size,...]` (0.1)
  fn parse_gnu_sparse_map_0_1(&mut self, value: String) {
    let parts = value.split(',');
    let mut offset = None;
    for (i, part) in parts.enumerate() {
      if i % 2 == 0 {
        // This is an offset
        if let Ok(parsed_offset) = part.parse::<u64>() {
          offset = Some(parsed_offset);
        } else {
          // TODO: log warning about invalid offset
        }
      } else {
        // This is a size
        if let (Some(offset), Ok(parsed_data_size)) = (offset, part.parse::<u64>()) {
          self.gnu_sparse_map_local.push(SparseFileInstruction {
            offset_before: offset,
            data_size: parsed_data_size,
          });
        } else {
          // TODO: log warning about invalid size
        }
        offset = None; // Reset offset for the next pair
      }
    }
  }

  pub fn drain_local_unparsed_attributes(&mut self) -> HashMap<String, String> {
    let mut local_unparsed_attributes =
      core::mem::replace(&mut self.unparsed_attributes, HashMap::new());
    // add the global unparsed attributes to the local ones
    for (key, value) in self.global_attributes.iter() {
      if !local_unparsed_attributes.contains_key(key) {
        local_unparsed_attributes.insert(key.clone(), value.clone());
      }
    }
    local_unparsed_attributes
  }

  fn ingest_attribute(&mut self, confidence: PaxConfidence, key: String, value: String) {
    match key.as_str() {
      GNU_SPARSE_NAME_01_01 => {
        if confidence == PaxConfidence::LOCAL {
          self
            .gnu_sparse_name_01_01
            .insert_with_confidence(confidence, value);
        } else {
          // TODO: log warning
        }
      },
      GNU_SPARSE_REALSIZE_1_0 => {
        if confidence == PaxConfidence::LOCAL {
          if let Ok(parsed_value) = value.parse::<usize>() {
            self
              .gnu_sparse_realsize_1_0
              .insert_with_confidence(confidence, parsed_value);
          }
        } else {
          // TODO: log warning
        }
      },
      GNU_SPARSE_MAJOR => {
        if let Ok(parsed_value) = value.parse::<u32>() {
          self
            .gnu_sprase_major
            .insert_with_confidence(confidence, parsed_value);
        }
      },
      GNU_SPARSE_MINOR => {
        if let Ok(parsed_value) = value.parse::<u32>() {
          self
            .gnu_sparse_minor
            .insert_with_confidence(confidence, parsed_value);
        }
      },
      GNU_SPARSE_REALSIZE_0_01 => {
        if confidence == PaxConfidence::LOCAL {
          if let Ok(parsed_value) = value.parse::<usize>() {
            self
              .gnu_sparse_realsize_0_01
              .insert_with_confidence(confidence, parsed_value);
          }
        } else {
          // TODO: log warning
        }
      },
      GNU_SPARSE_MAP_NUM_BLOCKS_0_01 => {
        // This is a user controlled value so we don't reserve capacity
      },
      GNU_SPARSE_DATA_BLOCK_OFFSET_0_0 => {
        if confidence == PaxConfidence::LOCAL {
          self
            .gnu_sprase_major
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          self
            .gnu_sparse_minor
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          if let Ok(parsed_value) = value.parse::<u64>() {
            self.sparse_instruction_builder.offset_before = Some(parsed_value);
          }
          self.try_finish_sparse_instruction();
        } else {
          // TODO: log warning
        }
      },
      GNU_SPARSE_DATA_BLOCK_SIZE_0_0 => {
        if confidence == PaxConfidence::LOCAL {
          self
            .gnu_sprase_major
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          self
            .gnu_sparse_minor
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          if let Ok(parsed_value) = value.parse::<u64>() {
            self.sparse_instruction_builder.data_size = Some(parsed_value);
          }
          self.try_finish_sparse_instruction();
        } else {
          // TODO: log warning
        }
      },
      GNU_SPARSE_MAP_0_1 => {
        if confidence == PaxConfidence::LOCAL {
          self
            .gnu_sprase_major
            .insert_with_confidence(PaxConfidence::LOCAL, 0);
          self
            .gnu_sparse_minor
            .insert_with_confidence(PaxConfidence::LOCAL, 1);
          self.parse_gnu_sparse_map_0_1(value);
        } else {
          // TODO: log warning
        }
      },
      ATIME => {
        if let Some(parsed_value) = Self::parse_time(value.as_str()) {
          self.atime.insert_with_confidence(confidence, parsed_value);
        }
      },
      GID => {
        if let Ok(parsed_value) = value.parse::<u32>() {
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
        if let Some(parsed_value) = Self::parse_time(value.as_str()) {
          self.mtime.insert_with_confidence(confidence, parsed_value);
        }
      },
      CTIME => {
        if let Some(parsed_value) = Self::parse_time(value.as_str()) {
          self.ctime.insert_with_confidence(confidence, parsed_value);
        }
      },
      PATH => {
        self.path.insert_with_confidence(confidence, value);
      },
      SIZE => {
        if let Ok(parsed_value) = value.parse::<usize>() {
          self
            .data_size
            .insert_with_confidence(confidence, parsed_value);
        }
      },
      UID => {
        if let Ok(parsed_value) = value.parse::<u32>() {
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
            self.global_attributes.insert(key, value);
          },
          PaxConfidence::LOCAL => {
            self.unparsed_attributes.insert(key, value);
          },
        }
      },
    }
  }

  /// "%d %s=%s\n", <length>, <keyword>, <value>
  ///
  /// This function parses the length decimal and computes the values for the parsing key state.
  fn state_parsing_new_kv(
    &mut self,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateParsingNewKV,
  ) -> Result<PaxParserState, TarParserError> {
    let debug_buffer_char = str::from_utf8(cursor.full_buffer());

    // Read the length until we hit a space or newline
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut &mut state.kv_cursor,
      false,
      |byte: &u8| *byte == b' ' || *byte == b'\n',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) => {
        // Successfully read the length, now we can parse it
      },
      Err(CopyUntilError::DelimiterNotFound { .. }) => {
        // Not enough data in the current `bytes` slice, preserve state and wait for more
        return Ok(PaxParserState::ParsingNewKV(state));
      },
      Err(CopyUntilError::IoRead(..)) => panic!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(FixedSizeBufferError { .. })),
      ) => {
        return Err(TarParserError::CorruptPaxLength {
          max_length_field_length: state.kv_cursor.full_buffer().len(),
        })
      },
    }

    // Convert the length bytes to a usize
    let length_str = core::str::from_utf8(state.kv_cursor.before()).unwrap_or("0");
    let length = match length_str.parse::<usize>() {
      Ok(value) => value,
      Err(e) => return Err(TarParserError::CorruptPaxLengthInteger(e)),
    };

    let length = length.saturating_sub(state.kv_cursor.before().len() + 1);
    if length == 0 {
      // If the length is 0, we are done with this key-value pair
      return Ok(PaxParserState::default());
    }
    Ok(PaxParserState::ParsingKey(StateParsingKey {
      length,
      keyword: Vec::new(),
    }))
  }

  /// Parses the key from the cursor and returns the next state.
  fn state_parsing_key(
    &mut self,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateParsingKey,
  ) -> Result<PaxParserState, TarParserError> {
    // Read the length until we hit an equals sign
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut &mut state.keyword,
      false,
      |byte: &u8| *byte == b'=',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) => {
        let length_after_equals = state.length.saturating_sub(state.keyword.len() + 1);
        if length_after_equals == 0 {
          // If the length is 0, we are done with this key-value pair
          return Ok(PaxParserState::default());
        }
        let key = String::from_utf8(state.keyword).map_err(|_| TarParserError::CorruptPaxKey)?;
        return Ok(PaxParserState::ParsingValue(StateParsingValue {
          key,
          length_after_equals,
          value: Vec::new(),
        }));
      },
      Err(CopyUntilError::DelimiterNotFound { .. }) => {
        // Not enough data in the current `bytes` slice, preserve state and wait for more.
        return Ok(PaxParserState::ParsingKey(state));
      },
      Err(CopyUntilError::IoRead(..)) => panic!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(TryReserveError { .. })),
      ) => {
        return Err(TarParserError::CorruptPaxLength {
          max_length_field_length: state.keyword.len(),
        })
      },
    }
  }

  fn state_parsing_value(
    &mut self,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateParsingValue,
  ) -> Result<PaxParserState, TarParserError> {
    if state.length_after_equals == 0 {
      // Record must end in a newline, so length of value part must be at least 1.
      return Err(TarParserError::CorruptPaxValue);
    }

    let value_len = state.length_after_equals - 1;
    let bytes_needed = value_len.saturating_sub(state.value.len());

    let bytes_available = cursor.full_buffer().len() - cursor.position();
    let bytes_to_read = bytes_needed.min(bytes_available);

    if bytes_to_read > 0 {
      let start = cursor.position();
      let end = start + bytes_to_read;
      state
        .value
        .extend_from_slice(&cursor.full_buffer()[start..end]);
      cursor.set_position(end);
    }

    // Check if we have the full value now
    if state.value.len() < value_len {
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
      return Err(TarParserError::CorruptPaxValue);
    }
    cursor.set_position(cursor.position() + 1);

    // We have a full key-value pair. Ingest it.
    let value = String::from_utf8(state.value).map_err(|_| TarParserError::CorruptPaxValue)?;

    self.ingest_attribute(self.current_pax_mode, state.key, value);

    // Ready for the next key-value pair
    Ok(PaxParserState::default())
  }
}

impl Write for PaxParser {
  type WriteError = TarParserError;
  type FlushError = core::convert::Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    let mut cursor = Cursor::new(input_buffer);

    let parser_state = core::mem::replace(&mut self.state, PaxParserState::NoNextStateSet);

    self.state = match parser_state {
      PaxParserState::ParsingNewKV(state) => self.state_parsing_new_kv(&mut cursor, state)?,
      PaxParserState::ParsingKey(state) => self.state_parsing_key(&mut cursor, state)?,
      PaxParserState::ParsingValue(state) => self.state_parsing_value(&mut cursor, state)?,
      PaxParserState::NoNextStateSet => {
        panic!("BUG: No next state set in PaxParser");
      },
    };

    Ok(cursor.position())
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use core::num::ParseIntError;

  use super::*;

  use alloc::{string::ToString as _, vec};

  use crate::{BytewiseWriter, WriteAll as _, WriteAllError};

  #[test]
  fn test_new_with_initial_global_attributes() {
    let mut globals = HashMap::new();
    globals.insert("gname".to_string(), "wheel".to_string());
    globals.insert("uid".to_string(), "0".to_string());

    let parser = PaxParser::new(globals);

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

  #[test]
  fn test_simple_kv_parsing() {
    let mut parser = PaxParser::default();
    let data = b"18 path=some/file\n";
    parser.write_all(data, false).unwrap();

    assert_eq!(parser.path.get(), Some(&"some/file".to_string()));
    assert_eq!(parser.state, PaxParserState::default());
  }

  #[test]
  fn test_multiple_kv_parsing() {
    let mut parser = PaxParser::default();
    let data = b"18 path=some/file\n12 size=123\n12 uid=1000\n";
    parser.write_all(data, false).unwrap();

    assert_eq!(parser.path.get(), Some(&"some/file".to_string()));
    assert_eq!(parser.state, PaxParserState::default());
  }

  #[test]
  fn test_multiple_kv_parsing_from_archive() {
    let mut parser = PaxParser::default();
    let data =
      b"30 mtime=1749954382.774290089\n20 atime=1749803808\n30 ctime=1749954382.774290089\n";
    let mut bytewise_writer = BytewiseWriter::new(&mut parser);
    let write_result = bytewise_writer.write_all(data, false);

    assert!(
      write_result.is_ok(),
      "Failed to write data: {:?}",
      write_result.err()
    );
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
    let mut parser = PaxParser::default();
    let data = b"45 GNU.sparse.map=1024,512,8192,2048,16384,0\n";
    parser.write_all(data, false).unwrap();

    let expected = vec![
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
    ];
    assert_eq!(parser.gnu_sparse_map_local, expected);
  }

  #[test]
  fn test_unparsed_attributes_and_drain() {
    let mut parser = PaxParser::default();
    let data = b"21 SCHILY.fflags=bar\n12 uid=1000\n";
    parser.write_all(data, false).unwrap();

    assert_eq!(parser.unparsed_attributes.len(), 1);
    assert_eq!(
      parser.unparsed_attributes.get("SCHILY.fflags"),
      Some(&"bar".to_string())
    );

    let drained = parser.drain_local_unparsed_attributes();

    assert_eq!(drained.len(), 1);
    assert_eq!(drained.get("SCHILY.fflags"), Some(&"bar".to_string()));
    assert!(parser.unparsed_attributes.is_empty());
  }

  #[test]
  fn test_parser_error_bad_length() {
    let mut parser = PaxParser::default();
    let data = b"abc path=foo\n";
    assert!(matches!(
      parser.write_all(data, false),
      Err(WriteAllError::Io(TarParserError::CorruptPaxLengthInteger(
        ParseIntError { .. }
      )))
    ));
  }

  #[test]
  fn test_parser_error_bad_value() {
    let mut parser = PaxParser::default();
    // The length 11 covers " path=foo ". It must end with '\n'.
    let data = b"11 path=foo ";
    assert_eq!(
      parser.write_all(data, false),
      Err(WriteAllError::Io(TarParserError::CorruptPaxValue))
    );
  }
}
