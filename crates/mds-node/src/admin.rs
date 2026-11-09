use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::{Deserialize, Serialize};

const SESSION_SECONDS: u64 = 12 * 60 * 60;

struct Session {
    expires: u64,
    csrf: String,
}

pub struct Door {
    pub user: String,
    password_hash: String,
    pub mcp_path: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
    hits: Mutex<HashMap<String, (u32, u64)>>,
    book: Mutex<()>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpKey {
    pub id: String,
    pub hash: String,
    pub issued_at: u64,
    #[serde(default)]
    pub revoked_at: Option<u64>,
}

pub fn hash_admin_password(password: &str) -> Result<String, String> {
    if password.trim().is_empty() {
        return Err("пароль пустой".into());
    }
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| "пароль не хешируется".into())
}

pub fn door_from_env(mcp_path: PathBuf) -> Option<Door> {
    let user = std::env::var("MDS_ADMIN_USER").unwrap_or_default();
    let password_hash = std::env::var("MDS_ADMIN_PASSWORD_HASH").unwrap_or_default();
    if user.trim().is_empty() || password_hash.trim().is_empty() {
        return None;
    }
    Some(Door {
        user,
        password_hash,
        mcp_path,
        sessions: Mutex::new(HashMap::new()),
        hits: Mutex::new(HashMap::new()),
        book: Mutex::new(()),
    })
}

impl Door {
    pub fn allow(&self, key: &str, limit: u32, window_secs: u64) -> bool {
        let now = now_secs();
        let mut hits = self.hits.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = hits.entry(key.to_string()).or_insert((0, now));
        if now.saturating_sub(entry.1) >= window_secs {
            *entry = (0, now);
        }
        if entry.0 >= limit {
            return false;
        }
        entry.0 += 1;
        true
    }

    pub fn login(&self, user: &str, password: &str) -> Option<(String, String)> {
        if !same(user.as_bytes(), self.user.as_bytes()) {
            let _ = password_matches(password, &self.password_hash);
            return None;
        }
        if !password_matches(password, &self.password_hash) {
            return None;
        }
        let token = random_hex(32);
        let csrf = random_hex(16);
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(
                token.clone(),
                Session {
                    expires: now_secs() + SESSION_SECONDS,
                    csrf: csrf.clone(),
                },
            );
        Some((token, csrf))
    }

    pub fn session(&self, token: &str) -> Option<String> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|poison| poison.into_inner());
        let now = now_secs();
        sessions.retain(|_, session| session.expires > now);
        sessions.get(token).map(|session| session.csrf.clone())
    }

    pub fn logout(&self, token: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(token);
    }

    pub fn issue_mcp(&self) -> Result<String, String> {
        let _guard = self.book.lock().unwrap_or_else(|poison| poison.into_inner());
        let secret = random_hex(32);
        let hash = mds_update::sha256_hex(secret.as_bytes());
        let id = hash.chars().take(8).collect::<String>();
        let mut keys = load_keys(&self.mcp_path);
        keys.push(McpKey {
            id,
            hash,
            issued_at: now_secs(),
            revoked_at: None,
        });
        save_keys(&self.mcp_path, &keys)?;
        Ok(secret)
    }

    pub fn revoke_mcp(&self, id: &str) -> Result<(), String> {
        let _guard = self.book.lock().unwrap_or_else(|poison| poison.into_inner());
        let id = id.trim();
        if id.is_empty() {
            return Err("нет номера ключа".into());
        }
        let mut keys = load_keys(&self.mcp_path);
        let mut found = false;
        let now = now_secs();
        for key in &mut keys {
            if key.id == id && key.revoked_at.is_none() {
                key.revoked_at = Some(now);
                found = true;
            }
        }
        if !found {
            return Err("такого ключа нет".into());
        }
        save_keys(&self.mcp_path, &keys)
    }

    pub fn mcp_keys(&self) -> Vec<McpKey> {
        let _guard = self.book.lock().unwrap_or_else(|poison| poison.into_inner());
        let mut keys = load_keys(&self.mcp_path);
        keys.reverse();
        keys
    }

    pub fn mcp_ok(&self, bearer: &str) -> bool {
        let _guard = self.book.lock().unwrap_or_else(|poison| poison.into_inner());
        let want = mds_update::sha256_hex(bearer.as_bytes());
        load_keys(&self.mcp_path).iter().any(|key| {
            key.revoked_at.is_none() && same(key.hash.as_bytes(), want.as_bytes())
        })
    }
}

fn load_keys(path: &Path) -> Vec<McpKey> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if text.starts_with('[') {
        return serde_json::from_str(text).unwrap_or_default();
    }
    if text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let issued_at = std::fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|span| span.as_secs())
            .unwrap_or_else(now_secs);
        return vec![McpKey {
            id: text.chars().take(8).collect(),
            hash: text.to_ascii_lowercase(),
            issued_at,
            revoked_at: None,
        }];
    }
    Vec::new()
}

fn save_keys(path: &Path, keys: &[McpKey]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let body = serde_json::to_string_pretty(keys).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body).map_err(|err| err.to_string())?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|err| err.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Москва, UTC+3. Ключ и дата выдачи без адреса человека.
pub fn format_stamp(secs: u64) -> String {
    let secs = secs.saturating_add(3 * 3600);
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{day:02}.{month:02}.{year} {hour:02}:{minute:02}")
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

fn password_matches(password: &str, encoded: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(encoded) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn same(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (one, other) in left.iter().zip(right) {
        diff |= one ^ other;
    }
    diff == 0
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|span| span.as_secs())
        .unwrap_or(0)
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("случайные байты");
    buf.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn cookie_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("mds_session=") {
            return Some(value.to_string());
        }
    }
    None
}

pub fn login_page(error: bool, base: &str) -> String {
    let note = if error {
        "<p class=\"error\">Имя или пароль не подошли.</p>"
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Кабинет</title>
<style>
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; font-family: Arial, Helvetica, sans-serif; color: #202124; background: #f8f9fa; }}
main {{ width: min(100% - 32px, 380px); padding: 32px 28px 28px; background: #fff; border: 1px solid #dadce0; border-radius: 16px; box-shadow: 0 1px 6px rgba(32,33,36,.12); box-sizing: border-box; }}
h1 {{ margin: 0 0 22px; text-align: center; font-family: Georgia, "Times New Roman", serif; font-size: 28px; font-weight: 500; color: #5c3a12; }}
form {{ display: grid; gap: 14px; }}
label {{ display: grid; gap: 6px; font-size: 14px; color: #3c4043; }}
input {{ height: 44px; border: 1px solid #dfe1e5; border-radius: 22px; padding: 0 16px; font-size: 16px; font-family: inherit; box-sizing: border-box; }}
input:focus {{ outline: none; border-color: #e6a817; box-shadow: 0 0 0 3px rgba(230,168,23,.22); }}
button {{ height: 44px; margin-top: 4px; border: 0; border-radius: 22px; background: #1a73e8; color: #fff; font-size: 15px; cursor: pointer; }}
p.error {{ margin: 0 0 14px; text-align: center; color: #a50e0e; font-size: 14px; }}
</style>
</head>
<body>
<main>
<h1>Кабинет ноды</h1>
{note}
<form method="post" action="{base}/login">
<label>Имя <input name="user" autocomplete="username" required></label>
<label>Пароль <input name="password" type="password" autocomplete="current-password" required></label>
<button type="submit">Войти</button>
</form>
</main>
</body>
</html>"#
    )
}

pub fn read_map(path: &Path) -> Option<mds_wire::NetworkMap> {
    let text = std::fs::read_to_string(path).ok()?;
    let map: mds_wire::NetworkMap = serde_json::from_str(&text).ok()?;
    mds_update::verify_network(&map).ok()?;
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_the_password_and_rejects_another() {
        let hash = hash_admin_password("correct horse").unwrap();
        assert!(!hash.contains("correct horse"));
        let door = Door {
            user: "keeper".into(),
            password_hash: hash,
            mcp_path: PathBuf::from("unused"),
            sessions: Mutex::new(HashMap::new()),
            hits: Mutex::new(HashMap::new()),
            book: Mutex::new(()),
        };
        assert!(door.login("keeper", "correct horse").is_some());
        assert!(door.login("keeper", "wrong").is_none());
        assert!(door.login("other", "correct horse").is_none());
    }

    #[test]
    fn login_slows_after_the_limit() {
        let door = Door {
            user: "a".into(),
            password_hash: hash_admin_password("b").unwrap(),
            mcp_path: PathBuf::from("unused"),
            sessions: Mutex::new(HashMap::new()),
            hits: Mutex::new(HashMap::new()),
            book: Mutex::new(()),
        };
        assert!(door.allow("login", 2, 60));
        assert!(door.allow("login", 2, 60));
        assert!(!door.allow("login", 2, 60));
    }

    fn test_door(path: PathBuf) -> Door {
        Door {
            user: "keeper".into(),
            password_hash: hash_admin_password("secret").unwrap(),
            mcp_path: path,
            sessions: Mutex::new(HashMap::new()),
            hits: Mutex::new(HashMap::new()),
            book: Mutex::new(()),
        }
    }

    #[test]
    fn issued_keys_keep_history_and_revoke_one() {
        let dir = std::env::temp_dir().join(format!("mds-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let door = test_door(dir.join("mcp.hash"));
        let first = door.issue_mcp().unwrap();
        let second = door.issue_mcp().unwrap();
        assert!(door.mcp_ok(&first));
        assert!(door.mcp_ok(&second));
        let keys = door.mcp_keys();
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().all(|key| key.revoked_at.is_none()));
        door.revoke_mcp(&keys[0].id).unwrap();
        assert!(!door.mcp_ok(&second));
        assert!(door.mcp_ok(&first));
        let keys = door.mcp_keys();
        assert_eq!(keys.iter().filter(|key| key.revoked_at.is_some()).count(), 1);
        assert!(format_stamp(0).starts_with("01.01.1970 03:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_single_hash_file_still_opens() {
        let dir = std::env::temp_dir().join(format!("mds-mcp-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.hash");
        let secret = random_hex(32);
        let hash = mds_update::sha256_hex(secret.as_bytes());
        std::fs::write(&path, &hash).unwrap();
        let door = test_door(path);
        assert!(door.mcp_ok(&secret));
        let keys = door.mcp_keys();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, hash.chars().take(8).collect::<String>());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
