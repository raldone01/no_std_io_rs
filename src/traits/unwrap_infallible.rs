use core::convert::Infallible;

/// Once https://github.com/rust-lang/rust/issues/61695 becomes stable, this trait will be removed without replacement in a major release.
pub trait UnwrapInfallible {
  type Unwrapped;
  fn unwrap_infallible(self) -> Self::Unwrapped;
}

impl<T> UnwrapInfallible for Result<T, Infallible> {
  type Unwrapped = T;

  #[inline]
  fn unwrap_infallible(self) -> Self::Unwrapped {
    match self {
      Ok(value) => value,
    }
  }
}
