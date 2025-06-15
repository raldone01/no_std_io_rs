# bidir-tar-gz-rs

Features of this crate:
* Create uncompressed and compressed tarballs. (`ustar` only)
* Extract files + limited metadata from uncompressed and compressed tarballs. (`ustar`, `v7`, `pax` and `gnu`)
* Fully `no_std + alloc` compatible.
* Reimplementation of a subset of `std::io` functionality.
* A usable streaming frontend API for the `miniz_oxide` crate.

# TODO:

* Add `std` feature to enable `std::io` compatibility.
* `pax`
* `gnu`
* gen `ustar` tar
* Add feature to opt in to time dependency.
* Fix clippy lints
* Add `emscripten` feature to enable `no_std_io` compatibility.
* gnu `sparse` files

# TODO (long term):
* After maturing enough consider moving `no_std_io` to its own crate.
* Add `alloc` feature only makes sense once `no_std_io` is split off into its own crate.
* Add filesystem trait and filesystem agnostic file api. (Default ship a memory based implementation serialize to tar_gz and extract from tar_gz.)

# TODO fever dreams:

* Add `alloc` feature to enable `no_std` only mode. (Fever dream)
