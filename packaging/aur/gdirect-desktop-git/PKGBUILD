# Maintainer: mizorewww <mizorewww@users.noreply.github.com>

pkgname=gdirect-desktop-git
pkgver=0.12.1.r19.gd9ea2b4
pkgrel=1
pkgdesc="Desktop client for gdirect: browse, transfer and mount Google Drive"
arch=('x86_64')
url="https://github.com/mizorewww/gdirect"
license=('GPL-2.0-or-later' 'OFL-1.1')
options=(!lto)
depends=(
  'glibc'
  'gcc-libs'
  'libxkbcommon'
  'libxkbcommon-x11'
  'libxcb'
  'vulkan-icd-loader'
  'fontconfig'
  'freetype2'
  'hicolor-icon-theme'
)
optdepends=(
  'rclone: for mounting Google Drive'
  'fuse3: for mounting Google Drive'
)
makedepends=(
  'cargo'
  'git'
  'pkgconf'
  'openssl'
  'wayland'
  'vulkan-headers'
  'libx11'
)
provides=('gdirect-desktop')
conflicts=('gdirect-desktop')
source=(
  "gdirect::git+https://github.com/mizorewww/gdirect.git"
  "gpui-kit-material::git+https://github.com/mizorewww/gpui-kit-material.git"
  "aria2-1.37.0.tar.xz::https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0.tar.xz"
)
sha256sums=(
  'SKIP'
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

  local kit_rev
  kit_rev="$(tr -d '[:space:]' < desktop/gpui-kit-material.rev)"
  git -C "${srcdir}/gpui-kit-material" checkout --force "${kit_rev}"

  cd desktop
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "${srcdir}/gdirect"
  export RUSTUP_TOOLCHAIN=stable
  ./scripts/build-libaria2.sh

  cd desktop
  cargo build --release --locked
}

package() {
  cd "${srcdir}/gdirect"
  install -Dm755 desktop/target/release/gdirect-desktop "${pkgdir}/usr/bin/gdirect-desktop"
  install -Dm644 desktop/packaging/gdirect.desktop "${pkgdir}/usr/share/applications/gdirect.desktop"
  install -Dm644 desktop/assets/gdirect.png "${pkgdir}/usr/share/icons/hicolor/512x512/apps/gdirect.png"
  install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
  install -Dm644 desktop/assets/fonts/OFL.txt "${pkgdir}/usr/share/licenses/${pkgname}/FONTS-OFL.txt"
}
