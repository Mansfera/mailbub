#!/bin/sh
# Builds a signed release APK (arm64) into dist/release/.
#
# The signing key stays out of the repo: the keystore in ~/.android/keystores and its password
# in the macOS Keychain. Both are created on the first run. Back them up: an installed app only
# accepts updates signed with the same key.
set -eu

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' src-tauri/tauri.conf.json | head -1)
OUT=dist/release
APK="$OUT/Mailbub_${VERSION}_arm64.apk"
KEYCHAIN_SERVICE=com.mansfera.mailbub.android-keystore

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export JAVA_HOME="${JAVA_HOME:-/Applications/Android Studio.app/Contents/jbr/Contents/Home}"
NDK_HOME="${NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)}"
export NDK_HOME
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

export MAILBUB_KEYSTORE="${MAILBUB_KEYSTORE:-$HOME/.android/keystores/com.mansfera.mailbub.jks}"
if [ ! -f "$MAILBUB_KEYSTORE" ]; then
  echo "android: creating signing key $MAILBUB_KEYSTORE" >&2
  mkdir -p "$(dirname "$MAILBUB_KEYSTORE")"
  MAILBUB_KEYSTORE_PASSWORD=$(openssl rand -base64 32)
  export MAILBUB_KEYSTORE_PASSWORD
  security add-generic-password -U -a "$USER" -s "$KEYCHAIN_SERVICE" -w "$MAILBUB_KEYSTORE_PASSWORD"
  "$JAVA_HOME/bin/keytool" -genkeypair -keystore "$MAILBUB_KEYSTORE" -storetype PKCS12 \
    -storepass:env MAILBUB_KEYSTORE_PASSWORD -alias mailbub -keyalg RSA -keysize 4096 \
    -validity 10000 -dname "CN=mansfera, O=mansfera"
  chmod 600 "$MAILBUB_KEYSTORE"
fi
MAILBUB_KEYSTORE_PASSWORD=$(security find-generic-password -a "$USER" -s "$KEYCHAIN_SERVICE" -w)
export MAILBUB_KEYSTORE_PASSWORD

bun tauri android build --apk --target aarch64

mkdir -p "$OUT"
cp src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk "$APK"
BUILD_TOOLS=$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -1)
"$BUILD_TOOLS/apksigner" verify "$APK"
echo "android: $APK ($(du -h "$APK" | cut -f1))"
