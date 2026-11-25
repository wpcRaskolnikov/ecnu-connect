use anyhow::{Result, ensure};
use clap::Parser;

mod browser;
mod cas;
mod core;
mod executable;

use browser::Browser;

#[derive(Parser)]
#[command(version, about = "Log in to the ECNU campus VPN")]
struct Cli {
    /// Browser to use (default: first available Chrome, Edge, or Firefox).
    #[arg(long, value_enum)]
    browser: Option<Browser>,
    /// Local SOCKS5 proxy port.
    #[arg(long, default_value_t = 1080, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
    /// Local HTTP proxy port.
    #[arg(long, default_value_t = 1081, value_parser = clap::value_parser!(u16).range(1..))]
    http_port: u16,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    login(cli.browser, cli.port, cli.http_port)
}

fn login(browser: Option<Browser>, port: u16, http_port: u16) -> Result<()> {
    ensure!(port != http_port, "SOCKS5 和 HTTP 代理不能使用同一个端口");
    let core = core::prepare(port, http_port)?;
    let callback = browser::capture(browser)?;
    eprintln!("已收到 CAS 回调，正在启动 zju-connect。");
    core::run(core, callback)
}
