## ADDED Requirements

### Requirement: Геймпад на каждой платформе
Ввод геймпада SHALL работать на macOS (через GameController), Linux и Windows (через gilrs).
Вибрация SHALL работать на macOS (CoreHaptics) и там, где gilrs её поддерживает (Linux; Windows —
для геймпадов, которым система даёт вибрацию). Световая панель геймпадов PlayStation SHALL меняться
на macOS и на Linux (светодиоды ядра); без прав на запись игра MUST продолжать работать и сказать
в логе, как дать права.

#### Scenario: Linux без прав на светодиоды
- **WHEN** игрок на Linux подключает DualShock 4, а писать в `/sys/class/leds` нельзя
- **THEN** управление и вибрация работают, панель не меняется, в логе одно сообщение с правилом udev
