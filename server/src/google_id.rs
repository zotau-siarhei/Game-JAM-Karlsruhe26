//! Игрок — аккаунт Google: проверка `id_token`, который игра шлёт службе комнат первым пакетом.
//!
//! Проверка локальная, без запроса к Google на каждый вход: подпись RS256 ключом из JWKS Google
//! (`kid` из заголовка), `aud` — один из клиентов игры, `iss` — Google, `exp` не прошёл (запас
//! [`CLOCK_SKEW`] на разницу часов). Идентификатор игрока — `sub`; почта и имя не используются.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde_json::Value;

/// Ключи Google для `id_token`.
pub const JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

/// Издатели `id_token` Google.
const ISSUERS: [&str; 2] = ["accounts.google.com", "https://accounts.google.com"];

/// Запас на разницу часов сервера и Google, с.
const CLOCK_SKEW: u64 = 300;

/// Не перечитывать ключи из-за неизвестного `kid` чаще этого.
const REFETCH_PAUSE: Duration = Duration::from_secs(60);

/// Срок кэша ключей, если Google не прислал `Cache-Control: max-age`.
const DEFAULT_MAX_AGE: Duration = Duration::from_secs(6 * 3600);

/// Проверенный игрок.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// `sub` Google: постоянный идентификатор игрока.
    pub sub: String,
}

/// Почему токен не принят.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    /// Не JWT, не RS256 или нет нужных полей.
    Malformed,
    /// Ключа с таким `kid` у Google нет.
    UnknownKey,
    BadSignature,
    WrongAudience,
    WrongIssuer,
    Expired,
    /// Ключи Google не скачались, а кэш пуст.
    KeysUnavailable(String),
}

/// Открытый ключ RSA из JWK: модуль и экспонента, big-endian.
struct RsaKey {
    n: Vec<u8>,
    e: Vec<u8>,
}

/// Набор ключей Google по `kid`.
pub struct GoogleKeys {
    keys: HashMap<String, RsaKey>,
}

impl GoogleKeys {
    /// Ключи из ответа JWKS. Ключи не RSA пропускаются.
    pub fn from_jwks(json: &str) -> Result<Self, IdError> {
        let jwks: Value = serde_json::from_str(json).map_err(|_| IdError::Malformed)?;
        let keys = jwks["keys"]
            .as_array()
            .ok_or(IdError::Malformed)?
            .iter()
            .filter(|key| key["kty"] == "RSA")
            .filter_map(|key| {
                let kid = key["kid"].as_str()?.to_string();
                let n = base64url_decode(key["n"].as_str()?)?;
                let e = base64url_decode(key["e"].as_str()?)?;
                Some((kid, RsaKey { n, e }))
            })
            .collect();
        Ok(Self { keys })
    }

    /// Проверяет `id_token` на момент `now` (секунды Unix) для клиентов `audiences`.
    pub fn verify(&self, token: &str, audiences: &[&str], now: u64) -> Result<Identity, IdError> {
        let mut parts = token.split('.');
        let (Some(header), Some(payload), Some(signature), None) = (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(IdError::Malformed);
        };
        let header = decode_json(header)?;
        if header["alg"] != "RS256" {
            return Err(IdError::Malformed);
        }
        let kid = header["kid"].as_str().ok_or(IdError::Malformed)?;
        let key = self.keys.get(kid).ok_or(IdError::UnknownKey)?;
        let signature = base64url_decode(signature).ok_or(IdError::Malformed)?;
        let signed = &token[..header_and_payload_len(token)];
        RsaPublicKeyComponents { n: &key.n, e: &key.e }
            .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
            .map_err(|_| IdError::BadSignature)?;

        // Подпись верна: поля писал Google.
        let claims = decode_json(payload)?;
        let audience = claims["aud"].as_str().ok_or(IdError::Malformed)?;
        if !audiences.contains(&audience) {
            return Err(IdError::WrongAudience);
        }
        if !claims["iss"].as_str().is_some_and(|issuer| ISSUERS.contains(&issuer)) {
            return Err(IdError::WrongIssuer);
        }
        let expires = claims["exp"].as_u64().ok_or(IdError::Malformed)?;
        if expires + CLOCK_SKEW < now {
            return Err(IdError::Expired);
        }
        let sub = claims["sub"].as_str().filter(|sub| !sub.is_empty()).ok_or(IdError::Malformed)?;
        Ok(Identity { sub: sub.to_string() })
    }
}

/// Длина «заголовок.полезная часть» — то, что подписано.
fn header_and_payload_len(token: &str) -> usize {
    token.rfind('.').unwrap_or(token.len())
}

/// Кэш ключей Google для службы: перечитывает их по сроку из `Cache-Control` и при неизвестном `kid`
/// (Google публикует новый ключ заранее, но кэш мог устареть), не чаще [`REFETCH_PAUSE`].
pub struct KeyCache {
    keys: Option<GoogleKeys>,
    valid_until: Option<Instant>,
    fetched_at: Option<Instant>,
}

impl Default for KeyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyCache {
    pub fn new() -> Self {
        Self {
            keys: None,
            valid_until: None,
            fetched_at: None,
        }
    }

    /// Проверяет токен, при нужде скачивая ключи. Блокирует поток на запрос к Google.
    pub fn verify(&mut self, token: &str, audiences: &[&str]) -> Result<Identity, IdError> {
        let now = unix_now();
        let stale = self.valid_until.is_none_or(|until| Instant::now() >= until);
        if stale {
            self.refresh()?;
        }
        let result = self.keys.as_ref().ok_or(IdError::KeysUnavailable("нет ключей".into()))?.verify(token, audiences, now);
        if result == Err(IdError::UnknownKey) && self.fetched_at.is_none_or(|at| at.elapsed() >= REFETCH_PAUSE) {
            self.refresh()?;
            return self.keys.as_ref().ok_or(IdError::UnknownKey)?.verify(token, audiences, now);
        }
        result
    }

    /// Скачивает ключи. Ошибка при непустом кэше не страшна: работаем на прежних ключах.
    fn refresh(&mut self) -> Result<(), IdError> {
        self.fetched_at = Some(Instant::now());
        match fetch_jwks() {
            Ok((keys, max_age)) => {
                self.keys = Some(keys);
                self.valid_until = Some(Instant::now() + max_age);
                Ok(())
            }
            Err(error) if self.keys.is_some() => {
                eprintln!("Ключи Google не обновлены, работаем на прежних: {error:?}");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

/// Скачивает JWKS Google и срок его жизни из `Cache-Control: max-age`.
fn fetch_jwks() -> Result<(GoogleKeys, Duration), IdError> {
    let unavailable = |error: ureq::Error| IdError::KeysUnavailable(error.to_string());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut response = agent.get(JWKS_URL).call().map_err(unavailable)?;
    let max_age = response
        .headers()
        .get("cache-control")
        .and_then(|value| value.to_str().ok())
        .and_then(max_age)
        .unwrap_or(DEFAULT_MAX_AGE);
    let body = response.body_mut().read_to_string().map_err(unavailable)?;
    Ok((GoogleKeys::from_jwks(&body)?, max_age))
}

/// `max-age` из `Cache-Control`.
fn max_age(header: &str) -> Option<Duration> {
    header
        .split(',')
        .find_map(|part| part.trim().strip_prefix("max-age="))
        .and_then(|seconds| seconds.parse().ok())
        .map(Duration::from_secs)
}

/// Секунды Unix сейчас.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn decode_json(part: &str) -> Result<Value, IdError> {
    serde_json::from_slice(&base64url_decode(part).ok_or(IdError::Malformed)?).map_err(|_| IdError::Malformed)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Ключ и токены, подписанные им и чужим ключом (`tests/google_id_fixture.json`, собран
    /// Python `cryptography`): закрытого ключа в репозитории нет.
    fn fixture() -> (GoogleKeys, Value) {
        let fixture: Value = serde_json::from_str(include_str!("../tests/google_id_fixture.json")).unwrap();
        let keys = GoogleKeys::from_jwks(&fixture["jwks"].to_string()).unwrap();
        (keys, fixture)
    }

    fn check(name: &str) -> Result<Identity, IdError> {
        let (keys, fixture) = fixture();
        let audience = fixture["audience"].as_str().unwrap();
        keys.verify(fixture[name].as_str().unwrap(), &[audience], fixture["now"].as_u64().unwrap())
    }

    #[test]
    fn верный_токен_даёт_sub() {
        assert_eq!(check("valid").unwrap().sub, "110169484474386276334");
        assert!(check("short_issuer").is_ok());
    }

    #[test]
    fn чужие_и_испорченные_токены_отклоняются() {
        assert_eq!(check("wrong_audience"), Err(IdError::WrongAudience));
        assert_eq!(check("wrong_issuer"), Err(IdError::WrongIssuer));
        assert_eq!(check("expired"), Err(IdError::Expired));
        assert_eq!(check("foreign_signature"), Err(IdError::BadSignature));
        assert_eq!(check("tampered"), Err(IdError::BadSignature));
        assert_eq!(check("unknown_kid"), Err(IdError::UnknownKey));
    }

    #[test]
    fn мусор_вместо_токена() {
        let (keys, _) = fixture();
        assert_eq!(keys.verify("abc", &["x"], 0), Err(IdError::Malformed));
        assert_eq!(keys.verify("a.b.c.d", &["x"], 0), Err(IdError::Malformed));
    }

    #[test]
    fn срок_кэша_из_cache_control() {
        assert_eq!(max_age("public, max-age=19832, must-revalidate"), Some(Duration::from_secs(19832)));
        assert_eq!(max_age("no-store"), None);
    }
}
