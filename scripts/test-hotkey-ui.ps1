[CmdletBinding()]
param(
    [string]$ExecutablePath,
    [Parameter(Mandatory = $true)]
    [string]$AcceptanceDataRoot
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $scriptDirectory)).TrimEnd('\')
if ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
    $ExecutablePath = Join-Path $repoRoot "target\debug\askbridge.exe"
}
if (-not [IO.Path]::IsPathRooted($AcceptanceDataRoot)) {
    throw "AcceptanceDataRoot must be an explicit absolute path."
}
$dataRoot = [IO.Path]::GetFullPath($AcceptanceDataRoot).TrimEnd('\')
$allowedRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target")).TrimEnd('\') + '\'
if (-not $dataRoot.StartsWith($allowedRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "AcceptanceDataRoot must be a new child of the repository target directory."
}
if (Test-Path -LiteralPath $dataRoot) {
    throw "AcceptanceDataRoot already exists; refusing to overwrite it."
}
New-Item -ItemType Directory -Path $dataRoot -Force | Out-Null

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class AskBridgeHotkeyNative {
    private delegate bool EnumWindowsProc(IntPtr window, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X; public int Y; }

    [StructLayout(LayoutKind.Sequential)]
    public struct Input {
        public uint Type;
        public InputUnion Union;
    }

    [StructLayout(LayoutKind.Explicit)]
    public struct InputUnion {
        [FieldOffset(0)] public KeyboardInput Keyboard;
        [FieldOffset(0)] public MouseInput Mouse;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct KeyboardInput {
        public ushort VirtualKey;
        public ushort ScanCode;
        public uint Flags;
        public uint Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct MouseInput {
        public int X;
        public int Y;
        public uint MouseData;
        public uint Flags;
        public uint Time;
        public UIntPtr ExtraInfo;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr FindWindow(string className, string windowName);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder className, int maxCount);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool RegisterHotKey(IntPtr window, int id, uint modifiers, uint virtualKey);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool UnregisterHotKey(IntPtr window, int id);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern void keybd_event(byte virtualKey, byte scanCode, uint flags, UIntPtr extraInfo);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint SendInput(uint count, Input[] inputs, int size);

    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll")]
    public static extern bool GetCursorPos(out Point point);

    [DllImport("user32.dll")]
    public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);

    [DllImport("user32.dll")]
    public static extern int GetSystemMetrics(int index);

    public static IntPtr FindTopLevelWindowForProcess(uint expectedProcessId, string expectedClassName) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr _) {
            uint processId;
            GetWindowThreadProcessId(window, out processId);
            StringBuilder className = new StringBuilder(256);
            GetClassName(window, className, className.Capacity);
            if (processId == expectedProcessId && className.ToString() == expectedClassName) {
                found = window;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

$promptTitle = [Text.Encoding]::UTF8.GetString(
    [Convert]::FromBase64String("QXNrQnJpZGdlIOaPkOmXrg==")
)

function Wait-AutomationWindow {
    param([string]$Name, [bool]$Present = $true, [int]$TimeoutSeconds = 10)

    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty,
        $Name
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $window = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst(
            [System.Windows.Automation.TreeScope]::Children,
            $condition
        )
        if ($Present -and $null -ne $window) { return $window }
        if (-not $Present -and $null -eq $window) { return $null }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Automation window '$Name' did not reach present=$Present."
}

function Wait-Window {
    param([string]$ClassName, [bool]$Present = $true, [int]$TimeoutSeconds = 10)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $window = [AskBridgeHotkeyNative]::FindWindow($ClassName, $null)
        if ($Present -and $window -ne [IntPtr]::Zero) { return $window }
        if (-not $Present -and $window -eq [IntPtr]::Zero) { return [IntPtr]::Zero }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Window class '$ClassName' did not reach present=$Present."
}

function Send-Hotkey {
    param([byte]$VirtualKey, [bool]$Shift = $false)

    $keys = if ($Shift) { @(0x12, 0x10, $VirtualKey) } else { @(0x12, $VirtualKey) }
    $inputs = [Collections.Generic.List[AskBridgeHotkeyNative+Input]]::new()
    foreach ($key in $keys) {
        $input = New-Object AskBridgeHotkeyNative+Input
        $input.Type = 1
        $input.Union.Keyboard.VirtualKey = [uint16]$key
        $inputs.Add($input)
    }
    for ($index = $keys.Count - 1; $index -ge 0; $index--) {
        $input = New-Object AskBridgeHotkeyNative+Input
        $input.Type = 1
        $input.Union.Keyboard.VirtualKey = [uint16]$keys[$index]
        $input.Union.Keyboard.Flags = 2
        $inputs.Add($input)
    }
    $array = $inputs.ToArray()
    $sent = [AskBridgeHotkeyNative]::SendInput(
        [uint32]$array.Length,
        $array,
        [Runtime.InteropServices.Marshal]::SizeOf([type][AskBridgeHotkeyNative+Input])
    )
    if ($sent -ne $array.Length) { throw "SendInput did not inject the complete hotkey." }
}

function Send-Key {
    param([byte]$VirtualKey)

    [AskBridgeHotkeyNative]::keybd_event($VirtualKey, 0, 0, [UIntPtr]::Zero)
    [AskBridgeHotkeyNative]::keybd_event($VirtualKey, 0, 2, [UIntPtr]::Zero)
}

function Assert-HotkeyAlreadyRegistered {
    param([int]$TestId, [uint32]$Modifiers, [uint32]$VirtualKey)

    if ([AskBridgeHotkeyNative]::RegisterHotKey([IntPtr]::Zero, $TestId, $Modifiers, $VirtualKey)) {
        [AskBridgeHotkeyNative]::UnregisterHotKey([IntPtr]::Zero, $TestId) | Out-Null
        throw "Expected hotkey was not already registered by AskBridge."
    }
    $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if ($errorCode -ne 1409) { throw "Hotkey registration probe returned Win32 error $errorCode instead of 1409." }
}

function Send-AppHotkey {
    param([IntPtr]$Window, [int]$HotkeyId)

    if (-not [AskBridgeHotkeyNative]::PostMessage($Window, 0x0312, [IntPtr]$HotkeyId, [IntPtr]::Zero)) {
        throw "WM_HOTKEY could not be posted for ID $HotkeyId."
    }
}

$previousDataEnvironment = $env:ASKBRIDGE_DATA_DIR
$process = $null
$originalCursor = New-Object AskBridgeHotkeyNative+Point
[AskBridgeHotkeyNative]::GetCursorPos([ref]$originalCursor) | Out-Null

try {
    $env:ASKBRIDGE_DATA_DIR = $dataRoot
    $process = Start-Process -FilePath (Resolve-Path -LiteralPath $ExecutablePath).Path -PassThru
    Start-Sleep -Seconds 1
    $process.Refresh()
    if ($process.HasExited) { throw "AskBridge exited before hotkey acceptance began." }
    $mainWindow = [AskBridgeHotkeyNative]::FindTopLevelWindowForProcess(
        [uint32]$process.Id,
        "AskBridge.Desktop.HiddenWindow.v1"
    )
    if ($mainWindow -eq [IntPtr]::Zero) { throw "AskBridge hidden top-level window was not found by process ID." }
    Assert-HotkeyAlreadyRegistered 0x7F00 0x4001 0x51
    Assert-HotkeyAlreadyRegistered 0x7F01 0x4005 0x51
    Assert-HotkeyAlreadyRegistered 0x7F02 0x4001 0x57

    $textPromptTimer = [Diagnostics.Stopwatch]::StartNew()
    Send-AppHotkey $mainWindow 0x102
    $prompt = Wait-AutomationWindow $promptTitle
    $textPromptTimer.Stop()
    [AskBridgeHotkeyNative]::PostMessage(
        [IntPtr]$prompt.Current.NativeWindowHandle,
        0x0010,
        [IntPtr]::Zero,
        [IntPtr]::Zero
    ) | Out-Null
    Wait-AutomationWindow $promptTitle $false | Out-Null

    $captureOverlayTimer = [Diagnostics.Stopwatch]::StartNew()
    Send-AppHotkey $mainWindow 0x100
    Wait-Window "AskBridge.CaptureOverlay.Window.v1" | Out-Null
    $captureOverlayTimer.Stop()
    $virtualX = [AskBridgeHotkeyNative]::GetSystemMetrics(76)
    $virtualY = [AskBridgeHotkeyNative]::GetSystemMetrics(77)
    $virtualWidth = [AskBridgeHotkeyNative]::GetSystemMetrics(78)
    $virtualHeight = [AskBridgeHotkeyNative]::GetSystemMetrics(79)
    if ($virtualWidth -lt 240 -or $virtualHeight -lt 180) { throw "Virtual desktop is too small for acceptance drag." }
    $startX = $virtualX + [int]($virtualWidth / 2) - 60
    $startY = $virtualY + [int]($virtualHeight / 2) - 45
    $endX = $startX + 120
    $endY = $startY + 90
    [AskBridgeHotkeyNative]::SetCursorPos($startX, $startY) | Out-Null
    [AskBridgeHotkeyNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    for ($step = 1; $step -le 6; $step++) {
        [AskBridgeHotkeyNative]::SetCursorPos(
            $startX + [int](($endX - $startX) * $step / 6),
            $startY + [int](($endY - $startY) * $step / 6)
        ) | Out-Null
        Start-Sleep -Milliseconds 30
    }
    $capturePromptTimer = [Diagnostics.Stopwatch]::StartNew()
    [AskBridgeHotkeyNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    $prompt = Wait-AutomationWindow $promptTitle
    $capturePromptTimer.Stop()
    [AskBridgeHotkeyNative]::PostMessage(
        [IntPtr]$prompt.Current.NativeWindowHandle,
        0x0010,
        [IntPtr]::Zero,
        [IntPtr]::Zero
    ) | Out-Null
    Wait-AutomationWindow $promptTitle $false | Out-Null

    $cancelOverlayTimer = [Diagnostics.Stopwatch]::StartNew()
    Send-AppHotkey $mainWindow 0x101
    Wait-Window "AskBridge.CaptureOverlay.Window.v1" | Out-Null
    $cancelOverlayTimer.Stop()
    Send-Key 0x1B
    Wait-Window "AskBridge.CaptureOverlay.Window.v1" $false | Out-Null
    Wait-AutomationWindow $promptTitle $false | Out-Null

    $logPath = Join-Path $dataRoot "logs\askbridge.log"
    if (-not (Test-Path -LiteralPath $logPath -PathType Leaf)) { throw "Structured log was not created." }
    Write-Host "Alt+W prompt, Alt+Q capture-to-prompt, and Alt+Shift+Q cancel acceptance passed."
    Write-Host ("WM_HOTKEY-to-text-prompt latency: {0} ms" -f $textPromptTimer.ElapsedMilliseconds)
    Write-Host ("WM_HOTKEY-to-capture-overlay latency: {0} ms" -f $captureOverlayTimer.ElapsedMilliseconds)
    Write-Host ("Mouse-up-to-image-prompt latency: {0} ms" -f $capturePromptTimer.ElapsedMilliseconds)
    Write-Host ("WM_HOTKEY-to-cancel-overlay latency: {0} ms" -f $cancelOverlayTimer.ElapsedMilliseconds)
}
catch {
    $logPath = Join-Path $dataRoot "logs\askbridge.log"
    if (Test-Path -LiteralPath $logPath -PathType Leaf) {
        Write-Host "ISOLATED_LOG_BEGIN"
        Get-Content -LiteralPath $logPath -Encoding UTF8
        Write-Host "ISOLATED_LOG_END"
    }
    throw
}
finally {
    [AskBridgeHotkeyNative]::SetCursorPos($originalCursor.X, $originalCursor.Y) | Out-Null
    $main = [AskBridgeHotkeyNative]::FindWindow("AskBridge.Desktop.HiddenWindow.v1", $null)
    if ($main -ne [IntPtr]::Zero) {
        [AskBridgeHotkeyNative]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    }
    if ($null -ne $process -and -not $process.HasExited) {
        Wait-Process -Id $process.Id -Timeout 5 -ErrorAction SilentlyContinue
    }
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
    $env:ASKBRIDGE_DATA_DIR = $previousDataEnvironment
    if (Test-Path -LiteralPath $dataRoot) {
        Remove-Item -LiteralPath $dataRoot -Recurse -Force
    }
}
