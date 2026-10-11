$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
$fixture = Join-Path $repository ('target\build-workflow-' + [Guid]::NewGuid().ToString('N'))
$scripts = Join-Path $fixture 'scripts'
$state = @{ fail = $false; bytes = 'first build'; arguments = @(); calls = 0 }
$checks = 0

function Assert-Workflow([bool]$Condition, [string]$Name) {
    if (-not $Condition) { throw "Build workflow check failed: $Name" }
    $script:checks++
}

function cargo {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
    $state.arguments = $Arguments
    $state.calls++
    if ($state.fail) { $global:LASTEXITCODE = 17; return }
    $index = [Array]::IndexOf($Arguments, '--target-dir')
    if ($index -lt 0) { throw 'Build must bind an explicit target directory.' }
    $output = Join-Path $Arguments[$index + 1] 'release\tomas-commander.exe'
    [IO.Directory]::CreateDirectory((Split-Path $output -Parent)) | Out-Null
    [IO.File]::WriteAllText($output, $state.bytes)
    $global:LASTEXITCODE = 0
}

try {
    [IO.Directory]::CreateDirectory($scripts) | Out-Null
    Copy-Item -LiteralPath (Join-Path $repository 'scripts\build.ps1') -Destination $scripts
    $builder = Join-Path $scripts 'build.ps1'
    $published = Join-Path $fixture 'release\tomas-commander.exe'
    & $builder | Out-Null
    Assert-Workflow ([IO.File]::ReadAllText($published) -eq 'first build') 'Initial release publication'
    Assert-Workflow (($state.arguments -contains '--locked') -and
        ($state.arguments -contains '--no-default-features') -and
        -not ($state.arguments -contains '--features')) 'Locked default-only shipping build'
    Assert-Workflow ($state.arguments[-1] -eq (Join-Path $fixture 'target')) 'One standard Cargo cache'

    $state.bytes = 'second build'
    & $builder | Out-Null
    Assert-Workflow ([IO.File]::ReadAllText($published) -eq 'second build') 'Replace current release'
    Assert-Workflow (@(Get-ChildItem (Split-Path $published -Parent) -File).Count -eq 1) 'Only one published file'

    $state.fail = $true
    $failed = $false
    try { & $builder | Out-Null } catch {
        if ($_.Exception.Message -notlike 'Cargo build failed*') { throw }
        $failed = $true
    }
    Assert-Workflow ($failed -and [IO.File]::ReadAllText($published) -eq 'second build') 'Failed build retains current release'
    $state.fail = $false

    $calls = $state.calls
    $rejected = $false
    try { & $builder -Features native-desktop-tests | Out-Null } catch {
        if ($_.Exception.Message -notlike 'Test features are allowed only*') { throw }
        $rejected = $true
    }
    Assert-Workflow ($rejected -and $state.calls -eq $calls) 'Shipping refuses diagnostic features before building'

    $state.bytes = 'diagnostic build'
    & $builder -Mode Diagnostic -Features native-desktop-tests | Out-Null
    $diagnostic = Join-Path $fixture 'target\diagnostic\release\tomas-commander.exe'
    Assert-Workflow ((Test-Path -LiteralPath $diagnostic) -and
        [IO.File]::ReadAllText($published) -eq 'second build' -and
        ($state.arguments -contains '--features')) 'Diagnostic is isolated from shipping'
    & $builder -Mode CleanDiagnostic | Out-Null
    Assert-Workflow (-not (Test-Path -LiteralPath (Join-Path $fixture 'target\diagnostic')) -and
        [IO.File]::ReadAllText($published) -eq 'second build') 'Cleanup removes only diagnostics'
    & $builder -Mode CleanDiagnostic | Out-Null
    Assert-Workflow (Test-Path -LiteralPath $published) 'Repeated cleanup is safe'

    $locked = [IO.File]::Open($published, 'Open', 'Read', 'None')
    try {
        $failed = $false
        try { & $builder | Out-Null } catch {
            if ($_.Exception.Message -notmatch 'being used by another process') { throw }
            $failed = $true
        }
        Assert-Workflow $failed 'Running/locked executable blocks publication explicitly'
    } finally {
        $locked.Dispose()
    }
    Assert-Workflow ([IO.File]::ReadAllText($published) -eq 'second build' -and
        @(Get-ChildItem (Split-Path $published -Parent) -File).Count -eq 1) 'Failed publication retains release and removes staging'
    Write-Output "PASS: $checks build workflow checks (isolated mock Cargo; no application launched)."
} finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
