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

# TODO fever dreams:

* Add `alloc` feature to enable `no_std` only mode. (Fever dream)
* gnu `sparse` files
