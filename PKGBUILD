pkgname=mintnix-git
_pkgname=mintnix
pkgver=0.3.4
pkgrel=1
pkgdesc="Linux-only fork of drg-mint-notag for Deep Rock Galactic mod management"
arch=('x86_64')
url="https://github.com/ile-du-fantome/mintnix"
license=('MIT')
provides=('mintnix')
conflicts=('mintnix')
depends=(
  'gcc-libs'
  'glibc'
  'gtk3'
  'libxkbcommon'
  'wayland'
  'libgl'
  'libx11'
  'libxcursor'
  'libxi'
  'libxrandr'
  'fontconfig'
  'freetype2'
)
makedepends=(
  'git'
  'rustup'
  'pkgconf'
  'mingw-w64-gcc'
  'clang'
  'mold'
)
source=(
  "$pkgname::git+https://github.com/ile-du-fantome/mintnix.git#branch=main"
  "oodleue::git+https://github.com/WorkingRobot/OodleUE.git#branch=main"
)
sha256sums=('SKIP' 'SKIP')

pkgver() {
  cd "$pkgname" || exit 1
  printf '%s.r%s.g%s' \
    "$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)" \
    "$(git rev-list --count HEAD)" \
    "$(git rev-parse --short=7 HEAD)"
}

prepare() {
  cd "$pkgname" || exit 1

  export RUSTUP_TOOLCHAIN=stable
  rustup toolchain install stable --profile minimal \
    --target x86_64-pc-windows-gnu --no-self-update

  cargo fetch --locked
}

build() {
  cd "$pkgname" || exit 1

  export RUSTUP_TOOLCHAIN=stable
  export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
  export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L /usr/x86_64-w64-mingw32/lib"

  cargo build --frozen --release --package hook --target x86_64-pc-windows-gnu

  MINT_HOOK_DLL="$PWD/target/x86_64-pc-windows-gnu/release/hook.dll" \
    cargo build --frozen --release --package "$_pkgname"
}

check() {
  cd "$pkgname" || exit 1

  export RUSTUP_TOOLCHAIN=stable
  export MINT_HOOK_DLL="$PWD/target/x86_64-pc-windows-gnu/release/hook.dll"

  cargo test --frozen --release --package "$_pkgname"
}

package() {
  cd "$pkgname" || exit 1

  install -Dm755 "target/release/$_pkgname" "$pkgdir/usr/lib/$_pkgname/$_pkgname"

  local oodle_sdk="$srcdir/oodleue/Engine/Source/Programs/Shared/EpicGames.Oodle/Sdk"
  local oodle_lib
  oodle_lib="$(find "$oodle_sdk" -path '*/linux/lib/liboo2corelinux64.so.9' -print -quit)"
  if [ -z "$oodle_lib" ]; then
    echo "mintnix: liboo2corelinux64.so.9 not found under $oodle_sdk" >&2
    exit 1
  fi
  install -Dm644 "$oodle_lib" "$pkgdir/usr/lib/$_pkgname/liboo2corelinux64.so.9"

  install -d "$pkgdir/usr/bin"
  cat > "$pkgdir/usr/bin/$_pkgname" <<'EOF'
#!/bin/sh
exec /usr/lib/mintnix/mintnix "$@"
EOF
  chmod 755 "$pkgdir/usr/bin/$_pkgname"

  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
