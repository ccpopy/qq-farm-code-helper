param([string]$SdkDirectory = $env:ANDROID_HOME)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$specFile = Join-Path $repository 'android/native/tunnel-build.json'
$patchFile = Join-Path $repository 'android/native/disable-idle-timeouts.patch'
$spec = Get-Content -Raw -LiteralPath $specFile | ConvertFrom-Json
$inputPaths = @('android/native/tunnel-build.json', 'android/native/disable-idle-timeouts.patch', 'scripts/build-android-tunnel.ps1')
$inputs = [ordered]@{}
foreach ($relative in $inputPaths) {
    $inputs[$relative] = (Get-FileHash -LiteralPath (Join-Path $repository $relative) -Algorithm SHA256).Hash.ToLowerInvariant()
}
$aar = Join-Path $repository 'android/app/libs/hev-socks5-tunnel-2.19.1-farm1.aar'
$manifestFile = "$aar.json"
if ((Test-Path -LiteralPath $aar) -and (Test-Path -LiteralPath $manifestFile)) {
    try {
        $manifest = Get-Content -Raw -LiteralPath $manifestFile | ConvertFrom-Json
        $valid = $manifest.sha256 -eq (Get-FileHash -LiteralPath $aar -Algorithm SHA256).Hash.ToLowerInvariant()
        foreach ($relative in $inputPaths) { $valid = $valid -and $manifest.inputs.$relative -eq $inputs[$relative] }
        if ($valid) { Write-Host 'Verified locally built VPN library (idle expiry disabled).'; return }
    } catch { Write-Host 'Rebuilding native VPN library: cached provenance is not valid.' }
}
$upstreamAar = Join-Path $repository 'android/app/libs/hev-socks5-tunnel-2.19.1.aar'
if (-not (Test-Path -LiteralPath $upstreamAar) -or
        (Get-FileHash -LiteralPath $upstreamAar -Algorithm SHA256).Hash.ToLowerInvariant() -ne $spec.upstreamAarSha256) {
    throw 'Obtain the verified upstream AAR through bootstrap-android.ps1 first.'
}
$windows = [Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
$ndk = Join-Path $SdkDirectory "ndk/$($spec.ndkVersion)"
$ndkBuild = Join-Path $ndk $(if ($windows) { 'ndk-build.cmd' } else { 'ndk-build' })
if (-not (Test-Path -LiteralPath $ndkBuild)) { throw "Install Android NDK $($spec.ndkVersion) under the selected SDK." }
$work = Join-Path $repository 'outputs/android/native'
$source = Join-Path $work 'jni'
New-Item -ItemType Directory -Path $work -Force | Out-Null
function Invoke-SourceGit([string]$directory, [string[]]$gitArguments) {
    & git -c core.autocrlf=false -c core.symlinks=false -C $directory @gitArguments
    if ($LASTEXITCODE -ne 0) { throw "Native source Git operation failed: $($gitArguments[0])" }
}
if (-not (Test-Path -LiteralPath (Join-Path $source '.git'))) {
    & git -c core.autocrlf=false -c core.symlinks=false clone --no-checkout --depth 1 --branch $spec.upstreamVersion https://github.com/heiher/hev-socks5-tunnel.git $source
    if ($LASTEXITCODE -ne 0) { throw 'Native source clone failed.' }
}
if ((& git -C $source rev-parse HEAD) -ne $spec.upstreamCommit) { throw 'Unexpected native source commit; refusing to build.' }
# Only this generated cache is restored. The application checkout is never reset.
Invoke-SourceGit $source @('restore', '--source=HEAD', '--staged', '--worktree', '.')
Invoke-SourceGit $source @('submodule', 'update', '--init', '--recursive', '--depth', '1')
$modules = @($source)
foreach ($property in $spec.submodules.PSObject.Properties) {
    $module = Join-Path $source $property.Name
    if ((& git -C $module rev-parse HEAD) -ne $property.Value) { throw "Unexpected submodule commit: $($property.Name)" }
    Invoke-SourceGit $module @('restore', '--source=HEAD', '--staged', '--worktree', '.')
    $modules += $module
}
# Git on Windows may materialize symlinks as text files. Copy their pinned
# targets so the NDK can build without administrator or developer-mode access.
foreach ($module in $modules) {
    foreach ($entry in @(& git -C $module ls-files -s)) {
        if ($entry -notmatch '^120000 [0-9a-f]+ 0\t(.+)$') { continue }
        $relative = $Matches[1]
        $link = Join-Path $module $relative
        $target = (& git -C $module show "HEAD:$relative") -join "`n"
        $resolved = [IO.Path]::GetFullPath((Join-Path (Split-Path $link) $target))
        if (-not $resolved.StartsWith($source + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Native include link resolves outside its source directory.'
        }
        Copy-Item -LiteralPath $resolved -Destination $link -Force
    }
}
$configSource = Join-Path $source 'src/hev-config.c'
[IO.File]::WriteAllText($configSource, [IO.File]::ReadAllText($configSource).Replace("`r`n", "`n"))
Invoke-SourceGit $source @('apply', '--check', $patchFile)
Invoke-SourceGit $source @('apply', $patchFile)
Write-Host 'Building all four Android ABIs with interruptible, unlimited idle waits...'
$workArgument = $work.Replace('\', '/')
$sourceArgument = $source.Replace('\', '/')
& $ndkBuild "NDK_PROJECT_PATH=$workArgument" "APP_BUILD_SCRIPT=$sourceArgument/Android.mk" "NDK_APPLICATION_MK=$sourceArgument/Application.mk" APP_MODULES=hev-socks5-tunnel REV_ID=107a508-farm1 -j8 *> (Join-Path $work 'build.log')
if ($LASTEXITCODE -ne 0) { Get-Content -LiteralPath (Join-Path $work 'build.log') -Tail 20; throw 'Native build failed.' }

Add-Type -AssemblyName System.IO.Compression.FileSystem
$temporary = Join-Path $work ("tunnel-" + [Guid]::NewGuid().ToString('N') + '.aar')
$output = [IO.Compression.ZipFile]::Open($temporary, [IO.Compression.ZipArchiveMode]::Create)
$upstream = [IO.Compression.ZipFile]::OpenRead($upstreamAar)
function Add-ArchiveFile([string]$name, [string]$path) {
    $entry = $output.CreateEntry($name, [IO.Compression.CompressionLevel]::Optimal)
    $entry.LastWriteTime = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
    $destination = $entry.Open(); $inputStream = [IO.File]::OpenRead($path)
    try { $inputStream.CopyTo($destination) } finally { $inputStream.Dispose(); $destination.Dispose() }
}
try {
    foreach ($entry in $upstream.Entries) {
        if ($entry.FullName.StartsWith('jni/') -or $entry.FullName.EndsWith('/')) { continue }
        $replacement = $output.CreateEntry($entry.FullName, [IO.Compression.CompressionLevel]::Optimal)
        $replacement.LastWriteTime = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
        $inputStream = $entry.Open(); $destination = $replacement.Open()
        try { $inputStream.CopyTo($destination) } finally { $inputStream.Dispose(); $destination.Dispose() }
    }
    foreach ($abi in $spec.abis) { Add-ArchiveFile "jni/$abi/libhev-socks5-tunnel.so" (Join-Path $work "libs/$abi/libhev-socks5-tunnel.so") }
    foreach ($module in $modules) {
        foreach ($name in @('LICENSE', 'COPYING')) {
            $license = Join-Path $module $name
            if (Test-Path -LiteralPath $license) {
                $label = if ($module -eq $source) { 'tunnel' } else { (Split-Path $module -Leaf) }
                Add-ArchiveFile "META-INF/licenses/$label-$name" $license
            }
        }
    }
} finally { $upstream.Dispose(); $output.Dispose() }
Move-Item -LiteralPath $temporary -Destination $aar -Force
$provenance = [ordered]@{ sha256 = (Get-FileHash -LiteralPath $aar -Algorithm SHA256).Hash.ToLowerInvariant(); inputs = $inputs }
[IO.File]::WriteAllText($manifestFile, ($provenance | ConvertTo-Json -Depth 5))
Write-Host "Built patched VPN library: $aar"
