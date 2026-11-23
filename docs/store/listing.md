# Карточка в Google Play

Тексты карточки — только English (en-US), игра сама на английском. Пределы Play: название до 30 символов,
краткое описание до 80, полное до 4000. Каждое поле — отдельный блок: строки в нём не переносятся вручную,
блок копируется в консоль целиком.

Графика лежит рядом: значок `icon-512.png` (512×512), обложка `feature-graphic.png` (1024×500), снимки экрана
`screenshots/*.jpg` (1920×1080, 7 штук; Play берёт от 2 до 8 для телефона). Сняты сборкой для компьютера с профилем `android-high` и `--touch` (`src/debug_shot.rs`).

Категория: **Игры → Аркады** (Arcade). Теги: Shooter, Arcade, Action, Offline, Single player.

## App name (26 символов)

```text
Space Guard: Rover Defense
```

## Short description (77 символов)

```text
Drive a moon rover, blast enemy waves and reshape the planet with explosions.
```

## Full description (≈1700 символов)

```text
Your rover just landed on a quiet green planet. Then the invaders arrived.

Space Guard is an arcade top-down 3D shooter about holding out as long as you can. One life, no ending: your score is the time you survive.

A PLANET YOU CAN BREAK
• Endless rolling hills, villages, forests and railways with trains and tunnels.
• Every explosion digs a crater, and repeated hits dig deeper. Break through the crust and lava floods out.
• Houses collapse, rails snap, trains derail, rocks shatter into debris.

SIX WEAPONS
• Machine gun, mortar, rockets, homing missiles, laser and an airstrike you aim from above.
• Supply crates drop by parachute; enemy wrecks leave trophies, ammo and repair kits.
• Hold to repair the rover — but you drive slower while you do it.

ENEMIES IN WAVES
• Rammers, mortar crews, rocket and laser tanks, interceptor drones.
• Some land in drop capsules right next to you.
• Every fourth wave brings the Colossus.

PROTECT THE VILLAGERS
• Villagers panic, run and hide from the fighting.
• Wreck their homes and the alarm rises: the Resident Protection Service rolls in to evacuate them — and they do not like you.

MADE FOR PHONES
• Touch controls: left thumb drives, right thumb aims and fires. Tap to shoot the nearest enemy.
• Gamepad and keyboard supported.
• Three difficulty levels with their own high-score tables, three graphics quality levels.
• A dry-humoured HQ on the radio, with optional subtitles.

Fully offline. No ads, no in-app purchases, no accounts, no data collection.

Made at Game Jam Karlsruhe 2026 by Nidzhat Guseynov, Ivan Tsarev and Sergei Zotov.

Requires Android 10 or newer, a 64-bit (ARM64) device and a GPU with Vulkan 1.1.
```

## Release notes 0.1.0 (до 500 символов)

```text
<en-US>
First test release of Space Guard.
• Drive a lunar rover across an endless hilly planet and hold off ever-stronger enemy waves
• Six weapons: machine gun, mortar, rockets, homing rockets, laser and airstrike
• Destructible world: craters, collapsing houses, wrecked railways
• Touch controls, gamepad or keyboard
• Three difficulty levels, each with its own high scores
• Low, Medium and High graphics quality
Please report crashes, freezes and stutters.
</en-US>
```
