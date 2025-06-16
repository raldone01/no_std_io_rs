# bidir-tar-gz-rs

Features of this crate:
* Create uncompressed and compressed tarballs. (`ustar` only)
* Extract files + limited metadata from uncompressed and compressed tarballs. (`ustar`, `v7`, `pax` and `gnu`)
* Fully `no_std + alloc` compatible.
* Reimplementation of a subset of `std::io` functionality.
* A usable streaming frontend API for the `miniz_oxide` crate.

# TODO crate:

* Add `std` feature to enable `std::io` compatibility.
* After maturing enough move `no_std_io` to its own crate.
* Add feature to opt in to time dependency.
* Fix clippy lints
* Add `alloc` feature

# TODO no_std_io:

* Add `emscripten` feature to enable `no_std_io` compatibility.
* Add buffered reader trait
* Buffered reader extension into iter bytes
* Add BufRead::consume (avoids buffer copy)
* Add BufRead::fill_buf -> Result<&[u8], Error> returns internal buffer, filling it with more data via Read if empty
* Compressed Reader and buffered reader and forked reader into buffered reader trait
* Proper pipe https://doc.rust-lang.org/std/io/fn.pipe.html
* Add chain read extension and ChainedReader (impl buffered reader if both readers are buffered readers)
* `[copy_to]` or `[copy_from]` or `[copy_buffered]`
* Add lines, split extension to readbuffered
* Add std::io take extension trait that creates ReaderLimited and WriterLimited
* Add seek trait https://doc.rust-lang.org/std/io/trait.Seek.html
* BorrowedCursor: Read + Seek + BufRead where AsRef<[u8]>, Write where AsMut<[u8]> https://doc.rust-lang.org/std/io/struct.BorrowedCursor.html replaces reader_slice and writer_slice
* Cursor: Read + Seek + BufRead + Write (has inner_buffer and pos) https://doc.rust-lang.org/std/io/struct.Cursor.html replaces writer_buffer

# TODO tar:
* Make tar creator, extractor, gz compressor, gz decompressor into Read/Write.
* gen tar
* gnu `sparse` files

# TODO (long term):
* Add filesystem trait and filesystem agnostic file api. (Default ship a memory based implementation serialize to tar_gz and extract from tar_gz.)
