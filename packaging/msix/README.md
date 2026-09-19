# Microsoft Store MSIX packaging

This directory contains the checked-in Microsoft Store identity and visual
assets for AskBridge. It is intentionally separate from the existing GitHub
`Setup.exe` and portable ZIP pipeline.

The identity values are the public values reserved in Partner Center:

- Package identity name: `55AD4ABA.AskBridge`
- Publisher: `CN=085D3D42-B8F4-43F7-BB9E-C0889168662A`
- Publisher display name: `王宏宇`
- Package family name: `55AD4ABA.AskBridge_3kthnvq439ewe`
- Store ID: `9P54M49BFH00`

Build an unsigned package for Partner Center with an explicit, empty output
directory on the D drive:

```powershell
./scripts/package-msix.ps1 -ArtifactRoot D:/your-chosen-msix-output
```

The script maps Cargo `x.y.z` to MSIX `x.y.z.0`, builds only the `askbridge`
binary with the `store` feature, verifies the exact Partner Center identity,
and creates an x64 `.msix` using the newest installed Windows SDK.

The unsigned MSIX is the Store submission artifact. Never commit a PFX or its
password.

## Signing

Partner Center certifies and re-signs Store submissions, so the unsigned `.msix`
built above is exactly what gets uploaded: that path involves no certificate, no
PFX and no HSM. (Submitting an MSI or EXE instead would be the exception, and
would have to carry its own signature.)

To install a build locally before submitting it, sign a throwaway copy with a
self-signed test certificate:

```powershell
./scripts/sign-msix-test.ps1 -MsixPath D:/your-chosen-msix-output/AskBridge-2.0.5-store-x64.msix
```

The script reuses or creates a single `CN=085D3D42-B8F4-43F7-BB9E-C0889168662A`
certificate in `Cert:\CurrentUser\My`, writes an
`...-testsigned.msix` package plus its public `.cer` next to the input, and
leaves the submission file byte-for-byte untouched. It prints the
`Import-Certificate`, `signtool verify /pa /all` and `Add-AppxPackage` commands
for the sideload step instead of running them. Never upload the test-signed copy.

## CI

The `windows` job packs the Store MSIX on every branch push and pull request and
retains it as the `AskBridge-store-msix-unsigned` workflow artifact together with
`package-metadata.json` and `SHA256SUMS.txt`. CI does not sign it, and the GitHub
Release does not carry it: the Store package is submitted to Partner Center, not
distributed from GitHub.
