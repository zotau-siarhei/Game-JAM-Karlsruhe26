"""Добавляет в копию android-activity фильтр ввода: `input::set_input_filter`.

Очередь ввода читает только winit и превращает события джойстика в касания, теряя правый стик
и курки. Фильтр получает событие первым, и то, что он забрал, до winit не доходит.
Запускает scripts/build-android.sh: `patch_android_activity.py <каталог копии>`.
"""
import pathlib
import sys

root = pathlib.Path(sys.argv[1]) / "src"

path = root / "input.rs"
text = path.read_text(encoding="utf-8")
text += """
static INPUT_FILTER: std::sync::OnceLock<fn(&InputEvent<'_>) -> bool> = std::sync::OnceLock::new();

/// Sets a filter that sees every input event first; events it returns `true` for are consumed.
pub fn set_input_filter(filter: fn(&InputEvent<'_>) -> bool) {
    let _ = INPUT_FILTER.set(filter);
}

pub(crate) fn input_filtered(event: &InputEvent<'_>) -> bool {
    INPUT_FILTER.get().is_some_and(|filter| filter(event))
}
"""
path.write_text(text, encoding="utf-8")

path = root / "native_activity" / "mod.rs"
text = path.read_text(encoding="utf-8")
old = "std::panic::catch_unwind(AssertUnwindSafe(|| callback(&event)));"
new = """std::panic::catch_unwind(AssertUnwindSafe(|| {
                    if crate::input::input_filtered(&event) {
                        InputStatus::Handled
                    } else {
                        callback(&event)
                    }
                }));"""
if text.count(old) != 1:
    sys.exit("android-activity: место для фильтра ввода не найдено")
path.write_text(text.replace(old, new), encoding="utf-8")
