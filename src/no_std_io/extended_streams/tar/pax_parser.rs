use alloc::{string::String, vec::Vec};
use hashbrown::HashMap;

use crate::no_std_io::extended_streams::tar::{
  confident_value::ConfidentValue, SparseFileInstruction,
};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum PaxConfidence {
  LOCAL = 1,
  GLOBAL,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum DateConfidence {
  ATIME = 1,
  MTIME,
}

struct PaxConfidentValue<T> {
  global: Option<PaxConfidence>,
  local: Option<(PaxConfidence, T)>,
}

/// "%d %s=%s\n", <length>, <keyword>, <value>
pub(crate) struct PaxParser {
  unparsed_global_attributes: HashMap<String, String>,
  unparsed_attributes: HashMap<String, String>,
  gnu_sparse_name_01_01: PaxConfidentValue<String>,
  gnu_sparse_realsize_1_0: PaxConfidentValue<usize>,
  gnu_sprase_major: PaxConfidentValue<u32>,
  gnu_sparse_minor: PaxConfidentValue<u32>,
  gnu_sparse_realsize_0_01: PaxConfidentValue<usize>,
  gnu_sparse_map: PaxConfidentValue<Vec<SparseFileInstruction>>,
  mtime: PaxConfidentValue<ConfidentValue<DateConfidence, u64>>,
  gid: PaxConfidentValue<u32>,
  gname: PaxConfidentValue<String>,
  link_path: PaxConfidentValue<String>,
  path: PaxConfidentValue<String>,
  data_size: PaxConfidentValue<usize>,
  uid: PaxConfidentValue<u32>,
  uname: PaxConfidentValue<String>,
}
