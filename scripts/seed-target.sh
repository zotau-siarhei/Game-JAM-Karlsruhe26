#!/usr/bin/env bash
# Засевает target нового worktree готовой сборкой из соседнего worktree или основной копии,
# чтобы не компилировать Bevy и остальные зависимости с нуля (это 10–20 минут).
#
# Запуск из корня нового worktree перед первой сборкой:
#   scripts/seed-target.sh            # взять самую свежую полную сборку
#   scripts/seed-target.sh <путь>     # взять target из указанного worktree
#   scripts/seed-target.sh --force    # заменить уже имеющийся target
#
# Источник — только полная сборка: в target/debug лежит бинарник игры (имя пакета из его
# Cargo.toml, например space_guard). Копия делается
# клоном файловой системы (APFS `cp -c`, на Linux `--reflink`): мгновенно и без лишнего места
# на диске. После засева cargo пересобирает только сам крейт игры, а зависимости берёт готовые.
# Если источник ещё собирается, cargo сам перепроверит отпечатки и дособерёт недостающее.
# Исходники соседей не трогаются, CARGO_TARGET_DIR на чужой target указывать не нужно.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

force=0
source=""
for arg in "$@"; do
    case "$arg" in
        --force) force=1 ;;
        *) source="$arg" ;;
    esac
done

# Бинарник полной сборки в worktree `$1`: имя пакета берётся из его Cargo.toml.
built_binary() {
    local name
    name=$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$1/Cargo.toml" | head -1)
    echo "$1/target/debug/$name"
}

if [[ -x "$(built_binary "$root")" && $force -eq 0 ]]; then
    echo "target уже собран, засев не нужен (--force, чтобы заменить)"
    exit 0
fi

mtime() { stat -f %m "$1" 2>/dev/null || stat -c %Y "$1"; }

if [[ -z "$source" ]]; then
    best=0
    while IFS= read -r dir; do
        [[ "$dir" == "$root" ]] && continue
        [[ -f "$dir/Cargo.toml" ]] || continue
        bin=$(built_binary "$dir")
        [[ -x "$bin" ]] || continue
        t=$(mtime "$bin")
        if (( t > best )); then
            best=$t
            source="$dir"
        fi
    done < <(git worktree list --porcelain | sed -n 's/^worktree //p')
fi

if [[ -z "$source" || ! -x "$(built_binary "$source")" ]]; then
    echo "нет полной сборки у соседей: собирай как обычно, cargo build" >&2
    exit 1
fi

if ! cmp -s "$source/Cargo.lock" Cargo.lock; then
    echo "внимание: Cargo.lock отличается от $source, часть зависимостей пересоберётся" >&2
fi

echo "засев target из $source"
rm -rf target
if [[ "$(uname)" == "Darwin" ]]; then
    # Клон APFS; на другой файловой системе — обычная копия.
    cp -c -R "$source/target" target 2>/dev/null || { rm -rf target; cp -R "$source/target" target; }
else
    cp -R --reflink=auto "$source/target" target
fi
echo "готово: теперь cargo build пересоберёт только сам крейт игры"
