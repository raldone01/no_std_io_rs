use alloc::vec::Vec;

use crate::extended_streams::tar::{ErrorSeverity, TarParserError, TarParserErrorKind};

pub trait TarViolationHandler {
  /// When a violation occurs, this method is called.
  /// It should return `true` if parsing should ignore the error and continue parsing - discarding the corruption/specification violation.
  ///
  /// If `is_fatal` is `true`, the parser will ignore the return value of this function.
  ///
  /// Note: Some errors are marked as fatal that seem recoverable because the parser implementation avoids creating intermediate buffer just for error recovery.
  #[must_use]
  fn handle(&mut self, error: &TarParserError) -> bool;
}

#[derive(Debug, Default)]
pub struct StrictTarViolationHandler;

impl TarViolationHandler for StrictTarViolationHandler {
  fn handle(&mut self, _error: &TarParserError) -> bool {
    false
  }
}

#[derive(Debug, Default)]
pub struct AuditTarViolationHandler {
  pub violations: Vec<TarParserError>,
}

impl AuditTarViolationHandler {
  #[must_use]
  pub fn new() -> Self {
    Self {
      violations: Vec::new(),
    }
  }
}

impl TarViolationHandler for AuditTarViolationHandler {
  fn handle(&mut self, error: &TarParserError) -> bool {
    self.violations.push(error.clone());
    true
  }
}

#[derive(Debug, Default)]
pub struct IgnoreTarViolationHandler;

impl TarViolationHandler for IgnoreTarViolationHandler {
  fn handle(&mut self, _error: &TarParserError) -> bool {
    true
  }
}

/// A wrapper around a `TarViolationHandler` that provides convenience methods for handling violations.
pub(crate) struct VHW<'a, VH: TarViolationHandler>(pub(crate) &'a mut VH);

impl<VH: TarViolationHandler> VHW<'_, VH> {
  /// Handles a potential violation in result form by calling the violation handler.
  pub(crate) fn hpvr<T, E: Into<TarParserErrorKind>>(
    &mut self,
    operation_result: Result<T, E>,
  ) -> Result<Option<T>, TarParserError> {
    match operation_result {
      Ok(v) => Ok(Some(v)),
      Err(e) => {
        let e = TarParserError::new(e.into(), ErrorSeverity::Recoverable);
        if self.0.handle(&e) {
          Ok(None)
        } else {
          Err(e)
        }
      },
    }
  }

  /// Handles a potential violation in error form by calling the violation handler.
  pub(crate) fn hpve<E: Into<TarParserErrorKind>>(
    &mut self,
    error: E,
  ) -> Result<(), TarParserError> {
    let e = TarParserError::new(error.into(), ErrorSeverity::Recoverable);
    if self.0.handle(&e) {
      Ok(())
    } else {
      Err(e)
    }
  }

  /// Handles a fatal violation in result form by calling the violation handler.
  pub(crate) fn hfvr<T, E: Into<TarParserErrorKind>>(
    &mut self,
    operation_result: Result<T, E>,
  ) -> Result<T, TarParserError> {
    match operation_result {
      Ok(v) => Ok(v),
      Err(e) => {
        let e = TarParserError::new(e.into(), ErrorSeverity::Recoverable);
        let _fatal_error = self.0.handle(&e);
        Err(e)
      },
    }
  }

  /// Handles a fatal violation in error form by calling the violation handler.
  pub(crate) fn hfve<T, E: Into<TarParserErrorKind>>(
    &mut self,
    error: E,
  ) -> Result<T, TarParserError> {
    let e = TarParserError::new(error.into(), ErrorSeverity::Recoverable);
    let _fatal_error = self.0.handle(&e);
    Err(e)
  }
}
