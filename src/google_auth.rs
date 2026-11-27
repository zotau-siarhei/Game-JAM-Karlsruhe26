//! Необязательный вход через Google для мультиплеера: OAuth 2.0 по коду устройства (Device
//! Authorization Grant).
//!
//! Игра просит у Google код, показывает его, копирует в буфер обмена и открывает страницу
//! `google.com/device`; игрок вводит код и выбирает аккаунт, а поток входа тем временем спрашивает
//! Google, подтверждён ли вход. Обратная ссылка в приложение не нужна, поэтому так входят все
//! нативные сборки, и Android без Java-кода тоже: свёрнутую игру система может заморозить, поток
//! просто продолжит, когда игрок вернётся.
//!
//! Идентификатор игрока — `sub` из `id_token`. Он, имя, почта и токен обновления лежат в
//! `account.json` рядом с настройками. Сеть трогается только по [`AuthRequest`] из меню мультиплеера:
//! при запуске файл читается без запросов. В браузерной сборке сети нет: [`SignIn::Unsupported`].
//!
//! Клиент Google — типа «TVs and Limited Input devices»: ID и секрет задаются при сборке
//! переменными `SPACE_GUARD_GOOGLE_CLIENT_ID` и `SPACE_GUARD_GOOGLE_CLIENT_SECRET` (на компьютере —
//! и при запуске). Без них вход не настроен: [`SignIn::NotConfigured`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::frame::GameSet;

/// Имя файла входа рядом с настройками.
const FILE_NAME: &str = "account.json";

/// Строки ошибок на экране мультиплеера.
const NO_CONNECTION: &str = "NO CONNECTION TO GOOGLE";
const EXPIRED: &str = "THE CODE HAS EXPIRED";
const DECLINED: &str = "SIGN-IN WAS DECLINED";
const BAD_REPLY: &str = "UNEXPECTED REPLY FROM GOOGLE";

/// Плагин входа через Google.
pub struct GoogleAuthPlugin;

impl Plugin for GoogleAuthPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(initial_state())
            .init_resource::<Worker>()
            .add_message::<AuthRequest>()
            .add_systems(Update, (handle_requests, pump).chain().in_set(GameSet::Presentation));
    }
}

/// Аккаунт Google, которым вошёл игрок.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// `sub` из `id_token`: постоянный идентификатор игрока у Google.
    pub id: String,
    pub name: String,
    pub email: String,
}

/// Состояние входа: по нему рисуется экран мультиплеера.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub enum SignIn {
    /// Браузерная сборка: сети нет.
    Unsupported,
    /// В сборке нет клиента Google.
    NotConfigured,
    SignedOut,
    /// Игра просит у Google код.
    Requesting,
    /// Код показан, ждём подтверждения на странице Google. `copied` — код уже в буфере обмена.
    Waiting { code: String, url: String, copied: bool },
    SignedIn(Account),
    /// Вход не удался: строка для экрана.
    Failed(String),
}

impl SignIn {
    /// Идёт вход: код запрашивается или показан.
    pub fn in_progress(&self) -> bool {
        matches!(self, SignIn::Requesting | SignIn::Waiting { .. })
    }
}

/// Действие с входом из меню мультиплеера.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthRequest {
    /// Начать вход (и повторить после ошибки).
    SignIn,
    /// Ещё раз скопировать код и открыть страницу Google.
    OpenPage,
    /// Прервать вход.
    Cancel,
    /// Выйти: отозвать токен и стереть файл.
    SignOut,
}

/// Содержимое `account.json`.
#[derive(Serialize, Deserialize)]
struct Stored {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    /// Токен обновления: по нему потом берётся свежий `id_token` для сервера.
    #[serde(default)]
    refresh_token: String,
}

impl Stored {
    fn account(&self) -> Account {
        Account {
            id: self.id.clone(),
            name: self.name.clone(),
            email: self.email.clone(),
        }
    }
}

/// Поток входа: слот, куда он пишет новое состояние, и флаг отмены.
#[derive(Resource, Default)]
struct Worker {
    slot: Arc<Mutex<Option<SignIn>>>,
    cancel: Arc<AtomicBool>,
}

/// Клиент OAuth Google.
struct Client {
    id: String,
    secret: String,
}

/// Клиент из переменных сборки; на компьютере переменные окружения при запуске важнее.
fn client() -> Option<Client> {
    fn pick(name: &str, built: Option<&'static str>) -> Option<String> {
        #[cfg(not(any(target_os = "android", target_os = "ios", target_arch = "wasm32")))]
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            return Some(value);
        }
        let _ = name;
        built.filter(|value| !value.is_empty()).map(str::to_string)
    }
    Some(Client {
        id: pick("SPACE_GUARD_GOOGLE_CLIENT_ID", option_env!("SPACE_GUARD_GOOGLE_CLIENT_ID"))?,
        secret: pick("SPACE_GUARD_GOOGLE_CLIENT_SECRET", option_env!("SPACE_GUARD_GOOGLE_CLIENT_SECRET"))?,
    })
}

/// Состояние без входа: войти можно, только если есть клиент.
fn signed_out() -> SignIn {
    if cfg!(target_arch = "wasm32") {
        SignIn::Unsupported
    } else if client().is_none() {
        SignIn::NotConfigured
    } else {
        SignIn::SignedOut
    }
}

/// Состояние при запуске: сохранённый вход читается из файла, без сети.
fn initial_state() -> SignIn {
    if cfg!(target_arch = "wasm32") {
        return SignIn::Unsupported;
    }
    match load() {
        Some(stored) => SignIn::SignedIn(stored.account()),
        None => signed_out(),
    }
}

/// Путь к `account.json`: рядом с настройками и рекордами.
fn account_path() -> Option<PathBuf> {
    Some(crate::records::records_path()?.parent()?.join(FILE_NAME))
}

fn load() -> Option<Stored> {
    let json = std::fs::read_to_string(account_path()?).ok()?;
    serde_json::from_str::<Stored>(&json).ok().filter(|stored| !stored.id.is_empty())
}

/// Записывает файл через временный; на Unix читать его может только владелец.
fn save(stored: &Stored) -> std::io::Result<()> {
    use std::io::Write;
    let path = account_path().ok_or_else(|| std::io::Error::other("нет каталога данных"))?;
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let temporary = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(stored)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&temporary)?.write_all(json.as_bytes())?;
    std::fs::rename(&temporary, &path)
}

/// Выход: токен отзывается в фоне, файл стирается.
fn sign_out() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(stored) = load()
        && !stored.refresh_token.is_empty()
    {
        net::revoke(stored.refresh_token);
    }
    if let Some(path) = account_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Выполняет действия меню.
fn handle_requests(mut requests: MessageReader<AuthRequest>, mut state: ResMut<SignIn>, mut worker: ResMut<Worker>) {
    for request in requests.read() {
        match request {
            AuthRequest::SignIn => start(&mut state, &mut worker),
            AuthRequest::OpenPage => {
                if let SignIn::Waiting { code, url, .. } = &*state {
                    copy_text(code);
                    open_url(url);
                }
            }
            AuthRequest::Cancel => {
                worker.cancel.store(true, Ordering::Relaxed);
                if state.in_progress() {
                    *state = signed_out();
                }
            }
            AuthRequest::SignOut => {
                worker.cancel.store(true, Ordering::Relaxed);
                sign_out();
                *state = signed_out();
            }
        }
    }
}

/// Запускает поток входа; прежний, если был, отменяется и пишет уже в свой, никому не нужный слот.
fn start(state: &mut SignIn, worker: &mut Worker) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = worker;
        *state = SignIn::Unsupported;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(client) = client() else {
            *state = SignIn::NotConfigured;
            return;
        };
        worker.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let slot = Arc::new(Mutex::new(None));
        worker.cancel = cancel.clone();
        worker.slot = slot.clone();
        *state = SignIn::Requesting;
        let spawned = std::thread::Builder::new()
            .name("google-sign-in".into())
            .spawn(move || net::sign_in(&client, &cancel, &slot));
        if spawned.is_err() {
            *state = SignIn::Failed(NO_CONNECTION.into());
        }
    }
}

/// Переносит новое состояние из потока входа в ресурс.
fn pump(worker: Res<Worker>, mut state: ResMut<SignIn>) {
    let next = worker.slot.lock().ok().and_then(|mut slot| slot.take());
    if let Some(next) = next {
        state.set_if_neq(next);
    }
}

/// Аккаунт из `id_token`: полезная часть JWT (base64url → JSON). Подпись не проверяется: токен пришёл
/// прямо от Google по TLS, проверять её будет сервер.
fn account_from_id_token(token: &str) -> Option<Account> {
    let payload = token.split('.').nth(1)?;
    let claims: serde_json::Value = serde_json::from_slice(&base64url_decode(payload)?).ok()?;
    let text = |name: &str| claims[name].as_str().unwrap_or_default().to_string();
    let id = text("sub");
    (!id.is_empty()).then(|| Account {
        id,
        name: text("name"),
        email: text("email"),
    })
}

/// Декодирует base64url (и обычный base64), с выравниванием `=` или без него.
fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Копирует текст в буфер обмена. `false` — не вышло или платформа не умеет (iOS, браузер).
fn copy_text(text: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::android::copy_text(text);
        true
    }
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        desktop::copy_text(text)
    }
    #[cfg(not(any(target_os = "android", target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = text;
        false
    }
}

/// Открывает адрес в браузере. На iOS и в браузере — ничего: адрес виден на экране.
fn open_url(url: &str) {
    #[cfg(target_os = "android")]
    crate::android::open_url(url);
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    desktop::open_url(url);
    #[cfg(not(any(target_os = "android", target_os = "macos", target_os = "windows", target_os = "linux")))]
    let _ = url;
}

/// Буфер обмена и браузер на компьютере — системными командами.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod desktop {
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// Передаёт текст команде на вход; `true` — команда нашлась и завершилась успешно.
    fn pipe_to(program: &str, args: &[&str], text: &str) -> bool {
        let Ok(mut child) = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        child.wait().is_ok_and(|status| status.success()) && written
    }

    pub fn copy_text(text: &str) -> bool {
        if cfg!(target_os = "macos") {
            return pipe_to("pbcopy", &[], text);
        }
        if cfg!(target_os = "windows") {
            return pipe_to("clip", &[], text);
        }
        // Linux: wl-copy и xclip сами уходят в фон и держат буфер, пока его не займут.
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        (wayland && pipe_to("wl-copy", &[], text))
            || pipe_to("xclip", &["-selection", "clipboard"], text)
            || pipe_to("xsel", &["--clipboard", "--input"], text)
    }

    pub fn open_url(url: &str) {
        let mut command = if cfg!(target_os = "macos") {
            Command::new("open")
        } else if cfg!(target_os = "windows") {
            let mut command = Command::new("rundll32");
            command.arg("url.dll,FileProtocolHandler");
            command
        } else {
            Command::new("xdg-open")
        };
        match command.arg(url).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            // Ждём в стороне, чтобы не оставлять зомби-процесс.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(error) => bevy::log::warn!("Вход Google: браузер не открылся: {error}"),
        }
    }
}

/// Запросы к Google: код устройства, ожидание подтверждения, отзыв токена.
#[cfg(not(target_arch = "wasm32"))]
mod net {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use bevy::log::warn;
    use bevy::platform::time::Instant;
    use serde_json::Value;

    use super::{Account, BAD_REPLY, Client, DECLINED, EXPIRED, NO_CONNECTION, SignIn, Stored};

    const DEVICE_URL: &str = "https://oauth2.googleapis.com/device/code";
    const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
    const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
    const SCOPE: &str = "openid email profile";
    const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

    /// Клиент HTTP: ответы 4xx — не ошибка, в их JSON причина.
    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .into()
    }

    /// POST формой; код ответа и JSON.
    fn post(agent: &ureq::Agent, url: &str, form: &[(&str, &str)]) -> Result<(u16, Value), String> {
        let mut response = agent
            .post(url)
            .send_form(form.iter().copied())
            .map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string().map_err(|error| error.to_string())?;
        let json = serde_json::from_str(&body).map_err(|error| format!("{error}: {body}"))?;
        Ok((status, json))
    }

    /// Строка для экрана по ошибке Google.
    fn google_error(reply: &Value) -> String {
        match reply["error"].as_str().unwrap_or_default() {
            "invalid_client" | "unauthorized_client" => "GOOGLE REJECTED THIS BUILD'S CLIENT".into(),
            "" => BAD_REPLY.into(),
            other => format!("GOOGLE ERROR: {}", other.to_uppercase()),
        }
    }

    /// Весь вход; пишет состояния в `slot`, пока вход не отменён.
    pub fn sign_in(client: &Client, cancel: &AtomicBool, slot: &Arc<Mutex<Option<SignIn>>>) {
        let send = |state: SignIn| {
            if !cancel.load(Ordering::Relaxed)
                && let Ok(mut slot) = slot.lock()
            {
                *slot = Some(state);
            }
        };
        match run(client, cancel, &send) {
            Ok(Some(account)) => send(SignIn::SignedIn(account)),
            Ok(None) => {}
            Err(message) => send(SignIn::Failed(message)),
        }
    }

    /// Код устройства, показ и опрос. `Ok(None)` — вход отменён.
    fn run(client: &Client, cancel: &AtomicBool, send: &dyn Fn(SignIn)) -> Result<Option<Account>, String> {
        let agent = agent();
        let (status, code) = post(&agent, DEVICE_URL, &[("client_id", &client.id), ("scope", SCOPE)]).map_err(|error| {
            warn!("Вход Google: код не получен: {error}");
            NO_CONNECTION.to_string()
        })?;
        if status != 200 {
            warn!("Вход Google: код не выдан ({status}): {code}");
            return Err(google_error(&code));
        }
        let text = |name: &str| code[name].as_str().map(str::to_string);
        let (Some(device_code), Some(user_code), Some(url)) =
            (text("device_code"), text("user_code"), text("verification_url").or_else(|| text("verification_uri")))
        else {
            warn!("Вход Google: в ответе нет кода: {code}");
            return Err(BAD_REPLY.into());
        };
        let deadline = Instant::now() + Duration::from_secs(code["expires_in"].as_u64().unwrap_or(1800));
        let mut interval = code["interval"].as_u64().unwrap_or(5).max(1);
        let copied = super::copy_text(&user_code);
        super::open_url(&url);
        send(SignIn::Waiting {
            code: user_code,
            url,
            copied,
        });

        let form = [
            ("client_id", client.id.as_str()),
            ("client_secret", client.secret.as_str()),
            ("device_code", device_code.as_str()),
            ("grant_type", DEVICE_GRANT),
        ];
        loop {
            let wake = Instant::now() + Duration::from_secs(interval);
            while Instant::now() < wake {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            if Instant::now() >= deadline {
                return Err(EXPIRED.into());
            }
            // Сеть пропала (телефон сменил сеть): спросим снова через интервал.
            let (status, reply) = match post(&agent, TOKEN_URL, &form) {
                Ok(answer) => answer,
                Err(error) => {
                    warn!("Вход Google: опрос не удался: {error}");
                    continue;
                }
            };
            if status == 200 {
                return finish(&reply).map(Some);
            }
            match reply["error"].as_str().unwrap_or_default() {
                "authorization_pending" => {}
                "slow_down" => interval += 5,
                "access_denied" => return Err(DECLINED.into()),
                "expired_token" => return Err(EXPIRED.into()),
                _ => {
                    warn!("Вход Google: отказ ({status}): {reply}");
                    return Err(google_error(&reply));
                }
            }
        }
    }

    /// Аккаунт из ответа с токенами; вход сохраняется в файл.
    fn finish(reply: &Value) -> Result<Account, String> {
        let account = reply["id_token"]
            .as_str()
            .and_then(super::account_from_id_token)
            .ok_or_else(|| BAD_REPLY.to_string())?;
        let stored = Stored {
            id: account.id.clone(),
            name: account.name.clone(),
            email: account.email.clone(),
            refresh_token: reply["refresh_token"].as_str().unwrap_or_default().to_string(),
        };
        if let Err(error) = super::save(&stored) {
            warn!("Вход Google: {} не записан: {error}", super::FILE_NAME);
        }
        Ok(account)
    }

    /// Отзывает токен в фоне; ответ не важен: файл стирается в любом случае.
    pub fn revoke(token: String) {
        std::thread::spawn(move || {
            if let Err(error) = agent().post(REVOKE_URL).send_form([("token", token.as_str())]) {
                warn!("Вход Google: токен не отозван: {error}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_с_выравниванием_и_без() {
        assert_eq!(base64url_decode("eyJzdWIiOiIxMjMifQ").unwrap(), br#"{"sub":"123"}"#);
        assert_eq!(base64url_decode("eyJzdWIiOiIxMjMifQ==").unwrap(), br#"{"sub":"123"}"#);
        assert_eq!(base64url_decode("_-8").unwrap(), [0xff, 0xef]);
        assert!(base64url_decode("a b").is_none());
    }

    #[test]
    fn аккаунт_из_id_token() {
        // {"sub":"1077","email":"rover@example.com","name":"Луноход"}
        let payload = "eyJzdWIiOiIxMDc3IiwiZW1haWwiOiJyb3ZlckBleGFtcGxlLmNvbSIsIm5hbWUiOiLQm9GD0L3QvtGF0L7QtCJ9";
        let account = account_from_id_token(&format!("eyJhbGciOiJSUzI1NiJ9.{payload}.c2ln")).unwrap();
        assert_eq!(account.id, "1077");
        assert_eq!(account.email, "rover@example.com");
        assert_eq!(account.name, "Луноход");
        assert!(account_from_id_token("без точек").is_none());
        // Без sub идентификатора нет.
        assert!(account_from_id_token("a.eyJuYW1lIjoieCJ9.b").is_none());
    }
}
