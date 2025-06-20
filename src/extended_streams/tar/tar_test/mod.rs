use alloc::{string::ToString, vec::Vec};

use crate::{
  extended_streams::tar::{
    expand_sparse_files, FileData, FileEntry, RegularFileEntry, TarInode, TarParser,
    TarParserOptions,
  },
  WriteAll,
};

struct SimpleFile {
  file_path: &'static str,
  data: &'static [u8],
}

fn first_diff_index(a: &[u8], b: &[u8]) -> Option<usize> {
  a.iter()
    .zip(b.iter())
    .position(|(x, y)| x != y)
    .or_else(|| {
      if a.len() != b.len() {
        Some(a.len().min(b.len()))
      } else {
        None
      }
    })
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
        // find index of byte that is different
        let diff_index = first_diff_index(data, self.data);
        if let Some(index) = diff_index {
          // Show the index and the first 40 different bytes
          let bytes_to_show = 40;
          let bytes_expected = &self.data[index..(index + bytes_to_show).min(self.data.len())];
          let bytes_found = &data[index..(index + bytes_to_show).min(data.len())];
          panic!(
            "Data for file {} does not match at index {}: expected len {}, found len {}, expected data {:?}, found data {:?}",
            self.file_path, index,
            self.data.len(), data.len(),
            bytes_expected, bytes_found
          );
        }
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
  create_simple_file!("test-archive/sparse_test_file.txt"),
];

const TAR_ARCHIVES: &[SimpleFile] = &[
  create_simple_file!("test-v7.tar"),
  create_simple_file!("test-ustar.tar"),
  create_simple_file!("test-pax.tar"),
  create_simple_file!("test-gnu-oldsparse.tar"),
  //create_simple_file!("test-gnu-sparse-0.0.tar"),
  //create_simple_file!("test-gnu-sparse-0.1.tar"),
  //create_simple_file!("test-gnu-sparse-1.0.tar"),
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
  let mut files = tar_parser.get_extracted_files().to_vec();
  expand_sparse_files(&mut files);
  assert_test_archive_simple_files(&files);
}

#[test]
fn test_tar_extract_uncompressed() {
  for archive in TAR_ARCHIVES {
    assert_parse_archive(archive);
  }
}
