//! Таблица рекордов: лучшие времена выживания, которые сохраняются между запусками.
//!
//! У каждого уровня сложности своя таблица: `records-easy.json`, `records.json` (нормальная —
//! прежний файл) и `records-hard.json` в каталоге данных пользователя. Файл, который не читается или
//! не разбирается, даёт пустую таблицу и не мешает запуску. Попадание в таблицу объявляет
//! сообщение [`RecordPlaced`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::difficulty::Difficulty;

/// Сколько мест в таблице.
pub const MAX_RECORDS: usize = 10;

/// Длина имени игрока.
pub const NAME_LENGTH: usize = 3;

/// Имя по умолчанию при первом запуске.
pub const DEFAULT_NAME: &str = "LUN";

/// Имя файла рекордов.
const FILE_NAME: &str = "records.json";

/// Результат забега попал в таблицу рекордов. Приходит один раз, когда открывается экран конца
/// забега (`run`), ещё до ввода имени. Повод для реплики штаба о рекорде.
#[derive(bevy::prelude::Message, Debug, Clone, Copy, PartialEq)]
pub struct RecordPlaced {
    /// Место в таблице, считая с нуля: 0 — лучшее время за всё время.
    pub place: usize,
    /// Время выживания, с.
    pub seconds: f32,
}

/// Один результат забега.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Имя из трёх букв или цифр.
    pub name: String,
    /// Время выживания в секундах.
    pub seconds: f64,
}

/// Таблица рекордов, отсортированная от лучшего времени к худшему.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Records {
    /// Имя, которое игрок ввёл в последний раз: подставляется по умолчанию.
    #[serde(default = "default_name")]
    pub last_name: String,
    #[serde(default)]
    pub records: Vec<Record>,
}

fn default_name() -> String {
    DEFAULT_NAME.to_string()
}

impl Default for Records {
    fn default() -> Self {
        Self {
            last_name: default_name(),
            records: Vec::new(),
        }
    }
}

/// Годится ли имя: ровно три латинские буквы или цифры.
pub fn is_valid_name(name: &str) -> bool {
    name.len() == NAME_LENGTH && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

impl Records {
    /// Разбирает текст файла. Некорректные записи отбрасываются, таблица сортируется и обрезается.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let mut table: Records = serde_json::from_str(json)?;
        table
            .records
            .retain(|record| is_valid_name(&record.name) && record.seconds.is_finite() && record.seconds >= 0.0);
        table
            .records
            .sort_by(|left, right| right.seconds.total_cmp(&left.seconds));
        table.records.truncate(MAX_RECORDS);
        if !is_valid_name(&table.last_name) {
            table.last_name = default_name();
        }
        Ok(table)
    }

    /// Текст файла для этой таблицы.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("таблица рекордов всегда записывается в JSON")
    }

    /// Место, которое заняло бы время `seconds`, считая с нуля. `None` — в таблицу не попадает.
    pub fn place_for(&self, seconds: f64) -> Option<usize> {
        let place = self
            .records
            .iter()
            .position(|record| seconds > record.seconds)
            .unwrap_or(self.records.len());
        (place < MAX_RECORDS).then_some(place)
    }

    /// Лучшее время в таблице.
    pub fn best(&self) -> Option<&Record> {
        self.records.first()
    }

    /// Вставляет результат и запоминает имя. Возвращает занятое место, считая с нуля,
    /// или `None`, если результат не попал в таблицу.
    pub fn insert(&mut self, name: &str, seconds: f64) -> Option<usize> {
        let name = name.to_ascii_uppercase();
        if !is_valid_name(&name) || !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        self.last_name.clone_from(&name);
        let place = self.place_for(seconds)?;
        self.records.insert(place, Record { name, seconds });
        self.records.truncate(MAX_RECORDS);
        Some(place)
    }

    /// Читает таблицу из файла. Если файла нет или он испорчен, возвращает пустую таблицу.
    pub fn load(path: &Path) -> Self {
        let Ok(json) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        Self::parse(&json).unwrap_or_else(|error| {
            bevy::log::warn!("Рекорды: файл {} не разобран: {error}", path.display());
            Self::default()
        })
    }

    /// Записывает таблицу через временный файл, чтобы прерванная запись не испортила её.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, self.to_json())?;
        std::fs::rename(&temporary, path)
    }
}

/// Путь к файлу рекордов по значениям переменных окружения. `None`, если каталог не определить.
fn records_path_from(home: Option<&str>, xdg_data_home: Option<&str>, macos: bool) -> Option<PathBuf> {
    let home = home.filter(|value| !value.is_empty());
    if macos {
        return Some(
            Path::new(home?)
                .join("Library/Application Support/Space Guard")
                .join(FILE_NAME),
        );
    }
    let data = match xdg_data_home.filter(|value| !value.is_empty()) {
        Some(data) => PathBuf::from(data),
        None => Path::new(home?).join(".local/share"),
    };
    Some(data.join("space-guard").join(FILE_NAME))
}

/// Имя файла рекордов уровня сложности. Нормальная — прежний `records.json`: рекорды, набранные
/// до появления сложности, становятся рекордами нормальной.
pub fn file_name_for(difficulty: Difficulty) -> &'static str {
    match difficulty {
        Difficulty::Easy => "records-easy.json",
        Difficulty::Normal => FILE_NAME,
        Difficulty::Hard => "records-hard.json",
    }
}

/// Путь к файлу рекордов уровня сложности. `None` — рекорды живут только в памяти.
pub fn records_path_for(difficulty: Difficulty) -> Option<PathBuf> {
    Some(records_path()?.with_file_name(file_name_for(difficulty)))
}

/// Путь к файлу рекордов нормальной сложности на этой системе. `None` — рекорды живут только
/// в памяти. Рядом лежат таблицы других уровней и файл настроек.
pub fn records_path() -> Option<PathBuf> {
    records_path_from(
        std::env::var("HOME").ok().as_deref(),
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        cfg!(target_os = "macos"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(count: usize) -> Records {
        let mut table = Records::default();
        for index in 0..count {
            // Времена 100, 90, 80, ...
            table.insert("AAA", 100.0 - 10.0 * index as f64).unwrap();
        }
        table
    }

    #[test]
    fn records_are_sorted_from_best() {
        let mut table = Records::default();
        assert_eq!(table.insert("AAA", 30.0), Some(0));
        assert_eq!(table.insert("BBB", 50.0), Some(0));
        assert_eq!(table.insert("CCC", 40.0), Some(1));
        let names: Vec<_> = table.records.iter().map(|record| record.name.as_str()).collect();
        assert_eq!(names, ["BBB", "CCC", "AAA"]);
        assert_eq!(table.best().unwrap().seconds, 50.0);
    }

    #[test]
    fn eleventh_result_pushes_out_the_last() {
        let mut table = table_with(MAX_RECORDS);
        let worst = table.records.last().unwrap().seconds;
        assert_eq!(table.insert("NEW", 55.0), Some(5));
        assert_eq!(table.records.len(), MAX_RECORDS);
        assert!(table.records.iter().all(|record| record.seconds > worst));
        assert_eq!(table.records[5].name, "NEW");
    }

    #[test]
    fn result_worse_than_the_table_is_not_inserted() {
        let mut table = table_with(MAX_RECORDS);
        let before = table.records.clone();
        assert_eq!(table.place_for(1.0), None);
        assert_eq!(table.insert("LOW", 1.0), None);
        assert_eq!(table.records, before);
        // Имя запоминается, даже если результат не попал в таблицу.
        assert_eq!(table.last_name, "LOW");
    }

    #[test]
    fn equal_time_goes_below_the_earlier_one() {
        let mut table = Records::default();
        table.insert("OLD", 60.0);
        assert_eq!(table.insert("NEW", 60.0), Some(1));
    }

    #[test]
    fn unparsable_file_gives_an_error_and_load_gives_empty_table() {
        assert!(Records::parse("не json").is_err());
        let path = std::env::temp_dir().join(format!("space-guard-broken-{}.json", std::process::id()));
        std::fs::write(&path, "{ испорчено").unwrap();
        assert_eq!(Records::load(&path), Records::default());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(Records::load(Path::new("/нет/такого/файла.json")), Records::default());
    }

    #[test]
    fn invalid_entries_are_dropped() {
        let json = r#"{ "last_name": "слишком длинное", "records": [
            { "name": "IVA", "seconds": 187.25 },
            { "name": "TOOLONG", "seconds": 300.0 },
            { "name": "AB", "seconds": 200.0 },
            { "name": "NEG", "seconds": -5.0 },
            { "name": "SER", "seconds": 250.5 }
        ] }"#;
        let table = Records::parse(json).unwrap();
        let names: Vec<_> = table.records.iter().map(|record| record.name.as_str()).collect();
        assert_eq!(names, ["SER", "IVA"]);
        assert_eq!(table.last_name, DEFAULT_NAME);
    }

    #[test]
    fn name_is_three_letters_or_digits() {
        assert!(is_valid_name("LUN"));
        assert!(is_valid_name("A1b"));
        assert!(!is_valid_name("LU"));
        assert!(!is_valid_name("LUNO"));
        assert!(!is_valid_name("L N"));
        assert!(!is_valid_name("ЛУН"));
        let mut table = Records::default();
        assert_eq!(table.insert("iva", 10.0), Some(0));
        assert_eq!(table.records[0].name, "IVA");
        assert_eq!(table.insert("??", 10.0), None);
    }

    #[test]
    fn save_and_load_give_the_same_table() {
        let directory = std::env::temp_dir().join(format!("space-guard-records-{}", std::process::id()));
        let path = directory.join("nested").join(FILE_NAME);
        let mut table = table_with(4);
        table.insert("IVA", 187.25);
        table.save(&path).unwrap();
        assert_eq!(Records::load(&path), table);
        // Повторная запись заменяет файл, временного файла не остаётся.
        table.insert("SER", 12.5);
        table.save(&path).unwrap();
        assert_eq!(Records::load(&path), table);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn path_depends_on_the_system() {
        assert_eq!(
            records_path_from(Some("/Users/a"), None, true),
            Some(PathBuf::from("/Users/a/Library/Application Support/Space Guard/records.json"))
        );
        assert_eq!(
            records_path_from(Some("/home/a"), Some("/data"), false),
            Some(PathBuf::from("/data/space-guard/records.json"))
        );
        assert_eq!(
            records_path_from(Some("/home/a"), None, false),
            Some(PathBuf::from("/home/a/.local/share/space-guard/records.json"))
        );
        assert_eq!(records_path_from(None, None, true), None);
        assert_eq!(records_path_from(None, None, false), None);
    }
}
