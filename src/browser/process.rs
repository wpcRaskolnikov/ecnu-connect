use std::io;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tempfile::TempDir;

use super::STARTUP_TIMEOUT;

pub(super) struct BrowserProcess {
    child: Child,
    profile: TempDir,
}

impl BrowserProcess {
    pub(super) fn spawn(mut command: Command, profile: TempDir) -> Result<Self> {
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("无法启动浏览器")?;
        Ok(Self { child, profile })
    }

    pub(super) fn wait_for_endpoint<T>(
        &mut self,
        file: &str,
        parse: impl Fn(&str) -> Option<Result<T>>,
    ) -> Result<T> {
        let file = self.profile.path().join(file);
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().context("无法检查浏览器进程")? {
                bail!("浏览器已退出（{status}），未启动调试接口；请确认已进入图形会话");
            }
            match std::fs::read_to_string(&file) {
                Ok(contents) => {
                    if let Some(endpoint) = parse(&contents) {
                        return endpoint;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("无法读取浏览器调试地址文件"),
            }
            thread::sleep(Duration::from_millis(100));
        }
        bail!("等待浏览器调试接口超时；请确认浏览器版本支持登录拦截")
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        // Give a protocol-level close time to finish before removing the profile.
        let until = Instant::now() + Duration::from_secs(1);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < until {
            thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
