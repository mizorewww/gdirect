# 从源码编译

[English](building.md) · [README](../README.zh-CN.md)

需要 Rust 1.91+、C++ 编译器、make、curl 和 OpenSSL 3。

```sh
git clone https://github.com/mizorewww/gdirect.git
cd gdirect
# macOS：brew install openssl@3
# Debian / Ubuntu：sudo apt-get install build-essential pkg-config libssl-dev ca-certificates curl xz-utils
./scripts/build-libaria2.sh
cargo install --path . --locked
```

Windows 需要使用 MinGW GNU 工具链编译，步骤如下：

1. 在 MSYS2 的 MINGW64 终端中安装依赖：`make patch tar xz mingw-w64-x86_64-gcc mingw-w64-x86_64-openssl mingw-w64-x86_64-pkgconf`。
2. 执行 `rustup target add x86_64-pc-windows-gnu`。
3. 设置环境变量 `OPENSSL_PREFIX=$(cygpath -m /mingw64)`。
4. 执行上面的两条构建命令，并给 `cargo install` 加上 `--target x86_64-pc-windows-gnu`。

## 桌面客户端

`desktop/` 是基于 [gpui-kit-material](https://github.com/mizorewww/gpui-kit-material) 的 Material 3 桌面客户端：文件、传输、线路和设置都在一个窗口里，和命令行共用同一套引擎与 SQLite 数据。它是独立的 Cargo 工作区，编译命令行时不会编译图形界面。在 gpui-kit-material 发布到 crates.io 之前，需要把它克隆到本仓库旁边，并使用 Rust 1.98+：

```sh
git clone https://github.com/mizorewww/gpui-kit-material.git ../gpui-kit-material
git -C ../gpui-kit-material checkout "$(cat desktop/gpui-kit-material.rev)"
./scripts/build-libaria2.sh
cargo run --release --manifest-path desktop/Cargo.toml
```

`desktop/gpui-kit-material.rev` 记录了本客户端编译和发布所对应的 gpui-kit-material 提交。在 `desktop/` 里执行 `cargo build --release --target aarch64-apple-darwin` 后，`python3 scripts/package-desktop.py --target aarch64-apple-darwin` 会把它打包成发布用的 macOS 应用。

`--db PATH` 使用另一个数据文件，和命令行的 `--db` 一样。

gpui-kit-material 正在开发中，并在把引擎补丁逐步迁移到上游。`desktop/Cargo.toml` 把同一批 crate 指向它 `vendor/` 下的副本，所以更新 gpui-kit-material 后如果编译失败，把 `desktop/Cargo.toml` 的 `[patch]` 改成和它 `Cargo.toml` 里的一致即可。
