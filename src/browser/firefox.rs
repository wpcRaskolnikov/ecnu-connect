use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cas::{CALLBACK_PATH, Callback, VPN_ENTRY, VPN_HOST};

use super::{
    DebugSocket, LOGIN_TIMEOUT, STARTUP_TIMEOUT, close_browser, connect, prompt_login, receive,
    send, wait_for_endpoint,
};

pub(super) fn callback(bin: &Path) -> Result<Callback> {
    // Snap Firefox has a private /tmp; a non-hidden path in HOME is visible to it.
    let profile_root = if cfg!(target_os = "linux") {
        std::env::var_os("HOME").map_or_else(std::env::temp_dir, std::path::PathBuf::from)
    } else {
        std::env::temp_dir()
    };
    let profile = tempfile::Builder::new()
        .prefix("ecnu-connect-firefox-")
        .tempdir_in(profile_root)
        .context("无法创建 Firefox 临时配置")?;
    let mut child = Command::new(bin)
        .args(["--no-remote", "--profile"])
        .arg(profile.path())
        .args(["--remote-debugging-port", "0", "about:blank"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动浏览器")?;
    let result = wait_for_endpoint(
        &profile.path().join("WebDriverBiDiServer.json"),
        &mut child,
        |text| {
            serde_json::from_str::<FirefoxEndpoint>(text)
                .ok()
                .map(FirefoxEndpoint::loopback)
        },
    )
    .and_then(connect_and_capture);
    close_browser(&mut child);
    result
}

#[derive(Deserialize)]
struct FirefoxEndpoint {
    ws_host: String,
    ws_port: u16,
}

impl FirefoxEndpoint {
    fn loopback(self) -> Result<SocketAddr> {
        let host = self.ws_host.trim_matches(['[', ']']);
        let ip = if host == "localhost" {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            host.parse::<IpAddr>()
                .context("Firefox 返回了无效的调试地址")?
        };
        ensure!(ip.is_loopback(), "拒绝连接非本机回环的 Firefox 调试地址");
        ensure!(self.ws_port != 0, "Firefox 返回了无效的调试端口");
        Ok(SocketAddr::new(ip, self.ws_port))
    }
}

fn connect_and_capture(endpoint: SocketAddr) -> Result<Callback> {
    let mut bidi = BiDi {
        socket: connect(endpoint, "/session")?,
        id: 0,
    };
    let result = bidi.capture();
    let _ = bidi.send("browser.close", json!({}));
    result
}

struct BiDi {
    socket: DebugSocket,
    id: u64,
}

impl BiDi {
    fn capture(&mut self) -> Result<Callback> {
        self.call("session.new", json!({"capabilities": {}}))?;
        let tab = self.call("browsingContext.create", json!({"type":"tab"}))?;
        let context = tab["context"]
            .as_str()
            .context("Firefox 未返回登录标签页 ID")?;
        self.call(
            "session.subscribe",
            json!({"events":["network.beforeRequestSent"],"contexts":[context]}),
        )?;
        self.call("network.addIntercept", json!({
            "phases":["beforeRequestSent"], "contexts":[context],
            "urlPatterns":[{"type":"pattern","protocol":"https","hostname":VPN_HOST,"pathname":CALLBACK_PATH}]
        }))?;
        let navigation = self.send(
            "browsingContext.navigate",
            json!({"context":context,"url":VPN_ENTRY,"wait":"none"}),
        )?;
        prompt_login();
        let deadline = Instant::now() + LOGIN_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("等待 Firefox 登录回调超时");
            }
            self.socket
                .get_ref()
                .set_read_timeout(Some(remaining))
                .context("无法设置 Firefox 登录超时")?;
            let event = receive(&mut self.socket)?;
            if event["id"].as_u64() == Some(navigation) && event["type"] != "success" {
                bail!("Firefox 无法打开 ECNU VPN 登录入口");
            }
            let params = &event["params"];
            if event["method"] != "network.beforeRequestSent"
                || params["context"].as_str() != Some(context)
                || params["isBlocked"].as_bool() != Some(true)
            {
                continue;
            }
            let request = &params["request"];
            let id = request["request"]
                .as_str()
                .context("Firefox 未返回回调请求 ID")?;
            let callback = request["url"]
                .as_str()
                .context("Firefox 未返回回调地址")
                .and_then(|url| Callback::parse(url, request["method"].as_str()));
            self.socket
                .get_ref()
                .set_read_timeout(Some(STARTUP_TIMEOUT))
                .context("无法设置 Firefox 读取超时")?;
            self.call("network.failRequest", json!({"request":id}))?;
            return callback.context("Firefox 已拦截无效的 CAS 回调，未转发");
        }
    }

    fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        self.id += 1;
        let message = json!({"id":self.id,"method":method,"params":params});
        send(&mut self.socket, message)?;
        Ok(self.id)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send(method, params)?;
        loop {
            let reply = receive(&mut self.socket)?;
            if reply["id"].as_u64() == Some(id) {
                ensure!(
                    reply["type"] == "success",
                    "Firefox 拒绝或不支持 WebDriver BiDi 操作 {method}"
                );
                return Ok(reply["result"].clone());
            }
        }
    }
}
