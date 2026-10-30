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
    private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool RegisterHotKey(IntPtr hWnd, int id, uint modifiers, uint virtualKey);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool UnregisterHotKey(IntPtr hWnd, int id);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr value);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr hWnd, StringBuilder className, int maxCount);

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
            if (processId == expectedProcessId &&
                ReadClassName(window) == expectedClassName) {
                found = window;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

# Per-monitor V2 so window rectangles and screen captures use real pixels on
# scaled displays instead of the virtualized ones.
[AskBridgeUiNative]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

function Find-SettingsWindow {
    param([int]$TimeoutSeconds = 20)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $children = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
            [System.Windows.Automation.TreeScope]::Children,
            [System.Windows.Automation.Condition]::TrueCondition
        )
        for ($index = 0; $index -lt $children.Count; $index++) {
            $candidate = $children.Item($index)
            if ($candidate.Current.ClassName -eq "AskBridgeSettingsWindow") {
                return $candidate
            }
        }
        Start-Sleep -Milliseconds 150
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "The AskBridge settings window did not appear."
}

function Find-ElementByAutomationId {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$AutomationId,
        [int]$TimeoutSeconds = 10
    )

    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::AutomationIdProperty,
        $AutomationId
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        # Re-read the root each attempt: every apply re-renders the page, which
        # makes previously returned elements stale.
        $liveRoot = Find-SettingsWindow
        $element = $liveRoot.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        if ($null -ne $element) { return $element }
        Start-Sleep -Milliseconds 120
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Settings element with automation ID '$AutomationId' was not found."
}

function Find-ElementByName {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name,
        [int]$TimeoutSeconds = 10
    )

    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty,
        $Name
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $liveRoot = Find-SettingsWindow
        $element = $liveRoot.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        if ($null -ne $element) { return $element }
        Start-Sleep -Milliseconds 120
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Visible UI element '$Name' was not found."
}

function Invoke-Element {
    param([System.Windows.Automation.AutomationElement]$Element)

    # Toggle buttons (aria-pressed / role=switch) expose TogglePattern instead
    # of InvokePattern, so both are accepted.
    $patternObject = $null
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$patternObject)) {
        ([System.Windows.Automation.InvokePattern]$patternObject).Invoke()
    }
    elseif ($Element.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$patternObject)) {
        ([System.Windows.Automation.TogglePattern]$patternObject).Toggle()
    }
    else {
        throw ("UI element '{0}' (type {1}, id '{2}', class '{3}', offscreen {4}) supports neither InvokePattern nor TogglePattern." -f `
            $Element.Current.Name, $Element.Current.ControlType.ProgrammaticName, `
            $Element.Current.AutomationId, $Element.Current.ClassName, $Element.Current.IsOffscreen)
    }
    Start-Sleep -Milliseconds 220
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
    Start-Sleep -Milliseconds 120
}

function Set-SwitchState {
    param(
        [System.Windows.Automation.AutomationElement]$Element,
        [bool]$On
    )

    $wanted = if ($On) {
        [System.Windows.Automation.ToggleState]::On
    } else {
        [System.Windows.Automation.ToggleState]::Off
    }
    # The page re-renders after every apply, so a single Toggle can land on a
    # node that is about to be replaced. Verify the result and retry once.
    for ($attempt = 0; $attempt -lt 3; $attempt++) {
        $patternObject = $null
        if (-not $Element.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$patternObject)) {
            throw "Switch '$($Element.Current.Name)' does not support TogglePattern."
        }
        $pattern = [System.Windows.Automation.TogglePattern]$patternObject
        if ($pattern.Current.ToggleState -eq $wanted) { return }
        $pattern.Toggle()
        Start-Sleep -Milliseconds 250
        try {
            if ($pattern.Current.ToggleState -eq $wanted) { return }
        }
        catch {
            # The element went stale; the caller re-resolves it on retry.
            return
        }
    }
}

function Wait-Config {
    param(
        [string]$Path,
        [scriptblock]$Predicate,
        [string]$Failure,
        [int]$TimeoutSeconds = 15
    )

    # Apply runs through the runtime window and rewrites config.json, so the
    # assertions poll the persisted file instead of guessing a sleep.
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path -LiteralPath $Path) {
            $config = Read-Config $Path
            if (& $Predicate $config) { return $config }
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    throw $Failure
}

function Select-Page {
    param(
        [System.Windows.Automation.AutomationElement]$Window,
        [string]$Name
    )

    Invoke-Element (Find-ElementByName $Window $Name)
    # The page transition is animated; give the card reveal time to settle so
    # the screenshot artifact shows the finished layout.
    Start-Sleep -Milliseconds 450
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

function Read-Config {
    param([string]$Path)

    return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
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
    ChatGptPwa = ConvertFrom-Utf8Base64 "5qGM6Z2i572R6aG156uv"
    ChatGptDedicated = ConvertFrom-Utf8Base64 "QXNrQnJpZGdlIOS4k+eUqCBDaHJvbWU="
    ChatGptPwaDescription = ConvertFrom-Utf8Base64 "5aSN55So546w5pyJ55m75b2V77yM5L2G5oiq5Zu+6ZyA6KaB5omL5Yqo5LiK5Lyg"
    ChatGptDedicatedDescription = ConvertFrom-Utf8Base64 "5pSv5oyB6Ieq5Yqo5LiK5Lyg5Zu+54mH77yM6ZyA6KaB5Y2V54us55m75b2V"
    OpenBrowser = ConvertFrom-Utf8Base64 "5omT5byAIEFza0JyaWRnZSDmtY/op4jlmag="
    CheckConnection = ConvertFrom-Utf8Base64 "5qOA5p+l6L+e5o6l"
    OpenProviderLogin = ConvertFrom-Utf8Base64 "5omT5byA6buY6K6k5L6b5bqU5ZWG55m75b2V6aG16Z2i"
    Apply = ConvertFrom-Utf8Base64 "5bqU55So5pu05pS5"
    RestoreHotkeys = ConvertFrom-Utf8Base64 "5oGi5aSN6buY6K6k5b+r5o236ZSu"
    RejectedPrefix = ConvertFrom-Utf8Base64 "5peg5rOV5bqU55So"
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

    $window = Find-SettingsWindow
    if ($window.Current.Name -ne $uiText.SettingsWindow) {
        throw "The settings window title was '$($window.Current.Name)', expected '$($uiText.SettingsWindow)'."
    }
    $settingsWindowHandle = [AskBridgeUiNative]::FindTopLevelWindowForProcess(
        [uint32]$primary.Id,
        "AskBridgeSettingsWindow"
    )
    if ($settingsWindowHandle -eq [IntPtr]::Zero) {
        throw "The settings window handle could not be resolved for the primary process."
    }
    # Park the window somewhere predictable so the screenshot artifact and the
    # coordinate probes are stable regardless of the previous desktop state.
    [AskBridgeUiNative]::SetWindowPos($settingsWindowHandle, [IntPtr]::Zero, 40, 40, 0, 0, 0x0001 -bor 0x0004 -bor 0x0040) | Out-Null
    [AskBridgeUiNative]::SetForegroundWindow($settingsWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 500

    foreach ($tab in @($uiText.Shortcuts, $uiText.Providers, $uiText.Browser, $uiText.General)) {
        Find-ElementByName $window $tab | Out-Null
    }

    $configPath = Join-Path $dataRoot "config.json"

    # Every apply re-renders the page, which invalidates previously resolved
    # automation elements. `Refresh-Window` re-reads the root so lookups always
    # start from the live tree.
    $window = Find-SettingsWindow

    # ---- hotkeys: replace, verify registration, disable, restore ----
    Select-Page $window $uiText.Shortcuts
    $captureHotkey = Find-ElementByAutomationId $window "hotkey-capture_with_prompt"
    Set-ElementValue $captureHotkey "Ctrl+Alt+Shift+F12"
    Invoke-Element (Find-ElementByAutomationId $window "apply")
    $config = Wait-Config $configPath {
        param($candidate)
        $candidate.hotkeys.capture_with_prompt.key -eq "F12" -and $candidate.hotkeys.capture_with_prompt.enabled
    } "The real settings UI did not persist the replacement capture hotkey."
    if (Test-HotkeyOccupied 28673 0x4001 0x51) {
        throw "Alt+Q remained registered after replacing it through the real settings UI."
    }
    if (-not (Test-HotkeyOccupied 28674 0x4007 0x7B)) {
        throw "The replacement Ctrl+Alt+Shift+F12 hotkey was not registered."
    }

    $window = Find-SettingsWindow
    $captureToggle = Find-ElementByAutomationId $window "enable-capture_with_prompt"
    Set-SwitchState $captureToggle $false
    Invoke-Element (Find-ElementByAutomationId $window "apply")
    $config = Wait-Config $configPath {
        param($candidate)
        -not $candidate.hotkeys.capture_with_prompt.enabled
    } "The capture hotkey was not disabled through the real settings UI."
    if (Test-HotkeyOccupied 28675 0x4007 0x7B) {
        throw "The disabled replacement hotkey remained registered."
    }

    $window = Find-SettingsWindow
    Invoke-Element (Find-ElementByAutomationId $window "restore-hotkeys")
    $config = Wait-Config $configPath {
        param($candidate)
        $candidate.hotkeys.capture_with_prompt.enabled -and $candidate.hotkeys.capture_with_prompt.key -eq "Q"
    } "Default hotkeys were not restored and persisted through the real settings UI."
    if (-not (Test-HotkeyOccupied 28676 0x4001 0x51)) {
        throw "Alt+Q was not registered after restoring default hotkeys."
    }

    # ---- hotkeys: a conflicting combination must roll back ----
    $reservedId = 28677
    if (-not [AskBridgeUiNative]::RegisterHotKey([IntPtr]::Zero, $reservedId, 0x4007, 0x7A)) {
        throw "Could not reserve Ctrl+Alt+Shift+F11 for the rollback acceptance probe."
    }
    try {
        $window = Find-SettingsWindow
        $captureHotkey = Find-ElementByAutomationId $window "hotkey-capture_with_prompt"
        Set-ElementValue $captureHotkey "Ctrl+Alt+Shift+F11"
        Invoke-Element (Find-ElementByAutomationId $window "apply")
        # Rollback is a no-write outcome, so this cannot poll for a change;
        # it waits for the page to report the rejection instead.
        Start-Sleep -Milliseconds 1500
        $config = Read-Config $configPath
        if (-not $config.hotkeys.capture_with_prompt.enabled -or $config.hotkeys.capture_with_prompt.key -ne "Q") {
            throw "A conflicting hotkey changed the persisted binding instead of rolling back."
        }
        if (-not (Test-HotkeyOccupied 28678 0x4001 0x51)) {
            throw "Alt+Q was lost after the conflicting replacement failed."
        }
        # The page must surface the rejection, not fail silently. Every apply
        # re-hydrates the page, so the tree is re-read rather than reused.
        $rejected = $null
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        do {
            $freshWindow = Find-SettingsWindow
            $texts = $freshWindow.FindAll(
                [System.Windows.Automation.TreeScope]::Descendants,
                (New-Object System.Windows.Automation.PropertyCondition(
                    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
                    [System.Windows.Automation.ControlType]::Text
                ))
            )
            for ($index = 0; $index -lt $texts.Count; $index++) {
                $candidateText = $texts.Item($index).Current.Name
                if ($candidateText.StartsWith($uiText.RejectedPrefix, [StringComparison]::Ordinal)) {
                    $rejected = $candidateText
                    break
                }
            }
            if ($null -ne $rejected) { break }
            Start-Sleep -Milliseconds 250
        } while ([DateTime]::UtcNow -lt $deadline)
        if ($null -eq $rejected) {
            $dump = @()
            $freshWindow = Find-SettingsWindow
            $all = $freshWindow.FindAll(
                [System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition
            )
            for ($index = 0; $index -lt $all.Count; $index++) {
                $current = $all.Item($index).Current
                $dump += ("{0} '{1}' id='{2}'" -f $current.ControlType.ProgrammaticName, $current.Name, $current.AutomationId)
            }
            throw "The rejected apply was not reported anywhere in the settings page. Tree: $($dump -join ' | ')"
        }
    }
    finally {
        [AskBridgeUiNative]::UnregisterHotKey([IntPtr]::Zero, $reservedId) | Out-Null
    }

    # ---- providers: round-trip default provider and a custom row ----
    $window = Find-SettingsWindow
    Select-Page $window $uiText.Providers
    Find-ElementByName $window $uiText.DefaultProvider | Out-Null
    # Each built-in provider appears both as a default-provider choice and as an
    # enable/URL row, so every surface is asserted through a stable automation ID.
    foreach ($provider in @("chatgpt", "gemini", "claude", "doubao")) {
        Find-ElementByAutomationId $window "default-provider-$provider" | Out-Null
        Find-ElementByAutomationId $window "enable-$provider" | Out-Null
        Find-ElementByAutomationId $window "url-$provider" | Out-Null
    }
    Invoke-Element (Find-ElementByAutomationId $window "default-provider-gemini")
    Invoke-Element (Find-ElementByAutomationId $window "add-custom")
    Set-ElementValue (Find-ElementByAutomationId $window "custom-0-id") "example"
    Set-ElementValue (Find-ElementByAutomationId $window "custom-0-display_name") "Example"
    Set-ElementValue (Find-ElementByAutomationId $window "custom-0-start_url") "https://example.com/chat"
    Set-ElementValue (Find-ElementByAutomationId $window "custom-0-url_patterns") "https://example.com/"
    Invoke-Element (Find-ElementByAutomationId $window "apply")
    $config = Wait-Config $configPath {
        param($candidate)
        $candidate.default_provider_id -eq "gemini" -and
            @($candidate.custom_providers | Where-Object { $_.id -eq "example" }).Count -eq 1
    } "Provider settings were not persisted from the real UI."
    $exampleProvider = @($config.custom_providers | Where-Object { $_.id -eq "example" })
    if ($exampleProvider[0].start_url -ne "https://example.com/chat" -or
        @($exampleProvider[0].url_patterns).Count -ne 1 -or
        $exampleProvider[0].url_patterns[0] -ne "https://example.com/") {
        throw "Custom provider fields did not round-trip through the real UI."
    }
    $window = Find-SettingsWindow
    Select-Page $window $uiText.Providers
    Set-ElementValue (Find-ElementByAutomationId $window "custom-0-id") ""
    Set-ElementValue (Find-ElementByAutomationId $window "custom-0-display_name") ""
    Set-ElementValue (Find-ElementByAutomationId $window "custom-0-start_url") ""
    Set-ElementValue (Find-ElementByAutomationId $window "custom-0-url_patterns") ""
    Invoke-Element (Find-ElementByAutomationId $window "default-provider-chatgpt")
    Invoke-Element (Find-ElementByAutomationId $window "apply")
    $config = Wait-Config $configPath {
        param($candidate)
        $candidate.default_provider_id -eq "chatgpt" -and @($candidate.custom_providers).Count -eq 0
    } "Provider settings were not restored through the real UI."

    # ---- browser page exposes the four explicit target choices ----
    $window = Find-SettingsWindow
    Select-Page $window $uiText.Browser
    foreach ($control in @(
        $uiText.OpenBrowser,
        $uiText.CheckConnection,
        $uiText.OpenProviderLogin
    )) {
        Find-ElementByName $window $control | Out-Null
    }
    # The four target choices are radio buttons whose names are their labels.
    $radioCondition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::RadioButton
    )
    $radios = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $radioCondition)
    if ($radios.Count -ne 4) {
        throw "Expected four ChatGPT target choices, found $($radios.Count)."
    }
    $radioNames = @()
    for ($index = 0; $index -lt $radios.Count; $index++) {
        $radioNames += $radios.Item($index).Current.Name
    }
    # A radio's accessible name concatenates its label and consequence text,
    # so the label is matched as a prefix.
    foreach ($expected in @($uiText.ChatGptPwa, $uiText.ChatGptDedicated)) {
        $matched = $false
        foreach ($name in $radioNames) {
            if ($name.StartsWith($expected, [StringComparison]::Ordinal)) { $matched = $true; break }
        }
        if (-not $matched) {
            throw "The browser page is missing the target choice '$expected'; saw $($radioNames -join ', ')."
        }
    }
    foreach ($expected in @($uiText.ChatGptPwaDescription, $uiText.ChatGptDedicatedDescription)) {
        $matched = $false
        foreach ($name in $radioNames) {
            if ($name.Contains($expected)) { $matched = $true; break }
        }
        if (-not $matched) {
            throw "The browser page is missing the consequence copy '$expected'; saw $($radioNames -join ', ')."
        }
    }

    # ---- general: quick prompt, startup, debug logging ----
    $window = Find-SettingsWindow
    Select-Page $window $uiText.General
    Set-ElementValue (Find-ElementByAutomationId $window "quick-prompt") "AskBridge Phase 7 UI acceptance prompt."
    Set-SwitchState (Find-ElementByAutomationId $window "start-on-login") $true
    Set-SwitchState (Find-ElementByAutomationId $window "debug-logging") $true
    Invoke-Element (Find-ElementByAutomationId $window "apply")
    $config = Wait-Config $configPath {
        param($candidate)
        $candidate.general.start_on_login -and
            $candidate.general.debug_logging -and
            $candidate.quick_prompt -eq "AskBridge Phase 7 UI acceptance prompt."
    } "Settings were not persisted from the real UI."
    $runValue = (Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction Stop).AskBridge
    if (([string]$runValue).IndexOf("askbridge.exe", [StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw "The current-user startup value does not target askbridge.exe."
    }

    $window = Find-SettingsWindow
    Select-Page $window $uiText.General
    Set-SwitchState (Find-ElementByAutomationId $window "start-on-login") $false
    Set-SwitchState (Find-ElementByAutomationId $window "debug-logging") $false
    Invoke-Element (Find-ElementByAutomationId $window "apply")
    $config = Wait-Config $configPath {
        param($candidate)
        -not $candidate.general.start_on_login -and -not $candidate.general.debug_logging
    } "Startup or runtime debug logging was not disabled through the real UI."
    if ($null -ne (Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue)) {
        throw "The current-user startup value remained after disabling it."
    }

    $window = Find-SettingsWindow
    Select-Page $window $uiText.General
    Set-SwitchState (Find-ElementByAutomationId $window "start-on-login") $true
    Invoke-Element (Find-ElementByAutomationId $window "apply")
    $config = Wait-Config $configPath {
        param($candidate)
        $candidate.general.start_on_login
    } "Startup was not re-enabled through the real UI."
    $reEnabledRunValue = [string](Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction Stop).AskBridge
    if (-not $config.general.start_on_login -or
        $reEnabledRunValue.IndexOf("askbridge.exe", [StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw "Startup was not re-enabled before the foreign Run ownership probe."
    }

    # A Run value owned by another program must survive the toggle.
    $foreignRunValue = '"C:\Windows\System32\notepad.exe"'
    New-Item -Path $runKey -Force | Out-Null
    New-ItemProperty -Path $runKey -Name "AskBridge" -Value $foreignRunValue -PropertyType String -Force | Out-Null
    $window = Find-SettingsWindow
    Select-Page $window $uiText.General
    Set-SwitchState (Find-ElementByAutomationId $window "start-on-login") $false
    Invoke-Element (Find-ElementByAutomationId $window "apply")
    $config = Wait-Config $configPath {
        param($candidate)
        -not $candidate.general.start_on_login
    } "Disabling startup did not persist through the real UI."
    $preservedForeignRunValue = [string](Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction Stop).AskBridge
    if ($config.general.start_on_login -or $preservedForeignRunValue -ne $foreignRunValue) {
        throw "Disabling startup removed or changed a Run value owned by another program."
    }

    # ---- artifacts: one screenshot per page plus the default view ----
    $window = Find-SettingsWindow
    Select-Page $window $uiText.Shortcuts
    Capture-Window $window $screenshot
    if (-not (Test-Path -LiteralPath $screenshot -PathType Leaf)) { throw "Settings screenshot was not created." }
    $screenshotDirectory = Split-Path -Parent $screenshot
    $pageIndex = 0
    foreach ($page in @($uiText.Providers, $uiText.Browser, $uiText.General)) {
        $pageIndex++
        $window = Find-SettingsWindow
        Select-Page $window $page
        Capture-Window $window (Join-Path $screenshotDirectory ("settings-page-{0}.png" -f $pageIndex))
    }
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
    # WebView2 spawns its own browser processes that keep the page's user data
    # folder locked after the host exits. They are identified by the data
    # directory we handed the WebView, so only ours are stopped.
    $webviewDataRoot = Join-Path $dataRoot "webview"
    Get-Process -Name msedgewebview2 -ErrorAction SilentlyContinue | ForEach-Object {
        $commandLine = $null
        try {
            $commandLine = (Get-CimInstance Win32_Process -Filter "ProcessId = $($_.Id)" -ErrorAction Stop).CommandLine
        }
        catch {
            $commandLine = $null
        }
        if ($commandLine -and $commandLine.IndexOf($webviewDataRoot, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
        }
    }
    # The browser processes exit asynchronously; give them a moment and retry
    # the removal instead of failing the run over a transient lock.
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        if (-not (Test-Path -LiteralPath $dataRoot)) { break }
        try {
            Remove-Item -LiteralPath $dataRoot -Recurse -Force -ErrorAction Stop
            break
        }
        catch {
            Start-Sleep -Milliseconds 250
        }
    }
}
