param(
    [string]$JdkDirectory = $env:JAVA_HOME,
    [string]$SdkDirectory = $env:ANDROID_HOME,
    [string]$DownloadMirror = 'https://ghfast.top'
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$platformWindows = [Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
$javacRelative = if ($platformWindows) { 'bin/javac.exe' } else { 'bin/javac' }
if (-not $JdkDirectory -or -not (Test-Path -LiteralPath (Join-Path $JdkDirectory $javacRelative))) {
    throw 'Set JAVA_HOME or -JdkDirectory to a JDK 17 directory (Android Studio is not required).'
}
if (-not $SdkDirectory) { $SdkDirectory = $env:ANDROID_SDK_ROOT }
if (-not $SdkDirectory -or -not (Test-Path -LiteralPath (Join-Path $SdkDirectory 'platforms/android-36/android.jar'))) {
    throw 'Set ANDROID_HOME or -SdkDirectory to an SDK containing platforms;android-36 and build-tools;36.0.0. Use command-line tools, not Android Studio.'
}
$env:JAVA_HOME = [IO.Path]::GetFullPath($JdkDirectory)
$env:ANDROID_HOME = [IO.Path]::GetFullPath($SdkDirectory)
$env:PATH = (Join-Path $env:JAVA_HOME 'bin') + [IO.Path]::PathSeparator + $env:PATH
Write-Host "Using JDK: $env:JAVA_HOME"
Write-Host "Using SDK: $env:ANDROID_HOME"

$aar = Join-Path $repository 'android/app/libs/hev-socks5-tunnel-2.19.1.aar'
$expected = '107958f7a07051386248ba8dba778b63d8dcefdfb2c3665b0ee9a8c93de5f6ca'
if ((Test-Path -LiteralPath $aar) -and (Get-FileHash -LiteralPath $aar -Algorithm SHA256).Hash.ToLowerInvariant() -eq $expected) {
    Write-Host 'Pinned VPN library already verified.'
    return
}
New-Item -ItemType Directory -Path (Split-Path $aar) -Force | Out-Null
$official = 'https://github.com/heiher/hev-socks5-tunnel/releases/download/2.19.1/hev-socks5-tunnel.aar'
$sources = @($official)
if ($DownloadMirror) {
    $mirrorUri = [Uri]$DownloadMirror
    if ($mirrorUri.Scheme -ne 'https' -or $mirrorUri.UserInfo -or $mirrorUri.Query -or $mirrorUri.Fragment) { throw 'DownloadMirror must be a plain HTTPS base URL.' }
    $sources += $DownloadMirror.TrimEnd('/') + '/' + $official
}
$temporary = New-TemporaryFile
try {
    foreach ($source in $sources) {
        try {
            Write-Host 'Downloading the pinned 592 KiB VPN library (SHA-256 enforced)...'
            Invoke-WebRequest -Uri $source -OutFile $temporary.FullName -TimeoutSec 45
            if ((Get-FileHash -LiteralPath $temporary.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'VPN library SHA-256 mismatch.' }
            Move-Item -LiteralPath $temporary.FullName -Destination $aar -Force
            Write-Host 'VPN library verified.'
            return
        } catch { Write-Warning 'This download source failed or returned unverified bytes; trying the next source.' }
    }
    throw 'Unable to obtain the verified VPN library. Check networking or pass -DownloadMirror.'
} finally {
    if (Test-Path -LiteralPath $temporary.FullName) { Remove-Item -LiteralPath $temporary.FullName }
}
