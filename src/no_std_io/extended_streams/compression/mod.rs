// TODO: add concatenated gzip stream support
// TODO: add concatenated zlib stream support
// TODO: add concatenated raw deflate stream support

mod reader_compressed;
mod writer_compressed;

pub use reader_compressed::*;
pub use writer_compressed::*;
