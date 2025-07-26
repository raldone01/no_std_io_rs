use alloc::vec::Vec;

use crate::extended_streams::tar::TarParserError;

pub trait TarViolationHandler {
  /// When a violation occurs, this method is called.
  /// It should return `true` if parsing should ignore the error and continue parsing - discarding the corruption/specification violation.
  ///
  /// If `is_fatal` is `true`, the parser will ignore the return value of this function.
  ///
  /// Note: Some errors are marked as fatal that seem recoverable because the parser implementation avoids creating intermediate buffer just for error recovery.
  #[must_use]
  fn handle(&mut self, error: &TarParserError, is_fatal: bool) -> bool;
}

#[derive(Debug, Default)]
pub struct StrictTarViolationHandler;

impl TarViolationHandler for StrictTarViolationHandler {
  fn handle(&mut self, _error: &TarParserError, _is_fatal: bool) -> bool {
    false
  }
}

#[derive(Debug, Default)]
pub struct AuditTarViolationHandler {
  pub violations: Vec<(TarParserError, bool)>,
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
  fn handle(&mut self, error: &TarParserError, fatal_error: bool) -> bool {
    self.violations.push((error.clone(), fatal_error));
    true
  }
}

#[derive(Debug, Default)]
pub struct IgnoreTarViolationHandler;

impl TarViolationHandler for IgnoreTarViolationHandler {
  fn handle(&mut self, _error: &TarParserError, _fatal_error: bool) -> bool {
    true
  }
}

/// A wrapper around a `TarViolationHandler` that provides convenience methods for handling violations.
pub(crate) struct VHW<'a, VH: TarViolationHandler>(pub(crate) &'a mut VH);

impl<VH: TarViolationHandler> VHW<'_, VH> {
  /// Handles a potential violation in result form by calling the violation handler.
  pub(crate) fn hpvr<T, E: Into<TarParserError>>(
    &mut self,
    operation_result: Result<T, E>,
  ) -> Result<Option<T>, TarParserError> {
    match operation_result {
      Ok(v) => Ok(Some(v)),
      Err(e) => {
        let e = e.into();
        if self.0.handle(&e, false) {
          Ok(None)
        } else {
          Err(e)
        }
      },
    }
  }

  /// Handles a potential violation in error form by calling the violation handler.
  pub(crate) fn hpve<E: Into<TarParserError>>(&mut self, error: E) -> Result<(), TarParserError> {
    let e = error.into();
    if self.0.handle(&e, false) {
      Ok(())
    } else {
      Err(e)
    }
  }

  /// Handles a fatal violation in result form by calling the violation handler.
  pub(crate) fn hfvr<T, E: Into<TarParserError>>(
    &mut self,
    operation_result: Result<T, E>,
  ) -> Result<T, TarParserError> {
    match operation_result {
      Ok(v) => Ok(v),
      Err(e) => {
        let e = e.into();
        let _fatal_error = self.0.handle(&e, true);
        Err(e)
      },
    }
  }

  /// Handles a fatal violation in error form by calling the violation handler.
  pub(crate) fn hfve<E: Into<TarParserError>>(&mut self, error: E) -> TarParserError {
    let e = error.into();
    let _fatal_error = self.0.handle(&e, true);
    e
  }
}
