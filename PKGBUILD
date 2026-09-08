# Maintainer: AZBrandCanada Developers
pkgname=azterm
pkgver=0.1.4
pkgrel=1
pkgdesc="Fast, modern native terminal, SSH bookmark manager, and SFTP client"
arch=('x86_64')
url="https://github.com/AZBrandCanada/azTerm"
license=('MIT' 'Apache-2.0')
depends=('libxkbcommon' 'openssl' 'libxcb' 'libx11' 'mesa' 'wayland')
makedepends=('rust' 'cargo')
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
    cargo build --release --locked
}

package() {
    install -Dm755 "target/release/azterm" "$pkgdir/usr/bin/azterm"
    install -Dm644 "assets/azterm.desktop" "$pkgdir/usr/share/applications/azterm.desktop"
    install -Dm644 "assets/azterm.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/azterm.svg"
}
