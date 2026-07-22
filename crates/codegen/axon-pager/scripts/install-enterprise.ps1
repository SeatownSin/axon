#
# Axon CLI installer for PowerShell (enterprise / managed deployment)
# https://github.com/SeatownSin/axon
#
# Standalone installer for managed enterprise deployments. Makes no calls to
# xAI infrastructure. Optional managed config: set AXON_DEPLOYMENT_KEY and
# AXON_PROXY_URL to fetch managed_config.toml / requirements.toml from YOUR
# organization's own proxy. There is no default proxy.
#
# Env: AXON_BIN_DIR, AXON_DEPLOYMENT_KEY, AXON_PROXY_URL
#
# Usage:
#   irm https://raw.githubusercontent.com/SeatownSin/axon/main/crates/codegen/axon-pager/scripts/install-enterprise.ps1 | iex
#

param(
    [Parameter(Position = 0)]
    [string]$Version
)

$ErrorActionPreference = 'Stop'

# PS 5.1 defaults to TLS 1.0; GitHub requires TLS 1.2.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'

if (-not $Version -and $env:AXON_VERSION) {
    $Version = $env:AXON_VERSION
}

$Repo = 'SeatownSin/axon'

if ($PSVersionTable.Platform -and $PSVersionTable.Platform -ne 'Win32NT') {
    Write-Error "This installer is for Windows. On macOS/Linux, use the install-enterprise.sh script."
    exit 1
}

$AxonDir = Join-Path $env:USERPROFILE '.axon'

# --- Helpers ---

function Download-File([string]$Url, [string]$OutFile) {
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
    'x86'     { 'x86_64' }
    'ARM64'   { 'aarch64' }
    default   { $null }
}

if (-not $arch) {
    Write-Error "Unsupported architecture: $env:PROCESSOR_ARCHITECTURE"
    exit 1
}

$platform = "windows-$arch"

# --- Resolve version ---

$DownloadDir = Join-Path $AxonDir 'downloads'
$BinDir = if ($env:AXON_BIN_DIR) { $env:AXON_BIN_DIR } else { Join-Path $AxonDir 'bin' }

New-Item -ItemType Directory -Path $DownloadDir -Force | Out-Null
New-Item -ItemType Directory -Path $BinDir -Force | Out-Null

$Channel = 'enterprise'

if ($Version) {
    $resolvedVersion = $Version
} else {
    Write-Host "Fetching latest version..." -ForegroundColor DarkGray
    try {
        $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing
        $resolvedVersion = $latest.tag_name -replace '^v', ''
    } catch {
        $resolvedVersion = $null
    }
    if (-not $resolvedVersion) {
        Write-Error "Failed to fetch latest version from GitHub Releases for $Repo"
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
$cliLines = @('installer = "internal"', 'channel = "enterprise"')

if (-not (Test-Path $ConfigFile)) {
    New-Item -ItemType Directory -Path (Split-Path $ConfigFile) -Force | Out-Null
    $content = "[cli]`r`n" + ($cliLines -join "`r`n") + "`r`n"
    [System.IO.File]::WriteAllText($ConfigFile, $content, [System.Text.Encoding]::UTF8)
} elseif ((Get-Content -Raw $ConfigFile) -match '(?m)^\[cli\]') {
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

# --- Fetch managed config from YOUR OWN proxy (opt-in) ---

if ($env:AXON_DEPLOYMENT_KEY) {
    $ProxyUrl = $env:AXON_PROXY_URL
    if (-not $ProxyUrl) {
        Write-Host '  Note: AXON_DEPLOYMENT_KEY set but AXON_PROXY_URL is empty; skipping managed-config fetch.' -ForegroundColor Yellow
    } else {
        Write-Host "  Fetching deployment config from $ProxyUrl..." -ForegroundColor DarkGray
        try {
            $headers = @{ 'Authorization' = "Bearer $($env:AXON_DEPLOYMENT_KEY)" }
            $deployResponse = Invoke-RestMethod -Uri "$ProxyUrl/deployment/config" -Headers $headers -UseBasicParsing
        } catch {
            Write-Host "  Warning: failed to fetch deployment config from $ProxyUrl/deployment/config" -ForegroundColor Yellow
            $deployResponse = $null
        }

        if ($deployResponse) {
            $managedConfig = $deployResponse.managed_config
            $requirements = $deployResponse.requirements

            $managedConfigPath = Join-Path $AxonDir 'managed_config.toml'
            $requirementsPath = Join-Path $AxonDir 'requirements.toml'

            if ($managedConfig -and $managedConfig -ne 'null') {
                [System.IO.File]::WriteAllText($managedConfigPath, $managedConfig, [System.Text.Encoding]::UTF8)
                Write-Host '  Managed config applied.' -ForegroundColor DarkGray
            } else {
                if (Test-Path $managedConfigPath) { Remove-Item $managedConfigPath -Force }
            }

            if ($requirements -and $requirements -ne 'null') {
                [System.IO.File]::WriteAllText($requirementsPath, $requirements, [System.Text.Encoding]::UTF8)
                Write-Host '  Requirements applied.' -ForegroundColor DarkGray
            } else {
                if (Test-Path $requirementsPath) { Remove-Item $requirementsPath -Force }
            }
        }
    }
}

Write-Host "Axon $resolvedVersion installed to $BinDir\axon.exe" -ForegroundColor Green

# --- Ensure axon is on PATH ---

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$pathEntries = if ($userPath) { $userPath -split ';' | Where-Object { $_ -ne '' } } else { @() }
if ($pathEntries -notcontains $BinDir) {
    $newPath = (@($BinDir) + $pathEntries) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "  Added $BinDir to your User PATH." -ForegroundColor DarkGray
    if ($env:Path -notlike "*$BinDir*") {
        $env:Path = "$BinDir;$env:Path"
    }
}

Write-Host ''
Write-Host "Run 'axon' to get started!" -ForegroundColor Cyan
