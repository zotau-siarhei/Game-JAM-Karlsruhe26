#!/usr/bin/env bash
# Браузерная сборка: WASM + страница web/index.html, игра на всю страницу. Нужен WebGPU
# (свежие Chrome, Edge, Safari): частицы считаются вычислительными шейдерами.
#
#   scripts/build-web.sh            # собрать в target/web
#   scripts/build-web.sh --serve    # собрать и открыть на http://localhost:8080
#   scripts/build-web.sh --release  # профиль web (release с параллельной сборкой): меньше и быстрее
#
# Один раз поставить: rustup target add wasm32-unknown-unknown
#                     cargo install wasm-bindgen-cli --version <версия wasm-bindgen из Cargo.lock>
# Открывать по http (localhost), не как файл: браузер не грузит модули и ассеты с file://.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

profile=debug
cargo_profile=()
serve=0
port=8080
for arg in "$@"; do
    case "$arg" in
        --release) profile=web; cargo_profile=(--profile web) ;;
        --serve) serve=1 ;;
        --port=*) port="${arg#--port=}" ;;
        *) echo "неизвестный ключ: $arg" >&2; exit 1 ;;
    esac
done

# wasm-bindgen CLI обязан совпадать по версии с крейтом wasm-bindgen в Cargo.lock.
want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print }' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }' || true)
if [[ "$have" != "$want" ]]; then
    echo "нужен wasm-bindgen-cli $want (сейчас: ${have:-нет}):" >&2
    echo "  cargo install wasm-bindgen-cli --version $want" >&2
    exit 1
fi

cargo build --target wasm32-unknown-unknown "${cargo_profile[@]}"

out=target/web
mkdir -p "$out"
# Таблица имён функций — половина файла (80 МБ), без неё в стеке паники нет имён.
wasm-bindgen --target web --no-typescript --remove-name-section --out-dir "$out" --out-name space_guard \
    "target/wasm32-unknown-unknown/$profile/space_guard.wasm"
cp web/index.html "$out/index.html"

# Ассеты рядом со страницей: загрузчик Bevy берёт их по HTTP из ./assets.
# На APFS копия — клон (`cp -c`): мгновенно и без лишнего места на диске.
rm -rf "$out/assets"
if [[ $(uname) == Darwin ]]; then
    cp -cR assets "$out/assets"
else
    cp -R assets "$out/assets"
fi
rm -rf "$out/assets/voice/raw"

echo "готово: $out ($(du -sh "$out/space_guard_bg.wasm" | cut -f1) wasm)"
if [[ $serve == 1 ]]; then
    echo "игра: http://localhost:$port"
    exec python3 -m http.server "$port" --bind 127.0.0.1 --directory "$out"
fi
