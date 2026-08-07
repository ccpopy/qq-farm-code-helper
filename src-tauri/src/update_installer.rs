use crate::updater::{PackageKind, PreparedUpdate};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    path::Path,
    process::{Command, Stdio},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub fn launch(update: PreparedUpdate) -> Result<(), String> {
    if !update.package_path.is_file() {
        return Err("已下载的更新包不存在".to_owned());
    }
    let script = build_update_script(&update);
    launch_powershell(&script)
}

fn build_update_script(update: &PreparedUpdate) -> String {
    let executable = powershell_literal(&update.current_executable);
    let package = powershell_literal(&update.package_path);
    let install_directory = powershell_literal(&update.install_directory);
    let failure_log = powershell_literal(
        &update
            .install_directory
            .join("qq-farm-code-helper-update-error.log"),
    );
    let mut lines = vec![
        "$ErrorActionPreference = 'Stop'".to_owned(),
        format!("$parentPid = {}", std::process::id()),
        format!("$exePath = {executable}"),
        format!("$packagePath = {package}"),
        format!("$installDir = {install_directory}"),
        format!("$failureLog = {failure_log}"),
        "$backupPath = $null".to_owned(),
        "$deadline = [DateTime]::UtcNow.AddSeconds(45)".to_owned(),
        "while ($true) {".to_owned(),
        "  $parent = Get-Process -Id $parentPid -ErrorAction SilentlyContinue".to_owned(),
        "  if (-not $parent) { break }".to_owned(),
        "  try {".to_owned(),
        "    if (-not $parent.Path -or -not [string]::Equals([IO.Path]::GetFullPath($parent.Path), [IO.Path]::GetFullPath($exePath), [StringComparison]::OrdinalIgnoreCase)) { break }".to_owned(),
        "  } catch { break }".to_owned(),
        "  if ([DateTime]::UtcNow -ge $deadline) {".to_owned(),
        "    Stop-Process -Id $parentPid -Force -ErrorAction SilentlyContinue".to_owned(),
        "    break".to_owned(),
        "  }".to_owned(),
        "  Start-Sleep -Milliseconds 200".to_owned(),
        "}".to_owned(),
        "$processName = [IO.Path]::GetFileNameWithoutExtension($exePath)".to_owned(),
        "Get-Process -Name $processName -ErrorAction SilentlyContinue | ForEach-Object {".to_owned(),
        "  try {".to_owned(),
        "    if ($_.Id -ne $PID -and $_.Path -and [string]::Equals([IO.Path]::GetFullPath($_.Path), [IO.Path]::GetFullPath($exePath), [StringComparison]::OrdinalIgnoreCase)) {".to_owned(),
        "      Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue".to_owned(),
        "    }".to_owned(),
        "  } catch {}".to_owned(),
        "}".to_owned(),
        "Start-Sleep -Milliseconds 350".to_owned(),
        "try {".to_owned(),
        "  Remove-Item -LiteralPath $failureLog -Force -ErrorAction SilentlyContinue".to_owned(),
    ];

    match update.kind {
        PackageKind::Installer => {
            lines.extend([
                "$arguments = @('/S', ('/D=' + $installDir))".to_owned(),
                "$installer = Start-Process -FilePath $packagePath -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait".to_owned(),
                "  if ($installer.ExitCode -ne 0) { throw ('安装程序退出码: ' + $installer.ExitCode) }".to_owned(),
                "  Remove-Item -LiteralPath $packagePath -Force -ErrorAction SilentlyContinue".to_owned(),
                "  if (-not (Test-Path -LiteralPath $exePath)) { throw '安装完成后未找到主程序' }".to_owned(),
                "  Start-Process -FilePath $exePath -WorkingDirectory $installDir".to_owned(),
            ]);
        }
        PackageKind::Portable => {
            lines.extend([
                "$backupPath = $exePath + '.update-backup'".to_owned(),
                "$stagedPath = $exePath + '.update-new'".to_owned(),
                "  Remove-Item -LiteralPath $backupPath -Force -ErrorAction SilentlyContinue".to_owned(),
                "  Remove-Item -LiteralPath $stagedPath -Force -ErrorAction SilentlyContinue".to_owned(),
                "  Copy-Item -LiteralPath $packagePath -Destination $stagedPath -Force".to_owned(),
                "  Move-Item -LiteralPath $exePath -Destination $backupPath -Force".to_owned(),
                "  Move-Item -LiteralPath $stagedPath -Destination $exePath -Force".to_owned(),
                "  $updatedProcess = Start-Process -FilePath $exePath -WorkingDirectory $installDir -PassThru".to_owned(),
                "  Start-Sleep -Seconds 2".to_owned(),
                "  if ($updatedProcess.HasExited) { throw '更新后的程序启动失败' }".to_owned(),
                "  Remove-Item -LiteralPath $backupPath -Force -ErrorAction SilentlyContinue".to_owned(),
                "  Remove-Item -LiteralPath $packagePath -Force -ErrorAction SilentlyContinue".to_owned(),
            ]);
        }
    }

    lines.extend([
        "} catch {".to_owned(),
        "  ($_ | Out-String) | Set-Content -LiteralPath $failureLog -Encoding UTF8".to_owned(),
        "  if ($backupPath -and (Test-Path -LiteralPath $backupPath)) {".to_owned(),
        "    Remove-Item -LiteralPath $exePath -Force -ErrorAction SilentlyContinue".to_owned(),
        "    Move-Item -LiteralPath $backupPath -Destination $exePath -Force".to_owned(),
        "  }".to_owned(),
        "  if (Test-Path -LiteralPath $exePath) {".to_owned(),
        "    Start-Process -FilePath $exePath -WorkingDirectory $installDir -ErrorAction SilentlyContinue".to_owned(),
        "  }".to_owned(),
        "  exit 1".to_owned(),
        "}".to_owned(),
    ]);
    lines.join("\r\n")
}

fn powershell_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn encoded_powershell_command(script: &str) -> String {
    let mut bytes = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    STANDARD.encode(bytes)
}

#[cfg(windows)]
fn launch_powershell(script: &str) -> Result<(), String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let encoded = encoded_powershell_command(script);
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
            &encoded,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("启动静默更新程序失败: {error}"))
}

#[cfg(not(windows))]
fn launch_powershell(_script: &str) -> Result<(), String> {
    Err("自动安装仅支持 Windows".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn prepared(kind: PackageKind) -> PreparedUpdate {
        PreparedUpdate {
            package_path: PathBuf::from("C:/Temp/update.exe"),
            current_executable: PathBuf::from("D:/Farm Tool/qq-farm-code-helper.exe"),
            install_directory: PathBuf::from("D:/Farm Tool"),
            kind,
        }
    }

    #[test]
    fn quotes_powershell_paths_with_apostrophes() {
        assert_eq!(
            powershell_literal(Path::new("D:/Farmer's/app.exe")),
            "'D:/Farmer''s/app.exe'"
        );
    }

    #[test]
    fn installer_script_keeps_the_current_install_directory() {
        let script = build_update_script(&prepared(PackageKind::Installer));

        assert!(script.contains("$arguments = @('/S', ('/D=' + $installDir))"));
        assert!(script.contains("$installDir = 'D:/Farm Tool'"));
        assert!(script.contains("Stop-Process"));
    }

    #[test]
    fn portable_script_replaces_the_current_executable() {
        let script = build_update_script(&prepared(PackageKind::Portable));

        assert!(script.contains("$backupPath = $exePath + '.update-backup'"));
        assert!(script.contains("Move-Item -LiteralPath $stagedPath -Destination $exePath"));
    }

    #[cfg(windows)]
    #[test]
    fn generated_update_scripts_are_valid_powershell() {
        for kind in [PackageKind::Installer, PackageKind::Portable] {
            let encoded = encoded_powershell_command(&build_update_script(&prepared(kind)));
            let parser = "$code=[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($env:QQ_FARM_UPDATE_SCRIPT));$tokens=$null;$errors=$null;[Management.Automation.Language.Parser]::ParseInput($code,[ref]$tokens,[ref]$errors)|Out-Null;if($errors.Count -gt 0){$errors|ForEach-Object{Write-Error $_.Message};exit 1}";
            let output = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", parser])
                .env("QQ_FARM_UPDATE_SCRIPT", encoded)
                .output()
                .unwrap();

            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
