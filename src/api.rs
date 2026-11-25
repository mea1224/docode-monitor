use crate::model::{Announcement, Balance, Credentials, Status, User};
use anyhow::{Context, Result, anyhow, bail};
use reqwest::{
    Url,
    blocking::{Client, Response},
    cookie::CookieStore as _,
    header::{HOST, HeaderValue},
};
use reqwest_cookie_store::CookieStoreMutex;
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    io::Read,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const BASE_URL: &str = "https://docode.cc";

/// Captures every Set-Cookie separately (including redirects) while retaining
/// domain, path, expiry, Secure and HttpOnly semantics in a real cookie store.
#[derive(Default, Debug)]
struct CompleteCookieJar {
    store: CookieStoreMutex,
    headers: Mutex<Vec<String>>,
}

impl reqwest::cookie::CookieStore for CompleteCookieJar {
    fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let headers: Vec<_> = headers.cloned().collect();
        if let Ok(mut all) = self.headers.lock() {
            all.extend(
                headers
                    .iter()
                    .map(|header| String::from_utf8_lossy(header.as_bytes()).into_owned()),
            );
        }
        self.store.set_cookies(&mut headers.iter(), url);
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        self.store.cookies(url)
    }
}

pub struct Api {
    client: Client,
    base: Url,
    cookies: Arc<CompleteCookieJar>,
    pub user: User,
    pub credentials: Credentials,
}

#[derive(Deserialize)]
struct Envelope<T> {
    success: bool,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

impl Api {
    pub fn login(base: &str, credentials: Credentials) -> Result<Self> {
        let base = Url::parse(base).context("API 地址无效")?;
        let cookies = Arc::new(CompleteCookieJar::default());
        let origin = base.origin();
        let client = Client::builder()
            .cookie_provider(cookies.clone())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("docode-rs/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 5 || attempt.url().origin() != origin {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .context("创建网络客户端失败")?;
        let response = client
            .post(base.join("/api/user/login?turnstile=")?)
            .json(&credentials)
            .send()
            .context("登录请求失败")?;
        let user: User = decode(response)?;
        if user.id <= 0 {
            bail!("登录响应缺少有效用户 ID");
        }
        if cookies.cookies(&base.join("/api/user/self")?).is_none() {
            bail!("登录成功，但服务器未返回可用于后续请求的 Cookie");
        }
        Ok(Self {
            client,
            base,
            cookies,
            user,
            credentials,
        })
    }

    pub fn balance(&self) -> Result<Balance> {
        let response = self
            .client
            .get(self.base.join("/api/user/self")?)
            .header("New-API-User", self.user.id.to_string())
            .header(HOST, self.base.authority())
            .send()
            .context("获取余额失败")?;
        decode(response)
    }

    pub fn announcements(&self) -> Result<Vec<Announcement>> {
        let response = self
            .client
            .get(self.base.join("/api/status")?)
            .send()
            .context("获取公告失败")?;
        Ok(decode::<Status>(response)?.announcements)
    }

    pub fn cookie_snapshot(&self) -> Result<(Vec<u8>, Vec<String>)> {
        let store = self
            .cookies
            .store
            .lock()
            .map_err(|_| anyhow!("Cookie 存储锁不可用"))?;
        // Serializing each cookie explicitly includes session cookies: the default
        // cookie_store persistence API intentionally omits non-persistent cookies.
        let cookies = serde_json::to_vec(&store.iter_any().collect::<Vec<_>>())?;
        let headers = self
            .cookies
            .headers
            .lock()
            .map_err(|_| anyhow!("Cookie 响应头锁不可用"))?
            .clone();
        Ok((cookies, headers))
    }
}

fn decode<T: DeserializeOwned>(response: Response) -> Result<T> {
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .context("读取服务器响应失败")?;
    if bytes.len() > 2 * 1024 * 1024 {
        bail!("服务器响应过大");
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("服务器返回非 JSON 响应（HTTP {status}）"))?;
    let message = value
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !status.is_success() || value.get("success").and_then(|v| v.as_bool()) == Some(false) {
        if !message.trim().is_empty() {
            bail!("{message}");
        }
        bail!("请求失败，服务器未提供 message（HTTP {status}）");
    }
    let envelope: Envelope<T> =
        serde_json::from_value(value).context("服务器响应字段缺失或类型不正确")?;
    if !envelope.success {
        bail!("{}", envelope.message);
    }
    envelope.data.context("服务器未返回 data 数据")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    pub(crate) fn server(responses: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let size = header
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + size {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (base, handle)
    }

    pub(crate) fn response(status: &str, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}",
            body.len()
        )
    }

    fn credentials() -> Credentials {
        Credentials {
            username: "sample".into(),
            password: "secret".into(),
        }
    }
    const LOGIN: &str = r#"{"success":true,"message":"","data":{"id":7,"username":"sample","display_name":"Sample","group":"vip"}}"#;

    #[test]
    fn full_login_self_status_flow_keeps_all_cookies_and_required_headers() {
        let (base, server) = server(vec![
            response(
                "200 OK",
                "Set-Cookie: session=abc; HttpOnly; Path=/\r\nSet-Cookie: route=def; Path=/\r\n",
                LOGIN,
            ),
            response(
                "200 OK",
                "",
                r#"{"success":true,"data":{"quota":6532248664,"used_quota":45117751336,"request_count":21219,"group":"vip2000"}}"#,
            ),
            response(
                "200 OK",
                "",
                r#"{"success":true,"data":{"announcements":[{"content":"hello","publishDate":"2026-11-14T18:54:11.543Z"}]}}"#,
            ),
        ]);
        let api = Api::login(&base, credentials()).unwrap();
        assert_eq!(api.balance().unwrap().request_count, 21219);
        assert_eq!(api.announcements().unwrap()[0].content, "hello");
        let (snapshot, headers) = api.cookie_snapshot().unwrap();
        assert_eq!(headers.len(), 2);
        let persisted: serde_json::Value = serde_json::from_slice(&snapshot).unwrap();
        assert_eq!(persisted.as_array().unwrap().len(), 2);
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /api/user/login?turnstile= HTTP/1.1"));
        let login: serde_json::Value =
            serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(login["password"], "secret");
        assert!(requests[1].to_lowercase().contains("new-api-user: 7\r\n"));
        for request in &requests[1..] {
            let lower = request.to_lowercase();
            let cookie = lower
                .lines()
                .find(|line| line.starts_with("cookie:"))
                .unwrap();
            assert!(cookie.contains("session=abc") && cookie.contains("route=def"));
            assert!(!cookie.contains("httponly") && !cookie.contains("path="));
        }
    }

    #[test]
    fn failure_surfaces_server_message_even_on_http_errors() {
        for status in ["200 OK", "401 Unauthorized", "503 Service Unavailable"] {
            let (base, server) = server(vec![response(
                status,
                "",
                r#"{"success":false,"message":"用户名或密码错误"}"#,
            )]);
            let error = Api::login(&base, credentials()).err().unwrap().to_string();
            assert_eq!(error, "用户名或密码错误");
            server.join().unwrap();
        }
    }

    #[test]
    fn missing_cookie_and_html_are_reported_without_panicking() {
        for (body, expected) in [(LOGIN, "Cookie"), ("<html>Gateway error</html>", "非 JSON")] {
            let (base, server) = server(vec![response("200 OK", "", body)]);
            assert!(
                Api::login(&base, credentials())
                    .err()
                    .unwrap()
                    .to_string()
                    .contains(expected)
            );
            server.join().unwrap();
        }
    }
}
