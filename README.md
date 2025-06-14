# bidir-tar-gz-rs

Features of this crate:
* Create uncompressed and compressed tarballs. (`ustar` only)
* Extract uncompressed and compressed tarballs. (`ustar`, `v7`, `pax` and `gnu` formats)
* Fully `no_std + alloc` compatible.
* Reimplementation of `io::Writer` and `io::Read` traits.
* A usable frontend API for the `miniz_oxide` crate.
