# Openchron

Openchron is a configuration tool for Keychron keyboards and mice, with extra features such as low-battery notifications and on-screen DPI popups. It is cross-platform and written in Rust, with a native, GPU-accelerated interface built on [GPUI](https://gpui.rs) through [GPUI Kit](https://gpui-kit.com).

Openchron is an independent project, not affiliated with or endorsed by Keychron.

## Features

- **Devices**: Keychron mice, wired or through their 2.4 GHz receiver (Keychron Link, 4K Link, Ultra-Link 8K, TurboLink 8K), and Keychron QMK keyboards. Devices appear and disappear as you plug them in and out; a mouse its receiver cannot reach (asleep, off or out of range) shows as asleep.
- **Mouse settings**: battery level and charging state, DPI stages, polling rate and onboard profile. Openchron applies a change, then reads it back from the mouse. Changes made on the mouse itself arrive through the status events it sends.
- **Low-battery notifications**: an operating-system notification when a device drops to the low threshold (20 % by default), and another at the critical one (10 %). Each is shown once until the battery charges or recovers.
- **DPI popup**: when a mouse switches DPI stage, a small window at the bottom center of the screen shows the new DPI for 1.5 s, without taking focus. A mouse without a DPI button needs the DPI Cycle function assigned to one of its buttons with Keychron Launcher, until Openchron can remap buttons.
- **Settings** (`secondary-,`): turn the notifications and the popup off, or change the thresholds and how long the popup stays.

`secondary` is Cmd on macOS and Ctrl elsewhere.

Openchron watches devices while its window is open (minimized included); closing the window quits it. Not available yet: running in the background with a tray icon, starting at login, keyboard keymaps, lighting and Keychron keyboard settings (keyboards are listed and identified only), mouse button remapping and macros, and firmware updates. `docs/keychron-hid.md` describes the protocols and what is verified on hardware.

## Configuration

Openchron stores its configuration as human-readable JSON in one directory, picked in this order:

1. `OPENCHRON_CONFIG_DIR`, when set and non-empty.
2. `$XDG_CONFIG_HOME/openchron`, when `XDG_CONFIG_HOME` is set to an absolute path (on every platform).
3. `~/.config/openchron` on macOS and Linux, `%APPDATA%\Openchron` on Windows.

| Path            | Content                                                   |
| --------------- | --------------------------------------------------------- |
| `settings.json` | Low-battery notifications and thresholds, DPI popup       |

Device settings live on the devices themselves. Writes are atomic. A file that can't be read is moved aside as `<name>.corrupt-<timestamp>` and reported. A file written by a newer Openchron version is used read-only and never overwritten.

## Download

Every [GitHub release](https://github.com/fmauNeko/openchron/releases) ships an archive per platform, each with a `.sha256` checksum:

| Platform                      | Archive                                                         |
| ----------------------------- | --------------------------------------------------------------- |
| macOS (Apple silicon / Intel) | `openchron-vX.Y.Z-{aarch64,x86_64}-apple-darwin.zip`            |
| Linux (x86_64 / arm64)        | `openchron-vX.Y.Z-{x86_64,aarch64}-unknown-linux-gnu.tar.gz`    |
| Windows (x64 / arm64)         | `openchron-vX.Y.Z-{x86_64,aarch64}-pc-windows-msvc.zip`         |

The builds are not code-signed yet:

- **macOS** (11 or newer): unzip `Openchron.app`, move it to `/Applications`, then clear the quarantine flag Gatekeeper sets on downloads: `xattr -dr com.apple.quarantine /Applications/Openchron.app`.
- **Windows**: SmartScreen may warn on first launch; choose _More info_ then _Run anyway_.
- **Linux**: the archive contains the `openchron` binary, a desktop entry (`ovh.dissidence.openchron.desktop`) and udev rules (`60-openchron.rules`) that give the logged-in user access to Keychron devices: `sudo cp 60-openchron.rules /etc/udev/rules.d/`, then `sudo udevadm control --reload-rules && sudo udevadm trigger`. The binary needs glibc 2.39 or newer (Ubuntu 24.04, Debian 13, Fedora 40 and later) and an X11 or Wayland desktop. It links `libxcb`, `libxkbcommon` and `libxkbcommon-x11`, and renders through Vulkan (EGL as a fallback): install your distribution's Vulkan driver, such as `mesa-vulkan-drivers`, if the window does not open.

## Development

[mise](https://mise.jdx.dev) manages the whole toolchain (Rust and the dev tools) from `mise.toml`, and its tasks are the project's commands:

```bash
mise install    # Rust + dev tools; also installs the commit-msg git hook
mise run run    # build and start Openchron
mise tasks      # list every task
```

System requirements:

- **macOS**: the Xcode Command Line Tools (`xcode-select --install`); full Xcode is not needed.
- **Linux** (Debian/Ubuntu): GPUI's system libraries, installed with `mise run deps:linux`, and the udev rules above to reach your devices.
- **Windows**: the MSVC build tools and the Windows SDK.

| Task                   | Description                                                                      |
| ---------------------- | -------------------------------------------------------------------------------- |
| `run`                  | Run Openchron (debug build)                                                      |
| `build`                | Debug build                                                                      |
| `build:release`        | Release build                                                                    |
| `package`              | Package the release build into `dist/` (archive + SHA-256) for the host platform |
| `fmt` / `fmt:check`    | Format / check formatting                                                        |
| `clippy`               | Clippy with warnings as errors                                                   |
| `typos`                | Spell check                                                                      |
| `deny`                 | Dependency audit: advisories, licenses, bans, sources (`deny.toml`)              |
| `lint:workflows`       | actionlint + zizmor on the GitHub Actions workflows                              |
| `lint:commits [range]` | Conventional Commits check of a git range (default `origin/develop..HEAD`)       |
| `lint`                 | Every static check above except `lint:commits`                                   |
| `test` / `test:cov`    | Tests / tests with a coverage summary                                            |
| `ci`                   | `lint`, `test:cov` and `build`, like the develop CI                              |

Tests never touch real devices: the device workers run against a scripted HID backend that replays reports captured from real hardware.

### Commits

Commits follow [Conventional Commits](https://www.conventionalcommits.org): they decide the next version and feed the changelog. A commit-msg hook, installed by `mise install`, checks every message with [cocogitto](https://docs.cocogitto.io). `mise exec -- cog commit <type> "<description>" [scope]` builds a valid message for you, for example `mise exec -- cog commit feat "add device list"`.

## Branching and releases

| Branch               | Purpose                                                        |
| -------------------- | -------------------------------------------------------------- |
| `main`               | Stable releases                                                |
| `develop`            | Integration branch, the bleeding edge; Renovate targets it     |
| `feature/*`, `fix/*` | Work in progress, merged into `develop` through pull requests  |

1. Changes reach `develop` through pull requests. CI checks commit messages, lint, tests with coverage and the build, on Linux.
2. To release, merge `develop` into `main` with a pull request.
3. On `main`, [release-please](https://github.com/googleapis/release-please) opens or updates a release pull request that bumps the version in `Cargo.toml`/`Cargo.lock` and updates `CHANGELOG.md`.
4. Merging the release pull request tags `vX.Y.Z`, publishes the GitHub release, builds the archives for macOS, Linux and Windows (x86_64 and arm64) and attaches them, then merges `main` back into `develop`.

[Renovate](https://docs.renovatebot.com) keeps Cargo dependencies, the mise tools and the GitHub Actions up to date (`renovate.json`).

## License

[MIT](LICENSE)
