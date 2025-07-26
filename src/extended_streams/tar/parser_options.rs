use alloc::string::String;

use hashbrown::HashMap;

pub struct TarParserLimits {
  /// The maximum number of sparse file instructions allowed in a single file.
  pub max_sparse_file_instructions: usize,
  /// The maximum length of a PAX key or value in bytes.
  /// This also limits the maximum file path length!
  pub max_pax_key_value_length: usize,
  /// The maximum number of global attributes that can be parsed.
  pub max_global_attributes: usize,
  /// The maximum number of unparsed global attributes that can be stored.
  pub max_unparsed_global_attributes: usize,
  /// The maximum number of unparsed local attributes that can be stored.
  pub max_unparsed_local_attributes: usize,
}

pub struct TarParserOptions {
  /// Tar can contain previous versions of the same file.
  ///
  /// If true, only the last version of each file will be kept.
  /// If false, all versions of each file will be kept.
  pub keep_only_last: bool,
  pub initial_global_extended_attributes: HashMap<String, String>,
  pub tar_parser_limits: TarParserLimits,
}

impl Default for TarParserOptions {
  fn default() -> Self {
    Self {
      keep_only_last: true,
      initial_global_extended_attributes: HashMap::new(),
      tar_parser_limits: TarParserLimits {
        max_sparse_file_instructions: 2048,
        max_pax_key_value_length: 1024 * 8,
        max_global_attributes: 1024,
        max_unparsed_global_attributes: 1024,
        max_unparsed_local_attributes: 1024,
      },
    }
  }
}
