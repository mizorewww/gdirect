#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
root="$PWD"
version=1.37.0
sha=60a420ad7085eb616cb6e2bdf0a7206d68ff3d37fb5a956dc44242eb2f79b66b
prefix="$root/target/libaria2"
mkdir -p target/vendor
archive="$root/target/vendor/aria2-$version.tar.xz"
# GitHub's release downloads go down now and then; Fedora's source archive
# keeps the same tarball. Either way the checksum below decides.
sha512=df3b8b4de8fa8d78f203ea00c059e43585e18a229009f202e42e6a9e59db67d09df0dbba8a016e99ed73c82f59e4f8b26f86c2288afdbb96a6807cbe2c56e6b3
if [ ! -f "$archive" ]; then
  for url in \
    "https://github.com/aria2/aria2/releases/download/release-$version/aria2-$version.tar.xz" \
    "https://src.fedoraproject.org/repo/pkgs/aria2/aria2-$version.tar.xz/sha512/$sha512/aria2-$version.tar.xz"; do
    if curl --fail --location --retry 3 "$url" -o "$archive.part"; then
      mv "$archive.part" "$archive"
      break
    fi
  done
  [ -f "$archive" ] || { echo "cannot download aria2 $version" >&2; exit 1; }
fi
if command -v sha256sum >/dev/null; then
  printf '%s  %s\n' "$sha" "$archive" | sha256sum -c -
else
  printf '%s  %s\n' "$sha" "$archive" | shasum -a 256 -c -
fi
rm -rf "target/vendor/aria2-$version"
tar -xf "$archive" -C target/vendor
cd "target/vendor/aria2-$version"
patch -p1 < "$root/vendor/aria2-$version.patch"
cp "$root/vendor/gdirect_route.h" src/
if [ "$(uname -s)" = Darwin ]; then
  openssl_prefix="${OPENSSL_PREFIX:-$(brew --prefix openssl@3)}"
  export OPENSSL_PREFIX="$openssl_prefix"
fi
if [ -n "${OPENSSL_PREFIX:-}" ]; then
  export OPENSSL_CFLAGS="-I$OPENSSL_PREFIX/include"
  export OPENSSL_LIBS="-L$OPENSSL_PREFIX/lib -lssl -lcrypto"
fi
# No iconv dependency is needed by this HTTP-only build.
export am_cv_func_iconv=no
export ac_cv_header_iconv_h=no
./configure --prefix="$prefix" --enable-libaria2 --enable-static --disable-shared \
  --with-openssl --without-wintls --without-appletls --without-gnutls --without-libnettle \
  --without-libgmp --without-libgcrypt --without-libcares --without-libssh2 \
  --without-sqlite3 --without-libxml2 --without-libexpat --without-libz \
  --disable-bittorrent --disable-metalink --disable-websocket --disable-nls \
  --disable-dependency-tracking
make -j "${JOBS:-4}" -C src libaria2.la
make -C src install-libLTLIBRARIES
mkdir -p "$prefix/include/aria2"
cp src/includes/aria2/aria2.h "$prefix/include/aria2/"
printf '\nBuilt patched libaria2 %s in %s\n' "$version" "$prefix"
