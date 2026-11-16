#!/bin/sh
# Обёртка rustc для cargo (`build.rustc-wrapper` в .cargo/config.toml): кэш сборки sccache,
# если он установлен (`brew install sccache`, `pacman -S sccache`), иначе — обычный rustc.
# Cargo вызывает её так: rustc-wrapper.sh <путь к rustc> <аргументы…>
# Выключить кэш на одну сборку: RUSTC_WRAPPER= cargo build
if command -v sccache >/dev/null 2>&1; then
    exec sccache "$@"
fi
exec "$@"
