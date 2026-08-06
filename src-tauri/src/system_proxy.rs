use crate::windows::{run_powershell, utf8_base64};
use serde_json::Value;
use std::{fs, path::PathBuf};

pub struct SystemProxyManager {
    backup_path: PathBuf,
}

impl SystemProxyManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            backup_path: data_dir.join("system-proxy-backup.json"),
        }
    }

    pub fn enable(&self, port: u16) -> Result<(), String> {
        self.recover_stale()?;
        let snapshot = capture_proxy_snapshot()?;
        write_json_atomic(&self.backup_path, &snapshot)?;
        if let Err(error) = apply_local_proxy(port) {
            let _ = self.restore();
            return Err(error);
        }
        Ok(())
    }

    pub fn restore(&self) -> Result<(), String> {
        if !self.backup_path.exists() {
            return Ok(());
        }
        let content = fs::read_to_string(&self.backup_path)
            .map_err(|error| format!("读取代理备份失败: {error}"))?;
        let snapshot: Value =
            serde_json::from_str(&content).map_err(|error| format!("代理备份格式无效: {error}"))?;
        restore_proxy_snapshot(&snapshot)?;
        fs::remove_file(&self.backup_path).map_err(|error| format!("删除代理备份失败: {error}"))?;
        Ok(())
    }

    pub fn recover_stale(&self) -> Result<(), String> {
        self.restore()
    }
}

fn capture_proxy_snapshot() -> Result<Value, String> {
    let output = run_powershell(
        r#"
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
$item = Get-ItemProperty -Path $path
$result = [ordered]@{}
foreach ($name in @('ProxyEnable', 'ProxyServer', 'ProxyOverride', 'AutoConfigURL')) {
  $property = $item.PSObject.Properties[$name]
  if ($null -eq $property) {
    $result[$name] = @{ exists = $false; value = $null }
  } else {
    $result[$name] = @{ exists = $true; value = $property.Value }
  }
}
$result | ConvertTo-Json -Compress -Depth 4
"#,
    )?;
    serde_json::from_str(&output).map_err(|error| format!("读取系统代理状态失败: {error}"))
}

fn apply_local_proxy(port: u16) -> Result<(), String> {
    let script = format!(
        r#"
$path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
New-ItemProperty -Path $path -Name ProxyEnable -PropertyType DWord -Value 1 -Force | Out-Null
New-ItemProperty -Path $path -Name ProxyServer -PropertyType String -Value '127.0.0.1:{port}' -Force | Out-Null
New-ItemProperty -Path $path -Name ProxyOverride -PropertyType String -Value '<local>' -Force | Out-Null
Remove-ItemProperty -Path $path -Name AutoConfigURL -ErrorAction SilentlyContinue
{refresh}
"#,
        refresh = refresh_script()
    );
    run_powershell(&script).map(|_| ())
}

fn restore_proxy_snapshot(snapshot: &Value) -> Result<(), String> {
    let encoded = utf8_base64(&snapshot.to_string());
    let script = format!(
        r#"
$path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
$json = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}'))
$state = $json | ConvertFrom-Json
foreach ($name in @('ProxyEnable', 'ProxyServer', 'ProxyOverride', 'AutoConfigURL')) {{
  $entry = $state.$name
  if ($entry.exists) {{
    $type = if ($name -eq 'ProxyEnable') {{ 'DWord' }} else {{ 'String' }}
    New-ItemProperty -Path $path -Name $name -PropertyType $type -Value $entry.value -Force | Out-Null
  }} else {{
    Remove-ItemProperty -Path $path -Name $name -ErrorAction SilentlyContinue
  }}
}}
{refresh}
"#,
        refresh = refresh_script()
    );
    run_powershell(&script).map(|_| ())
}

fn refresh_script() -> &'static str {
    r#"
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class QqFarmWinInetRefresh {
  [DllImport("wininet.dll", SetLastError = true)]
  public static extern bool InternetSetOption(IntPtr hInternet, int option, IntPtr buffer, int length);
}
'@
[QqFarmWinInetRefresh]::InternetSetOption([IntPtr]::Zero, 39, [IntPtr]::Zero, 0) | Out-Null
[QqFarmWinInetRefresh]::InternetSetOption([IntPtr]::Zero, 37, [IntPtr]::Zero, 0) | Out-Null
"#
}

fn write_json_atomic(path: &PathBuf, value: &Value) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let content =
        serde_json::to_vec_pretty(value).map_err(|error| format!("序列化代理备份失败: {error}"))?;
    fs::write(&temporary, content).map_err(|error| format!("写入代理备份失败: {error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("保存代理备份失败: {error}"))?;
    Ok(())
}
