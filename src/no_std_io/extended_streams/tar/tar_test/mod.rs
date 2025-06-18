use alloc::{string::String, vec::Vec};
use hashbrown::HashMap;

use crate::tar_gz_extract::{self, strip_gzip_header};

const MY_FILE: &[u8] = include_bytes!("test-archive/subfolder/my_file.txt");
const MY_FILE_PATH: &str = "test-archive/subfolder/my_file.txt";
const LOREM_TXT: &[u8] = include_bytes!("test-archive/lorem.txt");
const LOREM_TXT_PATH: &str = "test-archive/lorem.txt";
const TEST_FILE: &[u8] = include_bytes!("test-archive/test_file.txt");
const TEST_FILE_PATH: &str = "test-archive/test_file.txt";

const USTAR_TAR: &[u8] = include_bytes!("test-ustar.tar");
const USTAR_TAR_GZ: &[u8] = include_bytes!("test-ustar.tar.gz");

#[test]
fn straight_miniz() {
  use miniz_oxide::inflate::decompress_to_vec;

  let deflate_data = strip_gzip_header(USTAR_TAR_GZ);
  let decompressed = decompress_to_vec(deflate_data).expect("Failed to decompress USTAR TAR.GZ");
  assert_eq!(decompressed, USTAR_TAR);
}

#[test]
fn straight_compressed_reader() {
  use crate::compressed_reader::CompressedReader;

  let deflate_data = strip_gzip_header(USTAR_TAR_GZ);
  let reader = CompressedReader::new(deflate_data, false);
  let mut buffered_reader: crate::buffered_reader::BufferedReader<CompressedReader<'_>> =
    crate::buffered_reader::BufferedReader::new(16 * 1024 * 1024, reader);
  let mut decompressed_result = Vec::with_capacity(USTAR_TAR.len());
  let decompressed = buffered_reader
    .read_exact(1)
    .expect("Failed to read decompressed data");
  decompressed_result.extend_from_slice(&decompressed);
  let decompressed = buffered_reader
    .read_exact(USTAR_TAR.len() - 1)
    .expect("Failed to read decompressed data");
  decompressed_result.extend_from_slice(&decompressed);

  assert_eq!(decompressed_result, USTAR_TAR);
}

fn assert_test_archive(files: HashMap<String, Vec<u8>>) {
  let _dbg_file_paths: Vec<_> = files.keys().cloned().collect();
  let _dbg_file_contents: Vec<String> = files
    .values()
    .map(|v| String::from(String::from_utf8_lossy(v)))
    .collect();
  assert_eq!(files.len(), 3);
  assert_eq!(files.get(MY_FILE_PATH), Some(&MY_FILE.to_vec()));
  assert_eq!(files.get(LOREM_TXT_PATH), Some(&LOREM_TXT.to_vec()));
  assert_eq!(files.get(TEST_FILE_PATH), Some(&TEST_FILE.to_vec()));
}

#[test]
fn test_ustar_extract_uncompressed() {
  let files = tar_gz_extract::extract_tar_file(USTAR_TAR, 16 * 1024 * 1024)
    .expect("Failed to extract USTAR TAR");
  assert_test_archive(files);
}

#[test]
fn test_ustar_extract_compressed() {
  let files = tar_gz_extract::extract_tar_file(USTAR_TAR_GZ, 16 * 1024 * 1024)
    .expect("Failed to extract USTAR TAR.GZ");
  assert_test_archive(files);
}
