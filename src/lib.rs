#![no_std]
extern crate alloc;

mod gz_header;
mod no_std_io;
mod reader_bytewise;
mod reader_compressed;
mod reader_exact;
mod reader_slice;
mod tar_constants;
mod writer_buffer;
mod writer_buffered;
mod writer_bytewise;
mod writer_compressed;
//mod tar_gz_create;
//mod tar_gz_extract;
//#[cfg(test)]
//mod tar_test;

mod dynamic_error {
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
}
