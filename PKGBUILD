# Maintainer: ascolta contributors
pkgname=ascolta
pkgver=0.0.1
pkgrel=1
pkgdesc="A modern, feature-rich terminal client for MPD"
arch=('x86_64' 'aarch64')
url="https://github.com/eval-exec/ascolta"
license=('MIT')
depends=('glibc')
makedepends=('cargo' 'rust')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo build --release --locked
  ./target/release/ascolta completions bash > ascolta.bash
  ./target/release/ascolta completions zsh > _ascolta
  ./target/release/ascolta completions fish > ascolta.fish
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 target/release/ascolta "$pkgdir/usr/bin/ascolta"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 docs/ascolta.1 "$pkgdir/usr/share/man/man1/ascolta.1"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 config.example.toml \
    "$pkgdir/usr/share/doc/$pkgname/config.example.toml"
  install -Dm644 ascolta.bash \
    "$pkgdir/usr/share/bash-completion/completions/ascolta"
  install -Dm644 _ascolta "$pkgdir/usr/share/zsh/site-functions/_ascolta"
  install -Dm644 ascolta.fish \
    "$pkgdir/usr/share/fish/vendor_completions.d/ascolta.fish"
}
