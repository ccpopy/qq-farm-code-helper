# Vendor

`HevSocks5Tunnel.xcframework` is built from the pinned upstream tag by:

```bash
./scripts/build-hev.sh
```

Pinned version: `2.19.1` (same version the Android app bundles). The build script
clones the repository with submodules and runs `build-apple.sh`, which produces a
static xcframework for iOS device/simulator, macOS and tvOS. Only the iOS slices are
used by this project.

The binary is not committed. Re-run the script after cleaning `.build/`.
