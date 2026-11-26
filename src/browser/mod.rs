mod chromium;
mod firefox;
mod process;

use std::ffi::OsStr;
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use clap::ValueEnum;
use serde_json::Value;
use tungstenite::{Message, WebSocket, client::client_with_config, protocol::WebSocketConfig};

use crate::{cas::Callback, executable};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_MESSAGE: usize = 1024 * 1024;

type DebugSocket = WebSocket<TcpStream>;

#[derive(Clone, Copy, ValueEnum)]
pub enum Browser {
    Chrome,
    Edge,
    Firefox,
}

impl Browser {
    fn name(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Firefox => "Firefox",
        }
    }

    fn executables(self) -> &'static [&'static str] {
        #[cfg(windows)]
        return match self {
            Self::Chrome => &["chrome.exe"],
            Self::Edge => &["msedge.exe"],
            Self::Firefox => &["firefox.exe"],
        };
        #[cfg(not(windows))]
        match self {
            Self::Chrome => &[
                "google-chrome",
                "google-chrome-stable",
                "google-chrome-beta",
                "google-chrome-unstable",
            ],
            Self::Edge => &[
                "microsoft-edge",
                "microsoft-edge-stable",
                "microsoft-edge-beta",
                "microsoft-edge-dev",
            ],
            Self::Firefox => &["firefox", "firefox-esr"],
        }
    }

    fn find(self) -> Option<PathBuf> {
        self.executables()
            .iter()
            .find_map(|name| executable::find(OsStr::new(name)))
            .or_else(|| self.find_installed())
    }

    #[cfg(target_os = "macos")]
    fn find_installed(self) -> Option<PathBuf> {
        let bundle = match self {
            Self::Chrome => "Google Chrome.app/Contents/MacOS/Google Chrome",
            Self::Edge => "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            Self::Firefox => "Firefox.app/Contents/MacOS/firefox",
        };
        std::iter::once(PathBuf::from("/Applications"))
            .chain(std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Applications")))
            .map(|root| root.join(bundle))
            .find(|path| executable::is_executable(path))
    }

    #[cfg(windows)]
    fn find_installed(self) -> Option<PathBuf> {
        let path = match self {
            Self::Chrome => r"Google\Chrome\Application\chrome.exe",
            Self::Edge => r"Microsoft\Edge\Application\msedge.exe",
            Self::Firefox => r"Mozilla Firefox\firefox.exe",
        };
        [
            "LOCALAPPDATA",
            "ProgramW6432",
            "ProgramFiles",
            "ProgramFiles(x86)",
        ]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join(path))
        .find(|path| executable::is_executable(path))
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn find_installed(self) -> Option<PathBuf> {
        None
    }
}

pub fn capture(preferred: Option<Browser>) -> Result<Callback> {
    let order = [Browser::Chrome, Browser::Edge, Browser::Firefox];
    let candidates = preferred
        .as_ref()
        .map_or(order.as_slice(), std::slice::from_ref);
    let mut selected = None;
    for &browser in candidates {
        let bin = if matches!(browser, Browser::Chrome)
            && let Some(path) = std::env::var_os("ECNU_CHROME")
        {
            Some(executable::find(&path).context("找不到 ECNU_CHROME 指定的可执行文件")?)
        } else {
            browser.find()
        };
        if let Some(bin) = bin {
            selected = Some((browser, bin));
            break;
        }
    }
    let (browser, bin) = selected.with_context(|| match preferred {
        Some(browser) => format!("找不到指定的浏览器 {}", browser.name()),
        None => "找不到 Chrome、Edge 或 Firefox；请安装其中一个浏览器".into(),
    })?;
    eprintln!("使用 {} 打开 ECNU 登录页。", browser.name());
    match browser {
        Browser::Chrome | Browser::Edge => chromium::callback(&bin),
        Browser::Firefox => firefox::callback(&bin),
    }
    .with_context(|| format!("{} 登录失败", browser.name()))
}

fn connect(endpoint: SocketAddr, path: &str) -> Result<DebugSocket> {
    ensure!(
        endpoint.ip().is_loopback(),
        "拒绝连接非本机回环的浏览器调试地址"
    );
    let stream = TcpStream::connect_timeout(&endpoint, STARTUP_TIMEOUT)
        .context("无法连接浏览器本机调试端口")?;
    stream
        .set_read_timeout(Some(STARTUP_TIMEOUT))
        .context("无法设置浏览器读取超时")?;
    stream
        .set_write_timeout(Some(STARTUP_TIMEOUT))
        .context("无法设置浏览器写入超时")?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    client_with_config(format!("ws://{endpoint}{path}"), stream, Some(config))
        .map(|(socket, _)| socket)
        .map_err(|_| anyhow!("无法建立浏览器调试 WebSocket 连接"))
}

fn send(socket: &mut DebugSocket, message: Value) -> Result<()> {
    socket
        .send(Message::text(message.to_string()))
        .map_err(|_| anyhow!("无法向浏览器发送调试命令"))
}

fn receive(socket: &mut DebugSocket) -> Result<Value> {
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                // Debug messages may contain the CAS ticket; don't expose their contents.
                return serde_json::from_str(text.as_str())
                    .map_err(|_| anyhow!("无法解析浏览器调试消息"));
            }
            Ok(Message::Close(_)) => bail!("浏览器已关闭，未收到登录回调"),
            Ok(_) => {}
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(err).context("等待浏览器调试响应或登录回调超时");
            }
            Err(_) => bail!("浏览器调试连接已关闭或无法读取"),
        }
    }
}

fn poll_events(
    socket: &mut DebugSocket,
    mut matches_event: impl FnMut(&Value) -> Result<bool>,
) -> Result<Value> {
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let result = (|| {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("等待浏览器登录回调超时");
            }
            socket
                .get_ref()
                .set_read_timeout(Some(remaining))
                .context("无法设置登录超时")?;
            let event = receive(socket)?;
            if matches_event(&event)? {
                return Ok(event);
            }
        }
    })();
    let restore_timeout = socket
        .get_ref()
        .set_read_timeout(Some(STARTUP_TIMEOUT))
        .context("无法设置浏览器读取超时");
    let event = result?;
    restore_timeout?;
    Ok(event)
}

fn prompt_login() {
    eprintln!("请在打开的 ECNU 页面输入账号密码，并按学校提示完成验证（等待最多 10 分钟）。");
}
