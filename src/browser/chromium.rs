use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::cas::{CALLBACK_PATH, Callback, VPN_ENTRY, VPN_HOST};

use super::{
    DebugSocket, LOGIN_TIMEOUT, STARTUP_TIMEOUT, close_browser, connect, prompt_login, receive,
    send, wait_for_endpoint,
};

pub(super) fn callback(bin: &Path) -> Result<Callback> {
    let profile = tempfile::Builder::new()
        .prefix("ecnu-connect-")
        .tempdir()
        .context("无法创建临时浏览器配置")?;
    let mut profile_arg = OsString::from("--user-data-dir=");
    profile_arg.push(profile.path());
    let mut child = Command::new(bin)
        .arg(profile_arg)
        .args([
            "--no-startup-window",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-address=127.0.0.1",
            "--remote-debugging-port=0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动浏览器")?;

    let result = wait_for_endpoint(
        &profile.path().join("DevToolsActivePort"),
        &mut child,
        |text| parse_chromium_endpoint(text).map(|(port, path)| Ok((port, path.to_owned()))),
    )
    .and_then(|(port, path)| connect_and_capture(port, &path));
    close_browser(&mut child);
    result
}

fn parse_chromium_endpoint(text: &str) -> Option<(u16, &str)> {
    let mut lines = text.lines();
    let port = lines
        .next()?
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)?;
    let path = lines.next()?;
    let id = path.strip_prefix("/devtools/browser/")?;
    if id.is_empty() || !id.bytes().all(|ch| ch.is_ascii_hexdigit() || ch == b'-') {
        return None;
    }
    Some((port, path))
}

fn connect_and_capture(port: u16, path: &str) -> Result<Callback> {
    let endpoint = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut cdp = ChromiumCdp {
        socket: connect(endpoint, path)?,
        id: 0,
    };
    let result = wait_for_callback(&mut cdp);
    let _ = cdp.send("Browser.close", json!({}), None);
    result
}

fn wait_for_callback(cdp: &mut ChromiumCdp) -> Result<Callback> {
    let target = cdp.call("Target.createTarget", json!({"url":"about:blank"}), None)?;
    let target_id = target["targetId"].as_str().context("浏览器未创建登录页")?;
    let attach = cdp.call(
        "Target.attachToTarget",
        json!({"targetId":target_id,"flatten":true}),
        None,
    )?;
    let session = attach["sessionId"]
        .as_str()
        .context("浏览器未创建登录会话")?;
    let pattern = format!("https://{VPN_HOST}{CALLBACK_PATH}*");
    cdp.call("Fetch.enable", json!({"patterns":[{"urlPattern":pattern,"resourceType":"Document","requestStage":"Request"}]}), Some(session))?;
    let nav = cdp.send("Page.navigate", json!({"url":VPN_ENTRY}), Some(session))?;
    prompt_login();
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("等待浏览器登录回调超时");
        }
        cdp.socket
            .get_ref()
            .set_read_timeout(Some(remaining))
            .context("无法设置登录超时")?;
        let event = receive(&mut cdp.socket)?;
        if event["id"].as_u64() == Some(nav)
            && (event.get("error").is_some() || event["result"].get("errorText").is_some())
        {
            bail!("浏览器无法打开 ECNU VPN 登录入口");
        }
        if event["sessionId"].as_str() != Some(session) || event["method"] != "Fetch.requestPaused"
        {
            continue;
        }
        let params = &event["params"];
        let id = params["requestId"]
            .as_str()
            .context("浏览器未返回回调请求 ID")?;
        let request = &params["request"];
        let callback = request["url"]
            .as_str()
            .context("浏览器未返回回调地址")
            .and_then(|url| Callback::parse(url, request["method"].as_str()));
        cdp.socket
            .get_ref()
            .set_read_timeout(Some(STARTUP_TIMEOUT))
            .context("无法设置浏览器读取超时")?;
        // Abort at the request stage; only zju-connect should redeem this ticket.
        cdp.call(
            "Fetch.failRequest",
            json!({"requestId":id,"errorReason":"Aborted"}),
            Some(session),
        )?;
        return callback.context("已拦截无效的 CAS 回调，未转发");
    }
}

struct ChromiumCdp {
    socket: DebugSocket,
    id: u64,
}

impl ChromiumCdp {
    fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<u64> {
        self.id += 1;
        let mut command = json!({"id":self.id,"method":method,"params":params});
        if let Some(session) = session {
            command["sessionId"] = json!(session);
        }
        send(&mut self.socket, command)?;
        Ok(self.id)
    }

    fn call(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let id = self.send(method, params, session)?;
        loop {
            let message = receive(&mut self.socket)?;
            if message["id"].as_u64() == Some(id) {
                return message
                    .get("result")
                    .cloned()
                    .with_context(|| format!("浏览器拒绝操作 {method}"));
            }
        }
    }
}
