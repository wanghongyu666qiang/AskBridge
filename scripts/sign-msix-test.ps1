[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$MsixPath,

    [string]$TimestampUrl
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Partner Center certifies and re-signs Store submissions, so the package built
# by package-msix.ps1 is deliberately unsigned and stays that way.
#
# To smoke test a package locally you still have to sign it, because Windows only
# deploys packages whose publisher it trusts. Add-AppxPackage -AllowUnsigned is
# not a way around that: it accepts only packages whose manifest Publisher carries
# the unsigned-namespace OID, and such a package can never share identity with the
# signed one, so deploying the real Store package with it fails 0x80073D2C.
#
# So this script produces the test-signed copy; deployment then needs the
# certificate in Cert:\LocalMachine\TrustedPeople, which only an elevated session
# can write. Without that trust, Add-AppxPackage fails 0x80073CF0 / 0x800B0109.
$certSubject = "CN=085D3D42-B8F4-43F7-BB9E-C0889168662A"
$certFriendlyName = "AskBridge MSIX local test"
$certStorePath = "Cert:\CurrentUser\My"

function Find-WindowsSdkTool {
    param([Parameter(Mandatory = $true)][string]$Name)

    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    $kitsRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    if (-not (Test-Path -LiteralPath $kitsRoot -PathType Container)) {
        throw "Windows SDK bin directory was not found: $kitsRoot"
    }
    $versions = Get-ChildItem -LiteralPath $kitsRoot -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending
    foreach ($version in $versions) {
        $candidate = Join-Path $version.FullName "x64\$Name"
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return $candidate
        }
    }
    throw "$Name was not found in the installed Windows SDK."
}

function Get-TestSigningCertificate {
    $now = Get-Date
    $existing = @(
        Get-ChildItem -Path $certStorePath |
            Where-Object {
                $_.Subject -ceq $certSubject -and $_.HasPrivateKey -and
                $_.NotBefore -le $now -and $_.NotAfter -gt $now
            } |
            Sort-Object NotAfter -Descending
    )
    if ($existing.Count -gt 0) {
        return $existing[0]
    }

    New-SelfSignedCertificate `
        -Type Custom `
        -Subject $certSubject `
        -FriendlyName $certFriendlyName `
        -KeyUsage DigitalSignature `
        -KeyLength 2048 `
        -NotAfter $now.AddYears(3) `
        -CertStoreLocation $certStorePath `
        -TextExtension @(
            "2.5.29.37={text}1.3.6.1.5.5.7.3.3",
            "2.5.29.19={text}"
        )
}

$repoRoot = [IO.Path]::GetFullPath((Resolve-Path (Join-Path $PSScriptRoot "..")).Path).TrimEnd('\')
if (-not [IO.Path]::IsPathRooted($MsixPath)) {
    throw "MsixPath must be an explicit absolute path."
}
$sourcePackage = [IO.Path]::GetFullPath($MsixPath)
if (-not (Test-Path -LiteralPath $sourcePackage -PathType Leaf)) {
    throw "Unsigned MSIX package was not found: $sourcePackage"
}
if ([IO.Path]::GetExtension($sourcePackage) -ne ".msix") {
    throw "MsixPath must point at a .msix package: $sourcePackage"
}
if ($sourcePackage.StartsWith($repoRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to work inside the repository; package test copies in a dedicated artifact directory."
}
if ($TimestampUrl -and $TimestampUrl -notmatch '^https?://') {
    throw "TimestampUrl must be an http(s) URL when supplied."
}

$testPackage = Join-Path `
    ([IO.Path]::GetDirectoryName($sourcePackage)) `
    ($([IO.Path]::GetFileNameWithoutExtension($sourcePackage)) + "-testsigned.msix")
$certificatePath = [IO.Path]::ChangeExtension($testPackage, ".cer")

Write-Host "[1/4] Reusing or creating the local test certificate"
$cert = Get-TestSigningCertificate
if ($null -eq $cert) {
    throw "No test signing certificate is available; New-SelfSignedCertificate produced nothing."
}
Write-Host "      Thumbprint $($cert.Thumbprint), valid until $($cert.NotAfter.ToString('yyyy-MM-dd'))"

Write-Host "[2/4] Copying the package to a test-signed sibling"
Copy-Item -LiteralPath $sourcePackage -Destination $testPackage -Force

$signTool = Find-WindowsSdkTool -Name "signtool.exe"
Write-Host "[3/4] Signing the test copy"
$signArguments = @("sign", "/fd", "SHA256", "/sha1", $cert.Thumbprint)
if ($TimestampUrl) {
    $signArguments += @("/tr", $TimestampUrl, "/td", "SHA256")
}
$signArguments += @("/v", $testPackage)
& $signTool @signArguments
if ($LASTEXITCODE -ne 0) {
    Remove-Item -LiteralPath $testPackage -Force -ErrorAction SilentlyContinue
    throw "signtool sign failed with exit code $LASTEXITCODE."
}

Write-Host "[4/4] Verifying the test signature"
$signature = Get-AuthenticodeSignature -FilePath $testPackage
$boundToTestCertificate = $null -ne $signature.SignerCertificate -and
    $signature.SignerCertificate.Thumbprint -ceq $cert.Thumbprint
if ($boundToTestCertificate -and $signature.Status -in @("Ok", "Valid")) {
    $trustState = "verified and trusted"
}
elseif ($boundToTestCertificate -and $signature.Status -in @("NotTrusted", "UnknownError")) {
    # Expected until the certificate reaches Cert:\LocalMachine\TrustedPeople.
    $trustState = "verified; the test certificate is not trusted yet"
}
else {
    Remove-Item -LiteralPath $testPackage -Force -ErrorAction SilentlyContinue
    throw ("The signed package failed verification (status: {0})." -f $signature.Status)
}
Write-Host "      Signature $trustState, signer $($cert.Thumbprint)"
Export-Certificate -Cert $cert -FilePath $certificatePath | Out-Null

Write-Host ""
Write-Host "Test-signed package : $testPackage"
Write-Host "Public certificate  : $certificatePath"
Write-Host "Submission package  : $sourcePackage (left unsigned on purpose)"
Write-Host ""
Write-Host "Windows does not trust this certificate yet, and unsigned deployment is not an"
Write-Host "option for a Store package (Add-AppxPackage -AllowUnsigned needs the"
Write-Host "unsigned-namespace OID, which fails 0x80073D2C here)."
Write-Host "Run these yourself - this script deliberately does not:"
Write-Host "  # elevated PowerShell, trusts the certificate for the whole machine."
Write-Host "  # User-scope stores are ignored by deployment; it must be LocalMachine."
Write-Host "  Import-Certificate -FilePath `"$certificatePath`" -CertStoreLocation Cert:\LocalMachine\TrustedPeople"
Write-Host "  # normal PowerShell, installs the package for the current user"
Write-Host "  Add-AppxPackage -Path `"$testPackage`""
Write-Host "  # afterwards, drops the trust entry again"
Write-Host "  Remove-Item `"Cert:\LocalMachine\TrustedPeople\$($cert.Thumbprint)`""
Write-Host ""
Write-Host "Check a signature with Get-AuthenticodeSignature. Do not use signtool verify here:"
Write-Host "a self-signed package always ends its chain in an untrusted root."
Write-Host ""
Write-Host "Submit the unsigned package to Partner Center; never upload the test-signed copy."
