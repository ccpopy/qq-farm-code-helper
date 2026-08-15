use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn run_powershell(script: &str) -> Result<String, String> {
    let encoded = encode_utf16_base64(&wrap_powershell_script(script));
    let mut command = Command::new(powershell_path());
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-OutputFormat",
        "Text",
        "-EncodedCommand",
        &encoded,
    ]);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = command
        .output()
        .map_err(|error| format!("无法启动 PowerShell: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("PowerShell 执行失败: {}", output.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_start_matches('\u{feff}')
        .trim()
        .to_owned())
}

pub fn utf8_base64(value: &str) -> String {
    STANDARD.encode(value.as_bytes())
}

fn encode_utf16_base64(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    STANDARD.encode(bytes)
}

fn wrap_powershell_script(script: &str) -> String {
    format!(
        r#"
$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {{
  & {{
{script}
  }}
}} catch {{
  [Console]::Error.WriteLine($_.Exception.Message)
  exit 1
}}
"#
    )
}

fn powershell_path() -> String {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned());
    format!("{system_root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
}
