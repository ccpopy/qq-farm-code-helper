param(
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Debug',
    [switch]$Install,
    [string]$DeviceSerial,
    [string]$AdbPath,
    [string]$JdkDirectory = $env:JAVA_HOME,
    [string]$SdkDirectory = $env:ANDROID_HOME
)
$ErrorActionPreference = 'Stop'
$platformWindows = [Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
& (Join-Path $PSScriptRoot 'bootstrap-android.ps1') -JdkDirectory $JdkDirectory -SdkDirectory $SdkDirectory
if ($Configuration -eq 'Release' -and -not $env:ANDROID_KEYSTORE_PATH) {
    throw 'Release builds require your own persistent signing key. See android/README.md; debug signing is never used for releases.'
}
Push-Location (Join-Path $repository 'android')
try {
    $wrapper = if ($platformWindows) { '.\gradlew.bat' } else { './gradlew' }
    $variant = $Configuration.ToLowerInvariant()
    $tasks = @(':app:lintDebug', ":app:assemble$Configuration")
    & $wrapper @tasks --console plain
    if ($LASTEXITCODE -ne 0) { throw "Gradle failed ($LASTEXITCODE)." }
    $source = Join-Path $PWD "app/build/outputs/apk/$variant/app-$variant.apk"
    if (-not (Test-Path -LiteralPath $source)) { throw 'Signed APK was not produced.' }
    $versionMatch = [Regex]::Match((Get-Content -Raw -LiteralPath 'app/build.gradle'), "versionName '([^']+)'")
    $version = $versionMatch.Groups[1].Value
    $output = Join-Path $repository 'outputs/android'
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    $destination = Join-Path $output "QQFarmCodeHelper-android-v$version-$variant.apk"
    Copy-Item -LiteralPath $source -Destination $destination -Force
    Get-FileHash -LiteralPath $destination -Algorithm SHA256 | Format-List
    Write-Host "APK: $destination"
    if (-not $Install) { return }
    if (-not $AdbPath) {
        $adbCommand = Get-Command adb -ErrorAction SilentlyContinue
        if ($adbCommand) { $AdbPath = $adbCommand.Source }
        else { $AdbPath = Join-Path $env:ANDROID_HOME $(if ($platformWindows) { 'platform-tools/adb.exe' } else { 'platform-tools/adb' }) }
    }
    if (-not $DeviceSerial) {
        $devices = @(& $AdbPath devices | Select-String '^([^\s]+)\s+device$' | ForEach-Object { $_.Matches[0].Groups[1].Value })
        if ($devices.Count -ne 1) { throw 'Specify -DeviceSerial when not exactly one authorized device is connected.' }
        $DeviceSerial = $devices[0]
    }
    & $AdbPath -s $DeviceSerial install -r $destination
    if ($LASTEXITCODE -ne 0) { throw 'ADB installation failed; check the device approval prompt. No security settings were changed.' }
    & $AdbPath -s $DeviceSerial shell am start -n 'io.github.ccpopy.farmhelper/.MainActivity'
} finally { Pop-Location }
