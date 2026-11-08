# Maintainer: mizorewww <mizorewww@users.noreply.github.com>

pkgname=gdirect-desktop-bin
pkgver=0.12.1
pkgrel=1
pkgdesc="Desktop client for gdirect: browse, transfer and mount Google Drive"
arch=('x86_64')
url="https://github.com/mizorewww/gdirect"
license=('GPL-2.0-or-later' 'OFL-1.1')
depends=(
  'glibc'
  'gcc-libs'
  'libxkbcommon'
  'libxkbcommon-x11'
  'libxcb'
  'vulkan-icd-loader'
  'hicolor-icon-theme'
)
optdepends=(
  'rclone: for mounting Google Drive'
  'fuse3: for mounting Google Drive'
)
provides=('gdirect-desktop')
conflicts=('gdirect-desktop')
source_x86_64=("${pkgname}-${pkgver}.tar.gz::https://github.com/mizorewww/gdirect/releases/download/v${pkgver}/gdirect-desktop-linux-x64.tar.gz")
sha256sums_x86_64=('6314dbb92ad8b47107058692167e6bda32e2c79ea9b9b4b15f51c15dd4585182')

package() {
  cd "${srcdir}/gdirect-desktop"

  install -Dm755 gdirect-desktop "${pkgdir}/usr/bin/gdirect-desktop"
  install -Dm644 gdirect.desktop "${pkgdir}/usr/share/applications/gdirect.desktop"
  install -Dm644 gdirect.png "${pkgdir}/usr/share/icons/hicolor/512x512/apps/gdirect.png"

  install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
  install -Dm644 FONTS-OFL.txt "${pkgdir}/usr/share/licenses/${pkgname}/FONTS-OFL.txt"
  install -Dm644 OPENSSL-LICENSE.txt "${pkgdir}/usr/share/licenses/${pkgname}/OPENSSL-LICENSE.txt"
}
