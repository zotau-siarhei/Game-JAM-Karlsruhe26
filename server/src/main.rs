//! Сервер Space Guard (`openspec/changes/room-server`). Служба комнат и ретранслятор появятся здесь;
//! пока есть проверка игрока — аккаунта Google ([`google_id`]) и отладочная команда к ней.
//!
//! `space_guard_server check-account [путь/account.json]` — берёт вход игры с этого компьютера,
//! получает у Google свежий `id_token` по токену обновления, как игра перед подключением, и проверяет
//! его так, как будет проверять служба. Клиент Google — переменные `SPACE_GUARD_GOOGLE_CLIENT_ID` и
//! `SPACE_GUARD_GOOGLE_CLIENT_SECRET` или файл `google-auth.txt` в корне репозитория.

mod google_id;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::Value;

/// Файл клиента Google для отладки (в `.gitignore`).
const CLIENT_FILE: &str = "google-auth.txt";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check-account") => check_account(args.get(1).map(PathBuf::from)),
        _ => Err("использование: space_guard_server check-account [путь/account.json]".to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Вход игры → свежий `id_token` → проверка, как у службы комнат.
fn check_account(path: Option<PathBuf>) -> Result<(), String> {
    let path = path.or_else(default_account_path).ok_or("не найден каталог данных игры")?;
    let stored: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).map_err(|error| format!("{}: {error} — сначала войдите в игре", path.display()))?,
    )
    .map_err(|error| format!("{}: {error}", path.display()))?;
    let refresh_token = stored["refresh_token"].as_str().filter(|token| !token.is_empty()).ok_or("в account.json нет токена обновления")?;
    let (client_id, client_secret) = client().ok_or("нет клиента Google: google-auth.txt или переменные окружения")?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into();
    let form = [
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    let mut response = agent
        .post("https://oauth2.googleapis.com/token")
        .send_form(form)
        .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let reply: Value = serde_json::from_str(&response.body_mut().read_to_string().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let id_token = reply["id_token"]
        .as_str()
        .ok_or_else(|| format!("Google не дал id_token ({status}): {}", reply["error"]))?;
    println!("id_token получен по токену обновления");

    let mut keys = google_id::KeyCache::new();
    let identity = keys.verify(id_token, &[client_id.as_str()]).map_err(|error| format!("id_token отклонён: {error:?}"))?;
    println!("id_token проверен по ключам Google: sub {}", identity.sub);
    let same = stored["id"].as_str() == Some(identity.sub.as_str());
    println!("совпадает с идентификатором в account.json: {}", if same { "да" } else { "НЕТ" });
    if same { Ok(()) } else { Err("sub не совпал".into()) }
}

/// `account.json` игры на этом компьютере: рядом с настройками и рекордами (`records::records_path`).
fn default_account_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    if cfg!(target_os = "macos") {
        return Some(Path::new(&home).join("Library/Application Support/Space Guard/account.json"));
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&home).join(".local/share"));
    Some(data.join("space-guard/account.json"))
}

/// Клиент Google: переменные окружения, иначе `google-auth.txt` вверх от текущего каталога.
fn client() -> Option<(String, String)> {
    let file = std::env::current_dir()
        .ok()?
        .ancestors()
        .find_map(|directory| std::fs::read_to_string(directory.join(CLIENT_FILE)).ok())
        .unwrap_or_default();
    let pick = |name: &str| {
        std::env::var(name).ok().filter(|value| !value.is_empty()).or_else(|| {
            file.lines().find_map(|line| {
                let line = line.trim();
                let (key, value) = line.strip_prefix("export ").unwrap_or(line).split_once('=')?;
                (key.trim() == name).then(|| value.trim().trim_matches(['\'', '"']).to_string())
            })
        })
    };
    Some((pick("SPACE_GUARD_GOOGLE_CLIENT_ID")?, pick("SPACE_GUARD_GOOGLE_CLIENT_SECRET")?))
}
