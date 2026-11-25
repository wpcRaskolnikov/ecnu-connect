#[cfg(not(feature = "bundled-core"))]
use std::ffi::OsStr;
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use anyhow::{Context, Result, ensure};

use crate::cas::{Callback, LOGIN_DOMAIN, VPN_HOST, VPN_PORT};
use crate::executable;

pub struct Core {
    command: Command,
    #[cfg(feature = "bundled-core")]
    _temp_dir: Option<tempfile::TempDir>,
}

impl Core {
    fn external(path: PathBuf) -> Self {
        Self {
            command: Command::new(path),
            #[cfg(feature = "bundled-core")]
            _temp_dir: None,
        }
    }
}

fn find_core() -> Result<Core> {
    if let Some(path) = std::env::var_os("ZJU_CONNECT") {
        return executable::find(&path)
            .map(Core::external)
            .context("找不到 ZJU_CONNECT 指定的可执行文件");
    }

    let name = format!("zju-connect{}", std::env::consts::EXE_SUFFIX);
    #[cfg(feature = "bundled-core")]
    {
        extract_core(&name)
    }
    #[cfg(not(feature = "bundled-core"))]
    {
        if let Ok(cli) = std::env::current_exe() {
            let sibling = cli.with_file_name(&name);
            if executable::is_executable(&sibling) {
                return Ok(Core::external(sibling));
            }
        }
        executable::find(OsStr::new(&name))
            .map(Core::external)
            .with_context(|| {
                format!(
                    "找不到 {name}；请将它放在 ecnu-connect 同目录、加入 PATH 或设置 ZJU_CONNECT"
                )
            })
    }
}

#[cfg(feature = "bundled-core")]
fn extract_core(name: &str) -> Result<Core> {
    let dir = tempfile::Builder::new()
        .prefix("ecnu-connect-core-")
        .tempdir()
        .context("无法创建内嵌 zju-connect 的临时目录")?;
    let path = dir.path().join(name);
    std::fs::write(
        &path,
        include_bytes!(concat!(env!("OUT_DIR"), "/bundled-zju-connect")),
    )
    .context("无法提取内嵌 zju-connect")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .context("无法设置内嵌 zju-connect 的执行权限")?;
    }
    Ok(Core {
        command: Command::new(path),
        _temp_dir: Some(dir),
    })
}

pub fn prepare(port: u16, http_port: u16) -> Result<Core> {
    let socks_bind = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).to_string();
    let http_bind = SocketAddrV4::new(Ipv4Addr::LOCALHOST, http_port).to_string();
    let mut core = find_core()?;
    core.command.args([
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
    Ok(core)
}

pub fn run(mut core: Core, callback: Callback) -> Result<()> {
    let mut child = core
        .command
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
