use alloc::string::String;

use crate::no_std_io::extended_streams::tar::confident_value::ConfidentValue;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum PaxConfidence {
  LOCAL = 1,
  GLOBAL,
}

type PaxConfidentValue = ConfidentValue<PaxConfidence, String>;

/// "%d %s=%s\n", <length>, <keyword>, <value>
pub(crate) struct PaxParser {}
