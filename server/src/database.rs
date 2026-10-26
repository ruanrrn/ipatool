use rusqlite::{params, Connection, OptionalExtension, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Option<i64>,
    pub token: String,
    pub email: String,
    pub region: String,
    pub guid: Option<String>,
    pub cookie_user: Option<String>,
    pub cookies: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub id: Option<i64>,
    pub email: String,
    pub password_encrypted: String,
    pub key_id: String,
    pub iv: String,
    pub auth_tag: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUser {
    pub id: Option<i64>,
    pub username: String,
    pub password_hash: String,
    pub is_default: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: Option<i64>,
    pub token: String,
    pub username: String,
    pub created_at: Option<String>,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubToken {
    pub id: Option<i64>,
    pub username: String,
    pub token: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// WebDAV 上传配置（单行表，id 固定为 1）。
/// 下载完成后若 enabled=true，自动把 IPA PUT 到 url+remote_path。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDAVConfig {
    pub enabled: bool,
    pub url: String,
    pub username: String,
    pub password: String,
    pub remote_path: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionKey {
    pub id: Option<i64>,
    pub key_id: String,
    pub key_value: String,
    pub is_current: bool,
    pub created_at: Option<String>,
    pub last_rotation: i64,
    pub next_rotation: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: Option<i64>,
    pub job_id: Option<String>,
    pub app_version_id: Option<String>,
    pub app_name: String,
    pub app_id: String,
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    pub account_email: String,
    pub account_region: Option<String>,
    pub download_date: Option<String>,
    pub status: String,
    pub file_size: Option<i64>,
    pub file_path: Option<String>,
    pub install_url: Option<String>,
    pub artwork_url: Option<String>,
    pub artist_name: Option<String>,
    pub progress: Option<i64>,
    pub error: Option<String>,
    pub package_kind: Option<String>,
    pub ota_installable: Option<bool>,
    pub install_method: Option<String>,
    pub inspection_json: Option<String>,
    pub delisted: Option<bool>,
    pub created_at: Option<String>,
}

pub struct NewSubscription<'a> {
    pub app_id: &'a str,
    pub app_name: &'a str,
    pub bundle_id: Option<&'a str>,
    pub account_email: &'a str,
    pub account_region: Option<&'a str>,
    pub artwork_url: Option<&'a str>,
    pub artist_name: Option<&'a str>,
}

pub struct Database {
    connection: std::sync::Mutex<Connection>,
}

impl Database {
    pub fn new(db_path: &str) -> Result<Self> {
        let path = Path::new(db_path);

        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).unwrap();
            }
        }

        let connection = Connection::open(path)?;

        // PRAGMA 语句使用 query_row 而不是 execute
        let _ = connection.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        });
        let _ = connection.query_row("PRAGMA foreign_keys = ON", [], |row| row.get::<_, i32>(0));
        // WAL + NORMAL is crash-safe (no corruption) and avoids an fsync on every commit; the
        // busy timeout covers external tools (sqlite3 CLI, reset-admin-password) holding a lock.
        let _ = connection.execute_batch("PRAGMA synchronous = NORMAL;");
        let _ = connection.busy_timeout(std::time::Duration::from_secs(5));

        Self::create_tables(&connection)?;
        Self::migrate_tables(&connection)?;
        Self::seed_default_admin(&connection)?;

        Ok(Database {
            connection: std::sync::Mutex::new(connection),
        })
    }

    fn create_tables(conn: &Connection) -> Result<()> {
        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS accounts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                token TEXT UNIQUE NOT NULL,
                email TEXT NOT NULL,
                region TEXT DEFAULT 'US',
                guid TEXT,
                cookie_user TEXT,
                cookies TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                auth_session TEXT
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS credentials (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                email TEXT UNIQUE NOT NULL,
                password_encrypted TEXT NOT NULL,
                key_id TEXT NOT NULL,
                iv TEXT NOT NULL,
                auth_tag TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS encryption_keys (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                key_id TEXT UNIQUE NOT NULL,
                key_value TEXT NOT NULL,
                is_current BOOLEAN DEFAULT FALSE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                last_rotation INTEGER NOT NULL,
                next_rotation INTEGER NOT NULL
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS download_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT,
                app_version_id TEXT,
                app_name TEXT NOT NULL,
                app_id TEXT NOT NULL,
                bundle_id TEXT,
                version TEXT,
                account_email TEXT NOT NULL,
                account_region TEXT,
                download_date DATETIME DEFAULT CURRENT_TIMESTAMP,
                status TEXT DEFAULT 'completed',
                file_size INTEGER,
                file_path TEXT,
                install_url TEXT,
                artwork_url TEXT,
                artist_name TEXT,
                progress INTEGER DEFAULT 0,
                error TEXT,
                package_kind TEXT,
                ota_installable INTEGER,
                install_method TEXT,
                inspection_json TEXT,
                retry_count INTEGER DEFAULT 0,
                max_retries INTEGER DEFAULT 5,
                download_speed REAL,
                resume_position INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS app_subscriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                app_id TEXT NOT NULL,
                app_name TEXT NOT NULL,
                bundle_id TEXT,
                account_email TEXT NOT NULL,
                account_region TEXT,
                current_version TEXT,
                artwork_url TEXT,
                artist_name TEXT,
                subscribed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                last_checked DATETIME,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(app_id, account_email)
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS batch_download_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_name TEXT NOT NULL,
                status TEXT DEFAULT 'pending',
                total_count INTEGER DEFAULT 0,
                completed_count INTEGER DEFAULT 0,
                failed_count INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                completed_at DATETIME
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS batch_download_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                batch_id INTEGER NOT NULL,
                app_id TEXT NOT NULL,
                app_name TEXT,
                version TEXT,
                account_email TEXT,
                status TEXT DEFAULT 'pending',
                progress INTEGER DEFAULT 0,
                error TEXT,
                retry_count INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (batch_id) REFERENCES batch_download_tasks(id) ON DELETE CASCADE
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS admin_users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                is_default BOOLEAN DEFAULT TRUE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                token TEXT UNIQUE NOT NULL,
                username TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                expires_at DATETIME NOT NULL,
                FOREIGN KEY (username) REFERENCES admin_users(username) ON DELETE CASCADE
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS github_tokens (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT UNIQUE NOT NULL,
                token TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (username) REFERENCES admin_users(username) ON DELETE CASCADE
            )
        ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS webdav_config (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                enabled INTEGER NOT NULL DEFAULT 0,
                url TEXT NOT NULL DEFAULT '',
                username TEXT NOT NULL DEFAULT '',
                password TEXT NOT NULL DEFAULT '',
                remote_path TEXT NOT NULL DEFAULT '/',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            ",
            [],
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS purchase_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                adam_id TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'apple_api',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(account_id, adam_id)
            )
            ",
            [],
        )?;

        Ok(())
    }

    fn migrate_tables(conn: &Connection) -> Result<()> {
        let table_info: Vec<(i32, String, String, bool, i32, bool)> = conn
            .prepare("PRAGMA table_info(accounts)")?
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let has_region = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "region");
        if !has_region {
            let _ = conn.execute(
                "ALTER TABLE accounts ADD COLUMN region TEXT DEFAULT 'US'",
                [],
            );
        }

        // 加密后的 Apple 登录态（dsid / passwordToken / guid），用于重启后恢复会话。
        let has_auth_session = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "auth_session");
        if !has_auth_session {
            let _ = conn.execute("ALTER TABLE accounts ADD COLUMN auth_session TEXT", []);
        }

        // Preserve legacy token ownership while every old account/token is still available.
        Self::migrate_purchase_records(conn)?;

        // 账号唯一性：按 email 收口，只保留最新一条，避免 token 变了后产生重复账号。
        let _ = conn.execute(
            "DELETE FROM accounts
             WHERE id NOT IN (
               SELECT MAX(id) FROM accounts WHERE email IS NOT NULL AND email != '' GROUP BY email
             )
             AND email IS NOT NULL AND email != ''",
            [],
        );
        let _ = conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_accounts_email_unique ON accounts(email)",
            [],
        );

        let table_info: Vec<(i32, String, String, bool, i32, bool)> = conn
            .prepare("PRAGMA table_info(download_records)")?
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let has_progress = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "progress");
        let has_error = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "error");
        let has_package_kind = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "package_kind");
        let has_ota_installable = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "ota_installable");
        let has_install_method = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "install_method");
        let has_inspection_json = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "inspection_json");
        let has_job_id = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "job_id");
        let has_app_version_id = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "app_version_id");
        let has_file_path = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "file_path");

        if !has_job_id {
            let _ = conn.execute("ALTER TABLE download_records ADD COLUMN job_id TEXT", []);
        }
        if !has_app_version_id {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN app_version_id TEXT",
                [],
            );
        }
        if !has_file_path {
            let _ = conn.execute("ALTER TABLE download_records ADD COLUMN file_path TEXT", []);
        }

        if !has_progress {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN progress INTEGER DEFAULT 0",
                [],
            );
        }
        if !has_error {
            let _ = conn.execute("ALTER TABLE download_records ADD COLUMN error TEXT", []);
        }
        if !has_package_kind {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN package_kind TEXT",
                [],
            );
        }
        if !has_ota_installable {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN ota_installable INTEGER",
                [],
            );
        }
        if !has_install_method {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN install_method TEXT",
                [],
            );
        }
        if !has_inspection_json {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN inspection_json TEXT",
                [],
            );
        }

        let has_retry_count = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "retry_count");
        if !has_retry_count {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN retry_count INTEGER DEFAULT 5",
                [],
            );
        }

        let has_max_retries = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "max_retries");
        if !has_max_retries {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN max_retries INTEGER DEFAULT 5",
                [],
            );
        }

        let has_download_speed = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "download_speed");
        if !has_download_speed {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN download_speed REAL",
                [],
            );
        }

        let has_resume_position = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "resume_position");
        if !has_resume_position {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN resume_position INTEGER DEFAULT 0",
                [],
            );
        }

        let has_delisted = table_info
            .iter()
            .any(|(_, name, _, _, _, _)| name == "delisted");
        if !has_delisted {
            let _ = conn.execute(
                "ALTER TABLE download_records ADD COLUMN delisted INTEGER DEFAULT 0",
                [],
            );
        }

        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_download_records_job_id ON download_records(job_id)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_download_records_app_version_id ON download_records(app_version_id)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_download_records_file_path ON download_records(file_path)",
            [],
        );

        let _ = conn.execute("DELETE FROM encryption_keys WHERE key_id IS NULL", []);

        // purchase_records index for fast lookups by (account_id, adam_id)
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_purchase_records_account_app ON purchase_records(account_id, adam_id)",
            [],
        );

        Ok(())
    }

    /// Stable ownership identity; session tokens are never purchase identities.
    pub fn purchase_account_key(email: &str) -> Result<String> {
        let key = email.trim().to_lowercase();
        let mut parts = key.split('@');
        let local = parts.next().unwrap_or_default();
        let domain = parts.next().unwrap_or_default();
        if local.is_empty()
            || domain.is_empty()
            || parts.next().is_some()
            || key.chars().any(char::is_whitespace)
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "purchase account must be an email".into(),
            ));
        }
        Ok(key)
    }

    /// Run before account deduplication or token replacement. Unresolvable legacy
    /// rows remain untouched: a token prefix is evidence only when it has exactly
    /// one valid normalized owner across ALL matching accounts.
    fn migrate_purchase_keys(conn: &Connection) -> Result<()> {
        use std::collections::{HashMap, HashSet};
        let mut owners: HashMap<String, HashSet<Option<String>>> = HashMap::new();
        let mut stmt = conn.prepare("SELECT token, email FROM accounts")?;
        let accounts =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for account in accounts {
            let (token, email) = account?;
            if token.is_empty() {
                continue;
            }
            let legacy = format!("tok_{}", token.chars().take(16).collect::<String>());
            owners
                .entry(legacy)
                .or_default()
                .insert(Self::purchase_account_key(&email).ok());
        }
        let mut stmt = conn.prepare(
            "SELECT id, account_id, adam_id, source, created_at FROM purchase_records ORDER BY id",
        )?;
        let records = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>>>()?;
        for (id, key, app, source, created_at) in records {
            let target = if let Ok(email) = Self::purchase_account_key(&key) {
                Some(email)
            } else if key.starts_with("tok_") {
                owners
                    .get(&key)
                    .filter(|set| set.len() == 1)
                    .and_then(|set| set.iter().next())
                    .and_then(|email| email.clone())
            } else {
                None
            };
            if let Some(target) = target.filter(|target| target != &key) {
                // Preserve an existing canonical row's provenance on conflicts.
                conn.execute("INSERT INTO purchase_records(account_id, adam_id, source, created_at) VALUES (?, ?, ?, ?) ON CONFLICT(account_id, adam_id) DO NOTHING", params![target, app, source, created_at])?;
                conn.execute("DELETE FROM purchase_records WHERE id = ?", params![id])?;
            }
        }
        Ok(())
    }

    fn migrate_purchase_records(conn: &Connection) -> Result<()> {
        let tx = conn.unchecked_transaction()?;
        Self::migrate_purchase_keys(&tx)?;
        // Both manual uploads and filesystem discovery create completed records,
        // but use sentinel account names (手动上传/未知账号). Only real account
        // emails plus numeric Apple adam IDs are historical ownership evidence.
        let mut stmt = tx.prepare("SELECT account_email, app_id, COALESCE(download_date, CURRENT_TIMESTAMP) FROM download_records WHERE status = 'completed'")?;
        let history = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>>>()?;
        drop(stmt);
        for (email, app, date) in history {
            if let Ok(key) = Self::purchase_account_key(&email) {
                if !app.is_empty()
                    && app.bytes().all(|b| b.is_ascii_digit())
                    && app.bytes().any(|b| b != b'0')
                {
                    tx.execute("INSERT INTO purchase_records(account_id, adam_id, source, created_at) VALUES (?, ?, 'download_history', ?) ON CONFLICT(account_id, adam_id) DO NOTHING", params![key, app, date])?;
                }
            }
        }
        tx.commit()
    }

    pub fn hash_password(password: &str) -> String {
        bcrypt::hash(password, bcrypt::DEFAULT_COST).unwrap_or_else(|e| {
            // debug 模式下输出告警，release 下静默 fallback
            #[cfg(debug_assertions)]
            eprintln!(
                "[WARN] bcrypt::hash failed ({}), falling back to SHA-256",
                e
            );
            #[cfg(not(debug_assertions))]
            let _ = &e;
            let mut hasher = sha2::Sha256::new();
            sha2::Digest::update(&mut hasher, password.as_bytes());
            hex::encode(sha2::Digest::finalize(hasher))
        })
    }

    pub fn reset_admin_password(&self, username: &str, new_password: &str) -> Result<bool> {
        let conn = self.connection.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let updated = tx.execute(
            "UPDATE admin_users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
            params![Self::hash_password(new_password), username],
        )?;

        if updated > 0 {
            tx.execute("DELETE FROM sessions WHERE username = ?", params![username])?;
        }

        tx.commit()?;
        Ok(updated > 0)
    }

    fn seed_default_admin(conn: &Connection) -> Result<()> {
        // Only seed if no admin users exist at all (first run).
        // If the user renamed 'admin' to something else, we must NOT recreate 'admin'.
        let any_admin: Option<i64> = conn
            .query_row("SELECT id FROM admin_users LIMIT 1", [], |row| row.get(0))
            .optional()?
            .flatten();

        if any_admin.is_none() {
            let password = std::env::var("IPA_ADMIN_INITIAL_PASSWORD")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    let generated = format!("ipatool-{}", Uuid::new_v4());
                    eprintln!(
                        "[SECURITY] Generated one-time admin password for first run: {}",
                        generated
                    );
                    eprintln!(
                        "[SECURITY] Set IPA_ADMIN_INITIAL_PASSWORD to choose a deterministic initial password. Change it immediately after login."
                    );
                    generated
                });
            conn.execute(
                "INSERT INTO admin_users (username, password_hash, is_default) VALUES (?, ?, ?)",
                params!["admin", Self::hash_password(&password), true],
            )?;
        }

        Ok(())
    }

    pub fn create_admin_user(
        &self,
        username: &str,
        password_hash: &str,
        is_default: bool,
    ) -> Result<i64> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO admin_users (username, password_hash, is_default) VALUES (?, ?, ?)",
            params![username, password_hash, is_default],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_admin_user(&self, username: &str) -> Result<Option<AdminUser>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM admin_users WHERE username = ?")?;
        let user = stmt
            .query_row(params![username], |row| {
                Ok(AdminUser {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    password_hash: row.get(2)?,
                    is_default: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .optional()?;
        Ok(user)
    }

    pub fn update_admin_password(
        &self,
        username: &str,
        password_hash: &str,
        is_default: bool,
    ) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE admin_users SET password_hash = ?, is_default = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
            params![password_hash, is_default, username],
        )?;
        Ok(())
    }

    pub fn rename_admin_user(&self, old_username: &str, new_username: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET username = ? WHERE username = ?",
            params![new_username, old_username],
        )?;
        conn.execute(
            "UPDATE admin_users SET username = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
            params![new_username, old_username],
        )?;
        Ok(())
    }

    /// Atomic password change + optional rename in a single transaction.
    /// Returns the final username (new or unchanged).
    /// When renaming, the current session is moved to the new username so the user stays logged in.
    pub fn change_password_and_rename(
        &self,
        username: &str,
        new_password_hash: &str,
        is_default: bool,
        new_username: Option<&str>,
        session_token: Option<&str>,
    ) -> Result<String> {
        let conn = self.connection.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Update password
        tx.execute(
            "UPDATE admin_users SET password_hash = ?, is_default = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
            params![new_password_hash, is_default, username],
        )?;

        let final_username = match new_username {
            Some(new_name) => {
                let current_session_expires_at = match session_token {
                    Some(token) => tx
                        .query_row(
                            "SELECT expires_at FROM sessions WHERE token = ? AND username = ?",
                            params![token, username],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()?,
                    None => None,
                };

                // sessions.username references admin_users(username) without ON UPDATE CASCADE.
                // Delete old sessions before renaming, then recreate the current session under
                // the new username so the password-change flow stays logged in.
                tx.execute("DELETE FROM sessions WHERE username = ?", params![username])?;
                tx.execute(
                    "UPDATE admin_users SET username = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
                    params![new_name, username],
                )?;

                if let (Some(token), Some(expires_at)) = (session_token, current_session_expires_at)
                {
                    tx.execute(
                        "INSERT INTO sessions (token, username, expires_at) VALUES (?, ?, ?)",
                        params![token, new_name, expires_at],
                    )?;
                }

                new_name.to_string()
            }
            None => {
                // A password change must log out every other session (e.g. a stolen cookie);
                // only the session performing the change survives.
                match session_token {
                    Some(token) => tx.execute(
                        "DELETE FROM sessions WHERE username = ? AND token != ?",
                        params![username, token],
                    )?,
                    None => {
                        tx.execute("DELETE FROM sessions WHERE username = ?", params![username])?
                    }
                };
                username.to_string()
            }
        };

        tx.commit()?;
        Ok(final_username)
    }

    pub fn delete_admin_user(&self, username: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "DELETE FROM admin_users WHERE username = ?",
            params![username],
        )?;
        Ok(())
    }

    pub fn create_session(&self, token: &str, username: &str, expires_at: &str) -> Result<i64> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (token, username, expires_at) VALUES (?, ?, ?)",
            params![token, username, expires_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_session(&self, token: &str) -> Result<Option<SessionRecord>> {
        // Runs on every authenticated request: read-only (expired rows are filtered here and
        // purged by `cleanup_expired_sessions` on login) and with a cached statement.
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT * FROM sessions WHERE token = ? AND expires_at > CURRENT_TIMESTAMP",
        )?;
        let session = stmt
            .query_row(params![token], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    token: row.get(1)?,
                    username: row.get(2)?,
                    created_at: row.get(3)?,
                    expires_at: row.get(4)?,
                })
            })
            .optional()?;
        Ok(session)
    }

    pub fn delete_session(&self, token: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE token = ?", params![token])?;
        Ok(())
    }

    pub fn delete_sessions_by_username(&self, username: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE username = ?", params![username])?;
        Ok(())
    }

    pub fn cleanup_expired_sessions(&self) -> Result<usize> {
        let conn = self.connection.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM sessions WHERE expires_at <= CURRENT_TIMESTAMP",
            [],
        )?;
        Ok(deleted)
    }

    pub fn upsert_github_token(&self, username: &str, token: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO github_tokens (username, token)
             VALUES (?, ?)
             ON CONFLICT(username) DO UPDATE SET
                 token = excluded.token,
                 updated_at = CURRENT_TIMESTAMP",
            params![username, token],
        )?;
        Ok(())
    }

    pub fn get_github_token(&self, username: &str) -> Result<Option<GitHubToken>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, username, token, created_at, updated_at
             FROM github_tokens
             WHERE username = ?",
        )?;
        let token = stmt
            .query_row(params![username], |row| {
                Ok(GitHubToken {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    token: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .optional()?;
        Ok(token)
    }

    pub fn delete_github_token(&self, username: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "DELETE FROM github_tokens WHERE username = ?",
            params![username],
        )?;
        Ok(())
    }

    // ===== WebDAV Config =====

    pub fn get_webdav_config(&self) -> Result<Option<WebDAVConfig>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT enabled, url, username, password, remote_path, updated_at
             FROM webdav_config WHERE id = 1",
        )?;
        let config = stmt
            .query_row([], |row| {
                Ok(WebDAVConfig {
                    enabled: row.get::<_, i64>(0)? != 0,
                    url: row.get(1)?,
                    username: row.get(2)?,
                    password: row.get(3)?,
                    remote_path: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .optional()?;
        Ok(config)
    }

    pub fn upsert_webdav_config(
        &self,
        enabled: bool,
        url: &str,
        username: &str,
        password: &str,
        remote_path: &str,
    ) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO webdav_config (id, enabled, url, username, password, remote_path, updated_at)
             VALUES (1, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET
                 enabled = excluded.enabled,
                 url = excluded.url,
                 username = excluded.username,
                 password = excluded.password,
                 remote_path = excluded.remote_path,
                 updated_at = CURRENT_TIMESTAMP",
            params![
                enabled as i64,
                url,
                username,
                password,
                remote_path
            ],
        )?;
        Ok(())
    }

    pub fn delete_webdav_config(&self) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute("DELETE FROM webdav_config WHERE id = 1", [])?;
        Ok(())
    }

    pub fn get_all_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM accounts")?;
        let accounts = stmt
            .query_map([], |row| {
                Ok(Account {
                    id: row.get(0)?,
                    token: row.get(1)?,
                    email: row.get(2)?,
                    region: row.get(3)?,
                    guid: row.get(4)?,
                    cookie_user: row.get(5)?,
                    cookies: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(accounts)
    }

    pub fn get_account_by_token(&self, token: &str) -> Result<Option<Account>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM accounts WHERE token = ?")?;
        let account = stmt
            .query_row(params![token], |row| {
                Ok(Account {
                    id: row.get(0)?,
                    token: row.get(1)?,
                    email: row.get(2)?,
                    region: row.get(3)?,
                    guid: row.get(4)?,
                    cookie_user: row.get(5)?,
                    cookies: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .optional()?;
        Ok(account)
    }

    pub fn get_latest_account_region_by_email(&self, email: &str) -> Result<Option<String>> {
        let conn = self.connection.lock().unwrap();
        conn.query_row(
            "SELECT region FROM accounts WHERE email = ? ORDER BY id DESC LIMIT 1",
            params![email],
            |row| row.get(0),
        )
        .optional()
    }

    pub fn update_account_region(&self, token: &str, region: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET region = ?, updated_at = CURRENT_TIMESTAMP WHERE token = ?",
            params![region, token],
        )?;
        Ok(())
    }

    pub fn save_account(&self, account: &Account) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        Self::migrate_purchase_keys(&tx)?;
        tx.execute(
            "INSERT INTO accounts (token, email, region, guid, cookie_user, cookies)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(email) DO UPDATE SET
                 token = excluded.token,
                 region = excluded.region,
                 guid = excluded.guid,
                 cookie_user = excluded.cookie_user,
                 cookies = excluded.cookies,
                 updated_at = CURRENT_TIMESTAMP",
            params![
                account.token,
                account.email,
                account.region,
                account.guid,
                account.cookie_user,
                account.cookies,
            ],
        )?;
        tx.commit()
    }

    /// 保存加密后的 Apple 登录态（JSON 字符串），按 token 定位账号。
    pub fn update_account_session(&self, token: &str, auth_session: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        let updated = conn.execute(
            "UPDATE accounts SET auth_session = ?, updated_at = CURRENT_TIMESTAMP WHERE token = ?",
            params![auth_session, token],
        )?;
        if updated != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }

    /// 返回所有带有已保存登录态的账号：(token, email, auth_session)。
    pub fn get_account_sessions(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT token, email, auth_session FROM accounts
             WHERE auth_session IS NOT NULL AND auth_session != ''",
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn delete_account(&self, token: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute("DELETE FROM accounts WHERE token = ?", params![token])?;
        Ok(())
    }

    pub fn save_credentials(&self, credentials: &Credentials) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO credentials (email, password_encrypted, key_id, iv, auth_tag) 
             VALUES (?, ?, ?, ?, ?)",
            params![
                credentials.email,
                credentials.password_encrypted,
                credentials.key_id,
                credentials.iv,
                credentials.auth_tag,
            ],
        )?;
        Ok(())
    }

    pub fn delete_credentials(&self, email: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute("DELETE FROM credentials WHERE email = ?", params![email])?;
        Ok(())
    }

    pub fn get_credentials(&self, email: &str) -> Result<Option<Credentials>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM credentials WHERE email = ?")?;
        let cred = stmt
            .query_row(params![email], |row| {
                Ok(Credentials {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    password_encrypted: row.get(2)?,
                    key_id: row.get(3)?,
                    iv: row.get(4)?,
                    auth_tag: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .optional()?;
        Ok(cred)
    }

    pub fn get_all_credentials(&self) -> Result<Vec<Credentials>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM credentials")?;
        let creds = stmt
            .query_map([], |row| {
                Ok(Credentials {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    password_encrypted: row.get(2)?,
                    key_id: row.get(3)?,
                    iv: row.get(4)?,
                    auth_tag: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(creds)
    }

    pub fn save_encryption_key(&self, key: &EncryptionKey) -> Result<()> {
        let conn = self.connection.lock().unwrap();

        if key.is_current {
            conn.execute("UPDATE encryption_keys SET is_current = FALSE", [])?;
        }

        conn.execute(
            "INSERT OR REPLACE INTO encryption_keys (key_id, key_value, is_current, last_rotation, next_rotation) 
             VALUES (?, ?, ?, ?, ?)",
            params![
                key.key_id,
                key.key_value,
                if key.is_current { 1i64 } else { 0i64 },
                key.last_rotation,
                key.next_rotation,
            ],
        )?;
        Ok(())
    }

    pub fn get_current_encryption_key(&self) -> Result<Option<EncryptionKey>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM encryption_keys WHERE is_current = 1")?;
        let key = stmt
            .query_row([], |row| {
                Ok(EncryptionKey {
                    id: row.get(0)?,
                    key_id: row.get(1)?,
                    key_value: row.get(2)?,
                    is_current: row.get(3)?,
                    created_at: row.get(4)?,
                    last_rotation: row.get(5)?,
                    next_rotation: row.get(6)?,
                })
            })
            .optional()?;
        Ok(key)
    }

    pub fn get_all_encryption_keys(&self) -> Result<Vec<EncryptionKey>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM encryption_keys ORDER BY created_at DESC")?;
        let keys = stmt
            .query_map([], |row| {
                Ok(EncryptionKey {
                    id: row.get(0)?,
                    key_id: row.get(1)?,
                    key_value: row.get(2)?,
                    is_current: row.get(3)?,
                    created_at: row.get(4)?,
                    last_rotation: row.get(5)?,
                    next_rotation: row.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(keys)
    }

    pub fn reset_encryption_keys(&self) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute("DELETE FROM encryption_keys", [])?;
        Ok(())
    }

    pub fn add_download_record(&self, record: &DownloadRecord) -> Result<i64> {
        let conn = self.connection.lock().unwrap();
        if let Some(existing_id) = Self::find_existing_download_record_id(&conn, record)? {
            conn.execute(
                "UPDATE download_records SET
                 job_id = ?, app_version_id = ?, app_name = ?, app_id = ?, bundle_id = ?, version = ?,
                 account_email = ?, account_region = ?, status = ?, file_size = ?, file_path = ?,
                 install_url = ?, artwork_url = ?, artist_name = ?, progress = ?, error = ?,
                 package_kind = ?, ota_installable = ?, install_method = ?, inspection_json = ?,
                 delisted = ?,
                 download_date = COALESCE(?, download_date)
                 WHERE id = ?",
                params![
                    record.job_id,
                    record.app_version_id,
                    record.app_name,
                    record.app_id,
                    record.bundle_id,
                    record.version,
                    record.account_email,
                    record.account_region,
                    record.status,
                    record.file_size,
                    record.file_path,
                    record.install_url,
                    record.artwork_url,
                    record.artist_name,
                    record.progress,
                    record.error,
                    record.package_kind,
                    record
                        .ota_installable
                        .map(|value| if value { 1i64 } else { 0i64 }),
                    record.install_method,
                    record.inspection_json,
                    record.delisted.map(|value| if value { 1i64 } else { 0i64 }),
                    record.download_date,
                    existing_id,
                ],
            )?;
            return Ok(existing_id);
        }

        conn.execute(
            "INSERT INTO download_records 
             (job_id, app_version_id, app_name, app_id, bundle_id, version, account_email, account_region, status, file_size, file_path, install_url, artwork_url, artist_name, progress, error, package_kind, ota_installable, install_method, inspection_json, delisted, download_date) 
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, COALESCE(?, CURRENT_TIMESTAMP))",
            params![
                record.job_id,
                record.app_version_id,
                record.app_name,
                record.app_id,
                record.bundle_id,
                record.version,
                record.account_email,
                record.account_region,
                record.status,
                record.file_size,
                record.file_path,
                record.install_url,
                record.artwork_url,
                record.artist_name,
                record.progress,
                record.error,
                record.package_kind,
                record.ota_installable.map(|value| if value { 1i64 } else { 0i64 }),
                record.install_method,
                record.inspection_json,
                record.delisted.map(|value| if value { 1i64 } else { 0i64 }),
                record.download_date,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn find_existing_download_record_id(
        conn: &Connection,
        record: &DownloadRecord,
    ) -> Result<Option<i64>> {
        if let Some(job_id) = record.job_id.as_deref() {
            let existing = conn
                .query_row(
                    "SELECT id FROM download_records WHERE job_id = ? ORDER BY id DESC LIMIT 1",
                    params![job_id],
                    |row| row.get(0),
                )
                .optional()?;
            if existing.is_some() {
                return Ok(existing);
            }
        }

        if let Some(file_path) = record.file_path.as_deref() {
            let existing = conn
                .query_row(
                    "SELECT id FROM download_records WHERE file_path = ? ORDER BY id DESC LIMIT 1",
                    params![file_path],
                    |row| row.get(0),
                )
                .optional()?;
            if existing.is_some() {
                return Ok(existing);
            }
        }

        Ok(None)
    }

    pub fn get_all_download_records(&self) -> Result<Vec<DownloadRecord>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, job_id, app_version_id, app_name, app_id, bundle_id, version, account_email, account_region,
                    download_date, status, file_size, file_path, install_url, artwork_url,
                    artist_name, progress, error, package_kind, ota_installable,
                    install_method, inspection_json, delisted, created_at
             FROM download_records
             ORDER BY download_date DESC, id DESC",
        )?;
        let records = stmt
            .query_map([], |row| {
                Ok(DownloadRecord {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    app_version_id: row.get(2)?,
                    app_name: row.get(3)?,
                    app_id: row.get(4)?,
                    bundle_id: row.get(5)?,
                    version: row.get(6)?,
                    account_email: row.get(7)?,
                    account_region: row.get(8)?,
                    download_date: row.get(9)?,
                    status: row.get(10)?,
                    file_size: row.get(11)?,
                    file_path: row.get(12)?,
                    install_url: row.get(13)?,
                    artwork_url: row.get(14)?,
                    artist_name: row.get(15)?,
                    progress: row.get(16)?,
                    error: row.get(17)?,
                    package_kind: row.get(18)?,
                    ota_installable: row.get::<_, Option<i64>>(19)?.map(|value| value != 0),
                    install_method: row.get(20)?,
                    inspection_json: row.get(21)?,
                    delisted: row.get::<_, Option<i64>>(22)?.map(|value| value != 0),
                    created_at: row.get(23)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(records)
    }

    pub fn get_download_record(&self, id: i64) -> Result<Option<DownloadRecord>> {
        let conn = self.connection.lock().unwrap();
        conn.query_row(
            "SELECT id, job_id, app_version_id, app_name, app_id, bundle_id, version, account_email, account_region,
                    download_date, status, file_size, file_path, install_url, artwork_url,
                    artist_name, progress, error, package_kind, ota_installable,
                    install_method, inspection_json, delisted, created_at
             FROM download_records
             WHERE id = ? LIMIT 1",
            params![id],
            |row| {
                Ok(DownloadRecord {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    app_version_id: row.get(2)?,
                    app_name: row.get(3)?,
                    app_id: row.get(4)?,
                    bundle_id: row.get(5)?,
                    version: row.get(6)?,
                    account_email: row.get(7)?,
                    account_region: row.get(8)?,
                    download_date: row.get(9)?,
                    status: row.get(10)?,
                    file_size: row.get(11)?,
                    file_path: row.get(12)?,
                    install_url: row.get(13)?,
                    artwork_url: row.get(14)?,
                    artist_name: row.get(15)?,
                    progress: row.get(16)?,
                    error: row.get(17)?,
                    package_kind: row.get(18)?,
                    ota_installable: row.get::<_, Option<i64>>(19)?.map(|value| value != 0),
                    install_method: row.get(20)?,
                    inspection_json: row.get(21)?,
                    delisted: row.get::<_, Option<i64>>(22)?.map(|value| value != 0),
                    created_at: row.get(23)?,
                })
            },
        )
        .optional()
    }

    pub fn get_download_record_by_job_id(&self, job_id: &str) -> Result<Option<DownloadRecord>> {
        let conn = self.connection.lock().unwrap();
        conn.query_row(
            "SELECT id, job_id, app_version_id, app_name, app_id, bundle_id, version, account_email, account_region,
                    download_date, status, file_size, file_path, install_url, artwork_url,
                    artist_name, progress, error, package_kind, ota_installable,
                    install_method, inspection_json, delisted, created_at
             FROM download_records
             WHERE job_id = ?
             ORDER BY id DESC
             LIMIT 1",
            params![job_id],
            |row| {
                Ok(DownloadRecord {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    app_version_id: row.get(2)?,
                    app_name: row.get(3)?,
                    app_id: row.get(4)?,
                    bundle_id: row.get(5)?,
                    version: row.get(6)?,
                    account_email: row.get(7)?,
                    account_region: row.get(8)?,
                    download_date: row.get(9)?,
                    status: row.get(10)?,
                    file_size: row.get(11)?,
                    file_path: row.get(12)?,
                    install_url: row.get(13)?,
                    artwork_url: row.get(14)?,
                    artist_name: row.get(15)?,
                    progress: row.get(16)?,
                    error: row.get(17)?,
                    package_kind: row.get(18)?,
                    ota_installable: row.get::<_, Option<i64>>(19)?.map(|value| value != 0),
                    install_method: row.get(20)?,
                    inspection_json: row.get(21)?,
                    delisted: row.get::<_, Option<i64>>(22)?.map(|value| value != 0),
                    created_at: row.get(23)?,
                })
            },
        )
        .optional()
    }

    pub fn get_download_record_by_file_path(
        &self,
        file_path: &str,
    ) -> Result<Option<DownloadRecord>> {
        let conn = self.connection.lock().unwrap();
        conn.query_row(
            "SELECT id, job_id, app_version_id, app_name, app_id, bundle_id, version, account_email, account_region,
                    download_date, status, file_size, file_path, install_url, artwork_url,
                    artist_name, progress, error, package_kind, ota_installable,
                    install_method, inspection_json, delisted, created_at
             FROM download_records
             WHERE file_path = ?
             ORDER BY id DESC LIMIT 1",
            params![file_path],
            |row| {
                Ok(DownloadRecord {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    app_version_id: row.get(2)?,
                    app_name: row.get(3)?,
                    app_id: row.get(4)?,
                    bundle_id: row.get(5)?,
                    version: row.get(6)?,
                    account_email: row.get(7)?,
                    account_region: row.get(8)?,
                    download_date: row.get(9)?,
                    status: row.get(10)?,
                    file_size: row.get(11)?,
                    file_path: row.get(12)?,
                    install_url: row.get(13)?,
                    artwork_url: row.get(14)?,
                    artist_name: row.get(15)?,
                    progress: row.get(16)?,
                    error: row.get(17)?,
                    package_kind: row.get(18)?,
                    ota_installable: row.get::<_, Option<i64>>(19)?.map(|value| value != 0),
                    install_method: row.get(20)?,
                    inspection_json: row.get(21)?,
                    delisted: row.get::<_, Option<i64>>(22)?.map(|value| value != 0),
                    created_at: row.get(23)?,
                })
            },
        )
        .optional()
    }

    pub fn find_reusable_download_record(
        &self,
        app_id: &str,
        version: &str,
        account_email: &str,
    ) -> Result<Option<DownloadRecord>> {
        let conn = self.connection.lock().unwrap();
        conn.query_row(
            "SELECT id, job_id, app_version_id, app_name, app_id, bundle_id, version, account_email, account_region,
                    download_date, status, file_size, file_path, install_url, artwork_url,
                    artist_name, progress, error, package_kind, ota_installable,
                    install_method, inspection_json, delisted, created_at
             FROM download_records
             WHERE app_id = ?
               AND version = ?
               AND account_email = ?
               AND status = 'completed'
               AND file_path IS NOT NULL
             ORDER BY download_date DESC, id DESC
             LIMIT 1",
            params![app_id, version, account_email],
            |row| {
                Ok(DownloadRecord {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    app_version_id: row.get(2)?,
                    app_name: row.get(3)?,
                    app_id: row.get(4)?,
                    bundle_id: row.get(5)?,
                    version: row.get(6)?,
                    account_email: row.get(7)?,
                    account_region: row.get(8)?,
                    download_date: row.get(9)?,
                    status: row.get(10)?,
                    file_size: row.get(11)?,
                    file_path: row.get(12)?,
                    install_url: row.get(13)?,
                    artwork_url: row.get(14)?,
                    artist_name: row.get(15)?,
                    progress: row.get(16)?,
                    error: row.get(17)?,
                    package_kind: row.get(18)?,
                    ota_installable: row.get::<_, Option<i64>>(19)?.map(|value| value != 0),
                    install_method: row.get(20)?,
                    inspection_json: row.get(21)?,
                    delisted: row.get::<_, Option<i64>>(22)?.map(|value| value != 0),
                    created_at: row.get(23)?,
                })
            },
        )
        .optional()
    }

    pub fn delete_download_record(&self, id: i64) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute("DELETE FROM download_records WHERE id = ?", params![id])?;
        Ok(())
    }

    pub fn update_download_record(&self, id: i64, updates: &DownloadRecord) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE download_records SET 
             app_name = ?, app_id = ?, bundle_id = ?, version = ?, 
             account_email = ?, account_region = ?, status = ?, 
             file_size = ?, file_path = ?, install_url = ?, artwork_url = ?, 
             artist_name = ?, progress = ?, error = ?,
             package_kind = ?, ota_installable = ?, install_method = ?, inspection_json = ?,
             delisted = ?
             WHERE id = ?",
            params![
                updates.app_name,
                updates.app_id,
                updates.bundle_id,
                updates.version,
                updates.account_email,
                updates.account_region,
                updates.status,
                updates.file_size,
                updates.file_path,
                updates.install_url,
                updates.artwork_url,
                updates.artist_name,
                updates.progress,
                updates.error,
                updates.package_kind,
                updates
                    .ota_installable
                    .map(|value| if value { 1i64 } else { 0i64 }),
                updates.install_method,
                updates.inspection_json,
                updates
                    .delisted
                    .map(|value| if value { 1i64 } else { 0i64 }),
                id,
            ],
        )?;
        Ok(())
    }

    pub fn update_download_record_delivery(
        &self,
        id: i64,
        package_kind: Option<&str>,
        ota_installable: Option<bool>,
        install_method: Option<&str>,
        inspection_json: Option<&str>,
    ) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE download_records
             SET package_kind = ?, ota_installable = ?, install_method = ?, inspection_json = ?
             WHERE id = ?",
            params![
                package_kind,
                ota_installable.map(|value| if value { 1i64 } else { 0i64 }),
                install_method,
                inspection_json,
                id,
            ],
        )?;
        Ok(())
    }

    pub fn delete_download_record_by_file_path(&self, file_path: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "DELETE FROM download_records WHERE file_path = ?",
            params![file_path],
        )?;
        Ok(())
    }

    pub fn clear_all_download_records(&self) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute("DELETE FROM download_records", [])?;
        Ok(())
    }

    // 订阅相关方法
    pub fn add_subscription(&self, subscription: &NewSubscription<'_>) -> Result<i64> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO app_subscriptions 
             (app_id, app_name, bundle_id, account_email, account_region, artwork_url, artist_name, last_checked) 
             VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)",
            params![
                subscription.app_id,
                subscription.app_name,
                subscription.bundle_id,
                subscription.account_email,
                subscription.account_region,
                subscription.artwork_url,
                subscription.artist_name,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn remove_subscription(&self, app_id: &str, account_email: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "DELETE FROM app_subscriptions WHERE app_id = ? AND account_email = ?",
            params![app_id, account_email],
        )?;
        Ok(())
    }

    pub fn get_all_subscriptions(&self) -> Result<Vec<Subscription>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT * FROM app_subscriptions ORDER BY subscribed_at DESC")?;
        let subs = stmt
            .query_map([], |row| {
                Ok(Subscription {
                    id: row.get(0)?,
                    app_id: row.get(1)?,
                    app_name: row.get(2)?,
                    bundle_id: row.get(3)?,
                    account_email: row.get(4)?,
                    account_region: row.get(5)?,
                    current_version: row.get(6)?,
                    artwork_url: row.get(7)?,
                    artist_name: row.get(8)?,
                    subscribed_at: row.get(9)?,
                    last_checked: row.get(10)?,
                    created_at: row.get(11)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(subs)
    }

    pub fn update_subscription_version(
        &self,
        app_id: &str,
        account_email: &str,
        new_version: &str,
    ) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE app_subscriptions SET current_version = ?, last_checked = CURRENT_TIMESTAMP 
             WHERE app_id = ? AND account_email = ?",
            params![new_version, app_id, account_email],
        )?;
        Ok(())
    }

    // 批量下载相关方法
    pub fn create_batch_task(&self, task_name: &str, total_count: i64) -> Result<i64> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO batch_download_tasks (task_name, total_count) VALUES (?, ?)",
            params![task_name, total_count],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn add_batch_item(
        &self,
        batch_id: i64,
        app_id: &str,
        app_name: Option<&str>,
        version: Option<&str>,
        account_email: &str,
    ) -> Result<i64> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO batch_download_items (batch_id, app_id, app_name, version, account_email) 
             VALUES (?, ?, ?, ?, ?)",
            params![batch_id, app_id, app_name, version, account_email],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_batch_tasks(&self) -> Result<Vec<BatchDownloadTask>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT * FROM batch_download_tasks ORDER BY created_at DESC")?;
        let tasks = stmt
            .query_map([], |row| {
                Ok(BatchDownloadTask {
                    id: row.get(0)?,
                    task_name: row.get(1)?,
                    status: row.get(2)?,
                    total_count: row.get(3)?,
                    completed_count: row.get(4)?,
                    failed_count: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    completed_at: row.get(8)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tasks)
    }

    pub fn get_batch_items(&self, batch_id: i64) -> Result<Vec<BatchDownloadItem>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM batch_download_items WHERE batch_id = ?")?;
        let items = stmt
            .query_map(params![batch_id], |row| {
                Ok(BatchDownloadItem {
                    id: row.get(0)?,
                    batch_id: row.get(1)?,
                    app_id: row.get(2)?,
                    app_name: row.get(3)?,
                    version: row.get(4)?,
                    account_email: row.get(5)?,
                    status: row.get(6)?,
                    progress: row.get(7)?,
                    error: row.get(8)?,
                    retry_count: row.get(9)?,
                    created_at: row.get(10)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(items)
    }

    pub fn update_batch_item(
        &self,
        item_id: i64,
        status: &str,
        progress: i64,
        error: Option<&str>,
        retry_count: i64,
    ) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE batch_download_items SET status = ?, progress = ?, error = ?, retry_count = ? WHERE id = ?",
            params![status, progress, error, retry_count, item_id],
        )?;
        Ok(())
    }

    pub fn update_batch_task_progress(
        &self,
        batch_id: i64,
        completed_count: i64,
        failed_count: i64,
        status: &str,
    ) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE batch_download_tasks SET completed_count = ?, failed_count = ?, status = ?, updated_at = CURRENT_TIMESTAMP 
             WHERE id = ?",
            params![completed_count, failed_count, status, batch_id],
        )?;
        Ok(())
    }

    pub fn delete_batch_task(&self, batch_id: i64) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "DELETE FROM batch_download_tasks WHERE id = ?",
            params![batch_id],
        )?;
        Ok(())
    }

    // ===== Purchase Records =====

    /// Record account ownership; ignore only duplicate (email, app) conflicts.
    pub fn record_purchase(&self, account_id: &str, adam_id: &str, source: &str) -> Result<()> {
        let account_id = Self::purchase_account_key(account_id)?;
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO purchase_records (account_id, adam_id, source) VALUES (?, ?, ?) ON CONFLICT(account_id, adam_id) DO NOTHING",
            params![account_id, adam_id, source],
        )?;
        Ok(())
    }

    /// Check if a specific app is owned by an account
    pub fn is_purchased(&self, account_id: &str, adam_id: &str) -> Result<bool> {
        let account_id = Self::purchase_account_key(account_id)?;
        let conn = self.connection.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM purchase_records WHERE account_id = ? AND adam_id = ?",
            params![account_id, adam_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get all purchased adam_ids for an account
    pub fn get_purchased_app_ids(&self, account_id: &str) -> Result<Vec<String>> {
        let account_id = Self::purchase_account_key(account_id)?;
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT adam_id FROM purchase_records WHERE account_id = ?")?;
        let rows = stmt.query_map(params![account_id], |row| row.get(0))?;
        let mut ids = Vec::new();
        for id in rows {
            ids.push(id?);
        }
        Ok(ids)
    }

    /// Batch check: given a list of adam_ids, return the set that are purchased
    pub fn batch_check_purchased(
        &self,
        account_id: &str,
        adam_ids: &[String],
    ) -> Result<std::collections::HashSet<String>> {
        let account_id = Self::purchase_account_key(account_id)?;
        if adam_ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let conn = self.connection.lock().unwrap();
        let placeholders: Vec<String> = adam_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect();
        let sql = format!(
            "SELECT adam_id FROM purchase_records WHERE account_id = ?1 AND adam_id IN ({})",
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(account_id.to_string())];
        for id in adam_ids {
            params_vec.push(Box::new(id.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| row.get(0))?;
        let mut result = std::collections::HashSet::new();
        for id in rows {
            result.insert(id?);
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Option<i64>,
    pub app_id: String,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub account_email: String,
    pub account_region: Option<String>,
    pub current_version: Option<String>,
    pub artwork_url: Option<String>,
    pub artist_name: Option<String>,
    pub subscribed_at: Option<String>,
    pub last_checked: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDownloadTask {
    pub id: Option<i64>,
    pub task_name: String,
    pub status: String,
    pub total_count: i64,
    pub completed_count: i64,
    pub failed_count: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDownloadItem {
    pub id: Option<i64>,
    pub batch_id: i64,
    pub app_id: String,
    pub app_name: Option<String>,
    pub version: Option<String>,
    pub account_email: String,
    pub status: String,
    pub progress: i64,
    pub error: Option<String>,
    pub retry_count: i64,
    pub created_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}.sqlite", prefix, nonce))
    }

    fn sample_record(
        app_id: &str,
        version: &str,
        account_email: &str,
        file_path: &str,
    ) -> DownloadRecord {
        DownloadRecord {
            id: None,
            job_id: Some("job-1".to_string()),
            app_version_id: Some("ver-1".to_string()),
            app_name: "WeChat".to_string(),
            app_id: app_id.to_string(),
            bundle_id: Some("com.tencent.xin".to_string()),
            version: Some(version.to_string()),
            account_email: account_email.to_string(),
            account_region: Some("CN".to_string()),
            download_date: Some("2026-04-22T12:00:00Z".to_string()),
            status: "completed".to_string(),
            file_size: Some(1024),
            file_path: Some(file_path.to_string()),
            install_url: None,
            artwork_url: None,
            artist_name: None,
            progress: Some(100),
            error: None,
            package_kind: Some("signed".to_string()),
            ota_installable: Some(true),
            install_method: Some("ota".to_string()),
            inspection_json: None,
            delisted: None,
            created_at: None,
        }
    }

    fn purchase_db() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        Database::create_tables(&conn).unwrap();
        Database {
            connection: std::sync::Mutex::new(conn),
        }
    }

    fn insert_purchase(db: &Database, key: &str, app: &str, source: &str) {
        db.connection.lock().unwrap().execute(
            "INSERT INTO purchase_records(account_id, adam_id, source, created_at) VALUES (?, ?, ?, '2020-01-01')",
            params![key, app, source],
        ).unwrap();
    }

    #[test]
    fn purchase_migration_precedes_account_dedup_and_is_idempotent() {
        let db = purchase_db();
        {
            let conn = db.connection.lock().unwrap();
            conn.execute("INSERT INTO accounts(token,email) VALUES ('old-token-123456789', ' User@Example.com '), ('new-token-123456789', ' User@Example.com ')", []).unwrap();
        }
        insert_purchase(&db, "tok_old-token-123456", "123", "claim");
        insert_purchase(&db, "user@example.com", "123", "apple_api");
        insert_purchase(&db, " User@Example.com ", "456", "claim");
        for _ in 0..2 {
            Database::migrate_tables(&db.connection.lock().unwrap()).unwrap();
            assert_eq!(
                db.get_purchased_app_ids("user@example.com").unwrap().len(),
                2
            );
        }
        let conn = db.connection.lock().unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM purchase_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
        let source: String = conn.query_row("SELECT source FROM purchase_records WHERE account_id='user@example.com' AND adam_id='123'", [], |r| r.get(0)).unwrap();
        assert_eq!(source, "apple_api");
    }

    #[test]
    fn purchase_migration_preserves_ambiguous_and_orphan_token_keys() {
        let db = purchase_db();
        {
            let conn = db.connection.lock().unwrap();
            conn.execute("INSERT INTO accounts(token,email) VALUES ('shared-prefix-123-A', 'a@example.com'), ('shared-prefix-123-B', 'b@example.com')", []).unwrap();
        }
        insert_purchase(&db, "tok_shared-prefix-12", "123", "claim");
        insert_purchase(&db, "tok_missing-token-12", "456", "claim");
        Database::migrate_tables(&db.connection.lock().unwrap()).unwrap();
        assert!(db
            .get_purchased_app_ids("a@example.com")
            .unwrap()
            .is_empty());
        assert!(db
            .get_purchased_app_ids("b@example.com")
            .unwrap()
            .is_empty());
        let conn = db.connection.lock().unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM purchase_records WHERE account_id LIKE 'tok_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn purchase_migration_backfills_only_valid_completed_apple_history() {
        let db = purchase_db();
        {
            let conn = db.connection.lock().unwrap();
            for (email, app, status) in [
                (" User@Example.com ", "123", "completed"),
                ("user@example.com", "456", "failed"),
                ("手动上传", "789", "completed"),
                ("未知账号", "987", "completed"),
                ("user@example.com", "upload-123", "completed"),
                ("user@example.com", "", "completed"),
                ("", "321", "completed"),
            ] {
                conn.execute("INSERT INTO download_records(app_name,app_id,account_email,status) VALUES ('App',?,?,?)", params![app,email,status]).unwrap();
            }
            Database::migrate_tables(&conn).unwrap();
            Database::migrate_tables(&conn).unwrap();
        }
        assert_eq!(
            db.get_purchased_app_ids("user@example.com").unwrap(),
            vec!["123"]
        );
    }

    #[test]
    fn account_session_update_requires_an_existing_account() {
        let db = purchase_db();
        Database::migrate_tables(&db.connection.lock().unwrap()).unwrap();
        assert!(db
            .update_account_session("missing-token", "encrypted")
            .is_err());
    }

    #[test]
    fn purchase_apis_use_canonical_email_keys() {
        let db = purchase_db();
        db.record_purchase(" User@Example.com ", "123", "claim")
            .unwrap();
        db.record_purchase("USER@EXAMPLE.COM", "123", "apple_api")
            .unwrap();
        assert!(db.is_purchased("user@example.com", "123").unwrap());
        assert_eq!(
            db.get_purchased_app_ids(" USER@example.com ").unwrap(),
            vec!["123"]
        );
        assert!(db
            .batch_check_purchased("USER@example.com", &["123".into(), "456".into()])
            .unwrap()
            .contains("123"));
        assert!(db
            .record_purchase("tok_not-an-email", "123", "claim")
            .is_err());
    }

    #[test]
    fn purchase_migration_rolls_back_and_surfaces_write_errors() {
        let db = purchase_db();
        insert_purchase(&db, " First@Example.com ", "123", "claim");
        insert_purchase(&db, " Second@Example.com ", "456", "claim");
        let conn = db.connection.lock().unwrap();
        conn.execute_batch("CREATE TRIGGER reject_purchase BEFORE INSERT ON purchase_records WHEN NEW.account_id='second@example.com' BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
        assert!(Database::migrate_tables(&conn).is_err());
        let unchanged: i64 = conn.query_row("SELECT COUNT(*) FROM purchase_records WHERE account_id IN (' First@Example.com ', ' Second@Example.com ')", [], |r| r.get(0)).unwrap();
        assert_eq!(unchanged, 2);
    }

    #[test]
    fn purchase_token_prefix_collisions_for_same_normalized_owner_are_safe() {
        let db = purchase_db();
        {
            let conn = db.connection.lock().unwrap();
            conn.execute("INSERT INTO accounts(token,email) VALUES ('shared-prefix-123-A', ' User@Example.com '), ('shared-prefix-123-B', 'user@example.com')", []).unwrap();
        }
        insert_purchase(&db, "tok_shared-prefix-12", "123", "claim");
        Database::migrate_tables(&db.connection.lock().unwrap()).unwrap();
        assert!(db.is_purchased("user@example.com", "123").unwrap());
    }

    #[test]
    fn purchase_valid_email_with_tok_prefix_is_still_canonicalized() {
        let db = purchase_db();
        insert_purchase(&db, "tok_User@Example.com", "123", "claim");
        Database::migrate_tables(&db.connection.lock().unwrap()).unwrap();
        assert!(db.is_purchased("tok_user@example.com", "123").unwrap());
    }

    #[test]
    fn purchase_write_failure_prevents_account_token_replacement() {
        let db = purchase_db();
        Database::migrate_tables(&db.connection.lock().unwrap()).unwrap();
        let mut account = Account {
            id: None,
            token: "old-token-123456789".into(),
            email: "user@example.com".into(),
            region: "US".into(),
            guid: None,
            cookie_user: None,
            cookies: None,
            created_at: None,
            updated_at: None,
        };
        db.save_account(&account).unwrap();
        insert_purchase(&db, "tok_old-token-123456", "123", "claim");
        db.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_purchase BEFORE INSERT ON purchase_records BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
        account.token = "new-token-123456789".into();
        assert!(db.save_account(&account).is_err());
        assert!(db
            .record_purchase("user@example.com", "456", "claim")
            .is_err());
        assert!(db
            .get_account_by_token("old-token-123456789")
            .unwrap()
            .is_some());
    }

    #[test]
    fn purchase_relogin_migrates_old_token_before_overwrite() {
        let db = purchase_db();
        Database::migrate_tables(&db.connection.lock().unwrap()).unwrap();
        let mut account = Account {
            id: None,
            token: "old-token-123456789".into(),
            email: "User@Example.com".into(),
            region: "US".into(),
            guid: None,
            cookie_user: None,
            cookies: None,
            created_at: None,
            updated_at: None,
        };
        db.save_account(&account).unwrap();
        insert_purchase(&db, "tok_old-token-123456", "123", "claim");
        account.token = "new-token-123456789".into();
        db.save_account(&account).unwrap();
        assert!(db.is_purchased("user@example.com", "123").unwrap());
        assert!(db
            .get_account_by_token("old-token-123456789")
            .unwrap()
            .is_none());
    }

    #[test]
    fn reset_admin_password_updates_hash_and_revokes_sessions_without_changing_default_flag() {
        let db_path = temp_db_path("admin-password-reset");
        let db = Database::new(db_path.to_str().unwrap()).unwrap();

        db.create_session("session-token", "admin", "2999-01-01 00:00:00")
            .unwrap();
        assert!(db.get_session("session-token").unwrap().is_some());

        let before = db.get_admin_user("admin").unwrap().unwrap();
        assert!(before.is_default);
        assert!(db
            .reset_admin_password("admin", "new-secure-password")
            .unwrap());

        let after = db.get_admin_user("admin").unwrap().unwrap();
        assert!(after.is_default);
        assert_ne!(before.password_hash, after.password_hash);
        assert!(bcrypt::verify("new-secure-password", &after.password_hash).unwrap());
        assert!(db.get_session("session-token").unwrap().is_none());

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn change_password_and_rename_preserves_current_session_and_updates_username() {
        let db_path = temp_db_path("admin-change-password-rename-session");
        let db = Database::new(db_path.to_str().unwrap()).unwrap();

        db.create_session("current-session", "admin", "2999-01-01 00:00:00")
            .unwrap();
        db.create_session("old-session", "admin", "2999-01-01 00:00:00")
            .unwrap();

        let final_username = db
            .change_password_and_rename(
                "admin",
                &Database::hash_password("new-secure-password"),
                false,
                Some("owner"),
                Some("current-session"),
            )
            .unwrap();

        assert_eq!(final_username, "owner");

        let user = db.get_admin_user("owner").unwrap().unwrap();
        assert!(!user.is_default);
        assert!(bcrypt::verify("new-secure-password", &user.password_hash).unwrap());
        assert!(db.get_admin_user("admin").unwrap().is_none());

        let current_session = db.get_session("current-session").unwrap().unwrap();
        assert_eq!(current_session.username, "owner");
        assert!(db.get_session("old-session").unwrap().is_none());

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn change_password_without_rename_revokes_other_sessions_only() {
        let db_path = temp_db_path("admin-change-password-revoke");
        let db = Database::new(db_path.to_str().unwrap()).unwrap();

        db.create_session("current-session", "admin", "2999-01-01 00:00:00")
            .unwrap();
        db.create_session("stolen-session", "admin", "2999-01-01 00:00:00")
            .unwrap();

        let final_username = db
            .change_password_and_rename(
                "admin",
                &Database::hash_password("new-secure-password"),
                false,
                None,
                Some("current-session"),
            )
            .unwrap();

        assert_eq!(final_username, "admin");
        assert!(db.get_session("current-session").unwrap().is_some());
        assert!(db.get_session("stolen-session").unwrap().is_none());

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn get_session_ignores_expired_sessions() {
        let db_path = temp_db_path("admin-session-expired");
        let db = Database::new(db_path.to_str().unwrap()).unwrap();

        db.create_session("expired", "admin", "2000-01-01 00:00:00")
            .unwrap();
        assert!(db.get_session("expired").unwrap().is_none());
        assert_eq!(db.cleanup_expired_sessions().unwrap(), 1);

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn reset_admin_password_returns_false_for_missing_user() {
        let db_path = temp_db_path("admin-password-reset-missing");
        let db = Database::new(db_path.to_str().unwrap()).unwrap();

        assert!(!db
            .reset_admin_password("missing-admin", "new-secure-password")
            .unwrap());

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn find_reusable_download_record_returns_latest_completed_match() {
        let db_path = temp_db_path("download-record-reuse");
        let db = Database::new(db_path.to_str().unwrap()).unwrap();

        let old_record = sample_record(
            "414478124",
            "8.0.58",
            "ruan@example.com",
            "/tmp/wechat-old.ipa",
        );
        db.add_download_record(&old_record).unwrap();

        let mut latest_record = sample_record(
            "414478124",
            "8.0.58",
            "ruan@example.com",
            "/tmp/wechat-latest.ipa",
        );
        latest_record.job_id = Some("job-2".to_string());
        latest_record.download_date = Some("2026-04-22T13:00:00Z".to_string());
        db.add_download_record(&latest_record).unwrap();

        let reused = db
            .find_reusable_download_record("414478124", "8.0.58", "ruan@example.com")
            .unwrap()
            .unwrap();

        assert_eq!(reused.job_id.as_deref(), Some("job-2"));
        assert_eq!(reused.file_path.as_deref(), Some("/tmp/wechat-latest.ipa"));

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn find_reusable_download_record_ignores_failed_or_other_account_records() {
        let db_path = temp_db_path("download-record-filter");
        let db = Database::new(db_path.to_str().unwrap()).unwrap();

        let mut failed = sample_record(
            "414478124",
            "8.0.58",
            "ruan@example.com",
            "/tmp/wechat-failed.ipa",
        );
        failed.status = "failed".to_string();
        db.add_download_record(&failed).unwrap();

        let other_account = sample_record(
            "414478124",
            "8.0.58",
            "other@example.com",
            "/tmp/wechat-other.ipa",
        );
        db.add_download_record(&other_account).unwrap();

        let reused = db
            .find_reusable_download_record("414478124", "8.0.58", "ruan@example.com")
            .unwrap();

        assert!(reused.is_none());

        let _ = fs::remove_file(db_path);
    }
}
