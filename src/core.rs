use std::ffi::OsStr;
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use anyhow::{Context, Result, ensure};

use crate::cas::{Callback, LOGIN_DOMAIN, VPN_HOST, VPN_PORT};
use crate::executable;

fn find_core() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ECNU_ZJU_CONNECT") {
        return executable::find(&path).context("找不到 ECNU_ZJU_CONNECT 指定的可执行文件");
    }

    let name = format!("zju-connect{}", std::env::consts::EXE_SUFFIX);
    if let Ok(cli) = std::env::current_exe() {
        let sibling = cli.with_file_name(&name);
        if executable::is_executable(&sibling) {
            return Ok(sibling);
        }
    }
    executable::find(OsStr::new(&name)).with_context(|| {
        format!("找不到 {name}；请将它放在 ecnu-connect 同目录、加入 PATH 或设置 ECNU_ZJU_CONNECT")
    })
}

pub fn command(port: u16, http_port: u16) -> Result<Command> {
    let socks_bind = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).to_string();
    let http_bind = SocketAddrV4::new(Ipv4Addr::LOCALHOST, http_port).to_string();
    let mut command = Command::new(find_core()?);
    command.args([
        "--protocol",
        "atrust",
        "--server",
        VPN_HOST,
        "--port",
        &VPN_PORT.to_string(),
        "--login-domain",
        LOGIN_DOMAIN,
        "--auth-type",
        "auth/cas",
        "--socks-bind",
        &socks_bind,
        "--http-bind",
        &http_bind,
    ]);
    Ok(command)
}

pub fn run(mut command: Command, callback: Callback) -> Result<()> {
    let mut child = command
        .stdin(Stdio::piped())
        .spawn()
        .context("无法启动 zju-connect")?;
    let stdin = child
        .stdin
        .take()
        .context("无法获得 zju-connect 的标准输入")
        .and_then(|mut stdin| {
            writeln!(stdin, "{}", callback.as_str()).context("无法向 zju-connect 写入回调")?;
            Ok(stdin)
        });
    let mut stdin = match stdin {
        Ok(stdin) => stdin,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    // Let the user answer any additional prompts from the VPN gateway.
    thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut stdin);
    });
    let status = child.wait().context("无法等待 zju-connect 退出")?;
    ensure!(status.success(), "zju-connect 退出：{status}");
    Ok(())
}
