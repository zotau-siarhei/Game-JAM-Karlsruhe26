# Google Play Console: публикация Space Guard

Ответы для анкет консоли и порядок первого выпуска. Тексты и графика карточки — `listing.md` и файлы рядом.
Ключи подписи и пароли в репозитории не лежат: они в `~/.android` у владельца аккаунта разработчика.

## Пакет

- Пакет: `io.github.zotausiarhei.spaceguard`. После первой загрузки сменить его нельзя.
- Сборка: `scripts/build-aab.sh` → `target/android/space-guard.aab`. Ключ загрузки скрипт берёт сам из
  `~/.android/spaceguard-upload.keystore` и проверяет подпись. В выводе — отпечаток SHA-256 сертификата, он
  должен совпасть с сертификатом загрузки в консоли (Test and release → App integrity).
- Версия: `SPACE_GUARD_VERSION_NAME` (по умолчанию 0.1.0), код версии — число коммитов в `main`. Play не примет
  код, который уже видел: новый выпуск собирать из `main` с новыми коммитами.
- Устройства: Android 10+ (minSdk 29), target SDK 36, только arm64-v8a, Vulkan 1.1 обязателен
  (`uses-feature`). Play сам скроет игру на устройствах без них. Библиотека выровнена под страницы 16 КБ.

## Создание приложения

- App name: **Space Guard**; default language: **English (United States) – en-US**.
- App or game: **Game**; Free or paid: **Free**.

## Подпись (App integrity) — до первой загрузки

Схема как у VolumeX: ключ подписи приложения — свой, а не сгенерированный Google, `.aab` подписан ключом загрузки.

1. Test and release → App integrity → App signing → **Use a different key → Export and upload a key from Java
   keystore**. Выполнить команду PEPK, которую показывает консоль, с
   `--keystore=$HOME/.android/spaceguard-release.keystore --alias=spaceguard`, и загрузить получившийся архив.
2. В том же окне загрузить сертификат загрузки `spaceguard-upload-cert.pem` (как его выгрузить — в заметках
   рядом с ключом, `~/.android/spaceguard-release-notes.md`).

Если Google сгенерирует свой ключ, сборки из Play и из других магазинов (RuStore и т. п.) не смогут обновлять
друг друга.

3. На той же странице App integrity → **Automatic protection** — выключить и сохранить. Иначе загрузка падает
   с «Anti-tamper protection cannot be applied to app bundles without DEX files»: игра — `NativeActivity` без
   кода на Java (`hasCode="false"`), а защита встраивает проверки в DEX. Ничего не теряется: встраивать их некуда.

## Тестирование перед выпуском

Личному аккаунту разработчика, созданному после ноября 2023, нужен закрытый тест: не меньше 12 тестировщиков,
14 дней подряд, только потом откроется Production. Порядок: Closed testing → создать трек, список тестировщиков
(адреса Google), загрузить AAB, выпуск на тест.

## App content (Policy → App content)

| Раздел | Ответ |
|---|---|
| Privacy policy | https://github.com/zotau-siarhei/Game-JAM-Karlsruhe26/blob/main/PRIVACY.md |
| App access | All functionality is available without any access restrictions |
| Ads | No, my app does not contain ads |
| Advertising ID | No (нет разрешения `AD_ID`, рекламных SDK нет) |
| Government apps | No |
| Financial features | My app doesn't provide any financial features |
| Health | My app does not have any health features |
| News app | No |

### Data safety

- Does your app collect or share any of the required user data types? — **No**.
- Игра без разрешения на интернет: ничего не уходит с устройства. Настройки и рекорды (имя до 5 символов и время)
  лежат только на телефоне; резервная копия Android в аккаунте Google сбором данных не считается — разработчик
  к ней доступа не имеет.
- Итог в карточке: «No data collected», «No data shared».

### Content rating (анкета IARC)

Категория: **Game**. Факты для ответов:

- **Насилие — да, мультяшное.** Луноход стреляет по машинам врагов (танки, дроны, шагающий босс), они
  разлетаются на обломки. Мультяшные жители могут погибнуть от взрыва или под колёсами: исчезают с комичным «пуф»,
  без крови и тел. За это игрока не награждают, а наказывают (растёт тревога, приезжает Служба защиты жителей).
  Крови, расчленения и жестокости к людям нет. Оружие: пулемёт, мортира, ракеты, лазер, авиаудар.
- Страх и ужасы — нет. Сексуальное содержание, нагота — нет. Ненормативная лексика — нет (реплики штаба проверены).
- Наркотики, алкоголь, табак — нет. Азартные игры и их имитация — нет.
- Общение пользователей, обмен контентом, передача местоположения — нет (игра без сети).
- Покупки в приложении, цифровые товары — нет.

### Target audience and content

- Возраст: **13–15, 16–17, 18+** (не младше 13: игра не для детей, иначе действуют правила Families).
- «Could your store listing unintentionally appeal to children?» — рисовка мультяшная, но игра — шутер с
  разрушениями; ответ на усмотрение владельца аккаунта (обычно **No**).

## Store presence

- Category: **Games → Arcade**. Tags: Shooter, Arcade, Action, Offline, Single player.
- Contact details: e-mail владельца аккаунта (обязательно), website — https://zotau-siarhei.github.io/Game-JAM-Karlsruhe26/
- Main store listing: тексты из `listing.md` (только en-US, без переводов).
- Graphics:
  - App icon — `icon-512.png` (512×512 PNG);
  - Feature graphic — `feature-graphic.png` (1024×500);
  - Phone screenshots — `screenshots/*.jpg` (1920×1080, 16:9). Эти же снимки подходят для 7″ и 10″ планшетов.

## Выпуск

1. Собрать AAB из `main`: `scripts/build-aab.sh`; сверить отпечаток сертификата в выводе.
2. Closed testing (а позже Production) → Create new release → загрузить `target/android/space-guard.aab`.
3. Release name — версия (например, `0.1.0 (447)`), release notes — из `listing.md`.
4. Review release → Start rollout. Отчёт pre-launch (автотест на устройствах Firebase) приходит через ~час:
   проверить падения и ANR.
