[CmdletBinding()]
param(
    [string]$ExecutablePath,
    [Parameter(Mandatory = $true)]
    [string]$AcceptanceDataRoot,
    [Parameter(Mandatory = $true)]
    [string]$ScreenshotPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
    $scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
    $ExecutablePath = Join-Path (Split-Path -Parent $scriptDirectory) "target\debug\askbridge.exe"
}

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $scriptDirectory)).TrimEnd('\')
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target")).TrimEnd('\') + '\'
if (-not [IO.Path]::IsPathRooted($AcceptanceDataRoot) -or -not [IO.Path]::IsPathRooted($ScreenshotPath)) {
    throw "AcceptanceDataRoot and ScreenshotPath must be explicit absolute paths."
}
$dataRoot = [IO.Path]::GetFullPath($AcceptanceDataRoot).TrimEnd('\')
$screenshot = [IO.Path]::GetFullPath($ScreenshotPath)
if (-not $dataRoot.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "AcceptanceDataRoot must be a new child of the repository target directory."
}
$dataRootWithSeparator = $dataRoot + '\'
if (-not $screenshot.StartsWith($dataRootWithSeparator, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ScreenshotPath must be inside AcceptanceDataRoot."
}
if (Test-Path -LiteralPath $dataRoot) {
    throw "AcceptanceDataRoot already exists; refusing to overwrite it."
}
New-Item -ItemType Directory -Path $dataRoot -Force | Out-Null
New-Item -ItemType Directory -Path (Split-Path -Parent $screenshot) -Force | Out-Null

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class AskBridgeUiNative {
    private delegate bool EnumChildProc(IntPtr hWnd, IntPtr lParam);
    private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr SendMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool RegisterHotKey(IntPtr hWnd, int id, uint modifiers, uint virtualKey);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool UnregisterHotKey(IntPtr hWnd, int id);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageW", SetLastError = true)]
    public static extern IntPtr SendMessageText(IntPtr hWnd, uint msg, IntPtr wParam, string lParam);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool EnumChildWindows(IntPtr hWndParent, EnumChildProc callback, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern int GetDlgCtrlID(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr GetDlgItem(IntPtr hWnd, int controlId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool SetWindowText(IntPtr hWnd, string text);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int maxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int GetClassName(IntPtr hWnd, StringBuilder className, int maxCount);

    public static string ReadWindowText(IntPtr window) {
        StringBuilder text = new StringBuilder(8192);
        GetWindowText(window, text, text.Capacity);
        return text.ToString();
    }

    public static string ReadClassName(IntPtr window) {
        StringBuilder className = new StringBuilder(256);
        GetClassName(window, className, className.Capacity);
        return className.ToString();
    }

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

    public static IntPtr FindDescendantById(IntPtr parent, int controlId) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(parent, delegate(IntPtr child, IntPtr _) {
            if (GetDlgCtrlID(child) == controlId) {
                found = child;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

function Find-WindowByName {
    param([string]$Name, [int]$TimeoutSeconds = 10)

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
        if ($null -ne $window) { return $window }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Window '$Name' did not become visible."
}

function Find-ElementByName {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name
    )

    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty,
        $Name
    )
    $element = $Root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
    if ($null -eq $element) { throw "Visible UI element '$Name' was not found." }
    return $element
}

function Find-ElementByAutomationId {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$AutomationId
    )

    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::AutomationIdProperty,
        $AutomationId
    )
    $elements = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        if ($element.Current.ControlType -eq [System.Windows.Automation.ControlType]::Edit) {
            return $element
        }
    }
    $editCondition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit
    )
    $edits = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $editCondition)
    $details = for ($index = 0; $index -lt $edits.Count; $index++) {
        $edit = $edits.Item($index)
        "id=$($edit.Current.AutomationId),name=$($edit.Current.Name),offscreen=$($edit.Current.IsOffscreen),class=$($edit.Current.ClassName)"
    }
    throw "Edit control with automation ID '$AutomationId' was not found. Available edits: $($details -join '; ')"
}

function Click-Element {
    param([System.Windows.Automation.AutomationElement]$Element)

    $handle = [IntPtr]$Element.Current.NativeWindowHandle
    if ($handle -eq [IntPtr]::Zero) { throw "UI element '$($Element.Current.Name)' has no native handle." }
    [AskBridgeUiNative]::SendMessage($handle, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 150
}

function Set-ElementValue {
    param(
        [System.Windows.Automation.AutomationElement]$Element,
        [string]$Value
    )

    $patternObject = $null
    if (-not $Element.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$patternObject)) {
        throw "UI element '$($Element.Current.Name)' does not support ValuePattern."
    }
    ([System.Windows.Automation.ValuePattern]$patternObject).SetValue($Value)
}

function Set-NativeControlText {
    param(
        [System.Windows.Automation.AutomationElement]$Window,
        [int]$ParentControlId,
        [int]$ControlId,
        [string]$Value
    )

    $parentHandle = [AskBridgeUiNative]::GetDlgItem(
        [IntPtr]$Window.Current.NativeWindowHandle,
        $ParentControlId
    )
    if ($parentHandle -eq [IntPtr]::Zero) { throw "Native page ID '$ParentControlId' was not found." }
    $handle = [AskBridgeUiNative]::GetDlgItem($parentHandle, $ControlId)
    if ($handle -eq [IntPtr]::Zero) { throw "Native control ID '$ControlId' was not found." }
    $className = [AskBridgeUiNative]::ReadClassName($handle)
    if ($className -ne "Edit") {
        throw "Native page '$ParentControlId' control '$ControlId' has unexpected class '$className'."
    }
    $result = [AskBridgeUiNative]::SendMessageText($handle, 0x000C, [IntPtr]::Zero, $Value)
    if ($result -eq [IntPtr]::Zero) {
        throw "WM_SETTEXT failed for native control ID '$ControlId'."
    }
}

function Set-NativeComboSelection {
    param(
        [System.Windows.Automation.AutomationElement]$Window,
        [int]$ParentControlId,
        [int]$ControlId,
        [int]$SelectionIndex
    )

    $parentHandle = [AskBridgeUiNative]::GetDlgItem(
        [IntPtr]$Window.Current.NativeWindowHandle,
        $ParentControlId
    )
    if ($parentHandle -eq [IntPtr]::Zero) { throw "Native page ID '$ParentControlId' was not found." }
    $handle = [AskBridgeUiNative]::GetDlgItem($parentHandle, $ControlId)
    if ($handle -eq [IntPtr]::Zero) { throw "Native combo ID '$ControlId' was not found." }
    $result = [AskBridgeUiNative]::SendMessage(
        $handle,
        0x014E,
        [IntPtr]$SelectionIndex,
        [IntPtr]::Zero
    )
    if ($result.ToInt64() -eq -1) { throw "CB_SETCURSEL failed for native combo ID '$ControlId'." }
}

function Click-NativeControl {
    param(
        [System.Windows.Automation.AutomationElement]$Window,
        [int]$ParentControlId,
        [int]$ControlId
    )

    $parentHandle = if ($ParentControlId -eq 0) {
        [IntPtr]$Window.Current.NativeWindowHandle
    }
    else {
        [AskBridgeUiNative]::GetDlgItem(
            [IntPtr]$Window.Current.NativeWindowHandle,
            $ParentControlId
        )
    }
    if ($parentHandle -eq [IntPtr]::Zero) { throw "Native page ID '$ParentControlId' was not found." }
    $handle = [AskBridgeUiNative]::GetDlgItem($parentHandle, $ControlId)
    if ($handle -eq [IntPtr]::Zero) { throw "Native control ID '$ControlId' was not found." }
    [AskBridgeUiNative]::SendMessage($handle, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 150
}

function Test-HotkeyOccupied {
    param(
        [int]$TestId,
        [uint32]$Modifiers,
        [uint32]$VirtualKey
    )

    if ([AskBridgeUiNative]::RegisterHotKey([IntPtr]::Zero, $TestId, $Modifiers, $VirtualKey)) {
        [AskBridgeUiNative]::UnregisterHotKey([IntPtr]::Zero, $TestId) | Out-Null
        return $false
    }
    $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if ($errorCode -ne 1409) {
        throw "RegisterHotKey probe failed with unexpected Win32 error $errorCode."
    }
    return $true
}

function Capture-Window {
    param(
        [System.Windows.Automation.AutomationElement]$Window,
        [string]$Path
    )

    $bounds = $Window.Current.BoundingRectangle
    $width = [Math]::Max(1, [int][Math]::Ceiling($bounds.Width))
    $height = [Math]::Max(1, [int][Math]::Ceiling($bounds.Height))
    $bitmap = New-Object System.Drawing.Bitmap($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$bounds.Left, [int]$bounds.Top, 0, 0, $bitmap.Size)
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

function ConvertFrom-Utf8Base64 {
    param([string]$Value)

    return [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($Value))
}

$uiText = @{
    SettingsWindow = ConvertFrom-Utf8Base64 "QXNrQnJpZGdlIOiuvue9rg=="
    Shortcuts = ConvertFrom-Utf8Base64 "5b+r5o236ZSu"
    Providers = ConvertFrom-Utf8Base64 "5L6b5bqU5ZWG"
    Browser = ConvertFrom-Utf8Base64 "5rWP6KeI5Zmo"
    General = ConvertFrom-Utf8Base64 "5bi46KeE"
    DefaultProvider = ConvertFrom-Utf8Base64 "6buY6K6k5L6b5bqU5ZWG"
    Doubao = ConvertFrom-Utf8Base64 "6LGG5YyF"
    ChatGptPwa = ConvertFrom-Utf8Base64 "5qGM6Z2i572R6aG156uv77ya5aSN55So546w5pyJ55m75b2V77yM5L2G5oiq5Zu+6ZyA6KaB5omL5Yqo5LiK5Lyg"
    ChatGptDedicated = ConvertFrom-Utf8Base64 "QXNrQnJpZGdlIOS4k+eUqCBDaHJvbWXvvJrmlK/mjIHoh6rliqjkuIrkvKDlm77niYfvvIzpnIDopoHljZXni6znmbvlvZU="
    OpenBrowser = ConvertFrom-Utf8Base64 "5omT5byAIEFza0JyaWRnZSDmtY/op4jlmag="
    CheckConnection = ConvertFrom-Utf8Base64 "5qOA5p+l6L+e5o6l"
    OpenProviderLogin = ConvertFrom-Utf8Base64 "5omT5byA6buY6K6k5L6b5bqU5ZWG55m75b2V6aG16Z2i"
    StartOnLogin = ConvertFrom-Utf8Base64 "55m75b2VIFdpbmRvd3Mg5ZCO5ZCv5YqoIEFza0JyaWRnZe+8iOW9k+WJjeeUqOaIt++8jOS4jemcgOeuoeeQhuWRmOadg+mZkO+8iQ=="
    DebugLogging = ConvertFrom-Utf8Base64 "5ZCv55So6LCD6K+V5pel5b+X77yI56uL5Y2z55Sf5pWI77yb5pel5b+X5LuN5LiN6K6w5b2V6Zeu6aKY44CB5oiq5Zu+5oiW572R6aG15q2j5paH77yJ"
    Apply = ConvertFrom-Utf8Base64 "5bqU55So5pu05pS5"
}

$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$previousRunEntry = Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
$previousRunValue = if ($null -eq $previousRunEntry) { $null } else { [string]$previousRunEntry.AskBridge }
$previousDataEnvironment = $env:ASKBRIDGE_DATA_DIR
$primary = $null
$secondary = $null

try {
    $env:ASKBRIDGE_DATA_DIR = $dataRoot
    $primary = Start-Process -FilePath (Resolve-Path -LiteralPath $ExecutablePath).Path -PassThru
    Start-Sleep -Milliseconds 750
    $secondary = Start-Process -FilePath (Resolve-Path -LiteralPath $ExecutablePath).Path -PassThru
    if (-not $secondary.WaitForExit(5000)) {
        throw "The second AskBridge instance did not exit within five seconds."
    }
    $sameExecutableProcesses = @(Get-Process -Name askbridge -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.Equals((Resolve-Path -LiteralPath $ExecutablePath).Path, [StringComparison]::OrdinalIgnoreCase)
    })
    if ($sameExecutableProcesses.Count -ne 1 -or $sameExecutableProcesses[0].Id -ne $primary.Id) {
        throw "Single-instance acceptance expected only primary PID $($primary.Id), found $($sameExecutableProcesses.Id -join ',')."
    }

    $window = Find-WindowByName $uiText.SettingsWindow
    foreach ($tab in @($uiText.Shortcuts, $uiText.Providers, $uiText.Browser, $uiText.General)) {
        Find-ElementByName $window $tab | Out-Null
    }

    $configPath = Join-Path $dataRoot "config.json"
    Click-Element (Find-ElementByName $window $uiText.Shortcuts)
    Set-NativeControlText $window 2011 2101 "Ctrl+Alt+Shift+F12"
    Click-Element (Find-ElementByName $window $uiText.Apply)
    Start-Sleep -Milliseconds 300
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($config.hotkeys.capture_with_prompt.key -ne "F12" -or -not $config.hotkeys.capture_with_prompt.enabled) {
        throw "The real settings UI did not persist the replacement capture hotkey."
    }
    if (Test-HotkeyOccupied 28673 0x4001 0x51) {
        throw "Alt+Q remained registered after replacing it through the real settings UI."
    }
    if (-not (Test-HotkeyOccupied 28674 0x4007 0x7B)) {
        throw "The replacement Ctrl+Alt+Shift+F12 hotkey was not registered."
    }

    Click-NativeControl $window 2011 2102
    Click-Element (Find-ElementByName $window $uiText.Apply)
    Start-Sleep -Milliseconds 300
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($config.hotkeys.capture_with_prompt.enabled) {
        throw "The capture hotkey was not disabled through the real settings UI."
    }
    if (Test-HotkeyOccupied 28675 0x4007 0x7B) {
        throw "The disabled replacement hotkey remained registered."
    }

    Click-NativeControl $window 0 2052
    Start-Sleep -Milliseconds 300
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $config.hotkeys.capture_with_prompt.enabled -or $config.hotkeys.capture_with_prompt.key -ne "Q") {
        throw "Default hotkeys were not restored and persisted through the real settings UI."
    }
    if (-not (Test-HotkeyOccupied 28676 0x4001 0x51)) {
        throw "Alt+Q was not registered after restoring default hotkeys."
    }

    $reservedId = 28677
    if (-not [AskBridgeUiNative]::RegisterHotKey([IntPtr]::Zero, $reservedId, 0x4007, 0x7A)) {
        throw "Could not reserve Ctrl+Alt+Shift+F11 for the rollback acceptance probe."
    }
    try {
        Set-NativeControlText $window 2011 2101 "Ctrl+Alt+Shift+F11"
        Click-Element (Find-ElementByName $window $uiText.Apply)
        Start-Sleep -Milliseconds 300
        $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
        if (-not $config.hotkeys.capture_with_prompt.enabled -or $config.hotkeys.capture_with_prompt.key -ne "Q") {
            throw "A conflicting hotkey changed the persisted binding instead of rolling back."
        }
        if (-not (Test-HotkeyOccupied 28678 0x4001 0x51)) {
            throw "Alt+Q was lost after the conflicting replacement failed."
        }
    }
    finally {
        [AskBridgeUiNative]::UnregisterHotKey([IntPtr]::Zero, $reservedId) | Out-Null
    }

    Click-Element (Find-ElementByName $window $uiText.Providers)
    Find-ElementByName $window $uiText.DefaultProvider | Out-Null
    foreach ($provider in @("ChatGPT", "Gemini", "Claude", $uiText.Doubao)) {
        Find-ElementByName $window $provider | Out-Null
    }
    Set-NativeControlText $window 2012 2230 "example | Example | https://example.com/chat | https://example.com/"
    Set-NativeComboSelection $window 2012 2201 1
    Click-Element (Find-ElementByName $window $uiText.Apply)
    Start-Sleep -Milliseconds 300
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $exampleProvider = @($config.custom_providers | Where-Object { $_.id -eq "example" })
    if ($config.default_provider_id -ne "gemini" -or $exampleProvider.Count -ne 1) {
        throw "Provider settings were not persisted from the real UI."
    }
    if ($exampleProvider[0].start_url -ne "https://example.com/chat" -or
        @($exampleProvider[0].url_patterns).Count -ne 1 -or
        $exampleProvider[0].url_patterns[0] -ne "https://example.com/") {
        throw "Custom provider fields did not round-trip through the real UI."
    }
    Set-NativeControlText $window 2012 2230 ""
    Set-NativeComboSelection $window 2012 2201 0
    Click-Element (Find-ElementByName $window $uiText.Apply)
    Start-Sleep -Milliseconds 300
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($config.default_provider_id -ne "chatgpt" -or @($config.custom_providers).Count -ne 0) {
        throw "Provider settings were not restored through the real UI."
    }

    Click-Element (Find-ElementByName $window $uiText.Browser)
    foreach ($control in @($uiText.ChatGptPwa, $uiText.ChatGptDedicated, $uiText.OpenBrowser, $uiText.CheckConnection, $uiText.OpenProviderLogin)) {
        Find-ElementByName $window $control | Out-Null
    }

    Click-Element (Find-ElementByName $window $uiText.General)
    $startOnLogin = Find-ElementByName $window $uiText.StartOnLogin
    $debugLogging = Find-ElementByName $window $uiText.DebugLogging
    if ($null -eq $debugLogging) { throw "General setting is missing." }

    Set-NativeControlText $window 2014 2401 "AskBridge Phase 7 UI acceptance prompt."
    Click-Element $startOnLogin
    Click-Element $debugLogging
    Click-Element (Find-ElementByName $window $uiText.Apply)
    Start-Sleep -Milliseconds 300

    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $config.general.start_on_login -or
        -not $config.general.debug_logging -or
        $config.quick_prompt -ne "AskBridge Phase 7 UI acceptance prompt.") {
        throw "Settings were not persisted from the real UI. start_on_login=$($config.general.start_on_login), debug_logging=$($config.general.debug_logging), quick_prompt='$($config.quick_prompt)'"
    }
    $runValue = (Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction Stop).AskBridge
    if (([string]$runValue).IndexOf("askbridge.exe", [StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw "The current-user startup value does not target askbridge.exe."
    }

    Click-Element $startOnLogin
    Click-Element $debugLogging
    Click-Element (Find-ElementByName $window $uiText.Apply)
    Start-Sleep -Milliseconds 300
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($config.general.start_on_login -or $config.general.debug_logging) {
        throw "Startup or runtime debug logging was not disabled through the real UI."
    }
    if ($null -ne (Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue)) {
        throw "The current-user startup value remained after disabling it."
    }

    Click-Element $startOnLogin
    Click-Element (Find-ElementByName $window $uiText.Apply)
    Start-Sleep -Milliseconds 300
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $reEnabledRunValue = [string](Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction Stop).AskBridge
    if (-not $config.general.start_on_login -or
        $reEnabledRunValue.IndexOf("askbridge.exe", [StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw "Startup was not re-enabled before the foreign Run ownership probe."
    }

    $foreignRunValue = '"C:\Windows\System32\notepad.exe"'
    New-Item -Path $runKey -Force | Out-Null
    New-ItemProperty -Path $runKey -Name "AskBridge" -Value $foreignRunValue -PropertyType String -Force | Out-Null
    Click-Element $startOnLogin
    Click-Element (Find-ElementByName $window $uiText.Apply)
    Start-Sleep -Milliseconds 300
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $preservedForeignRunValue = [string](Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction Stop).AskBridge
    if ($config.general.start_on_login -or $preservedForeignRunValue -ne $foreignRunValue) {
        throw "Disabling startup removed or changed a Run value owned by another program."
    }

    Capture-Window $window $screenshot
    if (-not (Test-Path -LiteralPath $screenshot -PathType Leaf)) { throw "Settings screenshot was not created." }
    if (-not (Test-Path -LiteralPath (Join-Path $dataRoot "logs\askbridge.log") -PathType Leaf)) {
        throw "D-drive structured log was not created."
    }
    Write-Host "Settings UI, single-instance, hotkey transaction, provider round-trip, startup, runtime logging reload, and D-drive logging acceptance passed."
}
finally {
    if ($null -ne $secondary -and -not $secondary.HasExited) {
        Stop-Process -Id $secondary.Id -Force -ErrorAction SilentlyContinue
        Wait-Process -Id $secondary.Id -Timeout 5 -ErrorAction SilentlyContinue
    }
    if ($null -ne $primary -and -not $primary.HasExited) {
        $mainWindow = [AskBridgeUiNative]::FindTopLevelWindowForProcess(
            [uint32]$primary.Id,
            "AskBridge.Desktop.HiddenWindow.v1"
        )
        if ($mainWindow -ne [IntPtr]::Zero) {
            [AskBridgeUiNative]::PostMessage(
                $mainWindow,
                0x0010,
                [IntPtr]::Zero,
                [IntPtr]::Zero
            ) | Out-Null
        }
        Wait-Process -Id $primary.Id -Timeout 5 -ErrorAction SilentlyContinue
    }
    if ($null -ne $primary -and -not $primary.HasExited) {
        Stop-Process -Id $primary.Id -Force -ErrorAction SilentlyContinue
        Wait-Process -Id $primary.Id -Timeout 5 -ErrorAction SilentlyContinue
    }
    if ($null -eq $previousRunValue) {
        Remove-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
    }
    else {
        New-Item -Path $runKey -Force | Out-Null
        New-ItemProperty -Path $runKey -Name "AskBridge" -Value $previousRunValue -PropertyType String -Force | Out-Null
    }
    $env:ASKBRIDGE_DATA_DIR = $previousDataEnvironment
    if (Test-Path -LiteralPath $dataRoot) {
        Remove-Item -LiteralPath $dataRoot -Recurse -Force
    }
}
