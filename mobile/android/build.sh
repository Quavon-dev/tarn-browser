#!/bin/sh
set -eu

cd "$(dirname "$0")"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
APP_DIR=../app
EMBED_URL=https://github.com/longbridge/nsh/releases/download/v0.5.0/nsh-embed-android.zip
EMBED_SHA256=4e0c943c75c4618e84a0139394719068de341f5c4386714502a2fb1cba802d7f
OUT=build/tarn

mkdir -p "$OUT/jni/arm64-v8a" "$OUT/assets"

if [ ! -f "$OUT/nsh-classes.jar" ]; then
    curl -fsSL -o "$OUT/nsh-embed-android.zip" "$EMBED_URL"
    echo "$EMBED_SHA256  $OUT/nsh-embed-android.zip" | shasum -a 256 -c -
    unzip -p "$OUT/nsh-embed-android.zip" nsh-embed-android/nsh.aar > "$OUT/nsh.aar"
    unzip -p "$OUT/nsh.aar" classes.jar > "$OUT/nsh-classes.jar"
fi

(cd "$APP_DIR" && nsh package --target android)
rm -rf "$OUT/apk" "$OUT/assets"
unzip -o -q "$APP_DIR"/.nsh/out/*.apk 'lib/arm64-v8a/libnsh.so' 'assets/*' -d "$OUT/apk"
cp "$OUT/apk/lib/arm64-v8a/libnsh.so" "$OUT/jni/arm64-v8a/"
mkdir -p "$OUT/assets"
cp -R "$OUT/apk/assets/." "$OUT/assets/"

./gradle.sh --quiet clean "${GRADLE_TASK:-assembleDebug}"
ls -l host/build/outputs/apk/*/*.apk
