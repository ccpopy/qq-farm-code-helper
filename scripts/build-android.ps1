param(
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Debug',
    [string]$Version,
    [switch]$Bundle,
    [switch]$Install,
    [string]$DeviceSerial,
    [string]$AdbPath,
    [string]$JdkDirectory = $env:JAVA_HOME,
    [string]$SdkDirectory = $env:ANDROID_HOME
)
$ErrorActionPreference = 'Stop'
if ($Bundle -and $Configuration -ne 'Release') { throw 'App bundles are only produced for Release builds.' }
if ($Version -and $Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
    throw 'Version must be a stable X.Y.Z version.'
}
$platformWindows = [Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
& (Join-Path $PSScriptRoot 'bootstrap-android.ps1') -JdkDirectory $JdkDirectory -SdkDirectory $SdkDirectory
Push-Location (Join-Path $repository 'android')
try {
    $wrapper = if ($platformWindows) { '.\gradlew.bat' } else { './gradlew' }
    $variant = $Configuration.ToLowerInvariant()
    $tasks = @(':app:lintDebug', ":app:assemble$Configuration")
    if ($Bundle) { $tasks += ':app:bundleRelease' }
    if ($Version) { $tasks += "-PfarmVersion=$Version" }
    & $wrapper @tasks --console plain
    if ($LASTEXITCODE -ne 0) { throw "Gradle failed ($LASTEXITCODE)." }
    $source = Join-Path $PWD "app/build/outputs/apk/$variant/app-$variant.apk"
    if (-not (Test-Path -LiteralPath $source)) { throw 'Signed APK was not produced.' }
    $buildTools = Join-Path $env:ANDROID_HOME 'build-tools/36.0.0'
    $signer = Join-Path $buildTools $(if ($platformWindows) { 'apksigner.bat' } else { 'apksigner' })
    $certificate = & $signer verify --print-certs $source
    if ($LASTEXITCODE -ne 0 -or -not ($certificate -match 'certificate SHA-256 digest: cf51c2ee1ed4870e98751894f41cb5406b7f2cd29645df2fa5186f829235c749')) {
        throw 'APK signature does not match the fixed FarmHelper signing identity.'
    }
    $aapt = Join-Path $buildTools $(if ($platformWindows) { 'aapt.exe' } else { 'aapt' })
    $badging = & $aapt dump badging $source
    if ($LASTEXITCODE -ne 0 -or -not ($badging -match "^package: name='io.github.ccpopy.farmhelper'") -or
        -not ($badging -match "^application-label:'农场助手'$")) {
        throw 'APK package or display name differs from the canonical FarmHelper app.'
    }
    $versionMatch = [Regex]::Match(($badging -join "`n"), "(?m)^package:.*versionName='([^']+)'")
    if (-not $versionMatch.Success) { throw 'Unable to read the built APK version.' }
    $builtVersion = $versionMatch.Groups[1].Value
    if ($Version -and $builtVersion -ne $Version) { throw 'Built APK version does not match the requested version.' }
    $output = Join-Path $repository 'outputs/android'
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    $suffix = if ($Configuration -eq 'Release') { '' } else { '-debug' }
    $destination = Join-Path $output "QQFarmCodeHelper-android-v$builtVersion$suffix.apk"
    Copy-Item -LiteralPath $source -Destination $destination -Force
    Copy-Item -LiteralPath $source -Destination (Join-Path $output 'QQFarmCodeHelper.apk') -Force
    if ($Bundle) {
        $bundleSource = Join-Path $PWD 'app/build/outputs/bundle/release/app-release.aab'
        if (-not (Test-Path -LiteralPath $bundleSource)) { throw 'Signed AAB was not produced.' }
        $jarSigner = Join-Path $env:JAVA_HOME $(if ($platformWindows) { 'bin/jarsigner.exe' } else { 'bin/jarsigner' })
        $bundleVerification = & $jarSigner -verify $bundleSource 2>&1
        if ($LASTEXITCODE -ne 0) { $bundleVerification | Select-Object -Last 20; throw 'AAB signature verification failed.' }
        $keyTool = Join-Path $env:JAVA_HOME $(if ($platformWindows) { 'bin/keytool.exe' } else { 'bin/keytool' })
        $bundleCertificate = & $keyTool '-J-Duser.language=en' -printcert -jarfile $bundleSource
        $normalizedCertificate = ($bundleCertificate -join "`n").Replace(':', '').ToLowerInvariant()
        if ($LASTEXITCODE -ne 0 -or -not $normalizedCertificate.Contains('cf51c2ee1ed4870e98751894f41cb5406b7f2cd29645df2fa5186f829235c749')) {
            throw 'AAB signing certificate differs from the fixed FarmHelper identity.'
        }
        Copy-Item -LiteralPath $bundleSource -Destination (Join-Path $output "QQFarmCodeHelper-android-v$builtVersion.aab") -Force
        Write-Host 'AAB signature verified against the fixed FarmHelper identity.'
    }
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
