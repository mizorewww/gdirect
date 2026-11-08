use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn default_path() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("cannot locate local data directory")?
        .join("gdirect/state.sqlite3"))
}
pub struct Endpoint {
    pub ip: String,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
    /// The TLS identity that worked: "" for the real hostname, "g.cn" when a
    /// forged identity was needed (e.g. real SNI is reset on the path).
    pub sni: String,
}
/// What `best_route`/`scan` return: the endpoint and its working TLS identity.
pub struct Route {
    pub ip: std::net::IpAddr,
    /// Empty means the real hostname; "g.cn" means connect and verify as g.cn
    /// while the HTTP Host header carries the real Google hostname.
    pub sni: String,
}
pub struct Store {
    conn: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        // Create with restrictive permissions before SQLite ever writes credentials.
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS endpoints(ip TEXT PRIMARY KEY,latency_ms INTEGER,verified_at INTEGER NOT NULL,available INTEGER NOT NULL,error TEXT);
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS accounts(email TEXT PRIMARY KEY,credentials TEXT NOT NULL,token TEXT,expires_at INTEGER NOT NULL DEFAULT 0);
            DROP TABLE IF EXISTS downloads;")?;
        // Folder listings are cached per account; the old single-account cache is just dropped.
        if !columns(&conn, "folders")?.iter().any(|c| c == "account") {
            conn.execute_batch("DROP TABLE IF EXISTS folders;
                CREATE TABLE folders(account TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(account,id));")?;
        }
        migrate_single_account(&conn)?;
        // Added later: which TLS identity worked. Older databases gain it here.
        if !columns(&conn, "endpoints")?.iter().any(|c| c == "sni") {
            conn.execute_batch("ALTER TABLE endpoints ADD COLUMN sni TEXT NOT NULL DEFAULT ''")?;
        }
        Ok(Self { conn })
    }
    // verified_at is the last successful probe; failures keep it so stale IPs age out.
    pub fn endpoint(
        &self,
        ip: &str,
        latency: Option<i64>,
        error: Option<&str>,
        sni: &str,
    ) -> Result<()> {
        match latency {
            Some(ms) => self.conn.execute(
                "INSERT OR REPLACE INTO endpoints VALUES(?,?,?,1,NULL,?)",
                params![ip, ms, now(), sni],
            )?,
            None => self.conn.execute(
                "INSERT INTO endpoints VALUES(?,NULL,0,0,?,?) ON CONFLICT(ip) DO UPDATE SET latency_ms=NULL,available=0,error=excluded.error",
                params![ip, error, sni],
            )?,
        };
        Ok(())
    }
    /// Marks every working endpoint unverified (keeping it as a candidate
    /// for the next scan) and returns how many there were: they were verified
    /// over a network path that no longer applies.
    pub fn retire_endpoints(&self, reason: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE endpoints SET latency_ms=NULL,available=0,error=? WHERE available=1",
            [reason],
        )?)
    }
    /// The fallback identity proved itself: make it the stored one for this IP.
    pub fn set_sni(&self, ip: &str, sni: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE endpoints SET sni=?,available=1,verified_at=? WHERE ip=?",
            params![sni, now(), ip],
        )?;
        Ok(())
    }
    // Prunes endpoints without a success in 7 days and returns the best `limit` remaining.
    pub fn rescan_candidates(&self, limit: usize) -> Result<Vec<String>> {
        self.conn.execute(
            "DELETE FROM endpoints WHERE verified_at<=?",
            [now() - 7 * 86400],
        )?;
        let mut query = self.conn.prepare(
            "SELECT ip FROM endpoints ORDER BY available DESC,latency_ms,verified_at DESC LIMIT ?",
        )?;
        Ok(query
            .query_map([limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn best_route(&self) -> Result<Option<Route>> {
        let row = self.conn.query_row("SELECT ip,sni FROM endpoints WHERE available=1 AND verified_at>? ORDER BY latency_ms LIMIT 1",[now()-86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        row.map(|(ip, sni)| {
            Ok(Route {
                ip: ip.parse()?,
                sni,
            })
        })
        .transpose()
    }
    /// Every endpoint verified in the last day, fastest first.
    pub fn routes(&self, limit: usize) -> Result<Vec<Route>> {
        let mut query = self.conn.prepare(
            "SELECT ip,sni FROM endpoints WHERE available=1 AND verified_at>? ORDER BY latency_ms LIMIT ?",
        )?;
        let rows = query
            .query_map(params![now() - 86400, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(ip, sni)| {
                Ok(Route {
                    ip: ip.parse()?,
                    sni,
                })
            })
            .collect()
    }
    pub fn endpoints(&self) -> Result<Vec<Endpoint>> {
        let mut query = self.conn.prepare(
            "SELECT ip,latency_ms,error,sni FROM endpoints ORDER BY available DESC,latency_ms",
        )?;
        Ok(query
            .query_map([], |r| {
                Ok(Endpoint {
                    ip: r.get(0)?,
                    latency_ms: r.get(1)?,
                    error: r.get(2)?,
                    sni: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings VALUES(?,?)",
            params![key, value],
        )?;
        Ok(())
    }
    pub fn unset(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key=?", [key])?;
        Ok(())
    }
    /// Settings whose keys start with `prefix`, in key order.
    pub fn settings_with_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let mut query = self.conn.prepare(
            "SELECT key,value FROM settings WHERE substr(key,1,length(?1))=?1 ORDER BY key",
        )?;
        Ok(query
            .query_map([prefix], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
    /// Removes every setting whose key starts with `prefix`.
    pub fn remove_settings_with_prefix(&self, prefix: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM settings WHERE substr(key,1,length(?1))=?1",
            [prefix],
        )?)
    }
    /// Signed-in accounts, in sign-in order.
    pub fn accounts(&self) -> Result<Vec<String>> {
        let mut query = self
            .conn
            .prepare("SELECT email FROM accounts ORDER BY rowid")?;
        Ok(query
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    /// The account used when no --account is given.
    pub fn default_account(&self) -> Result<Option<String>> {
        let accounts = self.accounts()?;
        if let Some(chosen) = self.setting("account")?
            && accounts.contains(&chosen)
        {
            return Ok(Some(chosen));
        }
        Ok(accounts.into_iter().next())
    }
    pub fn set_default_account(&self, email: &str) -> Result<()> {
        self.set("account", email)
    }
    /// Picks a signed-in account: an exact (case-insensitive) address or an
    /// unambiguous prefix such as "work"; without a query, the default one.
    pub fn resolve_account(&self, query: Option<&str>) -> Result<Option<String>> {
        let Some(query) = query.map(str::trim).filter(|q| !q.is_empty()) else {
            return self.default_account();
        };
        let accounts = self.accounts()?;
        let query = query.to_lowercase();
        if let Some(exact) = accounts.iter().find(|a| a.to_lowercase() == query) {
            return Ok(Some(exact.clone()));
        }
        let matches: Vec<_> = accounts
            .iter()
            .filter(|a| a.to_lowercase().starts_with(&query))
            .collect();
        match matches.as_slice() {
            [one] => Ok(Some((*one).clone())),
            [] if accounts.is_empty() => bail!("no account signed in; run gdirect login"),
            [] => bail!(
                "no signed-in account matches {query}; signed in: {}",
                accounts.join(", ")
            ),
            many => bail!(
                "{query} matches several accounts: {}",
                many.iter()
                    .map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    pub fn credentials(&self, email: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT credentials FROM accounts WHERE email=?",
                [email],
                |r| r.get(0),
            )
            .optional()?)
    }
    /// Adds the account or replaces its credentials; a cached token is kept
    /// only when the credentials did not change.
    pub fn save_account(&self, email: &str, credentials: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts(email,credentials) VALUES(?1,?2) ON CONFLICT(email) DO UPDATE SET
                token=CASE WHEN credentials=?2 THEN token END,
                expires_at=CASE WHEN credentials=?2 THEN expires_at ELSE 0 END,
                credentials=?2",
            params![email, credentials],
        )?;
        Ok(())
    }
    pub fn token(&self, email: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT token FROM accounts WHERE email=? AND token IS NOT NULL AND expires_at>?",
                params![email, now() + 60],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn save_token(&self, email: &str, token: &str, expires_in: i64) -> Result<()> {
        // Refresh five minutes before Google's stated expiry.
        self.conn.execute(
            "UPDATE accounts SET token=?,expires_at=? WHERE email=?",
            params![token, now() + (expires_in - 300).max(60), email],
        )?;
        Ok(())
    }
    /// Removes one account with its token, folder cache and upload sessions.
    /// The shared OAuth client goes too once no account is left.
    pub fn remove_account(&self, email: &str) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM accounts WHERE email=?", [email])?;
        self.conn
            .execute("DELETE FROM folders WHERE account=?", [email])?;
        let prefix = upload_prefix(email);
        self.conn.execute(
            "DELETE FROM settings WHERE substr(key,1,length(?1))=?1",
            [prefix],
        )?;
        if self.accounts()?.is_empty() {
            self.conn.execute(
                "DELETE FROM settings WHERE key IN ('oauth_client','account') OR key LIKE 'upload:%'",
                [],
            )?;
        }
        Ok(removed > 0)
    }
    /// Settings key holding a resumable upload session for this account.
    pub fn upload_key(email: &str, session: &str) -> String {
        format!("{}{session}", upload_prefix(email))
    }
    /// Settings key of this account's background mount.
    pub fn mount_key(email: &str) -> String {
        format!("mount:{email}")
    }
    /// Accounts with a recorded background mount.
    pub fn mounted_accounts(&self) -> Result<Vec<String>> {
        let mut query = self
            .conn
            .prepare("SELECT substr(key,7) FROM settings WHERE key LIKE 'mount:%' ORDER BY key")?;
        Ok(query
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn cache(&self, account: &str, folder: &str, json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO folders VALUES(?,?,?,?)",
            params![account, folder, json, now()],
        )?;
        Ok(())
    }
    pub fn cached(&self, account: &str, folder: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT json FROM folders WHERE account=? AND id=?",
                [account, folder],
                |r| r.get(0),
            )
            .optional()?)
    }
}
fn upload_prefix(email: &str) -> String {
    format!("upload:{email}\n")
}
fn columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    Ok(conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?)
}
/// Databases from before multiple accounts kept one login in `auth` plus the
/// `oauth_credentials` setting; that login becomes the first, default account.
fn migrate_single_account(conn: &Connection) -> Result<()> {
    let has_auth: bool = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='auth'",
        [],
        |r| r.get::<_, i64>(0).map(|n| n > 0),
    )?;
    if !has_auth {
        return Ok(());
    }
    let legacy = conn
        .query_row(
            "SELECT a.account,a.token,a.expires_at,s.value FROM auth a JOIN settings s ON s.key='oauth_credentials' WHERE a.id=1",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let tx = conn.unchecked_transaction()?;
    if let Some((email, token, expires_at, credentials)) = legacy {
        tx.execute(
            "INSERT OR IGNORE INTO accounts VALUES(?,?,?,?)",
            params![email, credentials, token, expires_at],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO settings VALUES('account',?)",
            [&email],
        )?;
        tx.execute(
            "UPDATE OR IGNORE settings SET key=? WHERE key='mount'",
            [Store::mount_key(&email)],
        )?;
    }
    // Old upload sessions carry no account and cannot be resumed safely.
    tx.execute_batch(
        "DELETE FROM settings WHERE key IN ('oauth_credentials','mount') OR (key LIKE 'upload:%' AND instr(key,char(10))=0);
        DROP TABLE auth;",
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_require_recent_success() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE endpoints(ip TEXT PRIMARY KEY,latency_ms INTEGER,verified_at INTEGER,available INTEGER,error TEXT,sni TEXT NOT NULL DEFAULT '');").unwrap();
        let db = Store { conn };
        db.endpoint("183.56.143.147", Some(40), None, "g.cn")
            .unwrap();
        db.endpoint("8.8.8.8", Some(80), None, "").unwrap();
        let route = db.best_route().unwrap().unwrap();
        assert_eq!(route.ip.to_string(), "183.56.143.147");
        assert_eq!(route.sni, "g.cn");
        assert_eq!(db.routes(8).unwrap().len(), 2);
        db.endpoint("183.56.143.147", None, Some("timeout"), "")
            .unwrap();
        assert_eq!(db.best_route().unwrap().unwrap().ip.to_string(), "8.8.8.8");
        assert_eq!(db.routes(8).unwrap().len(), 1);
        db.conn
            .execute("UPDATE endpoints SET verified_at=0", [])
            .unwrap();
        assert!(db.best_route().unwrap().is_none());
    }
    #[test]
    fn fallback_identity_is_persisted() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE endpoints(ip TEXT PRIMARY KEY,latency_ms INTEGER,verified_at INTEGER,available INTEGER,error TEXT,sni TEXT NOT NULL DEFAULT '');").unwrap();
        let db = Store { conn };
        db.endpoint("8.8.8.8", Some(10), None, "").unwrap();
        db.set_sni("8.8.8.8", "g.cn").unwrap();
        let route = db.best_route().unwrap().unwrap();
        assert_eq!(route.sni, "g.cn");
        db.endpoint("8.8.8.8", None, Some("reset"), "").unwrap();
        assert!(db.best_route().unwrap().is_none());
        db.set_sni("8.8.8.8", "g.cn").unwrap();
        assert_eq!(db.best_route().unwrap().unwrap().sni, "g.cn");
    }
    #[test]
    fn old_databases_gain_the_sni_column() {
        let path =
            std::env::temp_dir().join(format!("gdirect-migrate-{}.sqlite3", std::process::id()));
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE endpoints(ip TEXT PRIMARY KEY,latency_ms INTEGER,verified_at INTEGER NOT NULL,available INTEGER NOT NULL,error TEXT);").unwrap();
        }
        let db = Store::open(&path).unwrap();
        db.endpoint("8.8.8.8", Some(10), None, "g.cn").unwrap();
        assert_eq!(db.best_route().unwrap().unwrap().sni, "g.cn");
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn stale_endpoints_are_pruned() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE endpoints(ip TEXT PRIMARY KEY,latency_ms INTEGER,verified_at INTEGER NOT NULL,available INTEGER NOT NULL,error TEXT,sni TEXT NOT NULL DEFAULT '');").unwrap();
        let db = Store { conn };
        db.endpoint("1.0.0.1", Some(30), None, "").unwrap();
        db.endpoint("1.0.0.2", Some(10), None, "").unwrap();
        db.endpoint("1.0.0.3", Some(20), None, "").unwrap();
        db.endpoint("1.0.0.3", None, Some("timeout"), "").unwrap();
        db.endpoint("1.0.0.4", None, Some("never worked"), "")
            .unwrap();
        db.conn
            .execute(
                "UPDATE endpoints SET verified_at=? WHERE ip='1.0.0.1'",
                [now() - 8 * 86400],
            )
            .unwrap();
        assert_eq!(db.rescan_candidates(256).unwrap(), ["1.0.0.2", "1.0.0.3"]);
        assert_eq!(db.rescan_candidates(1).unwrap(), ["1.0.0.2"]);
        assert_eq!(db.endpoints().unwrap().len(), 2);
    }
    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gdirect-{name}-{}.sqlite3", std::process::id()))
    }
    #[test]
    fn logout_keeps_endpoints_and_other_accounts() {
        let path = temp("logout");
        let db = Store::open(&path).unwrap();
        db.endpoint("8.8.8.8", Some(10), None, "").unwrap();
        db.save_account("a@example.com", "{a}").unwrap();
        db.save_account("b@example.com", "{b}").unwrap();
        db.set("oauth_client", "{}").unwrap();
        db.set(&Store::upload_key("a@example.com", "x"), "{}")
            .unwrap();
        db.set(&Store::upload_key("b@example.com", "x"), "{}")
            .unwrap();
        db.set(&Store::upload_key("a@example.com", "tree:abc\ndir:"), "{}")
            .unwrap();
        db.set(&Store::upload_key("b@example.com", "tree:abc\ndir:"), "{}")
            .unwrap();
        db.set("connections", "4").unwrap();
        db.cache("a@example.com", "root", "[1]").unwrap();
        db.cache("b@example.com", "root", "[2]").unwrap();
        assert!(db.remove_account("a@example.com").unwrap());
        assert_eq!(db.accounts().unwrap(), ["b@example.com"]);
        assert!(db.cached("a@example.com", "root").unwrap().is_none());
        assert_eq!(
            db.cached("b@example.com", "root").unwrap().as_deref(),
            Some("[2]")
        );
        assert!(
            db.setting(&Store::upload_key("a@example.com", "x"))
                .unwrap()
                .is_none()
        );
        assert!(
            db.setting(&Store::upload_key("b@example.com", "x"))
                .unwrap()
                .is_some()
        );
        assert!(
            db.setting(&Store::upload_key("a@example.com", "tree:abc\ndir:"))
                .unwrap()
                .is_none()
        );
        assert!(db.setting("oauth_client").unwrap().is_some());
        assert!(db.remove_account("b@example.com").unwrap());
        assert!(
            db.setting(&Store::upload_key("b@example.com", "tree:abc\ndir:"))
                .unwrap()
                .is_none()
        );
        assert!(db.setting("oauth_client").unwrap().is_none());
        assert_eq!(db.setting("connections").unwrap().as_deref(), Some("4"));
        assert!(db.best_route().unwrap().is_some());
        assert!(!db.remove_account("b@example.com").unwrap());
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn accounts_resolve_by_address_or_prefix() {
        let path = temp("resolve");
        let db = Store::open(&path).unwrap();
        assert!(db.resolve_account(None).unwrap().is_none());
        assert!(db.resolve_account(Some("x")).is_err());
        db.save_account("work@corp.com", "{}").unwrap();
        db.save_account("Wife@home.com", "{}").unwrap();
        db.save_account("me@home.com", "{}").unwrap();
        // Without a chosen default, the first signed-in account is used.
        assert_eq!(
            db.default_account().unwrap().as_deref(),
            Some("work@corp.com")
        );
        db.set_default_account("me@home.com").unwrap();
        assert_eq!(
            db.resolve_account(None).unwrap().as_deref(),
            Some("me@home.com")
        );
        assert_eq!(
            db.resolve_account(Some("WIFE@HOME.COM"))
                .unwrap()
                .as_deref(),
            Some("Wife@home.com")
        );
        assert_eq!(
            db.resolve_account(Some("wo")).unwrap().as_deref(),
            Some("work@corp.com")
        );
        assert!(
            db.resolve_account(Some("w"))
                .unwrap_err()
                .to_string()
                .contains("several")
        );
        assert!(db.resolve_account(Some("nobody")).is_err());
        db.remove_account("me@home.com").unwrap();
        assert_eq!(
            db.default_account().unwrap().as_deref(),
            Some("work@corp.com")
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn settings_prefixes_are_isolated() {
        let path = temp("prefix");
        let db = Store::open(&path).unwrap();
        let a = "upload:a@example.com\ntree:abc\n";
        let b = "upload:b@example.com\ntree:abc\n";
        db.set(&format!("{a}dir:"), "root-id").unwrap();
        db.set(&format!("{a}file:one/two.txt"), "{}").unwrap();
        db.set(&format!("{b}dir:"), "other-id").unwrap();
        db.set("upload:a@example.com\ntree:other\ndir:", "third")
            .unwrap();
        assert_eq!(
            db.settings_with_prefix(a).unwrap(),
            [
                (format!("{a}dir:"), "root-id".into()),
                (format!("{a}file:one/two.txt"), "{}".into()),
            ]
        );
        assert_eq!(
            db.settings_with_prefix(&format!("{a}file:")).unwrap().len(),
            1
        );
        assert_eq!(db.remove_settings_with_prefix(a).unwrap(), 2);
        assert!(db.settings_with_prefix(a).unwrap().is_empty());
        assert_eq!(db.settings_with_prefix(b).unwrap().len(), 1);
        assert!(
            db.setting("upload:a@example.com\ntree:other\ndir:")
                .unwrap()
                .is_some()
        );
        assert_eq!(db.remove_settings_with_prefix(b).unwrap(), 1);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn changed_credentials_drop_the_cached_token() {
        let path = temp("token");
        let db = Store::open(&path).unwrap();
        db.save_account("a@example.com", "{1}").unwrap();
        db.save_token("a@example.com", "secret", 3600).unwrap();
        db.save_account("a@example.com", "{1}").unwrap();
        assert_eq!(
            db.token("a@example.com").unwrap().as_deref(),
            Some("secret")
        );
        db.save_account("a@example.com", "{2}").unwrap();
        assert!(db.token("a@example.com").unwrap().is_none());
        assert_eq!(
            db.credentials("a@example.com").unwrap().as_deref(),
            Some("{2}")
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn single_account_databases_migrate() {
        let path = temp("legacy");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                CREATE TABLE auth(id INTEGER PRIMARY KEY CHECK(id=1),account TEXT NOT NULL,token TEXT NOT NULL,expires_at INTEGER NOT NULL);
                CREATE TABLE folders(id TEXT PRIMARY KEY,json TEXT NOT NULL,updated_at INTEGER NOT NULL);
                INSERT INTO auth VALUES(1,'old@example.com','tok',9999999999);
                INSERT INTO folders VALUES('root','[]',0);
                INSERT INTO settings VALUES('oauth_credentials','{c}'),('oauth_client','{k}'),('mount','{m}'),('upload:[\"f\"]','{u}'),('connections','4');").unwrap();
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.accounts().unwrap(), ["old@example.com"]);
        assert_eq!(
            db.default_account().unwrap().as_deref(),
            Some("old@example.com")
        );
        assert_eq!(
            db.credentials("old@example.com").unwrap().as_deref(),
            Some("{c}")
        );
        assert_eq!(db.token("old@example.com").unwrap().as_deref(), Some("tok"));
        assert_eq!(db.mounted_accounts().unwrap(), ["old@example.com"]);
        assert_eq!(
            db.setting(&Store::mount_key("old@example.com"))
                .unwrap()
                .as_deref(),
            Some("{m}")
        );
        assert!(db.setting("oauth_credentials").unwrap().is_none());
        assert!(db.setting("upload:[\"f\"]").unwrap().is_none());
        assert_eq!(db.setting("oauth_client").unwrap().as_deref(), Some("{k}"));
        assert_eq!(db.setting("connections").unwrap().as_deref(), Some("4"));
        assert!(db.cached("old@example.com", "root").unwrap().is_none());
        drop(db);
        // Opening again is a no-op.
        let db = Store::open(&path).unwrap();
        assert_eq!(db.accounts().unwrap(), ["old@example.com"]);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn expired_tokens_and_settings() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE accounts(email TEXT PRIMARY KEY,credentials TEXT NOT NULL,token TEXT,expires_at INTEGER NOT NULL DEFAULT 0); CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT);").unwrap();
        let db = Store { conn };
        db.save_account("test@example.com", "{}").unwrap();
        assert!(db.token("test@example.com").unwrap().is_none());
        db.save_token("test@example.com", "secret", 3600).unwrap();
        assert_eq!(
            db.token("test@example.com").unwrap().as_deref(),
            Some("secret")
        );
        assert!(db.token("other@example.com").unwrap().is_none());
        db.conn
            .execute("UPDATE accounts SET expires_at=0", [])
            .unwrap();
        assert!(db.token("test@example.com").unwrap().is_none());
        db.set("connections", "8").unwrap();
        assert_eq!(db.setting("connections").unwrap().as_deref(), Some("8"));
    }
}
