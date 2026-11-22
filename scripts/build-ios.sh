#!/usr/bin/env bash
# Нативная сборка под iOS: исполняемый файл игры и приложение .app, без проекта Xcode.
# Результат — target/ios/<sim|device>/SpaceGuard.app.
#
#   scripts/build-ios.sh                # для iPhone (aarch64-apple-ios)
#   scripts/build-ios.sh --sim          # для симулятора на Mac с Apple Silicon
#   scripts/build-ios.sh --release      # профиль release: LTO, дольше собирается, быстрее и меньше
#   scripts/build-ios.sh --install      # поставить: в загруженный симулятор или на подключённый iPhone
#   scripts/build-ios.sh --run          # поставить и запустить
#   scripts/build-ios.sh --run --env SPACE_GUARD_PERF=1 --env SPACE_GUARD_RENDER_DENSITY=1
#                                       # переменные окружения запуска: замеры и переопределения
#                                       # профиля платформы (`src/profile.rs`); можно несколько
#   scripts/build-ios.sh --small-textures
#                                       # текстуры моделей до 1024 при сборке (`scripts/web_textures.py`):
#                                       # .app меньше, загрузка быстрее; в игре профиль iOS и так
#                                       # уменьшает их до 1024 после загрузки
#
# Один раз поставить: Xcode и rustup target add aarch64-apple-ios aarch64-apple-ios-sim.
# Для телефона нужны сертификат «Apple Development» в связке ключей и профиль подготовки с этим
# iPhone для io.github.zotausiarhei.spaceguard (или для «*»); профиль ищется в каталогах Xcode,
# путь можно задать явно: IOS_PROFILE=/путь/профиль.mobileprovision.
# Симулятор — IOS_SIM (по умолчанию загруженный, иначе «iPhone 15 Pro»), устройство — IOS_DEVICE (UDID;
# без него — подключённый iPhone, иначе любое другое подключённое).
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

bundle_id=io.github.zotausiarhei.spaceguard
sim=0
profile_flag=()
profile_dir=debug
install=0
run=0
small_textures=0
launch_env=()
expect_env=0
for arg in "$@"; do
    if ((expect_env)); then
        launch_env+=("$arg")
        expect_env=0
        continue
    fi
    case "$arg" in
        --env) expect_env=1 ;;
        --env=*) launch_env+=("${arg#--env=}") ;;
        --small-textures) small_textures=1 ;;
        --sim) sim=1 ;;
        --release) profile_flag=(--release); profile_dir=release ;;
        --install) install=1 ;;
        --run) install=1; run=1 ;;
        *) echo "неизвестный ключ: $arg" >&2; exit 1 ;;
    esac
done
if ((expect_env)); then
    echo "--env: после ключа нужно КЛЮЧ=значение" >&2
    exit 1
fi

if ((sim)); then
    triple=aarch64-apple-ios-sim
    platform=iPhoneSimulator
    kind=sim
else
    triple=aarch64-apple-ios
    platform=iPhoneOS
    kind=device
fi

# Минимальная iOS — одна для Rust и для C++ зависимостей (meshoptimizer), как в Info.plist: иначе
# C++ собирается под новейшую систему и ссылается на `___chkstk_darwin`, которой нет при
# линковке под iOS 10 по умолчанию у Rust.
export IPHONEOS_DEPLOYMENT_TARGET=16.0

# Цель сборки — в обычном target этого worktree: зависимости для iOS собираются один раз.
cargo build --target "$triple" ${profile_flag[@]+"${profile_flag[@]}"}
binary=target/$triple/$profile_dir/space_guard

app=target/ios/$kind/SpaceGuard.app
rm -rf "$app"
mkdir -p "$app"
# Версия SDK в исполняемом файле — 18: iOS 27 завершает приложение без сцен (UIScene), если оно
# собрано со SDK 27 и новее, а winit 0.30 сцен не умеет. Со старой версией SDK это лишь
# предупреждение. Минимальная система — 16.0, как в Info.plist.
if ((sim)); then build_platform=iossim; else build_platform=ios; fi
xcrun vtool -set-build-version "$build_platform" 16.0 18.0 -replace -output "$app/space_guard" "$binary"
# Без отладочных символов: с ними файл весит 225 МБ, а места на телефоне мало.
xcrun strip -S -x "$app/space_guard"

# Info.plist: платформа и иконки.
plist=$app/Info.plist
cp ios/Info.plist "$plist"
buddy=/usr/libexec/PlistBuddy
"$buddy" -c "Add :CFBundleSupportedPlatforms array" -c "Add :CFBundleSupportedPlatforms:0 string $platform" "$plist"
"$buddy" -c "Add :DTPlatformName string $(tr '[:upper:]' '[:lower:]' <<< "$platform")" "$plist"

# Иконка — тот же рисунок, что у Android: квадрат без прозрачности, углы скругляет система.
icons=target/ios/icons
source_icon=android/icon/source.png
if [[ ! -f "$icons/.stamp" || "$source_icon" -nt "$icons/.stamp" ]]; then
    rm -rf "$icons"
    mkdir -p "$icons"
    for spec in AppIcon60x60@2x:120 AppIcon60x60@3x:180 AppIcon76x76@2x~ipad:152 AppIcon83.5x83.5@2x~ipad:167; do
        sips -z "${spec#*:}" "${spec#*:}" "$source_icon" --out "$icons/${spec%:*}.png" >/dev/null
    done
    touch "$icons/.stamp"
fi
cp "$icons"/*.png "$app/"
"$buddy" -c "Add :CFBundleIcons dict" \
    -c "Add :CFBundleIcons:CFBundlePrimaryIcon dict" \
    -c "Add :CFBundleIcons:CFBundlePrimaryIcon:CFBundleIconFiles array" \
    -c "Add :CFBundleIcons:CFBundlePrimaryIcon:CFBundleIconFiles:0 string AppIcon60x60" \
    -c "Add :CFBundleIcons~ipad dict" \
    -c "Add :CFBundleIcons~ipad:CFBundlePrimaryIcon dict" \
    -c "Add :CFBundleIcons~ipad:CFBundlePrimaryIcon:CFBundleIconFiles array" \
    -c "Add :CFBundleIcons~ipad:CFBundlePrimaryIcon:CFBundleIconFiles:0 string AppIcon60x60" \
    -c "Add :CFBundleIcons~ipad:CFBundlePrimaryIcon:CFBundleIconFiles:1 string AppIcon76x76" \
    -c "Add :CFBundleIcons~ipad:CFBundlePrimaryIcon:CFBundleIconFiles:2 string AppIcon83.5x83.5" \
    "$plist"
plutil -convert binary1 "$plist"

# Ассеты рядом с исполняемым файлом: загрузчик Bevy ищет `assets` в папке программы.
rsync -a --exclude '.DS_Store' --exclude '*.md' assets "$app/"
if ((small_textures)); then
    textures=(scripts/web_textures.py "$app/assets" target/ios/textures 1024)
    if python3 -c "import PIL" 2>/dev/null; then
        python3 -I "${textures[@]}"
    elif command -v uv >/dev/null; then
        uv run --quiet --no-project --with pillow python -I "${textures[@]}"
    else
        echo "нужен Pillow: brew install uv (или pip install pillow)" >&2
        exit 1
    fi
fi

# Подпись. Симулятору хватает подписи без сертификата, телефону нужен профиль подготовки.
if ((sim)); then
    codesign --force --sign - --timestamp=none "$app"
else
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    profile=${IOS_PROFILE:-}
    if [[ -z "$profile" ]]; then
        now=$(date -u +%s)
        for candidate in "$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles"/*.mobileprovision \
            "$HOME/Library/MobileDevice/Provisioning Profiles"/*.mobileprovision; do
            [[ -f "$candidate" ]] || continue
            security cms -D -i "$candidate" > "$work/p.plist" 2>/dev/null || continue
            app_id=$("$buddy" -c "Print :Entitlements:application-identifier" "$work/p.plist" 2>/dev/null) || continue
            expires=$(date -j -u -f "%Y-%m-%dT%H:%M:%SZ" "$(plutil -extract ExpirationDate raw "$work/p.plist")" +%s 2>/dev/null || echo 0)
            ((expires > now)) || continue
            case "$app_id" in
                *.$bundle_id) profile=$candidate; break ;;
                *.\*) profile=$candidate ;;
            esac
        done
    fi
    if [[ -z "$profile" ]]; then
        echo "нет действующего профиля подготовки для $bundle_id: создай его в Xcode (см. начало скрипта)" >&2
        echo "или укажи IOS_PROFILE=/путь/профиль.mobileprovision" >&2
        exit 1
    fi
    echo "профиль: $profile"
    cp "$profile" "$app/embedded.mobileprovision"
    security cms -D -i "$profile" > "$work/p.plist"
    # Права — как ставит Xcode: только идентификатор, команда и отладка. Профиль разрешает и
    # `keychain-access-groups` с шаблоном `*`, но в подписи шаблон недопустим: iOS не запустит.
    team=$("$buddy" -c "Print :TeamIdentifier:0" "$work/p.plist")
    # Отладку разрешают только профили разработчика; в профиле распространения ключа нет.
    task_allow=$("$buddy" -c "Print :Entitlements:get-task-allow" "$work/p.plist" 2>/dev/null || echo false)
    cat > "$work/entitlements.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>application-identifier</key>
	<string>$team.$bundle_id</string>
	<key>com.apple.developer.team-identifier</key>
	<string>$team</string>
	<key>get-task-allow</key>
	<$task_allow/>
</dict>
</plist>
PLIST
    # Сертификат — тот, что вписан в профиль: в связке ключей их бывает несколько (разные команды,
    # старый и новый), а подпись чужим профиль не примет. Сверка по SHA-1 сертификата.
    identity=${IOS_IDENTITY:-}
    if [[ -z "$identity" ]]; then
        allowed=""
        i=0
        while "$buddy" -c "Print :DeveloperCertificates:$i" "$work/p.plist" > /dev/null 2>&1; do
            plutil -extract "DeveloperCertificates.$i" raw "$work/p.plist" | base64 -D > "$work/cert.der"
            allowed+=" $(openssl x509 -inform der -in "$work/cert.der" -noout -fingerprint -sha1 | sed 's/.*=//; s/://g')"
            i=$((i + 1))
        done
        identities=$(security find-identity -v -p codesigning)
        while read -r _ hash _; do
            if [[ -n "$hash" && " $allowed " == *" $hash "* ]]; then
                identity=$hash
                break
            fi
        done <<< "$identities"
    fi
    if [[ -z "$identity" ]]; then
        echo "нет в связке ключей сертификата из профиля (команда $team): поставь его через Xcode" >&2
        echo "или укажи IOS_IDENTITY=<имя или SHA-1 сертификата>" >&2
        exit 1
    fi
    echo "подпись: $identity (команда $team)"
    codesign --force --sign "$identity" --entitlements "$work/entitlements.plist" --timestamp=none "$app"
fi
echo "готово: $app"

((install)) || exit 0
# Окружение запуска: simctl и devicectl передают процессу переменные с приставкой.
for pair in ${launch_env[@]+"${launch_env[@]}"}; do
    [[ "$pair" == *=* ]] || { echo "--env: нужно КЛЮЧ=значение, а не «$pair»" >&2; exit 1; }
    export "SIMCTL_CHILD_$pair" "DEVICECTL_CHILD_$pair"
done
if ((sim)); then
    # Вывод целиком, а не через `| grep -q`: при pipefail grep, закрывший канал раньше, дал бы SIGPIPE.
    booted=$(xcrun simctl list devices booted)
    if [[ "$booted" != *Booted* ]]; then
        xcrun simctl boot "${IOS_SIM:-iPhone 15 Pro}"
    fi
    open -a Simulator 2>/dev/null || open -b com.apple.iphonesimulator 2>/dev/null || true
    xcrun simctl install booted "$app"
    if ((run)); then
        xcrun simctl terminate booted "$bundle_id" 2>/dev/null || true
        xcrun simctl launch booted "$bundle_id"
    fi
else
    # Без IOS_DEVICE — подключённый iPhone, а если его нет — первое другое устройство (iPad):
    # к Mac бывают подключены сразу несколько.
    pick_device() {
        xcrun devicectl list devices 2>/dev/null | awk -v want="$1" '/physical/ && !/unavailable/ && $0 ~ want {
            for (i = 1; i <= NF; i++) if ($i ~ /^[0-9A-F]{8}-[0-9A-F]{16}$/) { print $i; exit }
        }'
    }
    device=${IOS_DEVICE:-$(pick_device iPhone)}
    device=${device:-$(pick_device .)}
    if [[ -z "$device" ]]; then
        echo "iPhone не подключён или не готов (xcrun devicectl list devices)" >&2
        exit 1
    fi
    xcrun devicectl device install app --device "$device" "$app"
    if ((run)); then
        xcrun devicectl device process launch --terminate-existing --device "$device" "$bundle_id"
    fi
fi
