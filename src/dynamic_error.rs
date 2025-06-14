use core::{error::Error, fmt::Display};

use alloc::string::String;

#[derive(Debug, Clone)]
pub struct DynamicError(pub String);

impl Error for DynamicError {
  fn source(&self) -> Option<&(dyn Error + 'static)> {
    None
  }

  fn description(&self) -> &str {
    "description() is deprecated; use Display"
  }

  fn cause(&self) -> Option<&dyn Error> {
    self.source()
  }
}

impl Display for DynamicError {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    write!(f, "DynamicError: {}", self.0)
  }
}
