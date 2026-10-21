use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use devzymer_tui::brand::{self, Bar, Caps, Mark, Task};
use sha2::{Digest, Sha256};

use crate::Ctx;

const RELEASES_API: &str = "https://api.github.com/repos/devadeiltonlima/devzymer";
const API_ENV: &str = "DEVZYMER_UPDATE_API";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub struct UpdateOptions {
    pub check: bool,
    pub version: Option<String>,
    pub force: bool,
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArchiveKind {
    Zip,
    TarGz,
}

#[derive(Debug, PartialEq, Eq)]
struct Replacement {
    stale: Option<PathBuf>,
}

struct Reporter {
    caps: Caps,
    fancy: bool,
    json: bool,
}

struct Step<'a> {
    reporter: &'a Reporter,
    label: String,
    task: Option<Task>,
}

impl Reporter {
    fn begin(&self, label: &str) -> Step<'_> {
        let task = self.fancy.then(|| Task::start(&self.caps, label));
        Step {
            reporter: self,
            label: label.to_string(),
            task,
        }
    }

    fn info(&self, label: &str, detail: &str) {
        if self.json {
            return;
        }
        if self.fancy {
            let _ = brand::line(&self.caps, Mark::Info, label, detail);
        } else {
            println!("{label}: {detail}");
        }
    }
}

impl Step<'_> {
    fn ok(mut self, detail: &str) {
        match self.task.take() {
            Some(task) => task.finish(detail),
            None => {
                if !self.reporter.json {
                    println!("{}: {detail}", self.label);
                }
            }
        }
    }

    fn fail(mut self, detail: &str) {
        match self.task.take() {
            Some(task) => task.fail(detail),
            None => {
                if !self.reporter.json {
                    println!("{}: falhou ({detail})", self.label);
                }
            }
        }
    }
}

pub async fn run(ctx: &Ctx, options: UpdateOptions) -> Result<()> {
    let caps = brand::detect();
    let reporter = Reporter {
        fancy: caps.color && !ctx.json,
        json: ctx.json,
        caps,
    };
    if reporter.fancy {
        brand::print_banner(&reporter.caps, false, CURRENT)?;
    }

    let api = std::env::var(API_ENV).unwrap_or_else(|_| RELEASES_API.to_string());
    let client = reqwest::Client::builder()
        .user_agent(format!("devzymer/{CURRENT}"))
        .timeout(Duration::from_secs(60))
        .build()?;
    let url = match options.version.as_deref() {
        Some(version) => format!("{api}/releases/tags/{}", normalize_tag(version)),
        None => format!("{api}/releases/latest"),
    };

    let step = reporter.begin("Consultando a última versão");
    let release = match fetch_release(&client, &url).await {
        Ok(release) => release,
        Err(error) => {
            step.fail(&error.to_string());
            return Err(error);
        }
    };
    let latest = release.tag_name.trim_start_matches('v').to_string();
    step.ok(&format!("v{latest}"));
    reporter.info("Versão instalada", &format!("v{CURRENT}"));

    let explicit = options.version.is_some();
    let newer = is_newer(&latest, CURRENT);
    let needs_change = if explicit { latest != CURRENT } else { newer };

    if !needs_change {
        if ctx.json {
            print_json(&latest, false, false)?;
        } else if reporter.fancy {
            brand::line(
                &reporter.caps,
                Mark::Ok,
                "Já está atualizado",
                &format!("v{CURRENT} é a versão mais recente"),
            )?;
        } else {
            println!("já está na versão mais recente ({CURRENT})");
        }
        return Ok(());
    }

    if options.check {
        if ctx.json {
            print_json(&latest, true, false)?;
        } else if reporter.fancy {
            brand::line(
                &reporter.caps,
                Mark::Warn,
                "Atualização disponível",
                &format!("v{CURRENT} → v{latest}. Rode `devzymer update`"),
            )?;
        } else {
            println!("atualização disponível: {CURRENT} -> {latest}. Rode `devzymer update`");
        }
        return Ok(());
    }

    let executable = std::env::current_exe().context("não foi possível localizar o executável")?;
    if !options.force && looks_like_local_build(&executable) {
        bail!(
            "este executável parece uma compilação local ({}). Atualize com `git pull` e \
             `cargo build --release`, ou use `--force`.",
            executable.display()
        );
    }

    let archive_name = archive_name()?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == archive_name)
        .ok_or_else(|| {
            anyhow!(
                "a versão v{latest} não publica o arquivo {archive_name} para esta plataforma. \
                 Compile do código com `cargo build --release`."
            )
        })?;
    let checksum_name = format!("{archive_name}.sha256");
    let checksum_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == checksum_name)
        .ok_or_else(|| anyhow!("a versão v{latest} não publica {checksum_name}"))?;

    let expected = fetch_text(&client, &checksum_asset.browser_download_url)
        .await
        .and_then(|text| parse_checksum(&text).ok_or_else(|| anyhow!("checksum ilegível")))?;

    let bytes = download(&client, asset, &reporter, "Baixando o devzymer").await?;

    let step = reporter.begin("Conferindo o SHA-256");
    let actual = sha256_hex(&bytes);
    if actual != expected {
        step.fail("não confere");
        bail!("o checksum do download não confere. Nada foi alterado.");
    }
    step.ok(&actual[..16]);

    let step = reporter.begin("Instalando a nova versão");
    let installed = match install(&executable, &bytes, kind_of(&archive_name), &latest) {
        Ok(replacement) => replacement,
        Err(error) => {
            step.fail(&error.to_string());
            return Err(error);
        }
    };
    step.ok(&executable.display().to_string());

    if ctx.json {
        return print_json(&latest, true, true);
    }
    let in_use = installed.stale.is_some();
    if reporter.fancy {
        brand::line(
            &reporter.caps,
            Mark::Ok,
            "Atualizado",
            &format!("v{CURRENT} → v{latest}"),
        )?;
        if in_use {
            brand::line(
                &reporter.caps,
                Mark::Warn,
                "Reinicie os agentes",
                "Claude Code, Codex e OpenCode ainda usam a versão anterior",
            )?;
        }
        println!();
    } else {
        println!("atualizado: {CURRENT} -> {latest}");
        if in_use {
            println!("reinicie os agentes (Claude Code, Codex, OpenCode) para usar a nova versão");
        }
    }
    Ok(())
}

fn print_json(latest: &str, update_available: bool, updated: bool) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "current": CURRENT,
            "latest": latest,
            "update_available": update_available,
            "updated": updated
        }))?
    );
    Ok(())
}

async fn fetch_release(client: &reqwest::Client, url: &str) -> Result<Release> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| anyhow!("não foi possível consultar o GitHub: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("versão não encontrada em {url}");
    }
    let response = response
        .error_for_status()
        .context("o GitHub recusou a consulta")?;
    response
        .json::<Release>()
        .await
        .context("resposta inesperada do GitHub")
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    Ok(client
        .get(url)
        .send()
        .await
        .map_err(|error| anyhow!("falha ao baixar o checksum: {error}"))?
        .error_for_status()?
        .text()
        .await?)
}

async fn download(
    client: &reqwest::Client,
    asset: &Asset,
    reporter: &Reporter,
    label: &str,
) -> Result<Vec<u8>> {
    let mut response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|error| anyhow!("falha ao baixar a atualização: {error}"))?
        .error_for_status()
        .context("o download foi recusado")?;
    let total = response.content_length();
    let mut bar = reporter
        .fancy
        .then(|| Bar::new(&reporter.caps, label, total));
    let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if let Some(bar) = bar.as_mut() {
                    bar.advance(chunk.len() as u64);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => {
                if let Some(bar) = bar.take() {
                    bar.fail("interrompido");
                }
                return Err(anyhow!(error).context("o download foi interrompido"));
            }
        }
    }
    let size = brand::human_bytes(bytes.len() as u64);
    match bar {
        Some(bar) => bar.finish(&size),
        None => {
            if !reporter.json {
                println!("download: {size}");
            }
        }
    }
    Ok(bytes)
}

fn normalize_tag(version: &str) -> String {
    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    }
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text
        .trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn parse_checksum(text: &str) -> Option<String> {
    let token = text.split_whitespace().next()?.to_ascii_lowercase();
    (token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit())).then_some(token)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn archive_name() -> Result<String> {
    platform_archive(std::env::consts::OS, std::env::consts::ARCH)
}

fn platform_archive(os: &str, arch: &str) -> Result<String> {
    let cpu = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => bail!("arquitetura sem binário publicado: {other}"),
    };
    match os {
        "windows" => Ok("devzymer-x64-pc-windows-msvc.zip".to_string()),
        "linux" => Ok(format!("devzymer-{cpu}-unknown-linux-gnu.tar.gz")),
        "macos" => Ok(format!("devzymer-{cpu}-apple-darwin.tar.gz")),
        other => bail!("sistema sem binário publicado: {other}"),
    }
}

fn kind_of(archive: &str) -> ArchiveKind {
    if archive.ends_with(".zip") {
        ArchiveKind::Zip
    } else {
        ArchiveKind::TarGz
    }
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "devzymer.exe"
    } else {
        "devzymer"
    }
}

fn looks_like_local_build(executable: &Path) -> bool {
    let text = executable
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let parts: Vec<&str> = text.split('/').collect();
    parts
        .windows(2)
        .any(|pair| pair[0] == "target" && (pair[1] == "release" || pair[1] == "debug"))
        || parts
            .iter()
            .any(|part| part.starts_with("target-") || part.contains("-target"))
}

fn staged_name() -> &'static str {
    if cfg!(windows) {
        "devzymer.update.exe"
    } else {
        ".devzymer.update"
    }
}

fn extract_binary(archive: &[u8], kind: ArchiveKind) -> Result<Vec<u8>> {
    match kind {
        ArchiveKind::Zip => extract_from_zip(archive, "devzymer.exe"),
        ArchiveKind::TarGz => extract_from_tar_gz(archive, "devzymer"),
    }
}

fn extract_from_tar_gz(archive: &[u8], wanted: &str) -> Result<Vec<u8>> {
    let mut decoder = flate2::read::GzDecoder::new(archive);
    let mut tar = Vec::new();
    decoder
        .read_to_end(&mut tar)
        .context("o arquivo .tar.gz está corrompido")?;
    let mut offset = 0usize;
    while offset + 512 <= tar.len() {
        let header = &tar[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let name_end = header[..100].iter().position(|b| *b == 0).unwrap_or(100);
        let name = String::from_utf8_lossy(&header[..name_end]).to_string();
        let size_text = String::from_utf8_lossy(&header[124..136])
            .trim_matches(|c: char| c == '\0' || c == ' ')
            .to_string();
        let size = usize::from_str_radix(&size_text, 8).context("tamanho inválido no tar")?;
        let data_start = offset + 512;
        let data_end = data_start + size;
        if data_end > tar.len() {
            bail!("o arquivo .tar.gz está truncado");
        }
        let plain = name.trim_start_matches("./");
        if plain == wanted {
            return Ok(tar[data_start..data_end].to_vec());
        }
        offset = data_start + size.div_ceil(512) * 512;
    }
    bail!("o arquivo não contém {wanted}")
}

fn le_u16(bytes: &[u8], at: usize) -> Option<usize> {
    bytes
        .get(at..at + 2)
        .map(|slice| usize::from(u16::from_le_bytes([slice[0], slice[1]])))
}

fn le_u32(bytes: &[u8], at: usize) -> Option<usize> {
    bytes
        .get(at..at + 4)
        .map(|slice| u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]) as usize)
}

fn extract_from_zip(archive: &[u8], wanted: &str) -> Result<Vec<u8>> {
    let corrupt = || anyhow!("o arquivo .zip está corrompido");
    let search_from = archive.len().saturating_sub(65_557);
    let eocd = (search_from..archive.len().saturating_sub(21))
        .rev()
        .find(|at| archive[*at..*at + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(corrupt)?;
    let entries = le_u16(archive, eocd + 10).ok_or_else(corrupt)?;
    let mut cursor = le_u32(archive, eocd + 16).ok_or_else(corrupt)?;

    for _ in 0..entries {
        if archive.get(cursor..cursor + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err(corrupt());
        }
        let method = le_u16(archive, cursor + 10).ok_or_else(corrupt)?;
        let compressed = le_u32(archive, cursor + 20).ok_or_else(corrupt)?;
        let uncompressed = le_u32(archive, cursor + 24).ok_or_else(corrupt)?;
        let name_len = le_u16(archive, cursor + 28).ok_or_else(corrupt)?;
        let extra_len = le_u16(archive, cursor + 30).ok_or_else(corrupt)?;
        let comment_len = le_u16(archive, cursor + 32).ok_or_else(corrupt)?;
        let local = le_u32(archive, cursor + 42).ok_or_else(corrupt)?;
        let name = String::from_utf8_lossy(
            archive
                .get(cursor + 46..cursor + 46 + name_len)
                .ok_or_else(corrupt)?,
        )
        .replace('\\', "/");
        cursor += 46 + name_len + extra_len + comment_len;

        if name.rsplit('/').next() != Some(wanted) {
            continue;
        }
        if archive.get(local..local + 4) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
            return Err(corrupt());
        }
        let local_name = le_u16(archive, local + 26).ok_or_else(corrupt)?;
        let local_extra = le_u16(archive, local + 28).ok_or_else(corrupt)?;
        let start = local + 30 + local_name + local_extra;
        let data = archive.get(start..start + compressed).ok_or_else(corrupt)?;
        let output = match method {
            0 => data.to_vec(),
            8 => {
                let mut buffer = Vec::with_capacity(uncompressed);
                flate2::read::DeflateDecoder::new(data)
                    .read_to_end(&mut buffer)
                    .context("falha ao descompactar o .zip")?;
                buffer
            }
            other => bail!("método de compressão zip não suportado: {other}"),
        };
        if output.len() != uncompressed {
            return Err(corrupt());
        }
        return Ok(output);
    }
    bail!("o arquivo não contém {wanted}")
}

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

fn install(
    executable: &Path,
    archive: &[u8],
    kind: ArchiveKind,
    expected_version: &str,
) -> Result<Replacement> {
    let binary = extract_binary(archive, kind)?;
    let directory = executable
        .parent()
        .ok_or_else(|| anyhow!("o executável não tem diretório"))?;
    let staged = directory.join(staged_name());
    std::fs::write(&staged, &binary)
        .with_context(|| format!("não foi possível gravar em {}", directory.display()))?;
    set_executable(&staged)?;

    let check = Command::new(&staged).arg("--version").output();
    let reported = check
        .as_ref()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string());
    match reported {
        Some(text) if text.contains(expected_version) => {}
        Some(text) => {
            let _ = std::fs::remove_file(&staged);
            bail!(
                "o binário baixado reporta '{}', e não a versão {expected_version}. Nada foi alterado.",
                text.trim()
            );
        }
        None => {
            let _ = std::fs::remove_file(&staged);
            bail!("o binário baixado não executa nesta máquina. Nada foi alterado.");
        }
    }

    let replacement = replace_executable(executable, &staged)?;
    remove_stale(directory, replacement.stale.as_deref());
    Ok(replacement)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn replace_executable(executable: &Path, staged: &Path) -> Result<Replacement> {
    if cfg!(windows) {
        let stale = executable.with_file_name(format!("{}.old-{}", binary_name(), stamp()));
        std::fs::rename(executable, &stale).with_context(|| {
            format!(
                "não foi possível mover o executável atual ({})",
                executable.display()
            )
        })?;
        if let Err(error) = std::fs::rename(staged, executable) {
            let _ = std::fs::rename(&stale, executable);
            let _ = std::fs::remove_file(staged);
            return Err(anyhow!(error).context("não foi possível instalar a nova versão"));
        }
        Ok(Replacement { stale: Some(stale) })
    } else {
        std::fs::rename(staged, executable).context("não foi possível instalar a nova versão")?;
        Ok(Replacement { stale: None })
    }
}

fn remove_stale(directory: &Path, keep: Option<&Path>) {
    let prefix = format!("{}.old-", binary_name());
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_stale = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(&prefix));
        if is_stale && keep != Some(path.as_path()) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{DeflateEncoder, GzEncoder};
    use flate2::Compression;
    use std::io::Write;

    fn tar_with(name: &str, data: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        let size = format!("{:011o}", data.len());
        header[124..135].copy_from_slice(size.as_bytes());
        let mut tar = header.to_vec();
        tar.extend_from_slice(data);
        tar.resize(tar.len() + (512 - data.len() % 512) % 512, 0);
        tar.extend_from_slice(&[0u8; 1024]);
        tar
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn zip_with(name: &str, data: &[u8], deflate: bool) -> Vec<u8> {
        let payload = if deflate {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap()
        } else {
            data.to_vec()
        };
        let method: u16 = if deflate { 8 } else { 0 };
        let mut zip = Vec::new();
        zip.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
        zip.extend_from_slice(&[20, 0, 0, 0]);
        zip.extend_from_slice(&method.to_le_bytes());
        zip.extend_from_slice(&[0; 8]);
        zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        zip.extend_from_slice(name.as_bytes());
        zip.extend_from_slice(&payload);
        let directory_start = zip.len();
        zip.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
        zip.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        zip.extend_from_slice(&method.to_le_bytes());
        zip.extend_from_slice(&[0; 8]);
        zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&[0; 12]);
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(name.as_bytes());
        let directory_size = zip.len() - directory_start;
        zip.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 1, 0, 1, 0]);
        zip.extend_from_slice(&(directory_size as u32).to_le_bytes());
        zip.extend_from_slice(&(directory_start as u32).to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        zip
    }

    #[test]
    fn versions_are_compared_numerically() {
        assert!(is_newer("0.1.2", "0.1.1"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("garbage", "0.1.1"));
        assert_eq!(parse_version("v2.3.4-alpha+5"), Some((2, 3, 4)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
    }

    #[test]
    fn a_release_tag_is_normalized() {
        assert_eq!(normalize_tag("0.1.2"), "v0.1.2");
        assert_eq!(normalize_tag("v0.1.2"), "v0.1.2");
    }

    #[test]
    fn the_archive_matches_the_platform() {
        assert_eq!(
            platform_archive("windows", "x86_64").unwrap(),
            "devzymer-x64-pc-windows-msvc.zip"
        );
        assert_eq!(
            platform_archive("windows", "aarch64").unwrap(),
            "devzymer-x64-pc-windows-msvc.zip"
        );
        assert_eq!(
            platform_archive("linux", "aarch64").unwrap(),
            "devzymer-arm64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            platform_archive("macos", "x86_64").unwrap(),
            "devzymer-x64-apple-darwin.tar.gz"
        );
        assert!(platform_archive("freebsd", "x86_64").is_err());
        assert!(platform_archive("linux", "riscv64").is_err());
    }

    #[test]
    fn a_checksum_file_is_parsed_in_both_formats() {
        let hash = "a".repeat(64);
        assert_eq!(
            parse_checksum(&format!("{hash}  devzymer.zip\r\n")),
            Some(hash.clone())
        );
        assert_eq!(parse_checksum(&hash.to_uppercase()), Some(hash));
        assert_eq!(parse_checksum("not a hash"), None);
        assert_eq!(parse_checksum(""), None);
    }

    #[test]
    fn sha256_matches_a_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_binary_is_extracted_from_a_tar_gz() {
        let archive = gzip(&tar_with("devzymer", b"payload-bytes"));
        assert_eq!(
            extract_binary(&archive, ArchiveKind::TarGz).unwrap(),
            b"payload-bytes"
        );
    }

    #[test]
    fn a_tar_gz_without_the_binary_is_rejected() {
        let archive = gzip(&tar_with("other", b"x"));
        assert!(extract_binary(&archive, ArchiveKind::TarGz).is_err());
        assert!(extract_binary(b"not gzip", ArchiveKind::TarGz).is_err());
    }

    #[test]
    fn a_binary_is_extracted_from_a_zip_in_both_methods() {
        let data = vec![7u8; 5000];
        for deflate in [true, false] {
            let archive = zip_with("devzymer.exe", &data, deflate);
            assert_eq!(extract_binary(&archive, ArchiveKind::Zip).unwrap(), data);
        }
    }

    #[test]
    fn a_zip_with_backslash_paths_still_resolves_the_binary() {
        let archive = zip_with("bin\\devzymer.exe", b"abc", true);
        assert_eq!(extract_binary(&archive, ArchiveKind::Zip).unwrap(), b"abc");
    }

    #[test]
    fn a_broken_zip_is_rejected_without_panicking() {
        assert!(extract_binary(b"", ArchiveKind::Zip).is_err());
        assert!(extract_binary(&[0u8; 100], ArchiveKind::Zip).is_err());
        let mut archive = zip_with("devzymer.exe", b"abc", false);
        archive.truncate(archive.len() - 30);
        assert!(extract_binary(&archive, ArchiveKind::Zip).is_err());
        assert!(extract_binary(&zip_with("other.exe", b"abc", false), ArchiveKind::Zip).is_err());
    }

    #[test]
    fn a_local_build_is_recognized_by_its_path() {
        assert!(looks_like_local_build(Path::new(
            "/work/devzymer/target/release/devzymer"
        )));
        assert!(looks_like_local_build(Path::new(
            "C:\\x\\devzymer-target\\release\\devzymer.exe"
        )));
        assert!(!looks_like_local_build(Path::new(
            "/home/u/.local/bin/devzymer"
        )));
    }

    #[test]
    fn replacing_the_executable_swaps_the_file_and_keeps_the_old_one_aside() {
        let dir = std::env::temp_dir().join(format!("dz-update-{}", stamp()));
        std::fs::create_dir_all(&dir).unwrap();
        let executable = dir.join(binary_name());
        let staged = dir.join(staged_name());
        std::fs::write(&executable, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();

        let replacement = replace_executable(&executable, &staged).unwrap();

        assert_eq!(std::fs::read(&executable).unwrap(), b"new");
        assert!(!staged.exists());
        if cfg!(windows) {
            let stale = replacement.stale.expect("the old file is kept aside");
            assert_eq!(std::fs::read(&stale).unwrap(), b"old");
            remove_stale(&dir, None);
            assert!(!stale.exists());
        } else {
            assert!(replacement.stale.is_none());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
