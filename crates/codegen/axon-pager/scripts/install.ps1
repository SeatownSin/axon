#
# Axon CLI installer for PowerShell
# https://github.com/SeatownSin/axon
#
# Downloads a released `axon` binary from this fork's GitHub Releases and
# installs it to %USERPROFILE%\.axon\bin. Makes no calls to xAI infrastructure.
#
# Env: AXON_CHANNEL (stable|alpha, default: stable), AXON_BIN_DIR
#
# Usage:
#   irm https://raw.githubusercontent.com/SeatownSin/axon/main/crates/codegen/axon-pager/scripts/install.ps1 | iex   # latest stable
#   $env:AXON_VERSION="0.2.0"; irm <url> | iex                                                                                   # specific version
#

param(
    [Parameter(Position = 0)]
    [string]$Version
)

$ErrorActionPreference = 'Stop'

# PS 5.1 defaults to TLS 1.0; GitHub requires TLS 1.2.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

# PS 5.1's Invoke-WebRequest progress bar is extremely slow; disable it.
$ProgressPreference = 'SilentlyContinue'

# Accept version from environment variable (useful with irm | iex).
if (-not $Version -and $env:AXON_VERSION) {
    $Version = $env:AXON_VERSION
}

$Repo = 'SeatownSin/axon'

# This script is Windows-only. PS 5.1 has no Platform property and only runs on Windows.
if ($PSVersionTable.Platform -and $PSVersionTable.Platform -ne 'Win32NT') {
    Write-Error "This installer is for Windows. On macOS/Linux, use the install.sh script."
    exit 1
}

$AxonDir = Join-Path $env:USERPROFILE '.axon'

# --- Helpers ---

function Download-String([string]$Url) {
    try {
        $response = Invoke-WebRequest -Uri $Url -UseBasicParsing
        return $response.Content
    } catch {
        return $null
    }
}

function Download-File([string]$Url, [string]$OutFile) {
    # Stream via HttpWebRequest — faster than Invoke-WebRequest on PS 5.1 and supports progress.
    $request = [System.Net.HttpWebRequest]::Create($Url)
    $request.Timeout = 300000  # 5 min
    $request.AutomaticDecompression = [System.Net.DecompressionMethods]::GZip -bor [System.Net.DecompressionMethods]::Deflate
    $response = $request.GetResponse()
    $totalBytes = $response.ContentLength
    $stream = $response.GetResponseStream()
    $fileStream = [System.IO.File]::Create($OutFile)
    $buffer = New-Object byte[] 65536
    $totalRead = 0
    $lastPercent = -1
    $lastMb = -1

    try {
        while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $fileStream.Write($buffer, 0, $read)
            $totalRead += $read
            $mb = [math]::Round($totalRead / 1MB, 1)
            if ($totalBytes -gt 0) {
                $percent = [math]::Min(100, [math]::Floor(($totalRead / $totalBytes) * 100))
                if ($percent -ne $lastPercent) {
                    $totalMb = [math]::Round($totalBytes / 1MB, 1)
                    Write-Host "`r  Downloading... ${mb} MB / ${totalMb} MB (${percent}%)" -NoNewline
                    $lastPercent = $percent
                }
            } elseif ($mb -ne $lastMb) {
                Write-Host "`r  Downloading... ${mb} MB" -NoNewline
                $lastMb = $mb
            }
        }
        Write-Host ''
    } finally {
        $fileStream.Close()
        $stream.Close()
        $response.Close()
    }
}

# --- Validate version ---

if ($Version -and $Version -notmatch '^\d+\.\d+\.\d+(-\S+)?$') {
    Write-Error "Invalid version format: $Version (expected X.Y.Z or X.Y.Z-suffix)"
    exit 1
}

# --- Detect architecture ---

$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64'   { 'x86_64' }
    'x86'     { 'x86_64' }   # 32-bit PS on 64-bit Windows
    'ARM64'   { 'aarch64' }
    default   { $null }
}

if (-not $arch) {
    Write-Error "Unsupported architecture: $env:PROCESSOR_ARCHITECTURE"
    exit 1
}

$platform = "windows-$arch"

# --- Resolve version and channel ---

$DownloadDir = Join-Path $AxonDir 'downloads'
$BinDir = if ($env:AXON_BIN_DIR) { $env:AXON_BIN_DIR } else { Join-Path $AxonDir 'bin' }

New-Item -ItemType Directory -Path $DownloadDir -Force | Out-Null
New-Item -ItemType Directory -Path $BinDir -Force | Out-Null

$Channel = if ($env:AXON_CHANNEL) { $env:AXON_CHANNEL } else { 'stable' }

# Resolve the version. When none is passed, query the GitHub API for the newest
# release tag (newest prerelease on the alpha channel, newest full release
# otherwise).
if ($Version) {
    $resolvedVersion = $Version
} else {
    Write-Host "Fetching latest $Channel version..." -ForegroundColor DarkGray
    try {
        if ($Channel -eq 'alpha') {
            $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases?per_page=10" -UseBasicParsing
            $resolvedVersion = ($releases | Select-Object -First 1).tag_name -replace '^v', ''
        } else {
            $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing
            $resolvedVersion = $latest.tag_name -replace '^v', ''
        }
    } catch {
        $resolvedVersion = $null
    }
    if (-not $resolvedVersion) {
        Write-Error "Failed to fetch latest $Channel version from GitHub Releases for $Repo"
        exit 1
    }
}

Write-Host "Installing Axon $resolvedVersion ($platform)..." -ForegroundColor Cyan

# --- Download binary ---

$BaseUrl = "https://github.com/$Repo/releases/download/v$resolvedVersion"
$binaryPath = Join-Path $DownloadDir "axon-$platform.exe"
$artifactBase = "$BaseUrl/axon-$resolvedVersion-$platform"

$downloaded = $false
foreach ($url in @("$artifactBase.exe", $artifactBase)) {
    try {
        Download-File $url $binaryPath
        $downloaded = $true
        break
    } catch {
        continue
    }
}

if (-not $downloaded) {
    if (Test-Path $binaryPath) { Remove-Item $binaryPath -Force }
    Write-Error "Binary download failed from $artifactBase.exe and $artifactBase"
    exit 1
}

# --- Install binary (locked-file safe) ---

$binName = 'axon.exe'
$dest = Join-Path $BinDir $binName
$old = "$dest.old"

if (Test-Path $old) { Remove-Item $old -Force -ErrorAction SilentlyContinue }

try {
    Copy-Item -Path $binaryPath -Destination $dest -Force
} catch {
    try {
        if (Test-Path $dest) { Rename-Item $dest $old -Force -ErrorAction SilentlyContinue }
        Copy-Item -Path $binaryPath -Destination $dest -Force
    } catch {
        if (Test-Path $old) { Rename-Item $old $dest -Force -ErrorAction SilentlyContinue }
        Write-Error "Failed to install $binName"
        exit 1
    }
}

Write-Host "  Installed to $BinDir\axon.exe." -ForegroundColor DarkGray

# --- Generate completions (best-effort) ---

$completionsDir = Join-Path (Join-Path $AxonDir 'completions') 'powershell'
try {
    New-Item -ItemType Directory -Path $completionsDir -Force | Out-Null
    & (Join-Path $BinDir 'axon.exe') completions powershell 2>$null |
        Set-Content (Join-Path $completionsDir 'axon.ps1') -ErrorAction SilentlyContinue
} catch {}

# --- Persist installer config ---

$ConfigFile = Join-Path $AxonDir 'config.toml'
$cliLines = @('installer = "internal"')
if ($Channel -ne 'stable') {
    $cliLines += "channel = `"$Channel`""
}

if (-not (Test-Path $ConfigFile)) {
    New-Item -ItemType Directory -Path (Split-Path $ConfigFile) -Force | Out-Null
    $content = "[cli]`r`n" + ($cliLines -join "`r`n") + "`r`n"
    [System.IO.File]::WriteAllText($ConfigFile, $content, [System.Text.Encoding]::UTF8)
} elseif ((Get-Content -Raw $ConfigFile) -match '(?m)^\[cli\]') {
    # Section-aware: only replace installer/channel under [cli], not other sections.
    $existingLines = Get-Content $ConfigFile
    $output = [System.Collections.ArrayList]::new()
    $inCli = $false

    foreach ($line in $existingLines) {
        if ($line -match '^\[cli\]\s*(#.*)?$') {
            [void]$output.Add($line)
            foreach ($cl in $cliLines) { [void]$output.Add($cl) }
            $inCli = $true
            continue
        }
        if ($line -match '^\[.+\]\s*(#.*)?$') {
            $inCli = $false
        }
        if ($inCli -and $line -match '^\s*(installer|channel)\s*=') {
            continue
        }
        [void]$output.Add($line)
    }
    [System.IO.File]::WriteAllLines($ConfigFile, [string[]]$output.ToArray(), [System.Text.Encoding]::UTF8)
} else {
    Add-Content -Path $ConfigFile -Value "`r`n[cli]`r`n$($cliLines -join "`r`n")`r`n"
}

Write-Host "Axon $resolvedVersion installed to $BinDir\axon.exe" -ForegroundColor Green

# --- Ensure axon is on PATH ---

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$pathEntries = if ($userPath) { $userPath -split ';' | Where-Object { $_ -ne '' } } else { @() }
if ($pathEntries -notcontains $BinDir) {
    $newPath = (@($BinDir) + $pathEntries) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "  Added $BinDir to your User PATH." -ForegroundColor DarkGray
    # Update current session so axon works immediately.
    if ($env:Path -notlike "*$BinDir*") {
        $env:Path = "$BinDir;$env:Path"
    }
}

Write-Host ''
Write-Host "Run 'axon' to get started!" -ForegroundColor Cyan
