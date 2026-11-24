## Why

После перезапуска из-за потери видеокарты старый процесс завершался через `std::process::exit(0)`. Этот выход
зовёт `atexit`-деструкторы C и C++, и они разрушают статические мьютексы, пока системные потоки процесса ещё
работают: поток `hwuiTask0` падал с «FORTIFY: pthread_mutex_lock called on a destroyed mutex» (SIGABRT). Игрок
этого не видит, новая игра уже открыта, но в Play Vitals такой выход попал бы как падение.

## What Changes

- Android завершает процесс через `_exit(0)` (`android::exit_process`): после выхода из игры и при второй
  активности в живом процессе.

## Capabilities

### New Capabilities

### Modified Capabilities
- `android-app-integration`: процесс уходит без падения системных потоков.

## Impact

`src/android.rs`, `src/main.rs`. Только Android.
