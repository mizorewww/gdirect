# Building from source

[中文](building.zh-CN.md) · [README](../README.md)

Requires Rust 1.91+, a C++ compiler, make, curl and OpenSSL 3.

```sh
git clone https://github.com/mizorewww/gdirect.git
cd gdirect
# macOS: brew install openssl@3
# Debian / Ubuntu: sudo apt-get install build-essential pkg-config libssl-dev ca-certificates curl xz-utils
./scripts/build-libaria2.sh
cargo install --path . --locked
```

Windows builds use the MinGW GNU toolchain. In an MSYS2 MINGW64 shell, install `make patch tar xz mingw-w64-x86_64-gcc mingw-w64-x86_64-openssl mingw-w64-x86_64-pkgconf`, run `rustup target add x86_64-pc-windows-gnu`, set `OPENSSL_PREFIX=$(cygpath -m /mingw64)`, then run the two commands above with `--target x86_64-pc-windows-gnu` added to `cargo install`.

## Desktop client

`desktop/` holds a Material 3 desktop client built on [gpui-kit-material](https://github.com/mizorewww/gpui-kit-material): files, transfers, endpoints and settings in one window, on the same engine and SQLite state as the command line. It is its own Cargo workspace, so the command-line build never compiles the GUI. Its dependencies, gpui-kit-material included, come from crates.io; it needs Rust 1.98+:

```sh
./scripts/build-libaria2.sh
cargo run --release --manifest-path desktop/Cargo.toml
```

`python3 scripts/package-desktop.py --target aarch64-apple-darwin` turns a release build (`cargo build --release --target aarch64-apple-darwin` in `desktop/`) into the macOS app bundle the releases ship.

`--db PATH` uses another state file, as the command line's `--db` does.
