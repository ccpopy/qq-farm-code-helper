use crate::windows::{run_powershell, utf8_base64};
use serde_json::Value;
use std::{fs, path::PathBuf, thread, time::Duration};

const APPLY_VERIFY_ATTEMPTS: usize = 3;
const APPLY_VERIFY_DELAY: Duration = Duration::from_millis(800);

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
        if let Err(error) = apply_local_proxy_verified(port) {
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
        restore_proxy_snapshot_verified(&snapshot)?;
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
    run_powershell(&local_proxy_script(port)).map(|_| ())
}

fn local_proxy_script(port: u16) -> String {
    format!(
        r#"
$path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
New-ItemProperty -Path $path -Name ProxyEnable -PropertyType DWord -Value 1 -Force | Out-Null
New-ItemProperty -Path $path -Name ProxyServer -PropertyType String -Value '127.0.0.1:{port}' -Force | Out-Null
{refresh}
"#,
        refresh = refresh_script()
    )
}

fn apply_local_proxy_verified(port: u16) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 1..=APPLY_VERIFY_ATTEMPTS {
        match apply_local_proxy(port) {
            Ok(()) => {
                thread::sleep(APPLY_VERIFY_DELAY);
                match capture_stable_proxy_snapshot(|snapshot| {
                    proxy_snapshot_matches_local(snapshot, port)
                }) {
                    Ok(snapshot) if proxy_snapshot_matches_local(&snapshot, port) => return Ok(()),
                    Ok(snapshot) => {
                        last_error = format!(
                            "第 {attempt} 次写入后的系统代理仍为 {}",
                            proxy_snapshot_summary(&snapshot),
                        );
                    }
                    Err(error) => {
                        last_error = format!("第 {attempt} 次写入后无法读回系统代理: {error}");
                    }
                }
            }
            Err(error) => {
                last_error = format!("第 {attempt} 次写入系统代理失败: {error}");
            }
        }
        if attempt < APPLY_VERIFY_ATTEMPTS {
            thread::sleep(APPLY_VERIFY_DELAY);
        }
    }
    Err(format!(
        "系统代理未能稳定切换到 127.0.0.1:{port}，已尝试 {APPLY_VERIFY_ATTEMPTS} 次；{last_error}"
    ))
}

fn restore_proxy_snapshot_verified(expected: &Value) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 1..=APPLY_VERIFY_ATTEMPTS {
        match restore_proxy_snapshot(expected) {
            Ok(()) => {
                thread::sleep(APPLY_VERIFY_DELAY);
                match capture_stable_proxy_snapshot(|snapshot| {
                    proxy_snapshots_match(expected, snapshot)
                }) {
                    Ok(snapshot) if proxy_snapshots_match(expected, &snapshot) => return Ok(()),
                    Ok(snapshot) => {
                        last_error = format!(
                            "第 {attempt} 次恢复后的系统代理仍为 {}",
                            proxy_snapshot_summary(&snapshot),
                        );
                    }
                    Err(error) => {
                        last_error = format!("第 {attempt} 次恢复后无法稳定读回系统代理: {error}");
                    }
                }
            }
            Err(error) => {
                last_error = format!("第 {attempt} 次恢复系统代理失败: {error}");
            }
        }
        if attempt < APPLY_VERIFY_ATTEMPTS {
            thread::sleep(APPLY_VERIFY_DELAY);
        }
    }
    Err(format!(
        "系统代理未能恢复到启动前状态，备份文件已保留以便重试；{last_error}"
    ))
}

fn capture_stable_proxy_snapshot(
    matches_expected: impl Fn(&Value) -> bool,
) -> Result<Value, String> {
    let first = capture_proxy_snapshot()?;
    if !matches_expected(&first) {
        return Ok(first);
    }
    thread::sleep(APPLY_VERIFY_DELAY);
    capture_proxy_snapshot()
}

fn proxy_snapshot_matches_local(snapshot: &Value, port: u16) -> bool {
    let expected_server = format!("127.0.0.1:{port}");
    proxy_snapshot_value(snapshot, "ProxyEnable").and_then(Value::as_i64) == Some(1)
        && proxy_snapshot_value(snapshot, "ProxyServer").and_then(Value::as_str)
            == Some(expected_server.as_str())
}

fn proxy_snapshots_match(expected: &Value, actual: &Value) -> bool {
    [
        "ProxyEnable",
        "ProxyServer",
        "ProxyOverride",
        "AutoConfigURL",
    ]
    .into_iter()
    .all(|name| proxy_snapshot_entry_matches(expected, actual, name))
}

fn proxy_snapshot_entry_matches(expected: &Value, actual: &Value, name: &str) -> bool {
    let expected_entry = expected.get(name);
    let actual_entry = actual.get(name);
    let expected_exists = expected_entry
        .and_then(|entry| entry.get("exists"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let actual_exists = actual_entry
        .and_then(|entry| entry.get("exists"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if expected_exists != actual_exists {
        return false;
    }
    !expected_exists
        || expected_entry.and_then(|entry| entry.get("value"))
            == actual_entry.and_then(|entry| entry.get("value"))
}

fn proxy_snapshot_value<'a>(snapshot: &'a Value, name: &str) -> Option<&'a Value> {
    let entry = snapshot.get(name)?;
    if entry.get("exists").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    entry.get("value")
}

fn proxy_snapshot_summary(snapshot: &Value) -> String {
    let enabled = proxy_snapshot_value(snapshot, "ProxyEnable")
        .map(Value::to_string)
        .unwrap_or_else(|| "<missing>".to_owned());
    let server = proxy_snapshot_value(snapshot, "ProxyServer")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");
    format!("ProxyEnable={enabled}, ProxyServer={server}")
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_only_the_enabled_expected_local_proxy() {
        let expected = json!({
            "ProxyEnable": { "exists": true, "value": 1 },
            "ProxyServer": { "exists": true, "value": "127.0.0.1:8899" },
        });
        assert!(proxy_snapshot_matches_local(&expected, 8899));

        let disabled = json!({
            "ProxyEnable": { "exists": true, "value": 0 },
            "ProxyServer": { "exists": true, "value": "127.0.0.1:8899" },
        });
        assert!(!proxy_snapshot_matches_local(&disabled, 8899));

        let wrong_port = json!({
            "ProxyEnable": { "exists": true, "value": 1 },
            "ProxyServer": { "exists": true, "value": "127.0.0.1:20808" },
        });
        assert!(!proxy_snapshot_matches_local(&wrong_port, 8899));
    }

    #[test]
    fn enabling_proxy_preserves_existing_bypass_and_pac_values() {
        let script = local_proxy_script(8899);
        assert!(script.contains("ProxyEnable"));
        assert!(script.contains("127.0.0.1:8899"));
        assert!(!script.contains("ProxyOverride"));
        assert!(!script.contains("AutoConfigURL"));
    }

    #[test]
    fn treats_missing_registry_values_as_a_failed_verification() {
        assert!(!proxy_snapshot_matches_local(&json!({}), 8899));
        assert_eq!(
            proxy_snapshot_summary(&json!({})),
            "ProxyEnable=<missing>, ProxyServer=<missing>",
        );
    }

    #[test]
    fn restore_verification_compares_value_existence_and_contents() {
        let expected = json!({
            "ProxyEnable": { "exists": true, "value": 0 },
            "ProxyServer": { "exists": true, "value": "127.0.0.1:20808" },
            "ProxyOverride": { "exists": false, "value": null },
            "AutoConfigURL": { "exists": false, "value": null },
        });
        assert!(proxy_snapshots_match(&expected, &expected));

        let wrong_server = json!({
            "ProxyEnable": { "exists": true, "value": 0 },
            "ProxyServer": { "exists": true, "value": "127.0.0.1:8899" },
            "ProxyOverride": { "exists": false, "value": null },
            "AutoConfigURL": { "exists": false, "value": null },
        });
        assert!(!proxy_snapshots_match(&expected, &wrong_server));

        let unexpected_override = json!({
            "ProxyEnable": { "exists": true, "value": 0 },
            "ProxyServer": { "exists": true, "value": "127.0.0.1:20808" },
            "ProxyOverride": { "exists": true, "value": "<local>" },
            "AutoConfigURL": { "exists": false, "value": null },
        });
        assert!(!proxy_snapshots_match(&expected, &unexpected_override));
    }

    #[test]
    #[ignore = "explicitly modifies the current Windows user's proxy settings"]
    fn live_enable_and_restore_round_trip() {
        if std::env::var("QQ_FARM_LIVE_PROXY_TEST").as_deref() != Ok("1") {
            return;
        }

        let root = std::env::temp_dir().join(format!(
            "qq-farm-live-proxy-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        ));
        fs::create_dir_all(&root).unwrap();
        let before = capture_proxy_snapshot().unwrap();
        let manager = SystemProxyManager::new(root.clone());

        let enable_result = manager.enable(8899);
        thread::sleep(Duration::from_secs(3));
        let during = capture_proxy_snapshot();
        let restore_result = manager.restore();
        if restore_result.is_err() {
            let _ = restore_proxy_snapshot(&before);
        }
        let after = capture_proxy_snapshot();
        let _ = fs::remove_dir_all(root);

        enable_result.unwrap();
        assert!(proxy_snapshot_matches_local(&during.unwrap(), 8899));
        restore_result.unwrap();
        assert!(proxy_snapshots_match(&before, &after.unwrap()));
    }
}
