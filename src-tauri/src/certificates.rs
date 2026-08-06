use crate::windows::{run_powershell, utf8_base64};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::{fs, path::PathBuf, sync::Arc};

pub const TARGET_HOST: &str = "gate-obt.nqf.qq.com";
const CA_COMMON_NAME: &str = "QQ Farm Code Helper Local CA";

pub struct CertificateManager {
    ca_path: PathBuf,
}

impl CertificateManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            ca_path: data_dir.join("temporary-ca.cer"),
        }
    }

    pub fn prepare(&self) -> Result<Arc<ServerConfig>, String> {
        self.remove_trusted()?;
        let material = generate_tls_material()?;
        fs::write(&self.ca_path, &material.ca_der)
            .map_err(|error| format!("写入临时证书失败: {error}"))?;
        if let Err(error) = self.install_trusted() {
            let _ = fs::remove_file(&self.ca_path);
            return Err(error);
        }
        Ok(material.server_config)
    }

    pub fn cleanup(&self) -> Result<(), String> {
        self.remove_trusted()?;
        if self.ca_path.exists() {
            fs::remove_file(&self.ca_path)
                .map_err(|error| format!("删除临时证书文件失败: {error}"))?;
        }
        Ok(())
    }

    pub fn recover_stale(&self) -> Result<(), String> {
        self.cleanup()
    }

    fn install_trusted(&self) -> Result<(), String> {
        let path = utf8_base64(&self.ca_path.to_string_lossy());
        let script = format!(
            r#"
$path = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{path}'))
$store = [Security.Cryptography.X509Certificates.X509Store]::new(
  [Security.Cryptography.X509Certificates.StoreName]::Root,
  [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
)
$certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($path)
try {{
  $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
  $store.Add($certificate)
}} finally {{
  $store.Close()
  $certificate.Dispose()
}}
"#
        );
        run_powershell(&script).map(|_| ())
    }

    /// 删除走物理存储 `Root\.Default`（即 HKCU 注册表里的那份），而不是逻辑
    /// `Root` 存储。逻辑存储由「受保护的根存储」提供程序接管，任何增删都会弹出
    /// 系统确认框；物理存储不经过该提供程序，因此可以静默移除。
    fn remove_trusted(&self) -> Result<(), String> {
        let subject = utf8_base64(&format!("CN={CA_COMMON_NAME}"));
        let script = format!(
            r#"
$subject = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{subject}'))
$store = [Security.Cryptography.X509Certificates.X509Store]::new(
  [Security.Cryptography.X509Certificates.StoreName]::Root,
  [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
)
try {{
  $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadOnly)
  $thumbprints = @($store.Certificates |
    Where-Object {{ $_.Subject -eq $subject }} |
    ForEach-Object {{ $_.Thumbprint }})
}} finally {{
  $store.Close()
}}
$stuck = @()
foreach ($thumbprint in $thumbprints) {{
  & "$env:SystemRoot\System32\certutil.exe" -f -user -delstore 'Root\.Default' $thumbprint | Out-Null
  if ($LASTEXITCODE -eq 0) {{ continue }}
  $key = "HKCU:\Software\Microsoft\SystemCertificates\Root\Certificates\$thumbprint"
  if (Test-Path -LiteralPath $key) {{
    Remove-Item -LiteralPath $key -Recurse -Force
  }} else {{
    $stuck += $thumbprint
  }}
}}
if ($stuck.Count -gt 0) {{ throw "临时证书未能从受信任根存储移除: $($stuck -join ', ')" }}
"#
        );
        run_powershell(&script).map(|_| ())
    }
}

pub struct TlsMaterial {
    pub ca_der: Vec<u8>,
    pub server_config: Arc<ServerConfig>,
}

pub fn generate_tls_material() -> Result<TlsMaterial, String> {
    let ca_key = KeyPair::generate().map_err(|error| format!("生成 CA 密钥失败: {error}"))?;
    let ca_cert = ca_params()
        .self_signed(&ca_key)
        .map_err(|error| format!("生成 CA 证书失败: {error}"))?;

    let leaf_key = KeyPair::generate().map_err(|error| format!("生成站点密钥失败: {error}"))?;
    let leaf_cert = leaf_params()?
        .signed_by(&leaf_key, &ca_cert, &ca_key)
        .map_err(|error| format!("生成站点证书失败: {error}"))?;

    let certificate = CertificateDer::from(leaf_cert.der().to_vec());
    let private_key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], private_key)
        .map_err(|error| format!("创建 TLS 配置失败: {error}"))?;

    Ok(TlsMaterial {
        ca_der: ca_cert.der().to_vec(),
        server_config: Arc::new(config),
    })
}

fn ca_params() -> CertificateParams {
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, CA_COMMON_NAME);
    let mut params = CertificateParams::default();
    params.distinguished_name = distinguished_name;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    params
}

fn leaf_params() -> Result<CertificateParams, String> {
    let mut params = CertificateParams::new(vec![TARGET_HOST.to_owned()])
        .map_err(|error| format!("创建站点证书参数失败: {error}"))?;
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, TARGET_HOST);
    params.distinguished_name = distinguished_name;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    Ok(params)
}
