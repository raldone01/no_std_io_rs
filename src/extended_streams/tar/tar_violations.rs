use alloc::vec::Vec;

use crate::extended_streams::tar::TarParserError;

pub trait TarViolationHandler {
  /// When a violation occurs, this method is called.
  /// It should return `true` if parsing should continue,
  /// or `false` if parsing should stop.
  ///
  /// Sometimes, the parsing may stop even if the handler returns `true`,
  /// since some errors are unrecoverable.
  ///
  /// The parser implementation also avoid creating intermediate buffer just for error recovery.
  #[must_use]
  fn handle(&mut self, error: &TarParserError) -> bool;
}

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
