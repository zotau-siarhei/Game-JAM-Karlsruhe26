в работе: Sergei Zotov


## 1. Уровень устройства

- [x] 1.1 `src/profile.rs`: `android_tier(gpu, память, low_ram)` по таблице решения 1 и выбор `android-low|android|android-high`; тесты на имена Adreno 610/720/740/810/830, Mali-G57/G715, Immortalis-G720, Xclipse 940, PowerVR, неизвестный; проверка: `cargo test`
- [x] 1.2 `src/android.rs`: имя GPU из временного `wgpu::Instance` (Vulkan), `MemTotal`, `ro.config.low_ram`, `ro.soc.model` в лог; проверка: эмулятор — строка уровня в logcat, время до меню не больше прежнего на 0,2 с
- [x] 1.3 Профили `android-low`, `android-high` и предел 60 у `android` в `assets/platform/profiles.json` по таблице решения 2; тест упорядоченности уровней; проверка: `cargo test`

## 2. Нагрев, память, частота, Game Mode

- [x] 2.1 Thermal headroom раз в 10 с через `dlsym` (`AThermal_acquireManager`, `AThermal_getThermalHeadroom`) → флаги `throttled`/`overheated` с запасом 0,1; проверка: `cargo test` на правило, эмулятор — без ошибок, `adb shell cmd thermalservice override-status 4` → строка в logcat
- [x] 2.2 `ActivityManager.MemoryInfo` через JNI раз в 10 с → `low_memory`; проверка: logcat на эмуляторе
- [x] 2.3 `ANativeWindow_setFrameRate` через `dlsym` после каждого создания окна; проверка: телефон — `adb shell dumpsys SurfaceFlinger | grep -i "frame rate"` во время игры
- [x] 2.4 Game Mode: `android/res/xml/game_mode_config.xml`, `<meta-data android:name="android.game_mode_config">` в манифесте, `GameManager.getGameMode()` через JNI при запуске и возврате; BATTERY → флаги; проверка: `adb shell cmd game mode battery io.github.zotausiarhei.spaceguard`, возврат в игру — 30 кадров

## 3. Google Play

- [ ] 3.1 `scripts/build-aab.sh`: `PAGE_ALIGNMENT_16K` в `BundleConfig.json` и проверка `bundletool dump config`; проверка: `scripts/build-aab.sh`

## 4. Проверка

- [ ] 4.1 A059: уровень «средний» в logcat, забег 5 минут на medium — FPS не ниже прежнего, `gpubusy` и температура ниже из-за предела 60
- [x] 4.2 Эмулятор с `debug.spaceguard.profile android-low` и `android-high` — игра идёт, числа из профиля в логе
- [x] 4.3 `CLAUDE.md`: уровни Android, нагрев, Game Mode, AAB; `openspec validate --all`
