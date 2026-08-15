use crate::windows::{run_powershell, utf8_base64};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
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

    /// 直接删除当前用户物理证书存储对应的注册表项，避免通过「受保护的根存储」
    /// 删除时出现系统确认框。证书仍先从逻辑 `Root` 存储中按唯一主题筛选，防止
    /// 误删其他根证书；删除后重新打开逻辑存储复核结果。
    fn remove_trusted(&self) -> Result<(), String> {
        let subject = utf8_base64(&format!("CN={CA_COMMON_NAME}"));
        let script = format!(
            r#"
$subject = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{subject}'))
function Get-HelperThumbprints {{
  $store = [Security.Cryptography.X509Certificates.X509Store]::new(
    [Security.Cryptography.X509Certificates.StoreName]::Root,
    [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
  )
  try {{
    $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadOnly)
    @($store.Certificates |
      Where-Object {{ $_.Subject -eq $subject }} |
      ForEach-Object {{ $_.Thumbprint }} |
      Sort-Object -Unique)
  }} finally {{
    $store.Close()
  }}
}}
$thumbprints = @(Get-HelperThumbprints)
$registryPath = 'Software\Microsoft\SystemCertificates\Root\Certificates'
$rootKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($registryPath, $true)
try {{
  if ($null -ne $rootKey) {{
    foreach ($thumbprint in $thumbprints) {{
      $rootKey.DeleteSubKeyTree($thumbprint, $false)
    }}
  }}
}} finally {{
  if ($null -ne $rootKey) {{ $rootKey.Dispose() }}
}}
$remaining = @(Get-HelperThumbprints)
if ($remaining.Count -gt 0) {{
  throw "临时证书未能从当前用户受信任根存储移除: $($remaining -join ', ')"
}}
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

pub fn upstream_client_config() -> Arc<ClientConfig> {
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
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
