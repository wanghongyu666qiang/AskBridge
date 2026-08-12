[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptRoot = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
$scripts = @(Get-ChildItem -LiteralPath $scriptRoot -File -Filter "*.ps1" | Sort-Object Name)
if ($scripts.Count -eq 0) {
    throw "No PowerShell scripts were found under $scriptRoot."
}

$failed = $false
foreach ($script in $scripts) {
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        $script.FullName,
        [ref]$tokens,
        [ref]$errors
    )

    if ($errors.Count -eq 0) {
        Write-Host ("[OK] {0}" -f $script.Name)
        continue
    }

    $failed = $true
    Write-Host ("[FAIL] {0}" -f $script.Name)
    foreach ($errorRecord in $errors) {
        $extent = $errorRecord.Extent
        Write-Host ("  line {0}, column {1}: {2}" -f $extent.StartLineNumber, $extent.StartColumnNumber, $errorRecord.Message)
    }
}

if ($failed) {
    throw "One or more PowerShell scripts contain parse errors."
}

Write-Host ("PowerShell syntax validation passed for {0} scripts." -f $scripts.Count)
