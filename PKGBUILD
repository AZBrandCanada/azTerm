# Maintainer: AZBrandCanada Developers <https://github.com/AZBrandCanada/azTerm>
pkgname=azterm-bin
pkgver=0.0.0
pkgrel=1
pkgdesc="Fast, GPU-accelerated terminal emulator, SSH bookmark manager, and SFTP client"
arch=('x86_64')
url="https://github.com/AZBrandCanada/azTerm"
license=('MIT' 'Apache-2.0')
depends=('libxkbcommon' 'libxkbcommon-x11' 'openssl' 'libxcb' 'libx11' 'mesa' 'wayland')
provides=('azterm')
conflicts=('azterm')
source=("$pkgname-$pkgver.tar.gz::$url/releases/download/v$pkgver/azterm-linux-x86_64.tar.gz")
sha256sums=('SKIP')

package() {
    install -Dm755 "$srcdir/azterm" "$pkgdir/usr/bin/azterm"
    install -Dm644 "$srcdir/azterm.desktop" "$pkgdir/usr/share/applications/azterm.desktop"
    install -Dm644 "$srcdir/azterm.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/azterm.svg"
}