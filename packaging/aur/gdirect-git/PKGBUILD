# Maintainer: mizorewww <mizorewww@users.noreply.github.com>

pkgname=gdirect-git
pkgver=0.12.1.r19.gd9ea2b4
pkgrel=1
pkgdesc="Browse, download, upload and mount Google Drive"
arch=('x86_64')
url="https://github.com/mizorewww/gdirect"
license=('GPL-2.0-or-later')
options=(!lto)
depends=(
  'glibc'
  'gcc-libs'
  'openssl'
)
optdepends=(
  'rclone: for mounting Google Drive'
  'fuse3: for mounting Google Drive'
)
makedepends=(
  'cargo'
  'git'
)
provides=('gdirect')
conflicts=('gdirect')
source=(
  "git+https://github.com/mizorewww/gdirect.git"
  "aria2-1.37.0.tar.xz::https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0.tar.xz"
)
sha256sums=(
  'SKIP'
  '60a420ad7085eb616cb6e2bdf0a7206d68ff3d37fb5a956dc44242eb2f79b66b'
)

pkgver() {
  cd "${srcdir}/gdirect"
  git describe --long --tags 2>/dev/null | sed -E 's/^v//;s/([^-]*-g)/r\1/;s/-/./g' ||
    printf "r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
}

prepare() {
  cd "${srcdir}/gdirect"
  mkdir -p target/vendor
  cp "${srcdir}/aria2-1.37.0.tar.xz" target/vendor/
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "${srcdir}/gdirect"
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  ./scripts/build-libaria2.sh
  cargo build --release --frozen
}

check() {
  cd "${srcdir}/gdirect"
  export RUSTUP_TOOLCHAIN=stable
  cargo test --frozen
}

package() {
  cd "${srcdir}/gdirect"
  install -Dm755 "target/release/gdirect" "${pkgdir}/usr/bin/gdirect"
  install -Dm644 README.md "${pkgdir}/usr/share/doc/gdirect/README.md"
  install -Dm644 README.zh-CN.md "${pkgdir}/usr/share/doc/gdirect/README.zh-CN.md"
  if [ -d docs ]; then
    find docs -type f -exec install -Dm644 "{}" "${pkgdir}/usr/share/doc/gdirect/{}" \;
  fi
  install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
