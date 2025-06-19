use alloc::{string::String, vec::Vec};
use hashbrown::HashMap;
use relative_path::RelativePathBuf;

use crate::no_std_io::{
  extended_streams::tar::{
    confident_value::ConfidentValue,
    tar_constants::pax_keys_well_known::{
      gnu::{
        GNU_SPARSE_DATA_BLOCK_OFFSET_0_0, GNU_SPARSE_DATA_BLOCK_SIZE_0_0, GNU_SPARSE_MAJOR,
        GNU_SPARSE_MAP_0_1, GNU_SPARSE_MAP_NUM_BLOCKS_0_01, GNU_SPARSE_MINOR,
        GNU_SPARSE_NAME_01_01, GNU_SPARSE_REALSIZE_0_01, GNU_SPARSE_REALSIZE_1_0,
      },
      ATIME, GID, GNAME, LINKPATH, MTIME, PATH, SIZE, UID, UNAME,
    },
    InodeBuilder, InodeConfidentValue, SparseFileInstruction, SparseFormat,
  },
  Cursor,
};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum PaxConfidence {
  GLOBAL = 1,
  LOCAL,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum DateConfidence {
  ATIME = 1,
  MTIME,
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

struct StateParsingKey {
  length: usize,
  keyword: Vec<u8>,
}

struct StateParsingValue {
  key: String,
  length_after_equals: usize,
  value: Vec<u8>,
}

#[derive(Default)]
enum PaxParserState {
  #[default]
  ExpectingNextKV,
  ParsingKey(StateParsingKey),
  ParsingValue(StateParsingValue),
  NoNextStateSet,
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
  gnu_sparse_name_01_01: PaxConfidentValue<RelativePathBuf>,
  gnu_sparse_realsize_1_0: PaxConfidentValue<usize>,
  gnu_sprase_major: PaxConfidentValue<u32>,
  gnu_sparse_minor: PaxConfidentValue<u32>,
  gnu_sparse_realsize_0_01: PaxConfidentValue<usize>,
  gnu_sparse_map_local: Vec<SparseFileInstruction>,
  mtime: PaxConfidentValue<ConfidentValue<DateConfidence, u64>>,
  gid: PaxConfidentValue<u32>,
  gname: PaxConfidentValue<String>,
  link_path: PaxConfidentValue<String>,
  path: PaxConfidentValue<RelativePathBuf>,
  data_size: PaxConfidentValue<usize>,
  uid: PaxConfidentValue<u32>,
  uname: PaxConfidentValue<String>,

  // state
  state: PaxParserState,
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

  fn get_sparse_format(&self) -> Option<SparseFormat> {
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

  pub fn load_pax_attributes_into_inode_builder(&self, inode_builder: &mut InodeBuilder) {
    if let Some(sparse_format) = self.get_sparse_format() {
      if inode_builder.sparse_format.is_some() {
        // TODO: log error that we found conflicting sparse formats
      } else {
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
        inode_builder.sparse_file_instructions = self.gnu_sparse_map_local.clone();
      }
    }
    inode_builder
      .file_path
      .update_with(Self::to_confident_value(self.path.get_with_confidence()));
    inode_builder.mtime.update_with(Self::to_confident_value(
      self
        .mtime
        .get_with_confidence()
        .and_then(|(confidence, value)| match value.get() {
          Some(value) => Some((confidence, value)),
          None => None,
        }),
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
    let mut sparse_instruction_builder = SparseFileInstructionBuilder::default();
    for (i, part) in parts.enumerate() {
      if i % 2 == 0 {
        // This is an offset
        if let Ok(parsed_value) = part.parse::<u64>() {
          sparse_instruction_builder.offset_before = Some(parsed_value);
        } else {
          // TODO: log warning about invalid offset
          sparse_instruction_builder.offset_before = None;
        }
      } else {
        // This is a size
        if let Ok(parsed_value) = part.parse::<u64>() {
          sparse_instruction_builder.data_size = Some(parsed_value);
        } else {
          // TODO: log warning about invalid size
          sparse_instruction_builder.data_size = None;
        }
      }
      match (
        sparse_instruction_builder.offset_before,
        sparse_instruction_builder.data_size,
      ) {
        (Some(_), Some(_)) => {
          self.gnu_sparse_map_local.push(SparseFileInstruction {
            offset_before: sparse_instruction_builder.offset_before.unwrap(),
            data_size: sparse_instruction_builder.data_size.unwrap(),
          });
        },
        _ => {},
      }
      self.sparse_instruction_builder = Default::default();
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
            .insert_with_confidence(confidence, RelativePathBuf::from(value));
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
          self.parse_gnu_sparse_map_0_1(value);
        } else {
          // TODO: log warning
        }
      },
      ATIME => {
        if let Ok(parsed_value) = value.parse::<u64>() {
          self.mtime.insert_with_confidence(
            PaxConfidence::LOCAL,
            ConfidentValue::new(DateConfidence::ATIME, parsed_value),
          );
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
        if let Ok(parsed_value) = value.parse::<u64>() {
          self.mtime.insert_with_confidence(
            PaxConfidence::LOCAL,
            ConfidentValue::new(DateConfidence::MTIME, parsed_value),
          );
        }
      },
      PATH => {
        self
          .path
          .insert_with_confidence(confidence, RelativePathBuf::from(value));
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

  fn state_expecting_next_kv(&mut self, cursor: &mut Cursor<&[u8]>) -> PaxParserState {
    todo!()
  }

  fn state_parsing_key(&mut self, cursor: &mut Cursor<&[u8]>) -> PaxParserState {
    todo!()
  }

  fn state_parsing_value(&mut self, cursor: &mut Cursor<&[u8]>) -> PaxParserState {
    todo!()
  }

  pub fn parse_bytes(&mut self, bytes: &[u8], pax_mode: PaxConfidence) -> usize {
    let mut cursor = Cursor::new(bytes);

    let parser_state = core::mem::replace(&mut self.state, PaxParserState::NoNextStateSet);

    self.state = match parser_state {
      PaxParserState::ExpectingNextKV => self.state_expecting_next_kv(&mut cursor),
      PaxParserState::ParsingKey(state_parsing_key) => self.state_parsing_key(&mut cursor),
      PaxParserState::ParsingValue(state_parsing_value) => self.state_parsing_value(&mut cursor),
      PaxParserState::NoNextStateSet => {
        panic!("BUG: No next state set in PaxParser");
      },
    };

    cursor.position()
  }
}
