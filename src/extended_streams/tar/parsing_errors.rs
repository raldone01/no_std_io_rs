use core::{fmt::Display, num::ParseIntError, str::Utf8Error};

use thiserror::Error;

use crate::{
  extended_streams::tar::{
    pax_parser::PaxParserError,
    tar_constants::{ParseOctalError, TarHeaderChecksumError},
    SparseFormat,
  },
  LimitedBackingBufferError,
};

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum GeneralParseError {
  #[error("Invalid octal number: {0}")]
  InvalidOctalNumber(#[from] ParseOctalError),
  #[error("Invalid UTF-8 string: {0}")]
  InvalidUtf8(#[from] Utf8Error),
  #[error("Invalid integer: {0}")]
  InvalidInteger(#[from] ParseIntError),
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TarHeaderParserError {
  #[error("Unknown magic+version: {magic:?}+{version:?}")]
  UnknownHeaderMagicVersion { magic: [u8; 6], version: [u8; 2] },
  #[error("Checksum error: {0}")]
  CorruptHeaderChecksum(#[from] TarHeaderChecksumError),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CorruptFieldContext {
  HeaderSize,
  HeaderName,
  HeaderMode,
  HeaderUid,
  HeaderGid,
  HeaderMtime,
  HeaderLinkname,
  HeaderUname,
  HeaderGname,
  HeaderDevMajor,
  HeaderDevMinor,
  HeaderAtime,
  HeaderCtime,
  HeaderRealSize,
  HeaderPrefix,
  GnuSparseNumberOfMaps(SparseFormat),
  GnuSparseMapOffsetValue(SparseFormat),
  GnuSparseMapSizeValue(SparseFormat),
  GnuSparseRealFileSize(SparseFormat),
  GnuSparseMajorVersion,
  GnuSparseMinorVersion,
  PaxWellKnownAtime,
  PaxWellKnownGid,
  PaxWellKnownMtime,
  PaxWellKnownCtime,
  PaxWellKnownSize,
  PaxWellKnownUid,
  PaxKvLength,
  PaxKvValue,
  PaxKvKey,
}

impl Display for CorruptFieldContext {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    match self {
      CorruptFieldContext::HeaderSize => write!(f, "header.size"),
      CorruptFieldContext::HeaderName => write!(f, "header.name"),
      CorruptFieldContext::HeaderMode => write!(f, "header.mode"),
      CorruptFieldContext::HeaderUid => write!(f, "header.uid"),
      CorruptFieldContext::HeaderGid => write!(f, "header.gid"),
      CorruptFieldContext::HeaderMtime => write!(f, "header.mtime"),
      CorruptFieldContext::HeaderLinkname => write!(f, "header.linkname"),
      CorruptFieldContext::HeaderUname => write!(f, "header.uname"),
      CorruptFieldContext::HeaderGname => write!(f, "header.gname"),
      CorruptFieldContext::HeaderDevMajor => write!(f, "header.dev_major"),
      CorruptFieldContext::HeaderDevMinor => write!(f, "header.dev_minor"),
      CorruptFieldContext::HeaderAtime => write!(f, "header.atime"),
      CorruptFieldContext::HeaderCtime => write!(f, "header.ctime"),
      CorruptFieldContext::HeaderRealSize => write!(f, "header.real_size"),
      CorruptFieldContext::HeaderPrefix => write!(f, "header.prefix"),
      CorruptFieldContext::GnuSparseNumberOfMaps(version) => {
        write!(
          f,
          "gnu_sparse.{}.number_of_maps",
          version.to_version_string()
        )
      },
      CorruptFieldContext::GnuSparseMapOffsetValue(version) => {
        write!(
          f,
          "gnu_sparse.{}.map_entry.offset",
          version.to_version_string()
        )
      },
      CorruptFieldContext::GnuSparseMapSizeValue(version) => {
        write!(
          f,
          "gnu_sparse.{}.map_entry.size",
          version.to_version_string()
        )
      },
      CorruptFieldContext::GnuSparseRealFileSize(version) => {
        write!(
          f,
          "gnu_sparse.{}.real_file_size",
          version.to_version_string()
        )
      },
      CorruptFieldContext::GnuSparseMajorVersion => write!(f, "gnu_sparse.major_version"),
      CorruptFieldContext::GnuSparseMinorVersion => write!(f, "gnu_sparse.minor_version"),
      CorruptFieldContext::PaxWellKnownAtime => write!(f, "pax.well_known.atime"),
      CorruptFieldContext::PaxWellKnownGid => write!(f, "pax.well_known.gid"),
      CorruptFieldContext::PaxWellKnownMtime => write!(f, "pax.well_known.mtime"),
      CorruptFieldContext::PaxWellKnownCtime => write!(f, "pax.well_known.ctime"),
      CorruptFieldContext::PaxWellKnownSize => write!(f, "pax.well_known.size"),
      CorruptFieldContext::PaxWellKnownUid => write!(f, "pax.well_known.uid"),
      CorruptFieldContext::PaxKvLength => write!(f, "pax.length_field"),
      CorruptFieldContext::PaxKvValue => write!(f, "pax.value_field"),
      CorruptFieldContext::PaxKvKey => write!(f, "pax.key_field"),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitExceededContext {
  GnuSparse1_0MapDecimalStringTooLong,
  GnuSparse1_0MapOffsetEntryDecimalStringTooLong,
  GnuSparse1_0MapSizeEntryDecimalStringTooLong,
  TooManySparseFileInstructions,
  PaxLengthFieldDecimalStringTooLong,
  PaxKvKeyTooLong,
  PaxKvValueTooLong,
  PaxTooManyUnparsedGlobalAttributes,
  PaxTooManyUnparsedLocalAttributes,
  PaxTooManyGlobalAttributes,
}

impl LimitExceededContext {
  pub(crate) fn context_unit(&self) -> (&'static str, &'static str) {
    match self {
      Self::GnuSparse1_0MapDecimalStringTooLong => (
        "bytes",
        "The decimal string for the number of sparse maps is too long",
      ),
      Self::GnuSparse1_0MapOffsetEntryDecimalStringTooLong => (
        "bytes",
        "The decimal string for a sparse map offset entry is too long",
      ),
      Self::GnuSparse1_0MapSizeEntryDecimalStringTooLong => (
        "bytes",
        "The decimal string for a sparse map size entry is too long",
      ),
      Self::TooManySparseFileInstructions => (
        "sparse file instructions",
        "Too many sparse file instructions",
      ),
      Self::PaxLengthFieldDecimalStringTooLong => (
        "bytes",
        "The decimal string for the PAX length field is too long",
      ),
      Self::PaxKvKeyTooLong => ("bytes", "The PAX key string is too long"),
      Self::PaxKvValueTooLong => ("bytes", "The PAX value string is too long"),
      Self::PaxTooManyUnparsedGlobalAttributes => (
        "unparsed global PAX attributes",
        "Too many unparsed global PAX attributes",
      ),
      Self::PaxTooManyUnparsedLocalAttributes => (
        "unparsed local PAX attributes",
        "Too many unparsed local PAX attributes",
      ),
      Self::PaxTooManyGlobalAttributes => {
        ("global PAX attributes", "Too many global PAX attributes")
      },
    }
  }

  pub(crate) fn context_str(&self) -> &'static str {
    match self {
      Self::GnuSparse1_0MapDecimalStringTooLong => "gnu_sparse.1.0.map.number_of_maps",
      Self::GnuSparse1_0MapOffsetEntryDecimalStringTooLong => "gnu_sparse.1.0.map_entry.offset",
      Self::GnuSparse1_0MapSizeEntryDecimalStringTooLong => "gnu_sparse.1.0.map_entry.size",
      Self::TooManySparseFileInstructions => "sparse_file_instructions",
      Self::PaxLengthFieldDecimalStringTooLong => "pax.length_field",
      Self::PaxKvKeyTooLong => "pax.key_field",
      Self::PaxKvValueTooLong => "pax.value_field",
      Self::PaxTooManyUnparsedGlobalAttributes => "pax.unparsed_global_attributes",
      Self::PaxTooManyUnparsedLocalAttributes => "pax.unparsed_local_attributes",
      Self::PaxTooManyGlobalAttributes => "pax.global_attributes",
    }
  }
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum GeneralTryReserveError {
  #[error("Alloc allocation error: {0}")]
  AllocTryReserveError(#[from] alloc::collections::TryReserveError),
  #[error("HashBrown allocation error: {0:?}")]
  HashBrownTryReserveError(hashbrown::TryReserveError),
}

impl ::core::convert::From<hashbrown::TryReserveError> for GeneralTryReserveError {
  fn from(source: hashbrown::TryReserveError) -> Self {
    GeneralTryReserveError::HashBrownTryReserveError { 0: source }
  }
}

// Equivalent to a bool but allows searching for errors more easily.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorSeverity {
  Fatal,
  Recoverable,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub struct TarParserError {
  pub kind: TarParserErrorKind,
  pub severity: ErrorSeverity,
}

impl TarParserError {
  pub(crate) fn new<EK: Into<TarParserErrorKind>>(kind: EK, severity: ErrorSeverity) -> Self {
    Self {
      kind: kind.into(),
      severity,
    }
  }

  pub fn is_fatal(&self) -> bool {
    self.severity == ErrorSeverity::Fatal
  }
}

impl Display for TarParserError {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    match self.severity {
      ErrorSeverity::Fatal => write!(f, "Fatal Tar parser error: {}", self.kind),
      ErrorSeverity::Recoverable => write!(f, "Recoverable Tar parser error: {}", self.kind),
    }
  }
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TarParserErrorKind {
  #[error("Tar header parser error: {0}")]
  HeaderParserError(#[from] TarHeaderParserError),
  #[error("PAX parser error: {0}")]
  PaxParserError(#[from] PaxParserError),
  #[error("Limit of {limit} {unit} exceeded: {context}", unit = context.context_unit().1, context = context.context_unit().0)]
  LimitExceeded {
    limit: usize,
    context: LimitExceededContext,
  },
  #[error("Allocation error: {try_reserve_error} while parsing: {context}", context = context.context_str())]
  TryReserveError {
    try_reserve_error: GeneralTryReserveError,
    context: LimitExceededContext,
  },
  #[error("Parsing field {field} failed: {error}")]
  CorruptField {
    field: CorruptFieldContext,
    error: GeneralParseError,
  },
}

#[must_use]
pub(crate) fn corrupt_field_to_tar_err<'a, T: Into<GeneralParseError>>(
  field: CorruptFieldContext,
) -> impl FnOnce(T) -> TarParserErrorKind + 'a {
  move |error| {
    let error_kind = TarParserErrorKind::CorruptField {
      field,
      error: error.into(),
    };
    error_kind
  }
}

#[must_use]
pub(crate) fn limit_exceeded_to_tar_err<'a, TryReserveError>(
  limit: usize,
  context: LimitExceededContext,
) -> impl FnOnce(LimitedBackingBufferError<TryReserveError>) -> TarParserErrorKind + 'a
where
  GeneralTryReserveError: From<TryReserveError>,
{
  move |error| {
    let error_kind = match error {
      LimitedBackingBufferError::MemoryLimitExceeded(_bytes_size) => {
        TarParserErrorKind::LimitExceeded { limit, context }
      },
      LimitedBackingBufferError::ResizeError(alloc_error) => TarParserErrorKind::TryReserveError {
        try_reserve_error: alloc_error.into(),
        context,
      },
    };
    error_kind
  }
}
