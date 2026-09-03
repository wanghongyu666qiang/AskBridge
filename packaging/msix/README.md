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

The unsigned MSIX is the Store submission artifact. A locally trusted test
certificate whose subject exactly matches the Publisher is required before
installing the packed file by sideloading. Never commit a PFX or its password.
