# Maintainer: neo-ncmp contributors
pkgname=neo-ncmp
pkgver=0.1.0
pkgrel=1
pkgdesc="A modern, feature-rich terminal client for MPD"
arch=('x86_64' 'aarch64')
url="https://github.com/eval-exec/neo-ncmp"
license=('MIT')
depends=('glibc')
makedepends=('cargo' 'rust')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 target/release/neo-ncmp "$pkgdir/usr/bin/neo-ncmp"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 docs/neo-ncmp.1 "$pkgdir/usr/share/man/man1/neo-ncmp.1"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 config.example.toml \
    "$pkgdir/usr/share/doc/$pkgname/config.example.toml"
}
