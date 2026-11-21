#!/usr/bin/env bash
# Нативная сборка под Android: библиотека игры и APK, без Gradle. Результат — target/android/space-guard.apk.
#
#   scripts/build-android.sh                 # собрать для телефона (arm64-v8a)
#   scripts/build-android.sh --abi=x86_64    # для эмулятора; --abi=all — оба варианта в одном APK
#   scripts/build-android.sh --install       # поставить через adb
#   scripts/build-android.sh --run           # поставить и запустить
#
# Один раз поставить: rustup target add aarch64-linux-android x86_64-linux-android
#                     sdkmanager 'ndk;29.0.14206865' 'platforms;android-36' 'build-tools;36.1.0'
# Подпись — отладочным ключом ~/.android/debug.keystore (создаётся сам, если его нет).
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

package=io.github.zotausiarhei.spaceguard
min_sdk=29
target_sdk=36
abis=(arm64-v8a)
install=0
run=0
for arg in "$@"; do
    case "$arg" in
        --abi=all) abis=(arm64-v8a x86_64) ;;
        --abi=arm64-v8a | --abi=x86_64) abis=("${arg#--abi=}") ;;
        --install) install=1 ;;
        --run) install=1; run=1 ;;
        *) echo "неизвестный ключ: $arg" >&2; exit 1 ;;
    esac
done

sdk=${ANDROID_HOME:-$HOME/Android/Sdk}
ndk=${ANDROID_NDK_HOME:-$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1)}
tools=$(ls -d "$sdk"/build-tools/* 2>/dev/null | sort -V | tail -1)
platform=$sdk/platforms/android-$target_sdk/android.jar
if [[ ! -d "$ndk" || ! -x "$tools/aapt2" || ! -f "$platform" ]]; then
    echo "нужны NDK, build-tools и platforms;android-$target_sdk в $sdk (см. начало скрипта)" >&2
    exit 1
fi
llvm=$(ls -d "$ndk"/toolchains/llvm/prebuilt/*/bin | head -1)

# Свой каталог сборки внутри target этого worktree; версии зависимостей — как у обычной сборки.
export CARGO_TARGET_DIR=$root/target/android
cp Cargo.lock android/Cargo.lock

# Копия android-activity с фильтром ввода (геймпад). Пересоздаётся только при смене патча:
# иначе cargo каждый раз пересобирал бы всё, что от неё зависит.
activity_version=$(awk '/^name = "android-activity"$/ { getline; gsub(/version = |"/, ""); print }' Cargo.lock)
patched=$CARGO_TARGET_DIR/patched/android-activity
stamp="$activity_version $(cksum < android/patch_android_activity.py)"
if [[ "$(cat "$patched/.space-guard-patch" 2>/dev/null)" != "$stamp" ]]; then
    registry=${CARGO_HOME:-$HOME/.cargo}/registry/src
    if ! ls -d "$registry"/*/"android-activity-$activity_version" >/dev/null 2>&1; then
        cargo fetch
    fi
    rm -rf "$patched"
    mkdir -p "$(dirname "$patched")"
    cp -R "$(ls -d "$registry"/*/"android-activity-$activity_version" | head -1)" "$patched"
    python3 -I android/patch_android_activity.py "$patched"
    echo "$stamp" > "$patched/.space-guard-patch"
fi

targets=()
for abi in "${abis[@]}"; do
    case "$abi" in
        arm64-v8a) triple=aarch64-linux-android ;;
        x86_64) triple=x86_64-linux-android ;;
    esac
    targets+=(--target "$triple")
    clang=$llvm/$triple$min_sdk-clang
    under=${triple//-/_}
    upper=${under^^}
    export "CC_$under=$clang" "CXX_$under=$clang++" "AR_$under=$llvm/llvm-ar"
    # C++ зависимостей (meshoptimizer) без исключений и RTTI: среды C++ с ними в системе Android нет,
    # библиотека не загрузилась бы (нет __gxx_personality_v0).
    export "CXXFLAGS_$under=-fno-exceptions -fno-rtti"
    export "CARGO_TARGET_${upper}_LINKER=$clang"
    # Страницы по 16 КБ: без этого библиотека не загрузится на новых устройствах.
    export "CARGO_TARGET_${upper}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384"
done
cargo build --manifest-path android/Cargo.toml --release "${targets[@]}"

out=$CARGO_TARGET_DIR/apk
rm -rf "$out"
mkdir -p "$out"
"$tools/aapt2" compile --dir android/res -o "$out/res.zip"

# Уже сжатые форматы хранятся как есть: чтение из APK быстрее, размер почти тот же.
"$tools/aapt2" link -o "$out/unsigned.apk" -I "$platform" \
    --manifest android/AndroidManifest.xml \
    --min-sdk-version "$min_sdk" --target-sdk-version "$target_sdk" \
    --version-code "$(git rev-list --count HEAD)" --version-name 0.1.0 \
    -A assets -0 mp3 -0 ogg -0 png -0 glb \
    "$out/res.zip"

# Библиотека лежит в APK несжатой: система отображает её в память прямо оттуда.
libs=()
for abi in "${abis[@]}"; do
    case "$abi" in
        arm64-v8a) triple=aarch64-linux-android ;;
        x86_64) triple=x86_64-linux-android ;;
    esac
    libs+=("lib/$abi/libspace_guard.so=$CARGO_TARGET_DIR/$triple/release/libspace_guard.so")
done
python3 -I - "$out/unsigned.apk" "${libs[@]}" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "a", zipfile.ZIP_STORED) as apk:
    for item in sys.argv[2:]:
        name, path = item.split("=", 1)
        apk.write(path, name)
PY
"$tools/zipalign" -f -P 16 4 "$out/unsigned.apk" "$out/aligned.apk"

keystore=$HOME/.android/debug.keystore
if [[ ! -f "$keystore" ]]; then
    mkdir -p "$(dirname "$keystore")"
    keytool -genkeypair -keystore "$keystore" -storepass android -keypass android \
        -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10950 \
        -dname "CN=Android Debug,O=Android,C=US"
fi
apk=$CARGO_TARGET_DIR/space-guard.apk
"$tools/apksigner" sign --ks "$keystore" --ks-pass pass:android --key-pass pass:android \
    --out "$apk" "$out/aligned.apk"
rm -f "$apk.idsig"
ls -lh "$apk"

if (( install )); then
    "$sdk/platform-tools/adb" install -r "$apk"
fi
if (( run )); then
    "$sdk/platform-tools/adb" shell am start -n "$package/android.app.NativeActivity"
fi
