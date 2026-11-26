use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cas::{CALLBACK_PATH, Callback, VPN_ENTRY, VPN_HOST};

use super::{
    DebugSocket, connect, poll_events, process::BrowserProcess, prompt_login, receive, send,
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
    let mut command = Command::new(bin);
    command
        .args(["--no-remote", "--profile"])
        .arg(profile.path())
        .args(["--remote-debugging-port", "0", "about:blank"]);
    let mut browser = BrowserProcess::spawn(command, profile)?;
    let endpoint = browser.wait_for_endpoint("WebDriverBiDiServer.json", |text| {
        serde_json::from_str::<FirefoxEndpoint>(text)
            .ok()
            .map(FirefoxEndpoint::loopback)
    })?;
    connect_and_capture(endpoint)
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
    let result = wait_for_callback(&mut bidi);
    let _ = bidi.send("browser.close", json!({}));
    result
}

fn wait_for_callback(bidi: &mut BiDi) -> Result<Callback> {
    bidi.call("session.new", json!({"capabilities": {}}))?;
    let tab = bidi.call("browsingContext.create", json!({"type":"tab"}))?;
    let context = tab["context"]
        .as_str()
        .context("Firefox 未返回登录标签页 ID")?;
    bidi.call(
        "session.subscribe",
        json!({"events":["network.beforeRequestSent"],"contexts":[context]}),
    )?;
    bidi.call("network.addIntercept", json!({
        "phases":["beforeRequestSent"], "contexts":[context],
        "urlPatterns":[{"type":"pattern","protocol":"https","hostname":VPN_HOST,"pathname":CALLBACK_PATH}]
    }))?;
    let navigation = bidi.send(
        "browsingContext.navigate",
        json!({"context":context,"url":VPN_ENTRY,"wait":"none"}),
    )?;
    prompt_login();
    let event = poll_events(&mut bidi.socket, |event| {
        if event["id"].as_u64() == Some(navigation) && event["type"] != "success" {
            bail!("Firefox 无法打开 ECNU VPN 登录入口");
        }
        let params = &event["params"];
        Ok(event["method"] == "network.beforeRequestSent"
            && params["context"].as_str() == Some(context)
            && params["isBlocked"].as_bool() == Some(true))
    })?;
    let request = &event["params"]["request"];
    let id = request["request"]
        .as_str()
        .context("Firefox 未返回回调请求 ID")?;
    let callback = request["url"]
        .as_str()
        .context("Firefox 未返回回调地址")
        .and_then(|url| Callback::parse(url, request["method"].as_str()));
    bidi.call("network.failRequest", json!({"request":id}))?;
    callback.context("Firefox 已拦截无效的 CAS 回调，未转发")
}

struct BiDi {
    socket: DebugSocket,
    id: u64,
}

impl BiDi {
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
