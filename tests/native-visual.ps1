param(
    [string]$Executable,
    [Parameter(Mandatory = $true)][string]$EvidenceDirectory,
    [string]$Prefix = 'ledger',
    [switch]$AssertStable
)

$ErrorActionPreference = 'Stop'
if (-not $Executable) { $Executable = Join-Path $PSScriptRoot '..\target\release\tomas-commander.exe' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class LedgerDesktop {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint from, uint to, bool attach);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr window, int x, int y, int width, int height, bool repaint);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
'@
[LedgerDesktop]::SetProcessDPIAware() | Out-Null

$root = Join-Path $EvidenceDirectory ("visual-fixture-" + [Guid]::NewGuid().ToString('N'))
$owned = [Collections.Generic.List[string]]::new()
$captures = [Collections.Generic.List[object]]::new()
$process = $null
$cleaned = $false
$completed = $false
$stableGeometry = $false

function Nodes {
    $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
    return $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
}

function Find([string]$Name) {
    $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
    $element = $window.FindFirst([Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $Name))
    if ($null -ne $element) { return $element }
    return @((Nodes) | Where-Object { $_.Current.Name.Trim() -eq $Name } | Select-Object -First 1)[0]
}

function Foreground {
    if ($process.HasExited) { throw 'Test app exited unexpectedly' }
    [uint32]$foregroundProcess = 0
    $foregroundThread = [LedgerDesktop]::GetWindowThreadProcessId([LedgerDesktop]::GetForegroundWindow(), [ref]$foregroundProcess)
    $thread = [LedgerDesktop]::GetCurrentThreadId()
    $attached = $false
    try {
        if ($foregroundThread -ne 0 -and $foregroundThread -ne $thread) {
            $attached = [LedgerDesktop]::AttachThreadInput($thread, $foregroundThread, $true)
        }
        [LedgerDesktop]::ShowWindow($process.MainWindowHandle, 9) | Out-Null
        [LedgerDesktop]::BringWindowToTop($process.MainWindowHandle) | Out-Null
        [LedgerDesktop]::SetForegroundWindow($process.MainWindowHandle) | Out-Null
    } finally {
        if ($attached) { [LedgerDesktop]::AttachThreadInput($thread, $foregroundThread, $false) | Out-Null }
    }
    Start-Sleep -Milliseconds 100
    if ([LedgerDesktop]::GetForegroundWindow() -ne $process.MainWindowHandle) {
        throw 'Cannot safely send input/capture: test app is not foreground'
    }
}

function Keys([string]$Text, [int]$Delay = 150) {
    Foreground
    [Windows.Forms.SendKeys]::SendWait($Text)
    Start-Sleep -Milliseconds $Delay
}

function Click([string]$Name) {
    $element = Find $Name
    if ($null -eq $element) { throw "Test app control not found: $Name" }
    Foreground
    $rect = $element.Current.BoundingRectangle
    [LedgerDesktop]::SetCursorPos([int]($rect.X + $rect.Width / 2), [int]($rect.Y + $rect.Height / 2)) | Out-Null
    Start-Sleep -Milliseconds 100
    [LedgerDesktop]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [LedgerDesktop]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
}

function Checked-Count {
    $count = 0
    foreach ($element in (Nodes)) {
        if ($element.Current.ControlType -eq [Windows.Automation.ControlType]::CheckBox) {
            $pattern = $element.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
            if ($pattern.Current.ToggleState -eq [Windows.Automation.ToggleState]::On) { $count++ }
        }
    }
    return $count
}

function Capture([string]$Name) {
    Foreground
    $rect = [LedgerDesktop+Rect]::new()
    $origin = [LedgerDesktop+Point]::new()
    if (-not [LedgerDesktop]::GetClientRect($process.MainWindowHandle, [ref]$rect) -or
        -not [LedgerDesktop]::ClientToScreen($process.MainWindowHandle, [ref]$origin)) {
        throw 'Cannot resolve test app client rectangle'
    }
    $file = Join-Path $EvidenceDirectory "$Prefix-$Name.png"
    $bitmap = [Drawing.Bitmap]::new($rect.Right, $rect.Bottom)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($origin.X, $origin.Y, 0, 0, $bitmap.Size)
        $bitmap.Save($file, [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    $rows = @((Nodes) | Where-Object { $_.Current.Name -match '^\[(\.|/|link)\] ' } | ForEach-Object {
        $r = $_.Current.BoundingRectangle
        [pscustomobject]@{ name = $_.Current.Name; x = $r.X; y = $r.Y; width = $r.Width; height = $r.Height }
    })
    $captures.Add([pscustomobject]@{ name = $Name; image = $file; rows = $rows })
}

try {
    [IO.Directory]::CreateDirectory($root) | Out-Null
    $owned.Add($root)
    $folder = Join-Path $root '00 Projects'
    [IO.Directory]::CreateDirectory($folder) | Out-Null
    $owned.Add($folder)
    for ($i = 0; $i -lt 240; $i++) {
        $suffix = if ($i % 7 -eq 0) { 'a deliberately long filename that must never push metadata out of alignment.md' } else { 'notes.txt' }
        $path = Join-Path $root ('{0:D3}-{1}' -f $i, $suffix)
        [IO.File]::WriteAllText($path, ('fixture ' * ($i + 1)))
        [IO.File]::SetLastWriteTime($path, [DateTime]::Now.AddDays(-$i))
        $owned.Add($path)
    }
    $czechName = '241-Czech-' + [char]0x010C + 'esk' + [char]0x00E1 + '-' + [char]0x017E + 'lu' + [char]0x0165 + 'ou' + [char]0x010D + 'k' + [char]0x00FD + '.txt'
    $czechPath = Join-Path $root $czechName
    [IO.File]::WriteAllText($czechPath, 'Unicode glyph fixture')
    $owned.Add($czechPath)
    $settings = Join-Path $root 'preferences.txt'
    $owned.Add($settings)
    $process = Start-Process -FilePath (Resolve-Path -LiteralPath $Executable).Path `
        -ArgumentList @('--fixture-root', "`"$root`"", '--settings', "`"$settings`"") -PassThru
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        $process.Refresh()
        if ($process.HasExited) { throw "Test app exited: $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and $null -ne (Find 'Light mode')) { break }
        Start-Sleep -Milliseconds 100
    }
    $screen = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    [LedgerDesktop]::MoveWindow($process.MainWindowHandle, $screen.X + 20, $screen.Y + 20, 1500, 960, $true) | Out-Null
    Start-Sleep -Milliseconds 600
    Click '[/] 00 Projects'
    Keys '{HOME}'
    Capture 'dark-home'
    for ($i = 1; $i -le 5; $i++) { Keys '{DOWN}'; Capture "down-$i" }
    for ($i = 0; $i -lt 45; $i++) { Keys '{DOWN}' 20 }
    Capture 'scrolled'
    Keys '{END}'
    Capture 'end'
    for ($i = 0; $i -lt 8; $i++) { Keys '{UP}' 20 }
    Capture 'up'
    Keys '{TAB}'
    if ($AssertStable -and @((Nodes) | Where-Object { $_.Current.HasKeyboardFocus -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit }).Count -gt 0) {
        throw 'Pane Tab unexpectedly focused a text editor'
    }
    for ($i = 0; $i -lt 4; $i++) { Keys '{DOWN}' }
    Capture 'right-pane'
    Keys '{TAB}'
    if ($AssertStable -and @((Nodes) | Where-Object { $_.Current.HasKeyboardFocus -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit }).Count -gt 0) {
        throw 'Return pane Tab unexpectedly focused a text editor'
    }
    Keys '{HOME}'
    for ($i = 0; $i -lt 3; $i++) { Keys '+{DOWN}' }
    Capture 'range-selection'
    if ($AssertStable -and (Checked-Count) -ne 4) { throw 'Shift+Down did not select the four-item range' }
    Keys '{ESC}'
    Keys ' '
    Capture 'space-selection'
    if ($AssertStable -and (Checked-Count) -ne 1) { throw 'Space did not select exactly the current file' }
    Keys '^f'
    Keys '037'
    Capture 'filtered'
    if ($AssertStable -and $null -eq (Find '[.] 037-notes.txt')) { throw 'Keyboard filter did not reveal the real matching file' }
    Keys '^a'
    Keys 'Czech'
    Capture 'czech-filename'
    if ($AssertStable -and $null -eq (Find "[.] $czechName")) { throw 'Unicode fixture did not appear in the filtered listing' }
    Keys '^a{BACKSPACE}{ESC}'
    Click '[/] 00 Projects'
    Keys '^+p'
    Capture 'palette'
    Keys 'toggle'
    Keys '{ENTER}'
    if ($null -eq (Find 'Dark mode')) {
        if ($AssertStable) { throw 'Palette Enter did not execute the selected theme command' }
        Capture 'palette-enter-failed'
        Click 'Toggle light and dark theme'
    }
    Capture 'light'
    Keys '^+p'
    Keys 'cycle'
    Keys '{DOWN}'
    Keys '{DOWN}'
    Keys '{UP}'
    Keys '{DOWN}'
    Capture 'palette-arrow-choice'
    Keys '{ENTER}'
    if ($AssertStable -and $null -eq (Find 'Accent: Red-orange')) { throw 'Palette arrows/Enter did not execute the chosen accent command' }
    foreach ($accent in @('Red-orange', 'Green', 'Yellow')) { Click "Accent: $accent" }
    foreach ($accent in @('Blue', 'Red-orange', 'Green', 'Yellow')) {
        Click "Accent: $accent"
        Capture "light-accent-after-$accent"
    }
    Click 'Dark mode'
    Capture 'dark-restored'
    foreach ($accent in @('Blue', 'Red-orange', 'Green', 'Yellow')) {
        Click "Accent: $accent"
        Capture "dark-accent-after-$accent"
    }
    [LedgerDesktop]::MoveWindow($process.MainWindowHandle, $screen.X + 20, $screen.Y + 20, 1340, 860, $true) | Out-Null
    Start-Sleep -Milliseconds 300
    Capture 'narrow'
    Keys '^+{LEFT}'
    Capture 'pane-resize'
    [LedgerDesktop]::MoveWindow($process.MainWindowHandle, $screen.X + 20, $screen.Y + 20, 1500, 960, $true) | Out-Null
    Start-Sleep -Milliseconds 300
    Click '[/] 00 Projects'
    Foreground
    [LedgerDesktop]::mouse_event(0x800, 0, 0, [uint32]4294966576, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    Capture 'wheel'
    Keys '{HOME}'
    Capture 'home-restored'
    if ($AssertStable) {
        $homeRows = $captures[0].rows
        for ($i = 1; $i -le 5; $i++) {
            $current = $captures[$i].rows
            if ($current.Count -ne $homeRows.Count) { throw "Arrow navigation changed virtualized row count at step $i" }
            for ($r = 0; $r -lt $homeRows.Count; $r++) {
                if ($current[$r].name -ne $homeRows[$r].name -or
                    [Math]::Abs($current[$r].x - $homeRows[$r].x) -gt 0.1 -or
                    [Math]::Abs($current[$r].y - $homeRows[$r].y) -gt 0.1 -or
                    [Math]::Abs($current[$r].width - $homeRows[$r].width) -gt 0.1 -or
                    [Math]::Abs($current[$r].height - $homeRows[$r].height) -gt 0.1) {
                    throw "Arrow navigation moved/reshaped a stationary visible row at step $i / row $r"
                }
                $stableGeometry = $true
            }
        }
    }
    $process.CloseMainWindow() | Out-Null
    if (-not $process.WaitForExit(10000)) { throw 'Test app did not close normally' }
    $completed = $true
} finally {
    if ($null -ne $process -and -not $process.HasExited) {
        $process.CloseMainWindow() | Out-Null
        if (-not $process.WaitForExit(5000)) { Stop-Process -Id $process.Id }
    }
    foreach ($path in ($owned | Sort-Object Length -Descending)) {
        if ([IO.File]::Exists($path)) { [IO.File]::Delete($path) }
        elseif ([IO.Directory]::Exists($path)) { [IO.Directory]::Delete($path, $false) }
    }
    $cleaned = -not [IO.Directory]::Exists($root)
    [pscustomobject]@{
        timestamp = [DateTimeOffset]::Now.ToString('o')
        captures = $captures
        syntheticFixtureCleaned = $cleaned
        completed = $completed
        input = 'Foreground-guarded Windows SendKeys and mouse input; UIA used only to locate/read real controls'
        pixels = 'App client rectangle only; screenshots require visual inspection, not an automatic visual PASS'
        stableGeometryAsserted = $stableGeometry
        physicalKeyboardAndDpiMatrix = 'NOT_RUN'
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory "$Prefix-visual.json") -Encoding UTF8
}
Write-Output "$($captures.Count) app-only captures; fixture cleaned: $cleaned"
