use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

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
        let secret = random_hex(32);
        let hash = mds_update::sha256_hex(secret.as_bytes());
        if let Some(parent) = self.mcp_path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(&self.mcp_path, hash).map_err(|err| err.to_string())?;
        Ok(secret)
    }

    pub fn revoke_mcp(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.mcp_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }

    pub fn mcp_ok(&self, bearer: &str) -> bool {
        let Ok(hash) = std::fs::read_to_string(&self.mcp_path) else {
            return false;
        };
        same(mds_update::sha256_hex(bearer.as_bytes()).as_bytes(), hash.trim().as_bytes())
    }
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

pub fn login_page(error: bool) -> String {
    let note = if error {
        "<p class=\"error\">Имя или пароль не подошли.</p>"
    } else {
        ""
    };
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Кабинет</title>
<body><main><h1>Кабинет ноды</h1>{note}
<form method=\"post\" action=\"/login\">
<label>Имя <input name=\"user\" autocomplete=\"username\"></label>
<label>Пароль <input name=\"password\" type=\"password\" autocomplete=\"current-password\"></label>
<button type=\"submit\">Войти</button>
</form></main></body>"
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
        };
        assert!(door.allow("login", 2, 60));
        assert!(door.allow("login", 2, 60));
        assert!(!door.allow("login", 2, 60));
    }
}
