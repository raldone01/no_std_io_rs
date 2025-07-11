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
