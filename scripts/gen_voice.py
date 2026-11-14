#!/usr/bin/env python3
"""Озвучка реплик через ElevenLabs по assets/voice/lines.json и voices.json.

  export ELEVENLABS_API_KEY=...            # ключ только в окружении, в репозиторий не класть
  python3 scripts/gen_voice.py plan        # что будет озвучено и сколько символов, без запросов
  python3 scripts/gen_voice.py design      # создать голоса по описаниям, вписать voice_id в voices.json
  python3 scripts/gen_voice.py gen --only intro_3,crit_2   # озвучить выбранные
  python3 scripts/gen_voice.py gen --batch first           # первая партия: интро, обучение, crit, death, оружие
  python3 scripts/gen_voice.py gen                          # всё, чего ещё нет
  python3 scripts/gen_voice.py fx --only hit_6              # заново наложить эффект на уже скачанный исходник

Исходники без эффектов лежат в assets/voice/raw/ (не коммитим), готовые файлы — в assets/voice/.
Нужны Python 3 и ffmpeg. Сторонних библиотек нет.
"""
import argparse, base64, json, os, re, subprocess, sys, urllib.request, urllib.error
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VOICE = ROOT / "assets" / "voice"
RAW = VOICE / "raw"
API = "https://api.elevenlabs.io"
FIRST = ("intro_", "run_start", "tut_", "first_crater", "crit_", "death", "weapon_", "boss_appears")


def load():
    lines = json.loads((VOICE / "lines.json").read_text())["lines"]
    cfg = json.loads((VOICE / "voices.json").read_text())
    return lines, cfg


def voice_key(line):
    return line.get("station") if line.get("speaker") == "bolt" else line.get("speaker", "dana")


def call(path, body, key):
    req = urllib.request.Request(API + path, data=json.dumps(body).encode(),
                                 headers={"xi-api-key": key, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.read()
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"HTTP {e.code}: {e.read().decode(errors='replace')[:300]}")


def strip_tags(text):
    return re.sub(r"\[[^\]]*\]\s*", "", text).strip()


def ffmpeg(args):
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", *args], check=True)


def apply_fx(line, cfg):
    """raw/<id>.mp3 -> <file> с эффектом. radio добавляет шипение настройки, +bleep — 8-битный бип в конце."""
    raw, out = RAW / f"{line['id']}.mp3", VOICE / line["file"]
    effect = line.get("effect", "none")
    base, bleep = effect.replace("+bleep", ""), effect.endswith("+bleep")
    chain = cfg["effects"].get(base, "")
    voice = f"[0:a]{chain + ',' if chain else ''}aresample=44100,aformat=channel_layouts=mono[v]"
    inputs, parts, labels = ["-i", str(raw)], [voice], []
    if base == "radio":  # щелчок и шипение настройки перед фразой
        inputs += ["-f", "lavfi", "-t", "0.25", "-i", "anoisesrc=color=white:amplitude=0.25:sample_rate=44100"]
        parts.append("[1:a]highpass=f=800,lowpass=f=4000,afade=t=out:st=0.15:d=0.1,aformat=channel_layouts=mono[n]")
        labels += ["[n]", "[v]"]
    else:
        labels += ["[v]"]
    if bleep:  # пиксельный бип-цензор
        idx = len(inputs) // 2 if base != "radio" else 2
        inputs += ["-f", "lavfi", "-t", "0.45", "-i", "sine=frequency=1000:sample_rate=44100"]
        parts.append(f"[{idx}:a]acrusher=bits=4:mix=1,volume=0.5,aformat=channel_layouts=mono[b]")
        labels.append("[b]")
    parts.append(f"{''.join(labels)}concat=n={len(labels)}:v=0:a=1,loudnorm=I=-16:TP=-1.5[o]")
    ffmpeg([*inputs, "-filter_complex", ";".join(parts), "-map", "[o]", "-ac", "1", "-b:a", "96k", str(out)])


def synth(line, cfg, key):
    if line.get("kind") == "sfx":
        return call("/v1/sound-generation", {"text": line["tts"], "prompt_influence": 0.5}, key)
    v = cfg["voices"][voice_key(line)]
    if not v.get("voice_id"):
        raise RuntimeError(f"нет voice_id для голоса «{voice_key(line)}» в voices.json (запусти design или впиши вручную)")
    path = f"/v1/text-to-speech/{v['voice_id']}?output_format=mp3_44100_128"
    try:
        return call(path, {"text": line["tts"], "model_id": cfg["model"]}, key)
    except RuntimeError as e:  # модель с тегами недоступна — обычная модель без тегов
        print(f"    {cfg['model']} не сработала ({e}); пробую {cfg['fallback_model']}")
        return call(path, {"text": strip_tags(line["tts"]), "model_id": cfg["fallback_model"],
                           "voice_settings": v.get("settings", {})}, key)


def select(lines, args):
    if args.only:
        want = set(args.only.split(","))
        return [l for l in lines if l["id"] in want]
    if args.batch == "first":
        lines = [l for l in lines if l["id"].startswith(FIRST)]
    return [l for l in lines if args.force or not (VOICE / l["file"]).is_file()]


def cmd_plan(lines, cfg, args):
    todo = select(lines, args)
    chars = sum(len(l["tts"]) for l in todo if l.get("kind") != "sfx")
    for l in todo:
        print(f"  {l['id']:22} {voice_key(l):9} {l.get('effect', ''):12} {l['tts'][:70]}")
    missing = sorted({voice_key(l) for l in todo if l.get("kind") != "sfx" and not cfg["voices"][voice_key(l)].get("voice_id")})
    print(f"\nК озвучке: {len(todo)} реплик, {chars} символов речи, {sum(1 for l in todo if l.get('kind') == 'sfx')} звуков.")
    if missing:
        print("Нет voice_id у голосов:", ", ".join(missing))


def cmd_design(lines, cfg, args):
    key = os.environ["ELEVENLABS_API_KEY"]
    RAW.mkdir(exist_ok=True)
    for name, v in cfg["voices"].items():
        if v.get("voice_id") and not args.force:
            continue
        print("Создаю голос", name)
        text = (v["sample"] + " ") * 3  # для предпросмотра нужно не меньше 100 символов
        res = json.loads(call("/v1/text-to-voice/design", {"voice_description": v["prompt"], "text": text[:900]}, key))
        prev = res["previews"][0]
        (RAW / f"_voice_{name}.mp3").write_bytes(base64.b64decode(prev["audio_base_64"]))
        made = json.loads(call("/v1/text-to-voice", {"voice_name": v["name"], "voice_description": v["prompt"],
                                                      "generated_voice_id": prev["generated_voice_id"]}, key))
        v["voice_id"] = made["voice_id"]
        (VOICE / "voices.json").write_text(json.dumps(cfg, ensure_ascii=False, indent=2) + "\n")
        print(f"  voice_id {made['voice_id']}, образец: assets/voice/raw/_voice_{name}.mp3")


def cmd_gen(lines, cfg, args):
    key = os.environ["ELEVENLABS_API_KEY"]
    RAW.mkdir(exist_ok=True)
    todo = select(lines, args)
    for i, l in enumerate(todo, 1):
        print(f"[{i}/{len(todo)}] {l['id']}")
        try:
            (RAW / f"{l['id']}.mp3").write_bytes(synth(l, cfg, key))
            apply_fx(l, cfg)
        except Exception as e:
            print("    ОШИБКА:", e)


def cmd_fx(lines, cfg, args):
    for l in lines:
        if (not args.only or l["id"] in args.only.split(",")) and (RAW / f"{l['id']}.mp3").is_file():
            apply_fx(l, cfg)
            print("fx", l["id"])


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("cmd", choices=["plan", "design", "gen", "fx"])
    p.add_argument("--only", default="")
    p.add_argument("--batch", default="")
    p.add_argument("--force", action="store_true")
    a = p.parse_args()
    if a.cmd in ("design", "gen") and not os.environ.get("ELEVENLABS_API_KEY"):
        sys.exit("Задай ELEVENLABS_API_KEY в окружении.")
    {"plan": cmd_plan, "design": cmd_design, "gen": cmd_gen, "fx": cmd_fx}[a.cmd](*load(), a)
