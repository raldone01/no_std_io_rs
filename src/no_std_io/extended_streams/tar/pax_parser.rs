use alloc::{string::String, vec::Vec};
use hashbrown::HashMap;
use relative_path::RelativePathBuf;

use crate::no_std_io::extended_streams::tar::{
  confident_value::ConfidentValue, GetOrInsertWithOption, InodeBuilder, InodeConfidentValue,
  SparseFileInstruction, SparseFormat,
};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum PaxConfidence {
  LOCAL = 1,
  GLOBAL,
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
  gnu_sparse_map: PaxConfidentValue<Vec<SparseFileInstruction>>,
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
    inode_builder
      .file_path
      .update_with(Self::to_confident_value(
        self
          .gnu_sparse_name_01_01
          .get_with_confidence()
          .or(self.path.get_with_confidence()),
      ));
    // TODO: all the other stuff

    inode_builder
      .sparse_format
      .get_or_insert_with_option(|| self.get_sparse_format());
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
    self.gnu_sparse_map.reset_local();
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
  }

  fn ingest_attribute(&mut self, confidence: PaxConfidence, key: String, value: String) {
    todo!()
  }
}
