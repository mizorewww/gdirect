# Maintainer: mizorewww <mizorewww@users.noreply.github.com>

pkgname=gdirect-bin
pkgver=0.13.0
pkgrel=1
pkgdesc="Browse, download, upload and mount Google Drive"
arch=('x86_64')
url="https://github.com/mizorewww/gdirect"
license=('GPL-2.0-or-later')
depends=(
  'glibc'
  'gcc-libs'
)
optdepends=(
  'rclone: for mounting Google Drive'
  'fuse3: for mounting Google Drive'
)
provides=('gdirect')
conflicts=('gdirect')
source_x86_64=("${pkgname}-${pkgver}.tar.gz::https://github.com/mizorewww/gdirect/releases/download/v${pkgver}/gdirect-${pkgver}-${CARCH}-unknown-linux-gnu.tar.gz")
sha256sums_x86_64=('981c7ddf2c884e03e5abf5f42f9d4cfe4bea3943c1fa89401b32977856cd126d')

package() {
  cd "${srcdir}/gdirect-${pkgver}-${CARCH}-unknown-linux-gnu"

  install -Dm755 gdirect "${pkgdir}/usr/bin/gdirect"
  install -Dm644 README.md "${pkgdir}/usr/share/doc/gdirect/README.md"
  install -Dm644 README.zh-CN.md "${pkgdir}/usr/share/doc/gdirect/README.zh-CN.md"
  if [ -d docs ]; then
    find docs -type f -exec install -Dm644 "{}" "${pkgdir}/usr/share/doc/gdirect/{}" \;
  fi
  install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
  install -Dm644 OPENSSL-LICENSE.txt "${pkgdir}/usr/share/licenses/${pkgname}/OPENSSL-LICENSE.txt"
}
