use alloc::{
  format,
  string::{String, ToString},
};

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum SparseFormat {
  GnuOld,
  Gnu0_0,
  Gnu0_1,
  Gnu1_0,
  GnuUnknownSparseFormat { major: u32, minor: u32 },
}

impl SparseFormat {
  /// Returns the major and minor version of the GNU sparse format.
  #[must_use]
  pub fn get_major_minor(&self) -> (u32, u32) {
    match self {
      SparseFormat::GnuOld => (0, 0),
      SparseFormat::Gnu0_0 => (0, 0),
      SparseFormat::Gnu0_1 => (0, 1),
      SparseFormat::Gnu1_0 => (1, 0),
      SparseFormat::GnuUnknownSparseFormat { major, minor } => (*major, *minor),
    }
  }

  /// Creates a new `SparseFormat` from the major and minor version.
  #[must_use]
  pub fn try_from_gnu_version(major: Option<u32>, minor: Option<u32>) -> Option<Self> {
    Some(match (major, minor) {
      (Some(0), Some(0) | None) => SparseFormat::Gnu0_0,
      (Some(0) | None, Some(1)) => SparseFormat::Gnu0_1,
      (Some(1), Some(0)) => SparseFormat::Gnu1_0,
      (None, None) => return None,
      (major, minor) => SparseFormat::GnuUnknownSparseFormat {
        major: major.unwrap_or(0),
        minor: minor.unwrap_or(0),
      },
    })
  }

  #[must_use]
  pub fn to_version_string(&self) -> String {
    match self {
      SparseFormat::GnuOld => "gnu_old".to_string(),
      SparseFormat::Gnu0_0 => "gnu_0.0".to_string(),
      SparseFormat::Gnu0_1 => "gnu_0.1".to_string(),
      SparseFormat::Gnu1_0 => "gnu_1.0".to_string(),
      SparseFormat::GnuUnknownSparseFormat { major, minor } => {
        format!("gnu_{major}.{minor}")
      },
    }
  }
}
