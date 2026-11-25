use crate::{
    api::{Api, BASE_URL},
    model::{Announcement, Balance, Credentials, User},
    storage::Storage,
};
use std::sync::mpsc::{self, Receiver, Sender};

pub enum Command {
    Login(Credentials),
    Balance,
    Announcements,
}
pub enum Event {
    Ready {
        credentials: Option<Credentials>,
        notice: Option<String>,
        path: String,
    },
    Login(Result<User, String>),
    Balance(Result<Balance, String>),
    Announcements(Result<Vec<Announcement>, String>),
    StorageError(String),
}

/// A new login uses a fresh cookie jar and replaces the complete saved account
/// in one transaction before the UI is told that login succeeded.
pub(crate) fn login_and_save(
    storage: &mut Storage,
    base: &str,
    credentials: Credentials,
) -> anyhow::Result<Api> {
    let session = Api::login(base, credentials)?;
    let (cookies, headers) = session.cookie_snapshot()?;
    storage.save(&session.credentials, &session.user, &cookies, &headers)?;
    Ok(session)
}

pub fn start(ctx: eframe::egui::Context) -> (Sender<Command>, Receiver<Event>) {
    let (commands_tx, commands_rx) = mpsc::channel();
    let (events_tx, events_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let send = |event| {
            let _ = events_tx.send(event);
            ctx.request_repaint();
        };
        let mut storage = match Storage::open_default() {
            Ok((storage, notice)) => {
                let credentials = match storage.load_credentials() {
                    Ok(credentials) => credentials,
                    Err(error) => {
                        send(Event::StorageError(format!(
                            "读取保存的登录信息失败：{error:#}"
                        )));
                        None
                    }
                };
                send(Event::Ready {
                    credentials,
                    notice,
                    path: storage.path.display().to_string(),
                });
                Some(storage)
            }
            Err(error) => {
                send(Event::StorageError(format!("初始化数据库失败：{error:#}")));
                None
            }
        };
        let mut api: Option<Api> = None;
        for command in commands_rx {
            match command {
                Command::Login(credentials) => {
                    api = None;
                    let result = (|| -> anyhow::Result<User> {
                        if storage.is_none() {
                            storage = Some(Storage::open_default()?.0);
                        }
                        let session =
                            login_and_save(storage.as_mut().unwrap(), BASE_URL, credentials)?;
                        let user = session.user.clone();
                        api = Some(session);
                        Ok(user)
                    })();
                    send(Event::Login(result.map_err(|e| format!("登录失败：{e:#}"))));
                    // Already saved atomically above. Failed logins never overwrite
                    // the database with a rejected password or an older session.
                    continue;
                }
                Command::Balance => {
                    let result = api
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("尚未登录"))
                        .and_then(Api::balance);
                    send(Event::Balance(
                        result.map_err(|e| format!("获取余额失败：{e:#}")),
                    ));
                }
                Command::Announcements => {
                    let result = api
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("尚未登录"))
                        .and_then(Api::announcements);
                    send(Event::Announcements(
                        result.map_err(|e| format!("获取公告失败：{e:#}")),
                    ));
                }
            }
            // Persist cookie rotations as well as the original login response.
            if let (Some(storage), Some(api)) = (storage.as_mut(), api.as_ref()) {
                let result = api.cookie_snapshot().and_then(|(cookies, headers)| {
                    storage.save(&api.credentials, &api.user, &cookies, &headers)
                });
                if let Err(error) = result {
                    send(Event::StorageError(format!("更新登录凭据失败：{error:#}")));
                }
            }
        }
    });
    (commands_tx, events_rx)
}
