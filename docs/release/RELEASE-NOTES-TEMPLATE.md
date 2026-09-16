# vieww Studio <VERSION> (beta)

Released <DATE> from commit `<COMMIT>`.

## Downloads

| Platform | File | Notes |
|---|---|---|
| Linux x86_64 (Debian, Ubuntu) | `viewwstudio-linux-x86_64.deb` | `sudo apt install ./viewwstudio-linux-x86_64.deb` |
| Linux x86_64 (other distributions) | `viewwstudio-linux-x86_64.AppImage` | `chmod +x` it, then run it |
| Linux x86_64 (no root) | `viewwstudio-linux-x86_64.tar.gz` | Unpack, then run `./install.sh` (installs to `~/.local`) |
| Windows x86_64 | `viewwstudio-windows-x86_64.msi` / `.zip` | See *Signing* below |
| macOS Apple silicon | `viewwstudio-macos-aarch64.dmg` | See *Signing* below |
| macOS Intel | `viewwstudio-macos-x86_64.dmg` | |

Verify a download with the published checksums:

```bash
sha256sum -c SHA256SUMS --ignore-missing     # macOS: shasum -a 256 -c SHA256SUMS
```

## Requirements

- **A Vulkan-capable graphics driver.** On Linux that means Mesa, or the NVIDIA or AMD driver. On Windows it ships with current GPU drivers. On macOS, <MoltenVK bundled / install the Vulkan SDK>. Studio draws every pixel on the CPU, but it uses Vulkan to show them in the window, so it will not start on a machine without a driver. That includes many VMs and remote desktops.
- Minimum hardware tested: <CPU / RAM from G1.8>.

## What's new

- <item>

## Known limitations

- **Unsigned builds** <remove if B2/B3 closed>:
  - On Windows, SmartScreen warns on first run: choose *More info → Run anyway*.
  - On macOS, Gatekeeper blocks the first launch: right-click the app, choose *Open*, then confirm.
- GPU rendering is not used by the Studio window in this beta. The D3D12 and Metal backends are not implemented.
- There are no ARM64 builds for Linux or Windows.
- <anything WAIVED in the checklist, with its issue link>

## Upgrading

Your workspaces and settings are kept. They are stored in:

- Linux: `~/.local/share`
- Windows: `%APPDATA%`
- macOS: `~/Library/Application Support`

Uninstalling Studio does not remove your projects.

## Reporting problems

<issues link>. Please include your OS version, GPU and driver version (`vulkaninfo --summary`).
