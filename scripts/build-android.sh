#!/usr/bin/env bash
# scripts/build-android.sh — compila libaudio2mp3.so para Android.
#
#   scripts/build-android.sh [--ndk <ruta>] [--api 24] [--no-package]
#
# Requisitos: rustup y el NDK de Android (ANDROID_NDK_HOME, ANDROID_NDK_ROOT o
# el más reciente en $ANDROID_HOME/ndk). Para empaquetar: zip o python3.
# Salida:
#   dist/android/jniLibs/<abi>/libaudio2mp3.so
#   dist/audio2mp3-android-v<versión>.zip y dist/SHA256SUMS
# Comprueba que los segmentos LOAD están alineados a 16 KB (Google Play) y que
# se exportan exactamente las funciones JNI definidas en audio2mp3-jni.
# Equivalente a scripts/build-android.ps1 (Windows).

set -euo pipefail

API=24
NDK=""
PACKAGE=1
PAGE_SIZE=16384

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ndk) NDK="$2"; shift 2 ;;
    --api) API="$2"; shift 2 ;;
    --no-package) PACKAGE=0; shift ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "Opción desconocida: $1" >&2; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() { echo "error: $*" >&2; exit 1; }

find_ndk() {
  if [[ -n "$NDK" ]]; then echo "$NDK"; return; fi
  for var in ANDROID_NDK_HOME ANDROID_NDK_ROOT; do
    if [[ -n "${!var:-}" && -d "${!var}" ]]; then echo "${!var}"; return; fi
  done
  local sdk
  for sdk in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Library/Android/sdk" \
             "$HOME/Android/Sdk" "${LOCALAPPDATA:-}/Android/Sdk"; do
    if [[ -n "$sdk" && -d "$sdk/ndk" ]]; then
      local latest
      latest="$(ls -1 "$sdk/ndk" | sort -V | tail -n 1)"
      if [[ -n "$latest" ]]; then echo "$sdk/ndk/$latest"; return; fi
    fi
  done
  fail "no se encontró el NDK. Usa --ndk <ruta> o define ANDROID_NDK_HOME."
}

case "$(uname -s)" in
  Linux) HOST=linux-x86_64; EXE=""; CMD="" ;;
  Darwin) HOST=darwin-x86_64; EXE=""; CMD="" ;;
  MINGW*|MSYS*|CYGWIN*) HOST=windows-x86_64; EXE=".exe"; CMD=".cmd" ;;
  *) fail "sistema no soportado: $(uname -s)" ;;
esac

NDK_PATH="$(find_ndk)"
BIN="$NDK_PATH/toolchains/llvm/prebuilt/$HOST/bin"
[[ -d "$BIN" ]] || fail "NDK sin toolchain LLVM para $HOST: $BIN"
READELF="$BIN/llvm-readelf$EXE"
NM="$BIN/llvm-nm$EXE"

# rust-target:abi:prefijo-del-clang
TARGETS=(
  "aarch64-linux-android:arm64-v8a:aarch64-linux-android"
  "armv7-linux-androideabi:armeabi-v7a:armv7a-linux-androideabi"
  "x86_64-linux-android:x86_64:x86_64-linux-android"
)

VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
EXPECTED="$(grep -o 'extern "system" fn Java_[A-Za-z0-9_]*' crates/audio2mp3-jni/src/lib.rs \
  | sed 's/.* fn //' | sort)"

echo "audio2mp3 v$VERSION | NDK $(basename "$NDK_PATH") | API $API"

DIST="$ROOT/dist"
JNILIBS="$DIST/android/jniLibs"
rm -rf "$DIST/android"
INSTALLED="$(rustup target list --installed)"

SUMMARY=("$(printf '%-12s %8s  %-8s %s' ABI KB Alineado JNI)")

for entry in "${TARGETS[@]}"; do
  IFS=: read -r TRIPLE ABI CLANG <<< "$entry"
  grep -qx "$TRIPLE" <<< "$INSTALLED" || rustup target add "$TRIPLE"

  LINKER="$BIN/$CLANG$API-clang$CMD"
  [[ -f "$LINKER" ]] || fail "no existe el linker $LINKER (¿API $API soportada por este NDK?)"
  if [[ -n "$CMD" ]] && command -v cygpath > /dev/null; then LINKER="$(cygpath -w "$LINKER")"; fi
  VAR="CARGO_TARGET_$(echo "$TRIPLE" | tr 'a-z-' 'A-Z_')"

  echo "-> $ABI ($TRIPLE)"
  env "${VAR}_LINKER=$LINKER" "${VAR}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=$PAGE_SIZE" \
    cargo build --release --quiet -p audio2mp3-jni --target "$TRIPLE"

  SO="target/$TRIPLE/release/libaudio2mp3.so"
  # Alineación de todos los segmentos LOAD.
  ALIGNS="$("$READELF" -lW "$SO" | awk '$1 == "LOAD" { print $NF }')"
  [[ -n "$ALIGNS" ]] || fail "$ABI: sin segmentos LOAD"
  for a in $ALIGNS; do
    (( a >= PAGE_SIZE )) || fail "$ABI: segmento LOAD alineado a $a, se requiere $PAGE_SIZE"
  done
  # Símbolos JNI exportados = funciones definidas en el puente.
  EXPORTED="$("$NM" -D --defined-only "$SO" | awk '{ print $NF }' | grep '^Java_' | sort || true)"
  [[ "$EXPORTED" == "$EXPECTED" ]] || fail "$ABI: los símbolos JNI exportados no coinciden con audio2mp3-jni"

  mkdir -p "$JNILIBS/$ABI"
  cp "$SO" "$JNILIBS/$ABI/"
  SIZE="$(wc -c < "$SO" | tr -d ' ')"
  SUMMARY+=("$(printf '%-12s %8s  %-8s %s' "$ABI" "$(awk "BEGIN { printf \"%.1f\", $SIZE / 1024 }")" \
    "16 KB" "$(wc -l <<< "$EXPORTED" | tr -d ' ')")")
done

printf '%s\n' "${SUMMARY[@]}"

if [[ "$PACKAGE" == 1 ]]; then
  ZIP_NAME="audio2mp3-android-v$VERSION.zip"
  ZIP="$DIST/$ZIP_NAME"
  rm -f "$ZIP"
  if command -v zip > /dev/null; then
    (cd "$DIST/android" && zip -qrX "$ZIP" jniLibs)
  elif command -v python3 > /dev/null && python3 -c 'import zipfile' 2> /dev/null; then
    (cd "$DIST/android" && python3 -m zipfile -c "$ZIP" jniLibs)
  elif [[ "$HOST" == windows-x86_64 && -x "${SYSTEMROOT:-C:/Windows}/System32/tar.exe" ]]; then
    # bsdtar de Windows (Git Bash sin zip): -a elige zip por la extensión.
    (cd "$DIST/android" && "${SYSTEMROOT:-C:/Windows}/System32/tar.exe" -a -c -f "$(cygpath -w "$ZIP")" jniLibs)
  else
    fail "se necesita zip o python3 para empaquetar (o usa --no-package)"
  fi
  if command -v sha256sum > /dev/null; then
    HASH="$(sha256sum "$ZIP" | cut -d' ' -f1)"
  else
    HASH="$(shasum -a 256 "$ZIP" | cut -d' ' -f1)"
  fi
  SUMS="$DIST/SHA256SUMS"
  touch "$SUMS"
  grep -v "  $ZIP_NAME\$" "$SUMS" > "$SUMS.tmp" || true
  echo "$HASH  $ZIP_NAME" >> "$SUMS.tmp"
  mv "$SUMS.tmp" "$SUMS"
  echo "Paquete: $ZIP"
  echo "SHA256:  $HASH"
fi
