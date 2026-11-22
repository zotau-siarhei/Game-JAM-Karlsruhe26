#!/usr/bin/env bash
# Пакет для Google Play (Android App Bundle): target/android/space-guard.aab.
#
#   scripts/build-aab.sh             # собрать и подписать
#   scripts/build-aab.sh --install   # ещё и поставить на подключённый телефон через bundletool
#
# Подпись ключом загрузки Play — из переменных окружения; сам ключ и пароль в репозиторий не класть:
#   SPACE_GUARD_UPLOAD_KEYSTORE=/путь/upload.jks   хранилище ключа
#   SPACE_GUARD_UPLOAD_ALIAS=upload                имя ключа в хранилище
#   SPACE_GUARD_UPLOAD_PASSWORD=…                  пароль хранилища
#   SPACE_GUARD_UPLOAD_KEY_PASSWORD=…              пароль ключа, если не совпадает с паролем хранилища
# Без них пакет подписывается отладочным ключом: годится для проверки на телефоне, не для Play.
# Версия для пользователей — SPACE_GUARD_VERSION_NAME (по умолчанию 0.1.0), код версии —
# SPACE_GUARD_VERSION_CODE или число коммитов, как у APK (в клоне с неполной историей — только
# переменная). Библиотеку и ассеты готовит scripts/build-android.sh (arm64-v8a); символы библиотеки
# для Play Vitals (имена функций в стеках падений) лежат в пакете отдельно, сама библиотека — без них.
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
if [[ -n "${SPACE_GUARD_VERSION_CODE:-}" ]]; then
    version_code=$SPACE_GUARD_VERSION_CODE
elif [[ $(git rev-parse --is-shallow-repository) == true ]]; then
    echo "история git неполная: задайте SPACE_GUARD_VERSION_CODE или сделайте git fetch --unshallow" >&2
    exit 1
else
    version_code=$(git rev-list --count HEAD)
fi
# SDK: ANDROID_HOME, иначе место Android Studio по умолчанию (macOS, затем Linux).
sdk=${ANDROID_HOME:-}
if [[ -z "$sdk" ]]; then
    sdk=$HOME/Library/Android/sdk
    [[ -d "$sdk" ]] || sdk=$HOME/Android/Sdk
fi
if [[ ! -d "$sdk/build-tools" ]]; then
    echo "Android SDK не найден в $sdk: задайте ANDROID_HOME" >&2
    exit 1
fi
tools=$(ls -d "$sdk"/build-tools/* 2>/dev/null | sort -V | tail -1)
platform=$sdk/platforms/android-$target_sdk/android.jar
target=$root/target/android

# Библиотека, ресурсы и ассеты с уменьшенными текстурами — как у APK; библиотека с таблицей символов.
SPACE_GUARD_SYMBOLS=1 SPACE_GUARD_VERSION_NAME=$version_name SPACE_GUARD_VERSION_CODE=$version_code \
    scripts/build-android.sh
ndk=${ANDROID_NDK_HOME:-$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1)}
llvm=$(ls -d "$ndk"/toolchains/llvm/prebuilt/*/bin | head -1)

# bundletool от Google: скачивается один раз, сверяется по SHA-256.
bundletool_version=1.18.3
bundletool_sha=a099cfa1543f55593bc2ed16a70a7c67fe54b1747bb7301f37fdfd6d91028e29
bundletool=$target/tools/bundletool-all-$bundletool_version.jar
if [[ ! -f "$bundletool" ]]; then
    mkdir -p "$(dirname "$bundletool")"
    curl -fsSL -o "$bundletool.part" \
        "https://github.com/google/bundletool/releases/download/$bundletool_version/bundletool-all-$bundletool_version.jar"
    # Хэш сравнивается строкой: у sha256sum в macOS нет ключа --quiet, как у GNU.
    if command -v sha256sum >/dev/null; then sum=(sha256sum); else sum=(shasum -a 256); fi
    actual=$("${sum[@]}" "$bundletool.part" | awk '{ print $1 }')
    if [[ "$actual" != "$bundletool_sha" ]]; then
        rm -f "$bundletool.part"
        echo "bundletool: SHA-256 $actual, а ожидался $bundletool_sha" >&2
        exit 1
    fi
    mv "$bundletool.part" "$bundletool"
fi

out=$target/aab
rm -rf "$out"
mkdir -p "$out/base"

# Модуль base: манифест и ресурсы в формате protobuf, ассеты, библиотека.
"$tools/aapt2" link --proto-format -o "$out/base.apk" -I "$platform" \
    --manifest android/AndroidManifest.xml \
    --min-sdk-version "$min_sdk" --target-sdk-version "$target_sdk" \
    --version-code "$version_code" --version-name "$version_name" \
    -A "$target/assets" \
    "$target/apk/res.zip"
# В пакет — библиотека без символов. Таблица символов (без DWARF и кода, как SYMBOL_TABLE у Android
# Gradle Plugin: ~90 МБ, в пакете ~11 МБ) — в метаданные пакета: Play Console читает по ней имена
# функций в стеках падений и ANR, на телефоны она не попадает.
library=$target/aarch64-linux-android/release/libspace_guard.so
symbols=$out/libspace_guard.so.sym
"$llvm/llvm-objcopy" --strip-debug --only-keep-debug "$library" "$symbols"
"$llvm/llvm-strip" --strip-all -o "$out/libspace_guard.so" "$library"
python3 -I - "$out/base.apk" "$out/base" "$out/libspace_guard.so" <<'PY'
import os, sys, zipfile
apk, base, lib = sys.argv[1:4]
with zipfile.ZipFile(apk) as source:
    source.extractall(base)
os.makedirs(os.path.join(base, "manifest"))
os.rename(os.path.join(base, "AndroidManifest.xml"), os.path.join(base, "manifest", "AndroidManifest.xml"))
os.makedirs(os.path.join(base, "lib", "arm64-v8a"))
os.rename(lib, os.path.join(base, "lib", "arm64-v8a", "libspace_guard.so"))
with zipfile.ZipFile(os.path.join(os.path.dirname(base), "base.zip"), "w", zipfile.ZIP_DEFLATED) as module:
    for folder, _, files in os.walk(base):
        for name in files:
            path = os.path.join(folder, name)
            module.write(path, os.path.relpath(path, base))
PY

# Уже сжатые форматы хранятся как есть (как -0 у APK), библиотека — несжатой и выровненной по 16 КБ:
# Google Play требует поддержки страниц 16 КБ у приложений с target SDK 35+, а без выравнивания в APK
# из пакета система на таких устройствах библиотеку не отобразит.
cat > "$out/BundleConfig.json" <<'JSON'
{
  "compression": {
    "uncompressedGlob": ["assets/**/*.mp3", "assets/**/*.ogg", "assets/**/*.png", "assets/**/*.glb"]
  },
  "optimizations": {
    "uncompressNativeLibraries": { "enabled": true, "alignment": "PAGE_ALIGNMENT_16K" }
  }
}
JSON
aab=$target/space-guard.aab
rm -f "$aab"
java -jar "$bundletool" build-bundle --modules="$out/base.zip" --config="$out/BundleConfig.json" --output="$aab" \
    --metadata-file="com.android.tools.build.debugsymbols/arm64-v8a/libspace_guard.so.sym:$symbols"
# Вывод целиком, а не через `| grep -q`: при pipefail grep, закрывший канал раньше, дал бы SIGPIPE.
config=$(java -jar "$bundletool" dump config --bundle="$aab")
if [[ "$config" != *PAGE_ALIGNMENT_16K* ]]; then
    echo "в $aab нет выравнивания библиотек по 16 КБ (PAGE_ALIGNMENT_16K)" >&2
    exit 1
fi

if [[ -n "${SPACE_GUARD_UPLOAD_KEYSTORE:-}" ]]; then
    if [[ ! -f "$SPACE_GUARD_UPLOAD_KEYSTORE" ]]; then
        echo "нет хранилища ключа загрузки: $SPACE_GUARD_UPLOAD_KEYSTORE" >&2
        exit 1
    fi
    : "${SPACE_GUARD_UPLOAD_ALIAS:?нужно имя ключа: SPACE_GUARD_UPLOAD_ALIAS}"
    : "${SPACE_GUARD_UPLOAD_PASSWORD:?нужен пароль: SPACE_GUARD_UPLOAD_PASSWORD}"
    export SPACE_GUARD_UPLOAD_KEY_PASSWORD=${SPACE_GUARD_UPLOAD_KEY_PASSWORD:-$SPACE_GUARD_UPLOAD_PASSWORD}
    # Пароли — из окружения: в командной строке и логе их нет. Алгоритм подписи jarsigner выбирает
    # по типу ключа (RSA, EC).
    jarsigner -keystore "$SPACE_GUARD_UPLOAD_KEYSTORE" -storepass:env SPACE_GUARD_UPLOAD_PASSWORD \
        -keypass:env SPACE_GUARD_UPLOAD_KEY_PASSWORD -digestalg SHA-256 \
        "$aab" "$SPACE_GUARD_UPLOAD_ALIAS" >/dev/null
    # Подпись проверяется сразу: битый пакет Play отклонил бы уже после загрузки.
    if ! jarsigner -verify "$aab" >/dev/null; then
        echo "подпись $aab не проверилась" >&2
        exit 1
    fi
    echo "подписан ключом загрузки «$SPACE_GUARD_UPLOAD_ALIAS», подпись проверена"
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
    java -jar "$bundletool" build-apks --bundle="$aab" --output="$apks" --connected-device ${device[@]+"${device[@]}"} \
        --adb="$sdk/platform-tools/adb" \
        --ks="$HOME/.android/debug.keystore" --ks-pass=pass:android --ks-key-alias=androiddebugkey --key-pass=pass:android
    java -jar "$bundletool" install-apks --apks="$apks" ${device[@]+"${device[@]}"} --adb="$sdk/platform-tools/adb"
fi
