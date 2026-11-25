use anyhow::{Context, Result, bail, ensure};
use url::Url;

pub const VPN_HOST: &str = "vpn.ecnu.edu.cn";
pub const VPN_PORT: u16 = 443;
pub const LOGIN_DOMAIN: &str = "ecnu.edu.cn";
pub const VPN_ENTRY: &str =
    "https://vpn.ecnu.edu.cn/passport/v1/public/casLogin?sfDomain=ecnu.edu.cn";
pub const CALLBACK_PATH: &str = "/passport/v1/auth/cas";

const MAX_CALLBACK_BYTES: usize = 8192;

pub struct Callback(String);

impl Callback {
    pub fn parse(raw: &str, method: Option<&str>) -> Result<Self> {
        ensure!(method == Some("GET"), "CAS 回调请求不是 GET");
        ensure!(
            raw.len() <= MAX_CALLBACK_BYTES && !raw.chars().any(char::is_control),
            "CAS 回调地址过长或包含控制字符"
        );
        let url = Url::parse(raw).context("CAS 回调地址格式无效")?;
        ensure!(
            url.scheme() == "https"
                && url.host_str() == Some(VPN_HOST)
                && url.port_or_known_default() == Some(VPN_PORT)
                && url.path() == CALLBACK_PATH
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "CAS 回调未指向预期的 ECNU VPN 地址"
        );
        let mut ticket = false;
        let mut domain = false;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "ticket" if !ticket && !value.is_empty() => ticket = true,
                "sfDomain" if !domain && value == LOGIN_DOMAIN => domain = true,
                "ticket" | "sfDomain" => bail!("CAS 回调的票据或登录域为空、无效或重复"),
                _ => {}
            }
        }
        ensure!(ticket && domain, "CAS 回调缺少票据或登录域");
        Ok(Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
