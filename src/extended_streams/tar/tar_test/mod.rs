use alloc::{
  string::{String, ToString},
  vec::Vec,
};

use hashbrown::HashMap;

use crate::{
  extended_streams::tar::{
    FileData, FileEntry, RegularFileEntry, TarInode, TarParser, TarParserOptions,
  },
  WriteAll,
};

struct SimpleFile {
  file_path: &'static str,
  data: &'static [u8],
}

impl SimpleFile {
  fn assert_exists_and_data_matches(&self, files: &[TarInode]) {
    let file = files.iter().find(|f| f.path.as_str() == self.file_path);
    assert!(
      file.is_some(),
      "File {} not found in archive",
      self.file_path
    );
    let file = file.unwrap();
    match &file.entry {
      FileEntry::RegularFile(RegularFileEntry {
        data: FileData::Regular(data),
        ..
      }) => {
        assert_eq!(
          data, self.data,
          "Data for file {} does not match expected data",
          self.file_path
        );
      },
      _ => panic!("Expected RegularFileEntry for file {}", self.file_path),
    }
  }
}

macro_rules! create_simple_file {
  ($file_path:expr) => {
    SimpleFile {
      file_path: $file_path,
      data: include_bytes!($file_path),
    }
  };
}

const SIMPLE_FILES: &[SimpleFile] = &[
  create_simple_file!("test-archive/subfolder/my_file.txt"),
  create_simple_file!("test-archive/lorem.txt"),
  create_simple_file!("test-archive/test_file.txt"),
];

const USTAR_TAR: &[u8] = include_bytes!("test-ustar.tar");
const USTAR_TAR_GZ: &[u8] = include_bytes!("test-ustar.tar.gz");

fn assert_test_archive_simple_files(files: &[TarInode]) {
  let _dbg_file_paths: Vec<_> = files.iter().map(|f| f.path.as_str().to_string()).collect();
  for file in SIMPLE_FILES {
    file.assert_exists_and_data_matches(&files);
  }
}

#[test]
fn test_ustar_extract_uncompressed() {
  let mut tar_parser = TarParser::new(TarParserOptions::default());
  let parser_result = tar_parser.write_all(USTAR_TAR, false);
  assert!(
    parser_result.is_ok(),
    "Failed to parse USTAR TAR: {:?}",
    parser_result.unwrap_err()
  );
  let files = tar_parser.get_extracted_files();
  assert_test_archive_simple_files(&files);
}
