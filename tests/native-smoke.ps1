param(
    [string]$Executable,
    [Parameter(Mandatory = $true)][string]$EvidenceDirectory
)

$ErrorActionPreference = 'Stop'
if (-not $Executable) { $Executable = Join-Path $PSScriptRoot '..\target\release\tomas-commander.exe' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root = Join-Path $EvidenceDirectory ("native-smoke-" + [Guid]::NewGuid().ToString('N'))
$owned = [Collections.Generic.List[string]]::new()
$checks = [Collections.Generic.List[object]]::new()
$process = $null
$window = $null
$measurements = $null
$fixtureCleanup = $false

function Assert-Check([bool]$Condition, [string]$Name) {
    if (-not $Condition) { throw "Native smoke check failed: $Name" }
    $checks.Add([pscustomobject]@{ name = $Name; result = 'PASS' })
}

function Elements {
    $process.Refresh()
    $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
    return $window.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.Condition]::TrueCondition
    )
}

function Button([string]$Name) {
    $process.Refresh()
    $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
    $condition = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::NameProperty, $Name
    )
    return $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}

function Invoke-Button([string]$Name) {
    $element = Button $Name
    if ($null -eq $element) { throw "Actual native button not found: $Name" }
    $pattern = $null
    if ($element.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
        $pattern.Invoke()
    } elseif ($element.TryGetCurrentPattern([Windows.Automation.TogglePattern]::Pattern, [ref]$pattern)) {
        $pattern.Toggle()
    } else { throw "Actual native button has no supported activation pattern: $Name" }
    Start-Sleep -Milliseconds 200
}

function Wait-Button([string]$Name) {
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        $process.Refresh()
        if ($process.HasExited) { throw "Native process exited with $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and $null -ne (Button $Name)) { return }
        Start-Sleep -Milliseconds 100
    }
    throw "Native button did not appear: $Name"
}

function Set-Edit([int]$Index, [string]$Value) {
    $edits = @((Elements) | Where-Object {
        $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit
    })
    if ($edits.Count -le $Index) { throw "Native edit field $Index not found ($($edits.Count) fields)" }
    $edits[$Index].SetFocus()
    Start-Sleep -Milliseconds 200
    $pattern = $edits[$Index].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
    $pattern.SetValue($Value)
    Start-Sleep -Milliseconds 200
    Assert-Check ($pattern.Current.Value -eq $Value) "Native edit $Index accepts exact text"
}

try {
    [IO.Directory]::CreateDirectory($root) | Out-Null
    $owned.Add($root)
    $destination = Join-Path $root 'Destination with spaces'
    [IO.Directory]::CreateDirectory($destination) | Out-Null
    $owned.Add($destination)
    $empty = Join-Path $root 'Empty'
    [IO.Directory]::CreateDirectory($empty) | Out-Null
    $owned.Add($empty)
    $source = Join-Path $root 'source.txt'
    [IO.File]::WriteAllText($source, 'real native UI fixture')
    $owned.Add($source)
    $settings = Join-Path $root 'preferences.txt'
    $owned.Add($settings)
    $copied = Join-Path $destination 'source.txt'
    $owned.Add($copied)
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath (Resolve-Path -LiteralPath $Executable).Path `
        -ArgumentList @('--fixture-root', "`"$root`"", '--settings', "`"$settings`"") -PassThru
    for ($attempt = 0; $attempt -lt 200; $attempt++) {
        $process.Refresh()
        if ($process.HasExited) { throw "Native process exited with $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 50
    }
    Assert-Check ($process.MainWindowHandle -ne [IntPtr]::Zero) 'Real standalone Windows window'
    $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
    Wait-Button 'Light mode'
    Wait-Button '[.] source.txt'
    $startup = $clock.Elapsed.TotalMilliseconds
    Assert-Check ($null -ne (Button 'Copy')) 'AccessKit exposes actual file commands'
    Assert-Check (@((Elements) | Where-Object {
        $_.Current.Name -eq 'FIXTURE BOUNDARY ENFORCED'
    }).Count -gt 0) 'Native fixture-boundary indicator'
    Set-Edit 0 $empty
    $go = @((Elements) | Where-Object { $_.Current.Name -eq 'Go' })
    $go[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Wait-Button '[/] .. / Parent folder'
    Assert-Check (@((Elements) | Where-Object { $_.Current.Name -eq '0 visible / 0 selected' }).Count -gt 0) 'Empty folder exposes parent navigation without counting it as an item'
    Set-Edit 0 $root
    $go = @((Elements) | Where-Object { $_.Current.Name -eq 'Go' })
    $go[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Wait-Button '[/] .. / Parent unavailable'
    Start-Sleep -Seconds 2
    $process.Refresh()
    $cpuBefore = $process.TotalProcessorTime.TotalMilliseconds
    Start-Sleep -Seconds 5
    $process.Refresh()
    $measurements = [pscustomobject]@{
        startupToAccessibleControlsMs = [Math]::Round($startup, 1)
        idlePrivateMiB = [Math]::Round($process.PrivateMemorySize64 / 1MB, 1)
        idleWorkingSetMiB = [Math]::Round($process.WorkingSet64 / 1MB, 1)
        idleCpuMillisecondsOverFiveSeconds = [Math]::Round($process.TotalProcessorTime.TotalMilliseconds - $cpuBefore, 1)
        executableBytes = (Get-Item -LiteralPath $Executable).Length
        conditions = 'One warm local synthetic-folder trial; UI Automation activates accessibility; not acceptance thresholds'
    }
    foreach ($theme in @('dark', 'light')) {
        if ($theme -eq 'light') { Invoke-Button 'Light mode'; Wait-Button 'Dark mode' }
        foreach ($accent in @('Blue', 'Red-orange', 'Green', 'Yellow')) {
            Wait-Button "Accent: $accent"
            Assert-Check ($null -ne (Button "Accent: $accent")) "$theme theme / $accent native appearance control"
            Invoke-Button "Accent: $accent"
        }
    }
    Set-Edit 2 $destination
    $go = @((Elements) | Where-Object { $_.Current.Name -eq 'Go' })
    Assert-Check ($go.Count -eq 2) 'Both actual pane navigation controls'
    $go[1].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 400
    Set-Edit 1 'source.txt'
    Wait-Button '[.] source.txt'
    Assert-Check ($null -ne (Button '[.] source.txt')) 'Real file listing and name filtering'
    Assert-Check ($null -ne (Button '[/] .. / Parent unavailable')) 'Filtered fixture root retains the unavailable navigation row'
    (Button 'Select source.txt').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 200
    Invoke-Button 'Copy'
    Wait-Button 'Confirm Copy'
    Assert-Check (-not [IO.File]::Exists($copied)) 'Copy has no effects before native approval'
    Invoke-Button 'Confirm Copy'
    for ($attempt = 0; $attempt -lt 100 -and -not [IO.File]::Exists($copied); $attempt++) {
        Start-Sleep -Milliseconds 100
    }
    Assert-Check ([IO.File]::ReadAllText($copied) -eq [IO.File]::ReadAllText($source)) 'Native UI copy preserves actual bytes and source'
    Wait-Button 'Copy'
    Start-Sleep -Milliseconds 400
    Invoke-Button 'Commands'
    Wait-Button 'Command palette / Ctrl Shift P'
    Assert-Check ($null -ne (Button 'Copy to opposite pane    Ctrl Shift C')) 'Real deterministic command palette'
    Invoke-Button 'Close / Escape'
    Invoke-Button 'Move'
    Wait-Button 'Acknowledge / Escape'
    Assert-Check ([IO.File]::ReadAllText($source) -eq 'real native UI fixture') 'Move conflict refuses overwriting copied destination'
    Invoke-Button 'Acknowledge / Escape'
    $pins = @((Elements) | Where-Object { $_.Current.Name -eq '+ pin' })
    $pins[1].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Invoke-Button 'Dark mode'
    $process.CloseMainWindow() | Out-Null
    if (-not $process.WaitForExit(10000)) { throw 'Native window did not close normally' }
    Assert-Check ($process.ExitCode -eq 0) 'Native application exits normally'
    Assert-Check ([IO.File]::ReadAllText($settings).Contains('dark=true')) 'Immediate close flushes the latest appearance preference to the real settings file'
    Assert-Check ([IO.File]::ReadAllText($settings).Contains('Destination with spaces')) 'Actual favorite persisted before close'
    $process = Start-Process -FilePath (Resolve-Path -LiteralPath $Executable).Path `
        -ArgumentList @('--fixture-root', "`"$root`"", '--settings', "`"$settings`"") -PassThru
    Wait-Button '/ Destination with spaces '
    Invoke-Button '/ Destination with spaces '
    Start-Sleep -Milliseconds 500
    $edits = @((Elements) | Where-Object { $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit })
    $value = $edits[0].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
    Assert-Check ($value.EndsWith('Destination with spaces')) 'Reloaded favorite navigates the actual pane after app restart'
    $process.CloseMainWindow() | Out-Null
    if (-not $process.WaitForExit(10000)) { throw 'Restarted native window did not close normally' }
}
finally {
    if ($null -ne $window -and $null -ne $process -and -not $process.HasExited -and $process.MainWindowHandle -ne [IntPtr]::Zero) {
        $nodes=@((Elements) | Select-Object -First 150 | ForEach-Object {
            $valuePattern=$null
            $value=$null
            if ($_.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern,[ref]$valuePattern)) { $value=$valuePattern.Current.Value }
            [pscustomobject]@{ name = $_.Current.Name; type = $_.Current.ControlType.ProgrammaticName; value=$value }
        })
        [pscustomobject]@{windowTitle=$process.MainWindowTitle;handle=$process.MainWindowHandle.ToInt64();nodes=$nodes} |
            ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'native-accessibility-tree.json') -Encoding UTF8
    }
    if ($null -ne $process -and -not $process.HasExited) {
        $process.CloseMainWindow() | Out-Null
        if (-not $process.WaitForExit(5000)) { Stop-Process -Id $process.Id -ErrorAction Stop }
    }
    foreach ($path in ($owned | Sort-Object Length -Descending -Unique)) {
        if ([IO.File]::Exists($path)) { [IO.File]::Delete($path) }
        elseif ([IO.Directory]::Exists($path)) { [IO.Directory]::Delete($path, $false) }
    }
    $fixtureCleanup = -not [IO.Directory]::Exists($root)
    [pscustomobject]@{
        timestamp = [DateTimeOffset]::Now.ToString('o')
        checks = $checks
        measurements = $measurements
        syntheticFixtureCleaned = $fixtureCleanup
        physicalCzUsKeyboardVerification = 'NOT_RUN'
        visualPixelVerification = 'NOT_RUN'
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'native-ledger-smoke.json') -Encoding UTF8
}
$checks | Format-Table -AutoSize
$measurements | Format-List
