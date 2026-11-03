#!/bin/sh
# Выпуск релиза: сборка трёх бинарников и APK, SHA256SUMS + подпись ed25519, GitHub-релиз.
#   scripts/release.sh            — собрать и опубликовать v<версия из Cargo.toml>
#   scripts/release.sh --dry-run  — только собрать dist/ и подписать
# Нужны: cargo, mingw-w64, cmake, gh (залогинен), ключ ~/.config/voiceon-release.key.
# Для APK: JDK 17+ (JAVA_HOME), Android SDK с NDK (android/local.properties: sdk.dir),
# cargo-ndk (в PATH или cargo.bin в android/local.properties) и ключ подписи Android —
# файл свойств из VOICEON_ANDROID_SIGNING или ~/.config/voiceon-android.properties.
set -eu
cd "$(dirname "$0")/.."

DRY=0
[ "${1:-}" = "--dry-run" ] && DRY=1

SIGNING="${VOICEON_ANDROID_SIGNING:-$HOME/.config/voiceon-android.properties}"
[ -f "$SIGNING" ] || { echo "нет $SIGNING — ключа подписи Android"; exit 1; }

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TAG="v$VERSION"
echo "==> версия $VERSION"

if [ $DRY -eq 0 ]; then
    [ -z "$(git status --porcelain)" ] || { echo "в дереве есть незакоммиченные изменения"; exit 1; }
    git rev-parse "$TAG" >/dev/null 2>&1 && { echo "тег $TAG уже есть"; exit 1; }
fi

cargo test --workspace -q

# Убираем из бинарников локальные пути (имя пользователя).
export RUSTFLAGS="--remap-path-prefix=$HOME/.cargo/registry/src=/cargo --remap-path-prefix=$HOME/.rustup=/rustup --remap-path-prefix=$PWD=/voiceon"
export CFLAGS="-ffile-prefix-map=$HOME=/build"
cargo build --release -p voiceon-server --target x86_64-unknown-linux-musl
cargo build --release -p voiceon
cargo build --release -p voiceon --target x86_64-pc-windows-gnu
unset RUSTFLAGS CFLAGS
cargo build --release -q -p voiceon-release
# APK для arm64-v8a, armeabi-v7a и x86_64; пути из .so вычищает сама Gradle-задача cargoNdk.
# Без демона: Gradle должен увидеть свежие переменные окружения.
(cd android && VOICEON_ANDROID_SIGNING="$SIGNING" ./gradlew -q --no-daemon :app:assembleRelease)

rm -rf dist && mkdir dist
cp target/x86_64-unknown-linux-musl/release/voiceon-server dist/voiceon-server-linux-x86_64
cp target/release/voiceon dist/voiceon-linux-x86_64
cp target/x86_64-pc-windows-gnu/release/voiceon.exe dist/voiceon-windows-x86_64.exe
cp android/app/build/outputs/apk/release/app-release.apk dist/voiceon-android.apk
for f in dist/*; do
    if strings "$f" | grep -q "$HOME"; then echo "в $f остался путь $HOME"; exit 1; fi
done
(cd dist && sha256sum voiceon-* > SHA256SUMS)
target/release/voiceon-release sign dist/SHA256SUMS
cat dist/SHA256SUMS

[ $DRY -eq 1 ] && { echo "==> dry-run: dist/ готов"; exit 0; }

git tag "$TAG"
git push -q origin main "$TAG"
NOTES=$(sed -n "/^## $VERSION/,/^## /p" CHANGELOG.md 2>/dev/null | sed '1d;$d')
gh release create "$TAG" dist/* --title "voiceOn $VERSION" --notes "${NOTES:-voiceOn $VERSION}"
echo "==> опубликован $TAG"
