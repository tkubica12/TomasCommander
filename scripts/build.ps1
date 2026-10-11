param(
    [ValidateSet('Release', 'Diagnostic', 'CleanDiagnostic')]
    [string]$Mode = 'Release',
    [ValidateSet('native-search-tests', 'native-answer-tests', 'native-desktop-tests')]
    [string[]]$Features = @()
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$target = Join-Path $root 'target'
$diagnostic = Join-Path $target 'diagnostic'
$release = Join-Path $root 'release'

if ($Features.Count -gt 0 -and $Mode -ne 'Diagnostic') {
    throw 'Test features are allowed only with -Mode Diagnostic.'
}

if ($Mode -eq 'CleanDiagnostic') {
    if (Test-Path -LiteralPath $diagnostic) {
        $item = Get-Item -LiteralPath $diagnostic
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Refusing to clean a linked diagnostic directory.'
        }
        $running = Get-Process | Where-Object {
            $_.Path -and $_.Path.StartsWith(
                $diagnostic + '\', [StringComparison]::OrdinalIgnoreCase
            )
        }
        if ($running) { throw 'Close diagnostic processes before cleanup.' }
        Remove-Item -LiteralPath $diagnostic -Recurse -Force
    }
    Write-Output 'Diagnostic outputs cleaned; release is unchanged.'
    return
}

$buildTarget = if ($Mode -eq 'Diagnostic') { $diagnostic } else { $target }
$arguments = @(
    'build', '--release', '--locked', '--bin', 'tomas-commander',
    '--no-default-features', '--target-dir', $buildTarget
)
if ($Features.Count -gt 0) {
    $arguments += @('--features', ($Features -join ','))
}

Push-Location $root
try {
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed (exit $LASTEXITCODE)." }
} finally {
    Pop-Location
}

$executable = Join-Path $buildTarget 'release\tomas-commander.exe'
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "Cargo did not produce $executable."
}
if ($Mode -eq 'Diagnostic') {
    Write-Output "Diagnostic only: $executable"
    Write-Output 'After testing and closing it, run: .\scripts\build.ps1 -Mode CleanDiagnostic'
    return
}

[IO.Directory]::CreateDirectory($release) | Out-Null
$destination = Join-Path $release 'tomas-commander.exe'
$staging = Join-Path $release ("publish-" + [Guid]::NewGuid().ToString('N') + '.tmp')
try {
    Copy-Item -LiteralPath $executable -Destination $staging
    if ((Get-FileHash -LiteralPath $staging).Hash -ne
        (Get-FileHash -LiteralPath $executable).Hash) {
        throw 'Release staging hash differs from the build.'
    }
    if (Test-Path -LiteralPath $destination) {
        [IO.File]::Replace($staging, $destination, [System.Management.Automation.Language.NullString]::Value)
    } else {
        [IO.File]::Move($staging, $destination)
    }
} finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Force
    }
}
Write-Output "Current executable: $destination"
