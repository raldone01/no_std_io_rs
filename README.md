# no_std_io

Features of this crate:
* A `no_std + alloc` optimized reimplementation of the streaming infrastructure of `std::io`.
* Custom `Read` and `Write` traits that support user defined error types.

## Extended streams

### CompressedReader and CompressedWriter

- Fully `no_std + alloc` compatible.
- A usable streaming frontend API for the `miniz_oxide` crate.
- Supports concatenated gzip, zlib and raw deflate streams.
- Supports auto detection of gzip, zlib, raw deflate, uncompressed streams.

### Tar Parser

- Fully `no_std + alloc` compatible.
- A streaming implementation that implements the `Write` trait.
- Supports all common tar formats: `ustar`, `v7`, `pax`, and `gnu`.
- It is very forgiving and strives to limit panics and resource exhaustion attacks.
- Most commonly used metadata is preserved.

### Tar Creator

- Creates tarballs using the `pax` format.
- Writing the gnu sparse `1.0` format is also supported.

# TODO crate:

* Add `std` feature to enable `std::io` compatibility.
* Add more impls for Rc Arc and Mutex for std feature for Read and Write traits.
* Add parking lot feature to enable more impls for Read and Write traits.
* After maturing enough move `no_std_io` to its own crate.
* Add feature to opt in to time dependency.
* Fix clippy lints
* Add `alloc` feature
* Add doc comments with examples to all public functions and structs.
* Add audit log to tar parser to track spec violations
* Write a tar fuzzer

# TODO no_std_io:

* Proper pipe https://doc.rust-lang.org/std/io/fn.pipe.html
* Add chain read extension and ChainedReader (impl buffered reader if both readers are buffered readers)
* `[copy_to]` or `[copy_from]` or `[copy_buffered]` (picks the bigger buffer)
* Add lines, split extension to readbuffered
* Add std::io take extension trait that creates ReaderLimited and WriterLimited
* Add seek trait https://doc.rust-lang.org/std/io/trait.Seek.html and implement it
* Cursor: Read + Seek + BufRead + Write (has inner_buffer and pos) https://doc.rust-lang.org/std/io/struct.Cursor.html replaces writer_buffer
* Add `emscripten` feature to enable `no_std_io` compatibility.

# TODO tar:
* Make tar creator, extractor, gz compressor, gz decompressor into Read/Write.
* gen tar
* gnu `sparse` files
* Cleanup the pax parser and how it uses the cursor. to do that add write_until and read_until
* https://www.gnu.org/software/tar/manual/html_section/Dumpdir.html#Dumpdir
* Configure limits for all growable buffers in the tar parser.

# TODO (long term):
* Add filesystem trait and filesystem agnostic file api. (Default ship a memory based implementation serialize to tar_gz and extract from tar_gz.)
