# Lunokhod

Игра для [Game Jam Karlsruhe 2026](https://gamejam-karlsruhe.hackops.tech/): луноход попадает на неизвестную планету и, как в аркаде, уничтожает врагов.

Сейчас это заглушка: окно с 3D-сценой (поверхность планеты, луноход-кубик, свет, камера). Геймплей ещё не проработан.

## Стек

- Rust 1.99: версия закреплена в `rust-toolchain.toml`, rustup поставит её сам.
- [Bevy](https://bevy.org) 0.20.1.

На Linux Bevy нужны системные библиотеки `alsa-lib` и `libudev` ([список по дистрибутивам](https://github.com/bevyengine/bevy/blob/main/docs/linux_dependencies.md)).

## Запуск

```sh
cargo run
```

Первая сборка Bevy занимает несколько минут, последующие — секунды. Ускорить пересборку во время разработки:

```sh
cargo run --features bevy/dynamic_linking
```

## OpenSpec

Требования и изменения ведутся через [OpenSpec](https://github.com/Fission-AI/OpenSpec): контекст проекта лежит в `openspec/config.yaml`, спецификации — в `openspec/specs/`.

- `/opsx:propose "идея"` в Claude Code — предложить изменение;
- `/opsx:apply` — реализовать его;
- `/opsx:archive` — перенести принятое изменение в спецификации;
- `openspec list --specs`, `openspec validate --all` — посмотреть и проверить спецификации.
