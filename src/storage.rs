use crate::model::{Credentials, User};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

pub struct Storage {
    connection: Connection,
    pub path: PathBuf,
}

impl Storage {
    pub fn open_default() -> Result<(Self, Option<String>)> {
        if let Some(path) = std::env::var_os("DOCODE_DATA_DIR") {
            return Ok((
                Self::open(&PathBuf::from(path).join("docode.sqlite3"))?,
                None,
            ));
        }
        let dirs = directories::BaseDirs::new().context("无法确定系统数据目录")?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        dirs.home_dir().hash(&mut hash);
        let user_folder = format!("user-{:016x}", hash.finish());
        let system_path = system_data_root().join(user_folder).join("docode.sqlite3");
        match Self::open(&system_path) {
            Ok(storage) => Ok((storage, None)),
            Err(_) => {
                let path = dirs
                    .data_local_dir()
                    .join("docode-rs")
                    .join("docode.sqlite3");
                let storage = Self::open(&path).context("系统公共目录和用户数据目录均不可写")?;
                Ok((
                    storage,
                    Some(format!(
                        "系统公共目录不可写，数据已保存在 {}",
                        path.display()
                    )),
                ))
            }
        }
    }

    pub fn open(path: &Path) -> Result<Self> {
        let parent = path.parent().context("数据库路径无效")?;
        std::fs::create_dir_all(parent).context("创建数据目录失败")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            // Create privately before SQLite opens it, so secrets are never world readable.
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .mode(0o600)
                .open(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        let connection = Connection::open(path).context("打开 SQLite 失败")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA secure_delete = ON;
            CREATE TABLE IF NOT EXISTS account (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                username TEXT NOT NULL,
                password BLOB NOT NULL,
                cookie_store BLOB NOT NULL,
                set_cookie_headers BLOB NOT NULL,
                user_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        Ok(Self {
            connection,
            path: path.to_owned(),
        })
    }

    pub fn load_credentials(&self) -> Result<Option<Credentials>> {
        let row: Option<(String, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT username, password FROM account WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(username, password)| {
            Ok(Credentials {
                username,
                password: String::from_utf8(unprotect(&password)?).context("保存的密码编码无效")?,
            })
        })
        .transpose()
    }

    pub fn save(
        &mut self,
        credentials: &Credentials,
        user: &User,
        cookies: &[u8],
        headers: &[String],
    ) -> Result<()> {
        if cookies.is_empty() {
            bail!("服务器未返回有效 Cookie，无法保存登录信息");
        }
        let transaction = self.connection.transaction()?;
        transaction.execute("INSERT INTO account (singleton, username, password, cookie_store, set_cookie_headers, user_json)
            VALUES (1, ?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(singleton) DO UPDATE SET username=excluded.username, password=excluded.password,
                cookie_store=excluded.cookie_store, set_cookie_headers=excluded.set_cookie_headers,
                user_json=excluded.user_json, updated_at=CURRENT_TIMESTAMP",
            params![credentials.username, protect(credentials.password.as_bytes())?, protect(cookies)?,
                protect(&serde_json::to_vec(headers)?)?, serde_json::to_string(user)?])?;
        transaction.commit()?;
        Ok(())
    }
}

fn system_data_root() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into()))
            .join("docode-rs")
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/Library/Application Support/docode-rs")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        PathBuf::from("/var/lib/docode-rs")
    }
}

#[cfg(not(windows))]
fn protect(data: &[u8]) -> Result<Vec<u8>> {
    Ok(data.to_vec())
}
#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Result<Vec<u8>> {
    crypt(data, true)
}
#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    crypt(data, false)
}

#[cfg(windows)]
fn crypt(data: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len().try_into()?,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // DPAPI binds secrets to the current Windows account; never prompts for UI.
    unsafe {
        let success = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error()).context("Windows 凭据加密/解密失败");
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrected_login_replaces_entire_saved_account_after_automatic_login_failure() {
        use crate::{
            api::tests::{response, server},
            worker::login_and_save,
        };

        // Cover both a stale password and a different/incorrect saved username.
        for old_username in ["new-user", "old-user"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("account.sqlite3");
            let mut storage = Storage::open(&path).unwrap();
            let old = Credentials {
                username: old_username.into(),
                password: "old-password".into(),
            };
            let old_user = User {
                id: 7,
                username: old.username.clone(),
                display_name: "Old".into(),
                group: "old-group".into(),
            };
            storage
                .save(
                    &old,
                    &old_user,
                    b"old-cookie",
                    &["session=old-cookie".into()],
                )
                .unwrap();
            drop(storage);

            let success = r#"{"success":true,"data":{"id":42,"username":"new-user","display_name":"New","group":"new-group"}}"#;
            let new_headers = "Set-Cookie: session=new-cookie; HttpOnly; Path=/\r\nSet-Cookie: route=new-route; Path=/\r\n";
            let (base, server) = server(vec![
                response(
                    "200 OK",
                    "Set-Cookie: rejected=discard; Path=/\r\n",
                    r#"{"success":false,"message":"用户名或密码错误"}"#,
                ),
                response("200 OK", new_headers, success),
                response("200 OK", new_headers, success),
            ]);
            let mut storage = Storage::open(&path).unwrap();
            let automatic = storage.load_credentials().unwrap().unwrap();
            let error = login_and_save(&mut storage, &base, automatic)
                .err()
                .unwrap();
            assert!(error.to_string().contains("用户名或密码错误"));
            assert_eq!(
                storage.load_credentials().unwrap().unwrap().password,
                "old-password"
            );

            let corrected = Credentials {
                username: "new-user".into(),
                password: "new-password".into(),
            };
            let session = login_and_save(&mut storage, &base, corrected).unwrap();
            let (expected_cookies, expected_headers) = session.cookie_snapshot().unwrap();
            drop(storage);

            let mut storage = Storage::open(&path).unwrap();
            let restored = storage.load_credentials().unwrap().unwrap();
            assert_eq!(restored.username, "new-user");
            assert_eq!(restored.password, "new-password");
            let (cookies, headers, user_json): (Vec<u8>, Vec<u8>, String) = storage.connection.query_row(
                "SELECT cookie_store, set_cookie_headers, user_json FROM account WHERE singleton = 1",
                [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).unwrap();
            assert_eq!(unprotect(&cookies).unwrap(), expected_cookies);
            assert_eq!(
                serde_json::from_slice::<Vec<String>>(&unprotect(&headers).unwrap()).unwrap(),
                expected_headers
            );
            let user: User = serde_json::from_str(&user_json).unwrap();
            assert_eq!(user.id, 42);
            assert_eq!(user.username, "new-user");
            assert_eq!(user.group, "new-group");
            let count: i64 = storage
                .connection
                .query_row("SELECT count(*) FROM account", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 1);
            assert!(
                !String::from_utf8(expected_cookies)
                    .unwrap()
                    .contains("discard")
            );

            // A subsequent startup must use the corrected credentials.
            login_and_save(&mut storage, &base, restored).unwrap();
            let requests = server.join().unwrap();
            for request in &requests[1..] {
                let body: serde_json::Value =
                    serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
                assert_eq!(body["username"], "new-user");
                assert_eq!(body["password"], "new-password");
                assert!(!request.to_lowercase().contains("\r\ncookie:"));
            }
        }
    }

    #[test]
    fn sqlite_round_trip_preserves_password_and_every_cookie_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account.sqlite3");
        let mut storage = Storage::open(&path).unwrap();
        assert!(storage.load_credentials().unwrap().is_none());
        let credentials = Credentials {
            username: "测试用户".into(),
            password: " a\"b密码 ".into(),
        };
        let user = User {
            id: 7,
            display_name: "测试".into(),
            username: credentials.username.clone(),
            group: "vip".into(),
        };
        let headers = vec![
            "session=abc; HttpOnly; Path=/".into(),
            "route=xyz; Secure".into(),
        ];
        storage
            .save(
                &credentials,
                &user,
                b"session-and-persistent-cookie-json",
                &headers,
            )
            .unwrap();
        drop(storage);
        let storage = Storage::open(&path).unwrap();
        let restored = storage.load_credentials().unwrap().unwrap();
        assert_eq!(restored.username, credentials.username);
        assert_eq!(restored.password, credentials.password);
        let saved: Vec<u8> = storage
            .connection
            .query_row("SELECT set_cookie_headers FROM account", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(&unprotect(&saved).unwrap()).unwrap(),
            headers
        );
    }
}
