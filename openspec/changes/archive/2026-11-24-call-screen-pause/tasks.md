## 1. Пауза от системы

- [x] 1.1 Сообщение `run::SystemPause`; `pause_on_escape_or_blur` читает его вместо `SystemSilence::is_changed()`
- [x] 1.2 `android::watch_audio` шлёт `SystemPause` при начале звонка
- [x] 1.3 `android::watch_display`: раз в 0,25 с `Display.getState()`, экран погас — `SystemPause`

## 2. Версия на экране

- [x] 2.1 `android::app_version` (`PackageManager`), `platform::app_version`, флаг профиля `menu.version` (Android — вкл.)
- [x] 2.2 `version_label.rs`: подпись справа от полосы здоровья, поверх занавеса, с обходом выреза

## 3. Проверка

- [x] 3.1 Эмулятор: забег, начатый во время звонка, не встаёт на паузу; звонок посреди забега ставит паузу
- [x] 3.2 Эмулятор: wake lock `PROXIMITY_SCREEN_OFF_WAKE_LOCK` и закрытый датчик гасят экран — забег на паузе
- [x] 3.3 CLAUDE.md: раздел Android — пауза при погасшем экране
- [x] 3.4 Эмулятор: подпись `v0.1.0-test (465)` видна в меню, забеге, на паузе и экране конца забега
