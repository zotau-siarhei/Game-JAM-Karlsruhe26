#!/usr/bin/env bash
# Пакет для Google Play (Android App Bundle): target/android/space-guard.aab.
#
#   scripts/build-aab.sh             # собрать и подписать
#   scripts/build-aab.sh --install   # ещё и поставить на подключённый телефон через bundletool
#
# Подпись ключом загрузки Play — из переменных окружения; сам ключ и пароль в репозиторий не класть:
#   SPACE_GUARD_UPLOAD_KEYSTORE=/путь/upload.jks   хранилище ключа
#   SPACE_GUARD_UPLOAD_ALIAS=upload                имя ключа в хранилище
#   SPACE_GUARD_UPLOAD_PASSWORD=…                  пароль (хранилища и ключа)
# Без них пакет подписывается отладочным ключом: годится для проверки на телефоне, не для Play.
# Версия для пользователей — SPACE_GUARD_VERSION_NAME (по умолчанию 0.1.0), код версии — число
# коммитов, как у APK. Библиотеку и ассеты готовит scripts/build-android.sh (arm64-v8a).
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

install=0
for arg in "$@"; do
    case "$arg" in
        --install) install=1 ;;
        *) echo "неизвестный ключ: $arg" >&2; exit 1 ;;
    esac
done

min_sdk=29
target_sdk=36
version_name=${SPACE_GUARD_VERSION_NAME:-0.1.0}
sdk=${ANDROID_HOME:-$HOME/Android/Sdk}
tools=$(ls -d "$sdk"/build-tools/* 2>/dev/null | sort -V | tail -1)
platform=$sdk/platforms/android-$target_sdk/android.jar
target=$root/target/android

# Библиотека, ресурсы и ассеты с уменьшенными текстурами — как у APK.
scripts/build-android.sh

# bundletool от Google: скачивается один раз, сверяется по SHA-256.
bundletool_version=1.18.3
bundletool_sha=a099cfa1543f55593bc2ed16a70a7c67fe54b1747bb7301f37fdfd6d91028e29
bundletool=$target/tools/bundletool-all-$bundletool_version.jar
if [[ ! -f "$bundletool" ]]; then
    mkdir -p "$(dirname "$bundletool")"
    curl -fsSL -o "$bundletool.part" \
        "https://github.com/google/bundletool/releases/download/$bundletool_version/bundletool-all-$bundletool_version.jar"
    if command -v sha256sum >/dev/null; then sum=(sha256sum); else sum=(shasum -a 256); fi
    echo "$bundletool_sha  $bundletool.part" | "${sum[@]}" -c --quiet
    mv "$bundletool.part" "$bundletool"
fi

out=$target/aab
rm -rf "$out"
mkdir -p "$out/base"

# Модуль base: манифест и ресурсы в формате protobuf, ассеты, библиотека.
"$tools/aapt2" link --proto-format -o "$out/base.apk" -I "$platform" \
    --manifest android/AndroidManifest.xml \
    --min-sdk-version "$min_sdk" --target-sdk-version "$target_sdk" \
    --version-code "$(git rev-list --count HEAD)" --version-name "$version_name" \
    -A "$target/assets" \
    "$target/apk/res.zip"
python3 -I - "$out/base.apk" "$out/base" "$target/aarch64-linux-android/release/libspace_guard.so" <<'PY'
import os, sys, zipfile
apk, base, lib = sys.argv[1:4]
with zipfile.ZipFile(apk) as source:
    source.extractall(base)
os.makedirs(os.path.join(base, "manifest"))
os.rename(os.path.join(base, "AndroidManifest.xml"), os.path.join(base, "manifest", "AndroidManifest.xml"))
os.makedirs(os.path.join(base, "lib", "arm64-v8a"))
os.link(lib, os.path.join(base, "lib", "arm64-v8a", "libspace_guard.so"))
with zipfile.ZipFile(os.path.join(os.path.dirname(base), "base.zip"), "w", zipfile.ZIP_DEFLATED) as module:
    for folder, _, files in os.walk(base):
        for name in files:
            path = os.path.join(folder, name)
            module.write(path, os.path.relpath(path, base))
PY

# Уже сжатые форматы хранятся как есть (как -0 у APK), библиотека — несжатой.
cat > "$out/BundleConfig.json" <<'JSON'
{
  "compression": {
    "uncompressedGlob": ["assets/**/*.mp3", "assets/**/*.ogg", "assets/**/*.png", "assets/**/*.glb"]
  },
  "optimizations": {
    "uncompressNativeLibraries": { "enabled": true }
  }
}
JSON
aab=$target/space-guard.aab
rm -f "$aab"
java -jar "$bundletool" build-bundle --modules="$out/base.zip" --config="$out/BundleConfig.json" --output="$aab"

if [[ -n "${SPACE_GUARD_UPLOAD_KEYSTORE:-}" ]]; then
    : "${SPACE_GUARD_UPLOAD_ALIAS:?нужно имя ключа: SPACE_GUARD_UPLOAD_ALIAS}"
    : "${SPACE_GUARD_UPLOAD_PASSWORD:?нужен пароль: SPACE_GUARD_UPLOAD_PASSWORD}"
    # Пароль — из окружения: в командной строке и логе его нет.
    jarsigner -keystore "$SPACE_GUARD_UPLOAD_KEYSTORE" -storepass:env SPACE_GUARD_UPLOAD_PASSWORD \
        -keypass:env SPACE_GUARD_UPLOAD_PASSWORD -sigalg SHA256withRSA -digestalg SHA-256 \
        "$aab" "$SPACE_GUARD_UPLOAD_ALIAS" >/dev/null
    echo "подписан ключом загрузки «$SPACE_GUARD_UPLOAD_ALIAS»"
else
    jarsigner -keystore "$HOME/.android/debug.keystore" -storepass android -keypass android \
        -sigalg SHA256withRSA -digestalg SHA-256 "$aab" androiddebugkey >/dev/null
    echo "подписан отладочным ключом: для Play задайте SPACE_GUARD_UPLOAD_KEYSTORE (см. начало скрипта)"
fi
ls -lh "$aab"

if (( install )); then
    # Набор APK для подключённого телефона из пакета — так же их нарезает Play.
    apks=$out/device.apks
    # Несколько устройств — выбор через ANDROID_SERIAL, как у adb.
    device=(${ANDROID_SERIAL:+--device-id=$ANDROID_SERIAL})
    java -jar "$bundletool" build-apks --bundle="$aab" --output="$apks" --connected-device "${device[@]}" \
        --adb="$sdk/platform-tools/adb" \
        --ks="$HOME/.android/debug.keystore" --ks-pass=pass:android --ks-key-alias=androiddebugkey --key-pass=pass:android
    java -jar "$bundletool" install-apks --apks="$apks" "${device[@]}" --adb="$sdk/platform-tools/adb"
fi
