use alloc::vec::Vec;

use crate::extended_streams::tar::TarParserError;

pub trait TarViolationHandler {
  fn handle(&mut self, error: TarParserError) -> Result<(), TarParserError>;
}

pub struct StrictTarViolationHandler;

impl TarViolationHandler for StrictTarViolationHandler {
  fn handle(&mut self, error: TarParserError) -> Result<(), TarParserError> {
    Err(error)
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
  fn handle(&mut self, error: TarParserError) -> Result<(), TarParserError> {
    self.violations.push(error);
    Ok(())
  }
}

#[derive(Debug, Default)]
pub struct IgnoreTarViolationHandler;

impl TarViolationHandler for IgnoreTarViolationHandler {
  fn handle(&mut self, _error: TarParserError) -> Result<(), TarParserError> {
    Ok(())
  }
}
