[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ArtifactRoot,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

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

function Convert-ToMsixVersion {
    param([Parameter(Mandatory = $true)][string]$CargoVersion)

    if ($CargoVersion -notmatch '^(\d+)\.(\d+)\.(\d+)$') {
        throw "Cargo version must have exactly three numeric parts for Store packaging: $CargoVersion"
    }
    $parts = @([int]$Matches[1], [int]$Matches[2], [int]$Matches[3], 0)
    if (@($parts | Where-Object { $_ -lt 0 -or $_ -gt 65535 }).Count -ne 0) {
        throw "Each MSIX version component must be between 0 and 65535: $CargoVersion"
    }
    return ($parts -join '.')
}

$repoRoot = [IO.Path]::GetFullPath((Resolve-Path (Join-Path $PSScriptRoot "..")).Path).TrimEnd('\')
if (-not [IO.Path]::IsPathRooted($ArtifactRoot)) {
    throw "ArtifactRoot must be an explicit absolute path."
}
$artifactRoot = [IO.Path]::GetFullPath($ArtifactRoot).TrimEnd('\')
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target")).TrimEnd('\')
if ($artifactRoot.Equals($repoRoot, [StringComparison]::OrdinalIgnoreCase) -or
    $artifactRoot.Equals($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ArtifactRoot must be a dedicated directory, not the repository or target root."
}
if (Test-Path -LiteralPath $artifactRoot) {
    $existingItems = @(Get-ChildItem -LiteralPath $artifactRoot -Force)
    if ($existingItems.Count -gt 0) {
        throw "ArtifactRoot already exists and is not empty; refusing to overwrite existing files."
    }
}
else {
    New-Item -ItemType Directory -Path $artifactRoot -Force | Out-Null
}

$makeAppx = Find-WindowsSdkTool -Name "makeappx.exe"
$templatePath = Join-Path $repoRoot "packaging\msix\AppxManifest.xml.in"
$assetSource = Join-Path $repoRoot "packaging\msix\Assets"
foreach ($required in @(
    $templatePath,
    (Join-Path $assetSource "StoreLogo.png"),
    (Join-Path $assetSource "Square150x150Logo.png"),
    (Join-Path $assetSource "Square44x44Logo.png")
)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "MSIX package input is missing: $required"
    }
}

Push-Location $repoRoot
try {
    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE." }
    $package = $metadata.packages |
        Where-Object { $_.name -eq "askbridge-win" } |
        Select-Object -First 1
    if ($null -eq $package) { throw "askbridge-win metadata was not found." }
    $cargoVersion = [string]$package.version
    $msixVersion = Convert-ToMsixVersion -CargoVersion $cargoVersion

    if (-not $SkipBuild) {
        Write-Host "[1/5] Building Microsoft Store binary"
        cargo build --package askbridge-win --bin askbridge --release --locked --features store
        if ($LASTEXITCODE -ne 0) { throw "Store build failed with exit code $LASTEXITCODE." }
    }
    else {
        Write-Host "[1/5] Reusing existing Microsoft Store binary"
    }

    $releaseRoot = Join-Path $repoRoot "target\release"
    $layoutRoot = Join-Path $artifactRoot "layout"
    $layoutAssets = Join-Path $layoutRoot "Assets"
    New-Item -ItemType Directory -Path $layoutAssets -Force | Out-Null

    Write-Host "[2/5] Staging deterministic MSIX layout"
    foreach ($input in @(
        (Join-Path $releaseRoot "askbridge.exe"),
        (Join-Path $releaseRoot "WebView2Loader.dll")
    )) {
        if (-not (Test-Path -LiteralPath $input -PathType Leaf)) {
            throw "Store build output is missing: $input"
        }
        Copy-Item -LiteralPath $input -Destination $layoutRoot
    }
    foreach ($asset in Get-ChildItem -LiteralPath $assetSource -Filter "*.png" -File) {
        Copy-Item -LiteralPath $asset.FullName -Destination $layoutAssets
    }

    Write-Host "[3/5] Rendering Partner Center identity and package version"
    $manifestText = [IO.File]::ReadAllText($templatePath)
    $manifestText = $manifestText.Replace("{{MSIX_VERSION}}", $msixVersion)
    if ($manifestText.Contains("{{")) {
        throw "The rendered AppxManifest.xml still contains an unresolved token."
    }
    $manifestPath = Join-Path $layoutRoot "AppxManifest.xml"
    [IO.File]::WriteAllText($manifestPath, $manifestText, [Text.UTF8Encoding]::new($false))

    [xml]$manifest = $manifestText
    $identity = $manifest.Package.Identity
    if ($identity.Name -cne "55AD4ABA.AskBridge" -or
        $identity.Publisher -cne "CN=085D3D42-B8F4-43F7-BB9E-C0889168662A" -or
        $identity.Version -cne $msixVersion -or
        $identity.ProcessorArchitecture -cne "x64" -or
        $manifest.Package.Properties.PublisherDisplayName -cne "王宏宇" -or
        $manifest.Package.Applications.Application.Id -cne "AskBridge") {
        throw "Rendered MSIX identity does not match the reserved Partner Center product."
    }

    Write-Host "[4/5] Packing unsigned Store submission MSIX"
    $msixPath = Join-Path $artifactRoot "AskBridge-$cargoVersion-store-x64.msix"
    & $makeAppx pack /d $layoutRoot /p $msixPath /o
    if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed with exit code $LASTEXITCODE." }

    Write-Host "[5/5] Writing package metadata and SHA-256"
    $hash = (Get-FileHash -LiteralPath $msixPath -Algorithm SHA256).Hash
    [ordered]@{
        product = "AskBridge"
        cargo_version = $cargoVersion
        msix_version = $msixVersion
        architecture = "x64"
        identity_name = "55AD4ABA.AskBridge"
        publisher = "CN=085D3D42-B8F4-43F7-BB9E-C0889168662A"
        publisher_display_name = "王宏宇"
        package_family_name = "55AD4ABA.AskBridge_3kthnvq439ewe"
        store_id = "9P54M49BFH00"
        signed = $false
        sha256 = $hash
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $artifactRoot "package-metadata.json") -Encoding UTF8
    "$hash  $(Split-Path -Leaf $msixPath)" |
        Set-Content -LiteralPath (Join-Path $artifactRoot "SHA256SUMS.txt") -Encoding ASCII

    Write-Host "MSIX package: $msixPath"
    Write-Host "Package layout: $layoutRoot"
    Write-Host "MSIX version: $msixVersion"
}
finally {
    Pop-Location
}
