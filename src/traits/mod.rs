mod backing_buffer;
mod buffered_read;
mod copy;
mod read;
mod read_all;
mod seek;
mod write;
mod write_all;

pub use backing_buffer::*;
pub use buffered_read::*;
pub use copy::*;
pub use read::*;
pub use read_all::*;
pub use seek::*;
pub use write::*;
pub use write_all::*;

pub(crate) fn advance<T: AsRef<[u8]> + ?Sized>(slice: &mut T, n: usize) {
  let slice_ref = &mut slice.as_ref();
  *slice_ref = &core::mem::take(slice_ref)[n..];
}
