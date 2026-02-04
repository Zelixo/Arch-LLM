# Maintainer: Your Name <youremail@example.com>
pkgname=arch-llm
pkgver=0.1.0
pkgrel=1
pkgdesc="Native Arch Linux app for interacting with Ollama"
arch=('x86_64')
url="https://github.com/yourusername/arch-llm"
license=('MIT')
depends=('gtk4' 'gcc-libs' 'glibc')
makedepends=('rust' 'cargo' 'pkgconf')
source=("$pkgname-$pkgver.tar.gz::https://github.com/yourusername/$pkgname/archive/v$pkgver.tar.gz")
# For local building:
# source=("$pkgname::git+file://$PWD")

build() {
  cd "$pkgname-$pkgver"
  cargo build --release --locked
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 "target/release/Arch-LLM" "$pkgdir/usr/bin/arch-llm"
}
