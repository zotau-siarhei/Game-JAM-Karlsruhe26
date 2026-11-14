#!/usr/bin/env python3
"""Генерация звуковых эффектов и музыки через ElevenLabs.

Списки: assets/sfx/sounds.json и assets/music/tracks.json. Поле `text` записи — описание звука
на английском, `seconds` — длительность (для музыки обязательна, для звука по желанию),
`loop` — звук должен зацикливаться без шва. Готовый файл кладётся рядом со списком.
Реплики озвучивает scripts/gen_voice.py.

  export ELEVENLABS_API_KEY=...             # ключ только в окружении, в репозиторий не класть
  python3 scripts/gen_audio.py plan         # что будет сгенерировано и сколько секунд, без запросов
  python3 scripts/gen_audio.py gen          # всё, чего ещё нет
  python3 scripts/gen_audio.py gen --kind sfx              # только звуки (или music)
  python3 scripts/gen_audio.py gen --only explosion_1,menu # выбранные записи, заново

Существующие файлы не трогаются, пока запись не названа в --only. Нужен только Python 3.
"""
import argparse, json, os, sys, urllib.request, urllib.error
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
API = "https://api.elevenlabs.io"
LISTS = {
    "sfx": ROOT / "assets" / "sfx" / "sounds.json",
    "music": ROOT / "assets" / "music" / "tracks.json",
}
# Длительность музыки, если в записи нет `seconds`.
DEFAULT_MUSIC_SECONDS = 30


def load(kinds):
    """Записи выбранных списков: (вид, каталог, запись)."""
    entries = []
    for kind in kinds:
        path = LISTS[kind]
        for entry in json.loads(path.read_text())["lines"]:
            entries.append((kind, path.parent, entry))
    return entries


def select(entries, only):
    """Записи к генерации: названные в --only или те, у которых нет файла."""
    if only:
        want = set(only.split(","))
        found = [e for e in entries if e[2]["id"] in want]
        missing = want - {e[2]["id"] for e in found}
        if missing:
            sys.exit("Нет таких записей: " + ", ".join(sorted(missing)))
        return found
    return [e for e in entries if e[2].get("text") and e[2].get("file") and not (e[1] / e[2]["file"]).is_file()]


def request_for(kind, entry):
    """Путь и тело запроса к ElevenLabs для записи."""
    if kind == "music":
        seconds = entry.get("seconds", DEFAULT_MUSIC_SECONDS)
        body = {"prompt": entry["text"], "music_length_ms": int(max(3, seconds) * 1000), "force_instrumental": True}
        return "/v1/music?output_format=mp3_44100_128", body
    body = {"text": entry["text"], "prompt_influence": 0.5}
    if entry.get("seconds"):
        body["duration_seconds"] = min(30, max(0.5, entry["seconds"]))
    if entry.get("loop"):
        body["loop"] = True
    return "/v1/sound-generation?output_format=mp3_44100_128", body


def call(path, body, key):
    req = urllib.request.Request(API + path, data=json.dumps(body).encode(),
                                 headers={"xi-api-key": key, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=300) as response:
            return response.read()
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"HTTP {error.code}: {error.read().decode(errors='replace')[:300]}")


def cmd_plan(todo, args):
    for kind, _, entry in todo:
        seconds = entry.get("seconds", DEFAULT_MUSIC_SECONDS if kind == "music" else "авто")
        print(f"{kind:5} {entry['id']:22} {seconds!s:>5} с  {entry['text'][:70]}")
    music = sum(e.get("seconds", DEFAULT_MUSIC_SECONDS) for k, _, e in todo if k == "music")
    print(f"\nК генерации: звуков {sum(1 for k, _, _ in todo if k == 'sfx')}, "
          f"треков {sum(1 for k, _, _ in todo if k == 'music')} на {music} с музыки.")


def cmd_gen(todo, args):
    key = os.environ["ELEVENLABS_API_KEY"]
    failed = 0
    for number, (kind, directory, entry) in enumerate(todo, 1):
        print(f"[{number}/{len(todo)}] {kind} {entry['id']}", flush=True)
        try:
            audio = call(*request_for(kind, entry), key)
            (directory / entry["file"]).write_bytes(audio)
        except Exception as error:
            failed += 1
            print("    ОШИБКА:", error)
    if failed:
        sys.exit(f"Не сгенерировано: {failed} из {len(todo)}.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Генерация звуков и музыки через ElevenLabs.")
    parser.add_argument("cmd", choices=["plan", "gen"])
    parser.add_argument("--kind", choices=["sfx", "music"], help="только один список")
    parser.add_argument("--only", default="", help="идентификаторы через запятую, генерируются заново")
    args = parser.parse_args()
    if args.cmd == "gen" and not os.environ.get("ELEVENLABS_API_KEY"):
        sys.exit("Задай ELEVENLABS_API_KEY в окружении.")
    todo = select(load([args.kind] if args.kind else list(LISTS)), args.only)
    {"plan": cmd_plan, "gen": cmd_gen}[args.cmd](todo, args)
