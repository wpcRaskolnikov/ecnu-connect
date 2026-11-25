fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "bundled-core")]
    bundle::prepare()?;
    Ok(())
}

#[cfg(feature = "bundled-core")]
mod bundle {
    use std::env;
    use std::fmt::Write as _;
    use std::fs::File;
    use std::io::{self, Cursor};
    use std::path::PathBuf;
    use std::time::Duration;

    use anyhow::{Context, Result, bail, ensure};
    use sha2::{Digest, Sha256};
    use ureq::tls::{RootCerts, TlsConfig};
    use zip::ZipArchive;

    const VERSION: &str = "v1.3.1";
    const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
    const MAX_CORE_BYTES: u64 = 64 * 1024 * 1024;

    pub(super) fn prepare() -> Result<()> {
        let target = env::var("TARGET").context("Cargo did not set TARGET")?;
        let (platform, expected_sha256) = asset(&target)?;
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").context("Cargo did not set OUT_DIR")?);
        let url = format!(
            "https://github.com/Mythologyli/zju-connect/releases/download/{VERSION}/zju-connect-{platform}.zip"
        );
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(90)))
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        let mut response = agent
            .get(&url)
            .call()
            .with_context(|| format!("failed to download zju-connect {VERSION} for {target}"))?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_ARCHIVE_BYTES)
            .read_to_vec()
            .context("failed to read the zju-connect release archive")?;
        let mut actual_sha256 = String::with_capacity(64);
        for byte in Sha256::digest(&bytes) {
            write!(actual_sha256, "{byte:02x}")?;
        }
        ensure!(
            actual_sha256 == expected_sha256,
            "zju-connect {VERSION} archive SHA-256 mismatch for {target}: expected {expected_sha256}, got {actual_sha256}"
        );

        let mut archive =
            ZipArchive::new(Cursor::new(bytes)).context("invalid zju-connect ZIP archive")?;
        let executable = if platform.starts_with("windows-") {
            "zju-connect.exe"
        } else {
            "zju-connect"
        };
        let mut core = archive
            .by_name(executable)
            .with_context(|| format!("zju-connect archive does not contain {executable}"))?;
        ensure!(
            core.is_file() && (1..=MAX_CORE_BYTES).contains(&core.size()),
            "invalid zju-connect binary in release archive"
        );
        let mut output = File::create(out_dir.join("bundled-zju-connect"))
            .context("failed to create the bundled core file")?;
        io::copy(&mut core, &mut output).context("failed to extract the bundled core")?;
        Ok(())
    }

    fn asset(target: &str) -> Result<(&'static str, &'static str)> {
        // SHA-256 of the fixed upstream archive for each Rust target.
        Ok(match target {
            "aarch64-apple-darwin" => (
                "darwin-arm64",
                "1003088f144a178c3fe0b8ebb4e00ee93f2aa8a53876db97b521e5afc82cdba2",
            ),
            "x86_64-apple-darwin" => (
                "darwin-amd64",
                "50e181be23af47b0dc73d838b6a121639ddd615a4cb908f9d518dbe270a6991f",
            ),
            "aarch64-unknown-linux-gnu" => (
                "linux-arm64",
                "33c13ae0cc276a55d4777bd7bafc1e4bcc534143100306904e874ccba4fad625",
            ),
            "x86_64-unknown-linux-gnu" => (
                "linux-amd64",
                "d7cd61a580b085570a364202c97d32fd9f07b38eb0dda75675a44067b9764238",
            ),
            "x86_64-pc-windows-msvc" => (
                "windows-amd64",
                "e14de6f0ede80f5ad9b6b865d33c61960206f27d02eb24f81643c709d97265c8",
            ),
            _ => bail!("bundled-core has no zju-connect release asset for {target}"),
        })
    }
}
