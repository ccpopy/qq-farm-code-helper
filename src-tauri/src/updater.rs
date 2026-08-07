use reqwest::{
    Client,
    header::{ACCEPT, ACCEPT_ENCODING},
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use url::Url;

const GITHUB_RELEASE_API: &str =
    "https://api.github.com/repos/ccpopy/qq-farm-code-helper/releases/latest";
const GITHUB_DOWNLOAD_PREFIX: &str = "/ccpopy/qq-farm-code-helper/releases/download/";
const DOWNLOAD_PROXY: &str = "https://gh.lessdo.top";
const USER_AGENT: &str = "qq-farm-code-helper-updater";
const MAX_ASSET_BYTES: u64 = 200 * 1024 * 1024;
const INSTALLED_EXECUTABLE_NAME: &str = "qq-farm-code-helper.exe";

static UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
    pub release_notes: String,
    pub published_at: String,
    pub package_name: String,
    pub package_size: u64,
    pub install_mode: String,
    pub install_target: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackageKind {
    Installer,
    Portable,
}

impl PackageKind {
    fn asset_suffix(self) -> &'static str {
        match self {
            Self::Installer => "-setup.exe",
            Self::Portable => "-portable.exe",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Installer => "installer",
            Self::Portable => "portable",
        }
    }
}

struct UpdateSelection {
    info: UpdateInfo,
    asset: GithubAsset,
    kind: PackageKind,
    current_executable: PathBuf,
}

pub struct PreparedUpdate {
    pub(crate) package_path: PathBuf,
    pub(crate) current_executable: PathBuf,
    pub(crate) install_directory: PathBuf,
    pub(crate) kind: PackageKind,
}

struct UpdateGuard;

impl UpdateGuard {
    fn acquire() -> Result<Self, String> {
        UPDATE_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "更新下载已经在进行中".to_owned())?;
        Ok(Self)
    }
}

impl Drop for UpdateGuard {
    fn drop(&mut self) {
        UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

pub async fn check_for_update() -> Result<UpdateInfo, String> {
    let release = fetch_latest_release().await?;
    let current_executable =
        std::env::current_exe().map_err(|error| format!("无法确定当前程序位置: {error}"))?;
    Ok(select_release(release, current_executable)?.info)
}

pub async fn prepare_update(use_proxy: bool) -> Result<PreparedUpdate, String> {
    let _guard = UpdateGuard::acquire()?;
    let release = fetch_latest_release().await?;
    let current_executable =
        std::env::current_exe().map_err(|error| format!("无法确定当前程序位置: {error}"))?;
    let selection = select_release(release, current_executable)?;
    if !selection.info.update_available {
        return Err(format!(
            "当前已经是最新版 v{}",
            selection.info.current_version
        ));
    }

    let package_path = download_and_verify(&selection.asset, use_proxy).await?;
    let install_directory = selection
        .current_executable
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .ok_or_else(|| "无法确定当前程序所在目录".to_owned())?;

    Ok(PreparedUpdate {
        package_path,
        current_executable: selection.current_executable,
        install_directory,
        kind: selection.kind,
    })
}

pub fn launch_prepared_update(update: PreparedUpdate) -> Result<(), String> {
    crate::update_installer::launch(update)
}

async fn fetch_latest_release() -> Result<GithubRelease, String> {
    let client = update_client()?;
    let response = client
        .get(GITHUB_RELEASE_API)
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| format!("检查 GitHub Release 失败: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "检查 GitHub Release 失败 (HTTP {})",
            status.as_u16()
        ));
    }
    response
        .json::<GithubRelease>()
        .await
        .map_err(|error| format!("解析 GitHub Release 失败: {error}"))
}

fn update_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|error| format!("创建更新连接失败: {error}"))
}

fn select_release(
    release: GithubRelease,
    current_executable: PathBuf,
) -> Result<UpdateSelection, String> {
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("当前版本号无效: {error}"))?;
    let latest_version = parse_release_version(&release.tag_name)?;
    let kind = package_kind_for_executable(&current_executable);
    let asset = select_asset(&release.assets, kind)?.clone();
    validate_asset(&asset)?;
    let install_directory = current_executable
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "无法确定当前程序所在目录".to_owned())?;
    let install_target = match kind {
        PackageKind::Installer => install_directory,
        PackageKind::Portable => current_executable.as_path(),
    };

    Ok(UpdateSelection {
        info: UpdateInfo {
            current_version: current_version.to_string(),
            latest_version: latest_version.to_string(),
            update_available: latest_version > current_version,
            release_url: release.html_url,
            release_notes: release.body,
            published_at: release.published_at,
            package_name: asset.name.clone(),
            package_size: asset.size,
            install_mode: kind.label().to_owned(),
            install_target: install_target.display().to_string(),
        },
        asset,
        kind,
        current_executable,
    })
}

fn parse_release_version(tag: &str) -> Result<Version, String> {
    let value = tag.trim();
    let value = value
        .strip_prefix('v')
        .or_else(|| value.strip_prefix('V'))
        .unwrap_or(value);
    Version::parse(value).map_err(|error| format!("GitHub Release 版本号无效: {error}"))
}

fn package_kind_for_executable(executable: &Path) -> PackageKind {
    let is_installed_name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(INSTALLED_EXECUTABLE_NAME));
    if is_installed_name {
        PackageKind::Installer
    } else {
        PackageKind::Portable
    }
}

fn select_asset(assets: &[GithubAsset], kind: PackageKind) -> Result<&GithubAsset, String> {
    assets
        .iter()
        .find(|asset| {
            asset
                .name
                .to_ascii_lowercase()
                .ends_with(kind.asset_suffix())
        })
        .ok_or_else(|| format!("最新 Release 中没有找到 {} 更新包", kind.asset_suffix()))
}

fn validate_asset(asset: &GithubAsset) -> Result<(), String> {
    if asset.size == 0 || asset.size > MAX_ASSET_BYTES {
        return Err("GitHub Release 更新包大小异常".to_owned());
    }
    let file_name = Path::new(&asset.name)
        .file_name()
        .and_then(|name| name.to_str());
    if file_name != Some(asset.name.as_str()) {
        return Err("GitHub Release 更新包名称无效".to_owned());
    }
    validated_github_download_url(&asset.browser_download_url)?;
    expected_sha256(asset.digest.as_deref().unwrap_or_default())?;
    Ok(())
}

fn validated_github_download_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "GitHub 更新包地址无效".to_owned())?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.path().starts_with(GITHUB_DOWNLOAD_PREFIX)
    {
        return Err("拒绝下载非本项目 GitHub Release 的更新包".to_owned());
    }
    Ok(url)
}

fn asset_download_url(asset_url: &str, use_proxy: bool) -> Result<Url, String> {
    let official = validated_github_download_url(asset_url)?;
    if !use_proxy {
        return Ok(official);
    }
    Url::parse(&format!("{DOWNLOAD_PROXY}/{}", official.as_str()))
        .map_err(|_| "GitHub 加速下载地址无效".to_owned())
}

async fn download_and_verify(asset: &GithubAsset, use_proxy: bool) -> Result<PathBuf, String> {
    let url = asset_download_url(&asset.browser_download_url, use_proxy)?;
    let response = update_client()?
        .get(url)
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|error| format!("下载更新包失败: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("下载更新包失败 (HTTP {})", status.as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ASSET_BYTES)
    {
        return Err("下载的更新包超过安全大小限制".to_owned());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取更新包失败: {error}"))?;
    let actual_size = bytes.len() as u64;
    if actual_size != asset.size {
        return Err(format!(
            "更新包大小校验失败：GitHub 标记 {} 字节，实际下载 {} 字节",
            asset.size, actual_size
        ));
    }
    verify_sha256(&bytes, asset.digest.as_deref().unwrap_or_default())?;

    let directory = std::env::temp_dir().join("qq-farm-code-helper-updates");
    fs::create_dir_all(&directory).map_err(|error| format!("创建更新临时目录失败: {error}"))?;
    let path = directory.join(format!("{}-{}", std::process::id(), asset.name));
    fs::write(&path, bytes).map_err(|error| format!("保存更新包失败: {error}"))?;
    Ok(path)
}

fn expected_sha256(digest: &str) -> Result<&str, String> {
    let value = digest
        .trim()
        .strip_prefix("sha256:")
        .ok_or_else(|| "GitHub Release 未提供 SHA-256 摘要，已拒绝执行更新包".to_owned())?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("GitHub Release 的 SHA-256 摘要无效".to_owned());
    }
    Ok(value)
}

fn verify_sha256(bytes: &[u8], digest: &str) -> Result<(), String> {
    let expected = expected_sha256(digest)?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("更新包 SHA-256 校验失败，已拒绝执行".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> GithubAsset {
        GithubAsset {
            name: name.to_owned(),
            browser_download_url: format!(
                "https://github.com/ccpopy/qq-farm-code-helper/releases/download/v0.1.5/{name}"
            ),
            size: 1024,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        }
    }

    #[test]
    fn parses_prefixed_release_versions() {
        assert_eq!(
            parse_release_version("v1.2.3").unwrap(),
            Version::new(1, 2, 3)
        );
    }

    #[test]
    fn selects_setup_for_installed_name_and_portable_for_other_names() {
        assert_eq!(
            package_kind_for_executable(Path::new("D:/Farm/qq-farm-code-helper.exe")),
            PackageKind::Installer
        );
        assert_eq!(
            package_kind_for_executable(Path::new("D:/Farm/QQFarmCodeHelper-v0.1.4-portable.exe")),
            PackageKind::Portable
        );
    }

    #[test]
    fn prefixes_downloads_with_the_configured_proxy() {
        let official = "https://github.com/ccpopy/qq-farm-code-helper/releases/download/v0.1.5/QQFarmCodeHelper-v0.1.5-setup.exe";
        assert_eq!(
            asset_download_url(official, true).unwrap().as_str(),
            "https://gh.lessdo.top/https://github.com/ccpopy/qq-farm-code-helper/releases/download/v0.1.5/QQFarmCodeHelper-v0.1.5-setup.exe"
        );
        assert_eq!(
            asset_download_url(official, false).unwrap().as_str(),
            official
        );
    }

    #[test]
    fn rejects_downloads_outside_the_project_release_path() {
        assert!(
            validated_github_download_url(
                "https://github.com/other/repository/releases/download/v1/app.exe"
            )
            .is_err()
        );
    }

    #[test]
    fn verifies_the_github_sha256_digest() {
        assert!(
            verify_sha256(
                b"abc",
                "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            )
            .is_ok()
        );
        assert!(verify_sha256(b"changed", &format!("sha256:{}", "0".repeat(64))).is_err());
    }

    #[test]
    fn selects_the_expected_release_asset() {
        let assets = vec![
            asset("QQFarmCodeHelper-v0.1.5-portable.exe"),
            asset("QQFarmCodeHelper-v0.1.5-setup.exe"),
        ];
        assert!(
            select_asset(&assets, PackageKind::Installer)
                .unwrap()
                .name
                .ends_with("-setup.exe")
        );
        assert!(
            select_asset(&assets, PackageKind::Portable)
                .unwrap()
                .name
                .ends_with("-portable.exe")
        );
    }

    #[tokio::test]
    #[ignore = "requires access to the public GitHub Release API"]
    async fn reads_the_live_github_release() {
        let release = fetch_latest_release().await.unwrap();

        assert!(release.tag_name.starts_with('v'));
        assert!(!release.assets.is_empty());
        for asset in &release.assets {
            if asset.name.ends_with(".exe") {
                validate_asset(asset).unwrap();
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires access to the configured GitHub download proxy"]
    async fn downloads_and_verifies_a_live_release_through_the_proxy() {
        let release = fetch_latest_release().await.unwrap();
        let asset = select_asset(&release.assets, PackageKind::Installer).unwrap();
        let path = download_and_verify(asset, true).await.unwrap();

        assert_eq!(fs::metadata(&path).unwrap().len(), asset.size);
        fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires direct access to GitHub release downloads"]
    async fn downloads_and_verifies_a_live_release_without_the_proxy() {
        let release = fetch_latest_release().await.unwrap();
        let asset = select_asset(&release.assets, PackageKind::Installer).unwrap();
        let path = download_and_verify(asset, false).await.unwrap();

        assert_eq!(fs::metadata(&path).unwrap().len(), asset.size);
        fs::remove_file(path).unwrap();
    }
}
