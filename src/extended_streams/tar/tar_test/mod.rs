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

const TAR_ARCHIVES: &[SimpleFile] = &[
  create_simple_file!("test-v7.tar"),
  create_simple_file!("test-ustar.tar"),
  create_simple_file!("test-pax.tar"),
  create_simple_file!("test-gnu-nosparse.tar"),
  create_simple_file!("test-gnu-sparse-0.0.tar"),
  create_simple_file!("test-gnu-sparse-0.1.tar"),
  create_simple_file!("test-gnu-sparse-1.0.tar"),
];

//const TAR_ARCHIVES_COMPRESSED: &[SimpleFile] = &[create_simple_file!("test-ustar.tar.gz")];

fn assert_test_archive_simple_files(files: &[TarInode]) {
  let _dbg_file_paths: Vec<_> = files.iter().map(|f| f.path.as_str().to_string()).collect();
  for file in SIMPLE_FILES {
    file.assert_exists_and_data_matches(&files);
  }
}

fn assert_parse_archive(archive: &SimpleFile) {
  let mut tar_parser = TarParser::new(TarParserOptions::default());
  let parser_result = tar_parser.write_all(archive.data, false);
  assert!(
    parser_result.is_ok(),
    "Failed to parse {}: {:?}",
    archive.file_path,
    parser_result.unwrap_err()
  );
  let files = tar_parser.get_extracted_files();
  assert_test_archive_simple_files(&files);
}

#[test]
fn test_tar_extract_uncompressed() {
  for archive in TAR_ARCHIVES {
    assert_parse_archive(archive);
  }
}
