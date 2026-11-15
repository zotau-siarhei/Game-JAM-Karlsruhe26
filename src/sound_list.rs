//! Общий формат списков звуков: реплики, звуковые эффекты и музыка.
//!
//! Список — файл JSON вида `{"lines": [{"id", "event", "text", "file", ...}]}` в каталоге
//! внутри `assets`, звуковые файлы лежат рядом со списком.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::asset::io::file::FileAssetReader;
use bevy::prelude::*;
use serde::Deserialize;

/// Одна запись списка.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct SoundEntry {
    /// Идентификатор, по которому код игры вызывает звук.
    pub id: String,
    /// Описание события, на которое звучит запись.
    #[serde(default)]
    pub event: String,
    /// Текст реплики или описание звука для генерации.
    #[serde(default)]
    pub text: String,
    /// Имя звукового файла рядом со списком. Пустое, пока записи нет.
    #[serde(default)]
    pub file: String,
    /// Кто говорит: `dana` — штаб, `bolt` — луноход.
    #[serde(default)]
    pub speaker: String,
    /// Уровень тона штаба: 1 вежливо, 2 беспокойство, 3 паника. Нет — годится для любого.
    #[serde(default)]
    pub tone: Option<u8>,
    /// Приоритет 0–3. Записи с приоритетом 3 обрывают текущую реплику.
    #[serde(default)]
    pub pri: u8,
    /// `voice` — речь, `sfx` — звук без слов.
    #[serde(default)]
    pub kind: String,
}

/// Содержимое файла списка.
#[derive(Deserialize)]
struct ListFile {
    lines: Vec<SoundEntry>,
}

/// Разбирает текст списка.
pub fn parse_list(json: &str) -> Result<Vec<SoundEntry>, serde_json::Error> {
    serde_json::from_str::<ListFile>(json).map(|file| file.lines)
}

/// Отделяет от идентификатора суффиксы варианта: номер `_2` и тон `_t3`.
///
/// `house_destroyed_t2_2` → `house_destroyed`, `kill_4` → `kill`, `death` → `death`.
pub fn base_id(id: &str) -> &str {
    let id = strip_number_suffix(id, "_");
    strip_number_suffix(id, "_t")
}

/// Убирает с конца `<разделитель><цифры>`, если он там есть.
fn strip_number_suffix<'a>(id: &'a str, separator: &str) -> &'a str {
    let digits = id.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return id;
    }
    let head = &id[..id.len() - digits];
    match head.strip_suffix(separator) {
        Some(base) if !base.is_empty() => base,
        _ => id,
    }
}

/// Подходит ли запись `id` под запрос `wanted`: это её основа или её полный идентификатор.
pub fn in_group(id: &str, wanted: &str) -> bool {
    id == wanted || base_id(id) == wanted
}

/// Запись списка вместе с загруженным звуком.
pub struct LoadedSound {
    pub entry: SoundEntry,
    /// `None`, если у записи ещё нет звукового файла.
    pub sound: Option<Handle<AudioSource>>,
}

/// Загруженный список: записи по идентификатору и в порядке файла.
#[derive(Default)]
pub struct SoundList {
    entries: HashMap<String, LoadedSound>,
    order: Vec<String>,
}

impl SoundList {
    /// Читает `assets/<dir>/<file>` и ставит на загрузку звуки, файлы которых уже есть.
    ///
    /// Ошибки не останавливают игру: о них пишется в лог с пометкой `label`.
    pub fn load(asset_server: &AssetServer, dir: &str, file: &str, label: &str) -> Self {
        let mut list = Self::default();
        let dir_path = FileAssetReader::get_base_path().join("assets").join(dir);
        let list_path = dir_path.join(file);

        let json = match std::fs::read_to_string(&list_path) {
            Ok(json) => json,
            Err(error) => {
                warn!("{label}: не прочитан {}: {error}", list_path.display());
                return list;
            }
        };
        let entries = match parse_list(&json) {
            Ok(entries) => entries,
            Err(error) => {
                warn!("{label}: ошибка в {}: {error}", list_path.display());
                return list;
            }
        };

        let mut silent = Vec::new();
        for entry in entries {
            let sound = if !entry.file.is_empty() && dir_path.join(&entry.file).is_file() {
                Some(asset_server.load(format!("{dir}/{}", entry.file)))
            } else {
                silent.push(entry.id.clone());
                None
            };
            let id = entry.id.clone();
            if list
                .entries
                .insert(id.clone(), LoadedSound { entry, sound })
                .is_some()
            {
                warn!("{label}: идентификатор «{id}» повторяется, взята последняя запись");
            } else {
                list.order.push(id);
            }
        }

        info!(
            "{label}: записей {}, со звуком {}",
            list.order.len(),
            list.order.len() - silent.len().min(list.order.len())
        );
        if !silent.is_empty() {
            let shown: Vec<_> = silent.iter().take(8).map(String::as_str).collect();
            let rest = silent.len() - shown.len();
            let tail = if rest > 0 {
                format!(" и ещё {rest}")
            } else {
                String::new()
            };
            warn!("{label}: нет звуковых файлов: {}{tail}", shown.join(", "));
        }
        list
    }

    /// Запись по идентификатору.
    pub fn get(&self, id: &str) -> Option<&LoadedSound> {
        self.entries.get(id)
    }

    /// Все записи в порядке файла.
    pub fn iter(&self) -> impl Iterator<Item = &LoadedSound> {
        self.order.iter().filter_map(|id| self.entries.get(id))
    }

    /// Озвученные записи группы: сама основа и её варианты с суффиксами. Полный идентификатор
    /// записи (`mg_fire_2`) выбирает только её.
    pub fn group<'a>(&'a self, base: &'a str) -> impl Iterator<Item = &'a LoadedSound> + 'a {
        self.iter()
            .filter(move |loaded| loaded.sound.is_some() && in_group(&loaded.entry.id, base))
    }

    /// Есть ли в списке записи группы, пусть даже без звуковых файлов.
    pub fn has_group(&self, base: &str) -> bool {
        self.iter().any(|loaded| in_group(&loaded.entry.id, base))
    }
}

/// Маленький генератор случайных чисел для выбора вариантов и разброса высоты звука.
///
/// Как ресурс он принадлежит звуку и засевается часами. Игровая логика берёт случайность из
/// `rng::GameRng`, который засевается сидом мира.
#[derive(Resource)]
pub struct Dice(u64);

impl Default for Dice {
    fn default() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0x9E37_79B9, |elapsed| elapsed.as_nanos() as u64);
        Self(nanos | 1)
    }
}

impl Dice {
    /// Генератор с заданным сидом: одна и та же последовательность при каждом запуске.
    pub fn seeded(seed: u64) -> Self {
        // Шаг SplitMix64 разносит близкие сиды; младший бит не даёт состоянию стать нулевым.
        let mut mixed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((mixed ^ (mixed >> 31)) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Случайное число от 0 до `limit`, не включая `limit`. Для `limit == 0` возвращает 0.
    pub fn below(&mut self, limit: usize) -> usize {
        if limit == 0 {
            return 0;
        }
        (self.next() % limit as u64) as usize
    }

    /// Случайное число в диапазоне `low..high`.
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        let unit = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
        low + (high - low) * unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_is_parsed() {
        let json = r#"{ "lines": [
            { "id": "run_start", "event": "Старт забега", "text": "Поехали!", "file": "run_start.mp3" }
        ] }"#;
        let entries = parse_list(json).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "run_start");
        assert_eq!(entries[0].event, "Старт забега");
        assert_eq!(entries[0].text, "Поехали!");
        assert_eq!(entries[0].file, "run_start.mp3");
        assert_eq!(entries[0].tone, None);
    }

    #[test]
    fn entry_without_text_and_file_is_allowed() {
        let entries = parse_list(r#"{ "lines": [ { "id": "death" } ] }"#).unwrap();
        assert_eq!(entries[0].id, "death");
        assert!(entries[0].file.is_empty());
    }

    #[test]
    fn extra_fields_are_read_or_ignored() {
        let json = r#"{ "lines": [ { "id": "hit_3", "speaker": "dana", "tone": 2, "pri": 1,
            "kind": "voice", "delivery": "dry", "tts": "[deadpan] Ouch.", "accent": "" } ] }"#;
        let entries = parse_list(json).unwrap();
        assert_eq!(entries[0].speaker, "dana");
        assert_eq!(entries[0].tone, Some(2));
        assert_eq!(entries[0].pri, 1);
        assert_eq!(entries[0].kind, "voice");
    }

    #[test]
    fn broken_list_is_an_error() {
        assert!(parse_list(r#"{ "lines": [ { "text": "без id" } ] }"#).is_err());
        assert!(parse_list("не json").is_err());
    }

    #[test]
    fn base_id_drops_variant_suffixes() {
        assert_eq!(base_id("death"), "death");
        assert_eq!(base_id("kill_4"), "kill");
        assert_eq!(base_id("house_destroyed_t2"), "house_destroyed");
        assert_eq!(base_id("house_destroyed_t3_2"), "house_destroyed");
        assert_eq!(base_id("streak3_1"), "streak3");
        assert_eq!(base_id("crit_reply_4"), "crit_reply");
        assert_eq!(base_id("explosion_big_2"), "explosion_big");
        assert_eq!(base_id("_2"), "_2");
    }

    #[test]
    fn full_id_selects_single_entry() {
        assert!(in_group("mg_fire_2", "mg_fire"));
        assert!(in_group("mg_fire_2", "mg_fire_2"));
        assert!(!in_group("mg_fire_1", "mg_fire_2"));
        assert!(!in_group("explosion_big_1", "explosion"));
    }

    #[test]
    fn dice_stays_in_range() {
        let mut dice = Dice(12345);
        for _ in 0..1000 {
            assert!(dice.below(7) < 7);
            let value = dice.range(0.9, 1.1);
            assert!((0.9..1.1).contains(&value));
        }
        assert_eq!(dice.below(0), 0);
    }

    /// Списки из репозитория должны читаться игрой, идентификаторы в них не повторяются.
    #[test]
    fn repository_lists_are_valid() {
        let lists = [
            ("voice/lines.json", include_str!("../assets/voice/lines.json")),
            ("sfx/sounds.json", include_str!("../assets/sfx/sounds.json")),
            ("music/tracks.json", include_str!("../assets/music/tracks.json")),
        ];
        for (name, json) in lists {
            let entries = parse_list(json).unwrap_or_else(|error| panic!("{name}: {error}"));
            let mut ids: Vec<_> = entries.iter().map(|entry| entry.id.as_str()).collect();
            ids.sort_unstable();
            let total = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), total, "{name}: идентификаторы повторяются");
        }
    }

    /// У каждой записи списков из репозитория звуковой файл лежит рядом со списком.
    #[test]
    fn repository_lists_have_files() {
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let lists = [
            ("voice", include_str!("../assets/voice/lines.json")),
            ("sfx", include_str!("../assets/sfx/sounds.json")),
            ("music", include_str!("../assets/music/tracks.json")),
        ];
        for (dir, json) in lists {
            let missing: Vec<_> = parse_list(json)
                .unwrap()
                .into_iter()
                .filter(|entry| entry.file.is_empty() || !assets.join(dir).join(&entry.file).is_file())
                .map(|entry| entry.id)
                .collect();
            assert!(missing.is_empty(), "{dir}: нет звуковых файлов у записей {missing:?}");
        }
    }

    /// Каждый звуковой файл из списков разбирается звуковой библиотекой. Файл, который она
    /// не разберёт, уронит игру в момент проигрывания.
    #[test]
    fn repository_sound_files_decode() {
        use bevy::audio::Decodable;

        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let lists = [
            ("voice", include_str!("../assets/voice/lines.json")),
            ("sfx", include_str!("../assets/sfx/sounds.json")),
            ("music", include_str!("../assets/music/tracks.json")),
        ];
        let mut broken = Vec::new();
        for (dir, json) in lists {
            for entry in parse_list(json).unwrap() {
                let Ok(bytes) = std::fs::read(assets.join(dir).join(&entry.file)) else {
                    continue;
                };
                let source = AudioSource { bytes: bytes.into() };
                let samples = std::panic::catch_unwind(|| source.decoder().take(1000).count());
                if samples.unwrap_or(0) < 1000 {
                    broken.push(format!("{dir}/{}", entry.file));
                }
            }
        }
        assert!(broken.is_empty(), "звуковая библиотека не читает файлы {broken:?}");
    }
}
