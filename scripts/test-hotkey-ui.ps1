[CmdletBinding()]
param(
    [string]$ExecutablePath,
    [string]$FixtureChromePath,
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
if ([string]::IsNullOrWhiteSpace($FixtureChromePath)) {
    $FixtureChromePath = @(
        "D:\Google\Chrome\Application\chrome.exe",
        "D:\Program Files\Google\Chrome\Application\chrome.exe",
        "C:\Program Files\Google\Chrome\Application\chrome.exe",
        "C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        (Join-Path $env:LOCALAPPDATA "Google\Chrome\Application\chrome.exe")
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if ([string]::IsNullOrWhiteSpace($FixtureChromePath) -or -not [IO.Path]::IsPathRooted($FixtureChromePath)) {
    throw "FixtureChromePath must identify an installed Chrome by explicit absolute path."
}
$fixtureChrome = (Resolve-Path -LiteralPath $FixtureChromePath).Path
if (-not [IO.Path]::GetFileName($fixtureChrome).Equals("chrome.exe", [StringComparison]::OrdinalIgnoreCase)) {
    throw "FixtureChromePath must name chrome.exe."
}
New-Item -ItemType Directory -Path $dataRoot -Force | Out-Null

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @"
using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

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

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr window, StringBuilder windowText, int maxCount);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);

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

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool OpenClipboard(IntPtr owner);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool EmptyClipboard();

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool CloseClipboard();

    [DllImport("user32.dll")]
    public static extern uint GetClipboardSequenceNumber();

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

    public static IntPtr FindVisibleWindowByTitleSubstring(string expectedTitle) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr _) {
            if (!IsWindowVisible(window)) {
                return true;
            }
            StringBuilder title = new StringBuilder(512);
            GetWindowText(window, title, title.Capacity);
            if (title.ToString().IndexOf(expectedTitle, StringComparison.OrdinalIgnoreCase) >= 0) {
                found = window;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static bool ClearClipboard() {
        for (int attempt = 0; attempt < 20; attempt++) {
            if (OpenClipboard(IntPtr.Zero)) {
                bool emptied = EmptyClipboard();
                CloseClipboard();
                return emptied;
            }
            Thread.Sleep(25);
        }
        return false;
    }
}

public sealed class AskBridgeHotkeyFixtureServer : IDisposable {
    private readonly TcpListener listener;
    private readonly Thread thread;
    private volatile bool stopping;

    public AskBridgeHotkeyFixtureServer() {
        listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        Port = ((IPEndPoint)listener.LocalEndpoint).Port;
        Title = "AskBridge Local Hotkey Fixture " + Port;
        thread = new Thread(Run);
        thread.IsBackground = true;
        thread.Start();
    }

    public int Port { get; private set; }
    public string Title { get; private set; }

    private void Run() {
        while (!stopping) {
            try {
                using (TcpClient client = listener.AcceptTcpClient()) {
                    client.ReceiveTimeout = 1000;
                    client.SendTimeout = 1000;
                    using (NetworkStream stream = client.GetStream()) {
                        byte[] request = new byte[4096];
                        try { stream.Read(request, 0, request.Length); } catch (IOException) { }
                        byte[] body = Encoding.UTF8.GetBytes(Page.Replace("__FIXTURE_TITLE__", Title));
                        byte[] header = Encoding.ASCII.GetBytes(
                            "HTTP/1.1 200 OK\r\n" +
                            "Content-Type: text/html; charset=utf-8\r\n" +
                            "Cache-Control: no-store\r\n" +
                            "Content-Security-Policy: default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'\r\n" +
                            "Content-Length: " + body.Length + "\r\n" +
                            "Connection: close\r\n\r\n");
                        stream.Write(header, 0, header.Length);
                        stream.Write(body, 0, body.Length);
                    }
                }
            } catch (SocketException) when (stopping) {
                return;
            } catch (ObjectDisposedException) when (stopping) {
                return;
            } catch {
                if (stopping) {
                    return;
                }
            }
        }
    }

    public void Dispose() {
        stopping = true;
        listener.Stop();
        thread.Join(2000);
    }

    private const string Page = @"<!doctype html>
<meta charset='utf-8'>
<title>__FIXTURE_TITLE__</title>
<style>
  body { margin: 0; background: #f4f5f7; font: 16px sans-serif; }
  main { position: fixed; left: 40px; right: 40px; bottom: 40px; min-height: 220px; }
  textarea { box-sizing: border-box; width: 100%; height: 120px; font: inherit; }
  [role='group'] { display: flex; width: 80px; height: 64px; margin-bottom: 12px; border: 1px solid #777; }
  img { width: 48px; height: 48px; margin: 8px; }
</style>
<main id='composer'>
  <textarea id='editor' aria-label='Message'></textarea>
</main>
<script>
  document.getElementById('editor').addEventListener('paste', async event => {
    const item = Array.from(event.clipboardData?.items || []).find(
      candidate => candidate.kind === 'file' && candidate.type.startsWith('image/')
    );
    if (!item) return;
    event.preventDefault();
    const file = item.getAsFile();
    if (!file) return;
    const bitmap = await createImageBitmap(file);
    if (bitmap.width < 1 || bitmap.height < 1) return;
    const dimensions = bitmap.width + 'x' + bitmap.height;
    bitmap.close();
    const group = document.createElement('div');
    group.setAttribute('role', 'group');
    group.setAttribute('aria-label', 'Attached image');
    const image = document.createElement('img');
    image.alt = 'Attached screenshot';
    image.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==';
    group.appendChild(image);
    document.getElementById('composer').insertBefore(group, document.getElementById('editor'));
    document.title = '__FIXTURE_TITLE__ | IMAGE_RECEIVED:' + dimensions;
  });
</script>";
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

function Wait-WindowTitleContains {
    param([string]$Title, [int]$TimeoutSeconds = 10)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $window = [AskBridgeHotkeyNative]::FindVisibleWindowByTitleSubstring($Title)
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "A visible window containing title '$Title' was not found."
}

function Wait-LogPattern {
    param([string]$Path, [string]$Pattern, [int]$TimeoutSeconds = 15)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            $match = Select-String -LiteralPath $Path -Pattern $Pattern -Encoding UTF8 | Select-Object -Last 1
            if ($null -ne $match) { return $match.Line }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Log '$Path' did not match '$Pattern' before the timeout."
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

function Stop-FixtureChromeProcesses {
    param([string]$ProfilePath)

    $resolvedProfile = [IO.Path]::GetFullPath($ProfilePath).TrimEnd('\')
    $expectedPrefix = $dataRoot.TrimEnd('\') + '\'
    if (-not ($resolvedProfile + '\').StartsWith($expectedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Fixture Chrome profile must remain below the isolated acceptance root."
    }

    $processes = @(Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" -ErrorAction SilentlyContinue)
    $owned = [Collections.Generic.HashSet[int]]::new()
    foreach ($candidate in $processes) {
        if ($candidate.CommandLine -and
            $candidate.CommandLine.IndexOf("--user-data-dir", [StringComparison]::OrdinalIgnoreCase) -ge 0 -and
            $candidate.CommandLine.IndexOf($resolvedProfile, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            $owned.Add([int]$candidate.ProcessId) | Out-Null
        }
    }
    do {
        $added = $false
        foreach ($candidate in $processes) {
            if ($owned.Contains([int]$candidate.ParentProcessId) -and $owned.Add([int]$candidate.ProcessId)) {
                $added = $true
            }
        }
    } while ($added)
    $ids = @($owned)
    if ($ids.Count -gt 0) {
        Stop-Process -Id $ids -Force -ErrorAction SilentlyContinue
        Wait-Process -Id $ids -Timeout 5 -ErrorAction SilentlyContinue
    }
}

$previousDataEnvironment = $env:ASKBRIDGE_DATA_DIR
$process = $null
$fixtureServer = $null
$acceptanceClipboardSequence = [uint32]0
$fixtureChromeProfile = Join-Path $dataRoot "FixtureChromeProfile"
$fixtureShortcut = Join-Path $dataRoot "AskBridgeLocalFixture.lnk"
$originalCursor = New-Object AskBridgeHotkeyNative+Point
[AskBridgeHotkeyNative]::GetCursorPos([ref]$originalCursor) | Out-Null

try {
    $fixtureServer = [AskBridgeHotkeyFixtureServer]::new()
    $fixtureUrl = "http://127.0.0.1:$($fixtureServer.Port)/"
    $shortcutShell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shortcutShell.CreateShortcut($fixtureShortcut)
        $shortcut.TargetPath = $fixtureChrome
        $shortcut.WorkingDirectory = Split-Path -Parent $fixtureChrome
        $shortcut.Arguments = @(
            "--user-data-dir=`"$fixtureChromeProfile`""
            "--app=`"$fixtureUrl`""
            "--no-first-run"
            "--no-default-browser-check"
            "--disable-extensions"
            "--disable-background-networking"
            "--disable-component-update"
            "--disable-sync"
            "--metrics-recording-only"
            "--host-resolver-rules=`"MAP * ~NOTFOUND, EXCLUDE 127.0.0.1`""
        ) -join ' '
        $shortcut.Save()
    }
    finally {
        if ($null -ne $shortcutShell) {
            [Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcutShell) | Out-Null
        }
    }

    $fixtureProviderId = "askbridge-local-fixture"
    $fixtureConfig = [ordered]@{
        schema_version = 3
        default_provider_id = $fixtureProviderId
        general = [ordered]@{
            start_on_login = $false
            auto_submit = $false
            debug_logging = $false
        }
        browser = [ordered]@{
            target_preferences = [ordered]@{
                $fixtureProviderId = "desktop_pwa"
            }
            desktop_shortcuts = [ordered]@{
                $fixtureProviderId = $fixtureShortcut
            }
        }
        custom_providers = @(
            [ordered]@{
                id = $fixtureProviderId
                display_name = $fixtureServer.Title
                enabled = $true
                start_url = "https://fixture.askbridge.invalid/"
                url_patterns = @("https://fixture.askbridge.invalid/")
                is_custom = $true
                adapter_override = $null
            }
        )
    }
    $fixtureConfig | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $dataRoot "config.json") -Encoding UTF8

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

    $logPath = Join-Path $dataRoot "logs\askbridge.log"
    $textHandoffTimer = [Diagnostics.Stopwatch]::StartNew()
    Send-AppHotkey $mainWindow 0x102
    Wait-LogPattern $logPath 'page preparation completed.*attachment_prepared=false' | Out-Null
    $textHandoffTimer.Stop()
    Wait-AutomationWindow $promptTitle $false 1 | Out-Null

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
    Wait-Window "AskBridge.CaptureOverlay.Window.v1" | Out-Null
    Send-Key 0x0D
    Wait-LogPattern $logPath 'page preparation completed.*attachment_prepared=true' 45 | Out-Null
    Wait-WindowTitleContains ($fixtureServer.Title + " | IMAGE_RECEIVED:") 5 | Out-Null
    $acceptanceClipboardSequence = [AskBridgeHotkeyNative]::GetClipboardSequenceNumber()
    $capturePromptTimer.Stop()
    Wait-AutomationWindow $promptTitle $false 1 | Out-Null

    $cancelOverlayTimer = [Diagnostics.Stopwatch]::StartNew()
    Send-AppHotkey $mainWindow 0x101
    Wait-Window "AskBridge.CaptureOverlay.Window.v1" | Out-Null
    $cancelOverlayTimer.Stop()
    Send-Key 0x1B
    Wait-Window "AskBridge.CaptureOverlay.Window.v1" $false | Out-Null
    Wait-AutomationWindow $promptTitle $false | Out-Null

    if (-not (Test-Path -LiteralPath $logPath -PathType Leaf)) { throw "Structured log was not created." }
    Write-Host "Alt+W local handoff, Alt+Q capture-to-local-loopback, and Alt+Shift+Q cancel acceptance passed."
    Write-Host ("WM_HOTKEY-to-text-handoff latency: {0} ms" -f $textHandoffTimer.ElapsedMilliseconds)
    Write-Host ("WM_HOTKEY-to-capture-overlay latency: {0} ms" -f $captureOverlayTimer.ElapsedMilliseconds)
    Write-Host ("Mouse-up-to-verified-image-attachment latency: {0} ms" -f $capturePromptTimer.ElapsedMilliseconds)
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
    $main = if ($null -ne $process) {
        [AskBridgeHotkeyNative]::FindTopLevelWindowForProcess(
            [uint32]$process.Id,
            "AskBridge.Desktop.HiddenWindow.v1"
        )
    }
    else {
        [IntPtr]::Zero
    }
    if ($main -ne [IntPtr]::Zero) {
        [AskBridgeHotkeyNative]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    }
    if ($null -ne $process -and -not $process.HasExited) {
        Wait-Process -Id $process.Id -Timeout 5 -ErrorAction SilentlyContinue
    }
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
    Stop-FixtureChromeProcesses $fixtureChromeProfile
    if ($null -ne $fixtureServer) {
        $fixtureServer.Dispose()
    }
    if ($acceptanceClipboardSequence -ne 0 -and
        [AskBridgeHotkeyNative]::GetClipboardSequenceNumber() -eq $acceptanceClipboardSequence -and
        -not [AskBridgeHotkeyNative]::ClearClipboard()) {
        Write-Warning "The acceptance screenshot could not be cleared from the clipboard."
    }
    $env:ASKBRIDGE_DATA_DIR = $previousDataEnvironment
    if (Test-Path -LiteralPath $dataRoot) {
        for ($attempt = 0; $attempt -lt 40; $attempt++) {
            Remove-Item -LiteralPath $dataRoot -Recurse -Force -ErrorAction SilentlyContinue
            if (-not (Test-Path -LiteralPath $dataRoot)) { break }
            Start-Sleep -Milliseconds 100
        }
        if (Test-Path -LiteralPath $dataRoot) {
            throw "The isolated acceptance root remained locked after cleanup."
        }
    }
}
