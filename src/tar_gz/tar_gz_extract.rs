use alloc::string::String;

use hashbrown::HashMap;
use relative_path::RelativePathBuf;
use thiserror::Error;

use crate::{
  no_std_io::{BufferedReader, LimitedReader, Read},
  tar_gz::{file_entry::ExtractedFile, tar_constants::TarTypeFlag},
};

struct TarExtractionState<'a, R: Read> {
  reader: BufferedReader<'a, R>,
  global_extended_attributes: HashMap<String, String>,
  extracted_files: HashMap<RelativePathBuf, ExtractedFile>,
  found_type_flags: HashMap<TarTypeFlag, usize>,
}

#[derive(Error, Debug)]
pub enum TarExtractionError<U> {
  #[error("Underlying read error: {0:?}")]
  Io(U),
}

impl<'a, R: Read> TarExtractionState<'a, R> {
  #[must_use]
  fn new(reader: &'a mut R, max_temp_buffer_size: usize) -> Self {
    Self {
      reader: BufferedReader::new(reader, max_temp_buffer_size, 1),
      global_extended_attributes: HashMap::new(),
      extracted_files: HashMap::new(),
      found_type_flags: HashMap::new(),
    }
  }

  fn parse(&mut self) -> Result<(), TarExtractionError<R::ReadError>> {
    todo!()
  }
}
