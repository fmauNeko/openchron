# AGENTS

## Project Snapshot

| Aspect  | Details                                                                                                  |
| ------- | -------------------------------------------------------------------------------------------------------- |
| Type    | Single Rust crate (`openchron`): library (`src/lib.rs`) + thin binary (`src/main.rs`), a cross-platform desktop application |
| Purpose | Configuration tool for Keychron keyboards and mice, with low-battery notifications and DPI popups        |
| UI      | [GPUI Kit](https://gpui-kit.com) (`gpui-kit` crate): GPUI + styled components                             |
| Stack   | Rust 2024 edition, toolchain and dev tools pinned in `mise.toml`                                         |
| Testing | `cargo test` with `#[gpui_kit::test]` headless window tests                                              |
| Linting | rustfmt, Clippy (pedantic, `-D warnings`), typos, cargo-deny, actionlint, zizmor                        |
| Commits | Conventional Commits, enforced by a cocogitto commit-msg hook                                            |
| CI/CD   | GitHub Actions; release-please on `main`; Renovate on `develop`                                          |

## Quick Reference

```bash
mise install              # toolchain + dev tools (+ commit-msg hook)
mise run run              # start the app (debug)
mise run lint             # fmt:check, clippy, typos, lint:workflows, deny
mise run test             # tests
mise run test:cov         # tests with coverage summary
mise run ci               # everything the develop CI runs
mise run build:release && mise run package   # dist/ archive for the host platform
mise run deps:linux       # Debian/Ubuntu system libraries (Linux only)
```

Always go through `mise run` / `mise exec --` so the pinned toolchain is used: mise sets `RUSTUP_TOOLCHAIN`.

## Layout

| Path                 | Content                                                                             |
| -------------------- | ----------------------------------------------------------------------------------- |
| `src/main.rs`        | Thin entry point: calls `openchron::app::run`                                       |
| `src/lib.rs`         | Crate root: every module is `pub mod`                                               |
| `src/app.rs`         | Bootstrap: init order, actions, key bindings, menus, the main window and quit      |
| `src/assets.rs`      | Asset sources: Openchron icons layered over the GPUI Kit defaults                   |
| `src/protocol/`      | Pure request builders and reply parsers: `mouse` (Keychron mouse protocol, `0xFFC1`), `via` (VIA raw HID, `0xFF60`) |
| `src/hid/`           | `Backend`/`Transport` traits, `native` (hidapi), `fake` (scripted, tests only)      |
| `src/devices/`       | `catalog` (product IDs), `model` (state, commands, updates), `session` + `mouse` + `keyboard` (protocol sessions), `worker` (thread per interface), `manager` (enumeration thread), `store` (app-level `DeviceStore`) |
| `src/battery.rs`     | Low-battery thresholds, hysteresis and notifications                                |
| `src/notifications.rs` | System notification hook (recorded in tests)                                      |
| `src/osd.rs`         | DPI popup window                                                                    |
| `src/main_window.rs` | Main window: device sidebar and the selected device's page                          |
| `src/device_page.rs` | Device page: battery, DPI stages, polling rate, profiles                            |
| `src/settings.rs`, `src/settings_ui.rs` | Settings model and store (`settings.json`), settings dialog        |
| `src/paths.rs`, `src/persist.rs` | Config dir resolution (XDG-aware), atomic JSON persistence              |
| `src/keybindings.rs` | `bind`: key binding registration that resolves the `secondary-` alias for the host OS |
| `docs/keychron-hid.md` | Keychron HID protocols, captures and what is verified on hardware                 |
| `script/package.sh`  | Release archive packaging for macOS, Linux and Windows                              |
| `packaging/`         | macOS `Info.plist` template, Linux desktop entry and udev rules                     |
| `.github/workflows/` | `develop.yaml` (CI), `release-please.yaml` (release), `build.yaml` (archives)       |
| `.vscode/`           | Recommended extensions and CodeLLDB launch configurations                           |
| `mise.toml`          | Tool versions, tasks and the postinstall hook                                       |
| `deny.toml`, `cog.toml` | cargo-deny policy, cocogitto commit-msg hook                                     |

## Universal Conventions

### Code Style

- `cargo fmt` formatted and Clippy-clean with the lints in `Cargo.toml` (pedantic, `unwrap_used`, `expect_used`, `unsafe_code = "forbid"`).
- No `unwrap()`/`expect()` outside tests (`clippy.toml` allows them in tests). Propagate errors with `?`, or report them where they can be handled.
- Lint suppressions are a last resort: use `#[expect(lint, reason = "...")]` on the smallest item, never `#[allow]` without a reason, never crate-wide.
- Match exhaustively on enums you own; no `_ =>` arm.

### GPUI Kit

- Import GPUI through the Kit: `use gpui_kit::prelude::*;` plus explicit `gpui_kit::{...}` items. Styled components live in `gpui_kit::component`; theme colors come from `gpui_kit::component::ActiveTheme` (`cx.theme()`).
- Call `gpui_kit::init(cx)` before using components, and open windows with `gpui_kit::open_window`, which mounts the Kit `Root`. The one exception is the DPI popup (`osd`): it opens with `cx.open_window` because the Kit `Root` paints an opaque background over its transparent window.
- Define actions with `gpui_kit::actions!`.
- Bind keys through `crate::keybindings::bind`, never `KeyBinding::new` with `secondary-` (it panics in GPUI >= 0.3.10; only `KeyBinding::load` resolves the alias). `window.press` in tests resolves `secondary-` already.
- `gpui-kit` pins an exact `gpui-pre` snapshot of Zed's GPUI, so any update can change GPUI's API. Renovate never automerges it.
- Docs: <https://gpui-kit.com> and <https://docs.rs/gpui-kit>.

### Testing

- Tests sit next to the code (`#[cfg(test)] mod tests`) and use Given/When/Then comments.
- UI behavior is tested in headless windows with `#[gpui_kit::test]` and `gpui_kit::test::TestWindowExt` (`find`, `click`, `press`, `input`, ...). Elements a test looks up need an `.id(...)` and `.test_support()`; `.test_support()` is inert outside tests.
- Assert behavior (visibility, state, callbacks), never UI copy.
- Never mutate the environment (`std::env::set_var` is unsafe in Rust 2024): environment-dependent code takes an injected lookup.
- Tests never open real HID devices or post OS notifications: sessions and the manager run on `hid::fake`, the device store is fed by hand (`devices::store::init(cx, None)`), and `notifications::install_recorder` records notifications. Protocol tests replay frames captured from real hardware (`protocol::mouse::tests`).
- Protocol changes need a capture from real hardware first; record it in `docs/keychron-hid.md` and mark what is verified.

### Architecture

- HID I/O never runs on the GPUI thread. The `devices::manager` thread owns the `hidapi` context, rescans every 2 s (`hidapi` has no hotplug events) and runs a `devices::worker` thread per supported interface. A worker owns a `Session`, reads the device, then applies commands and the reports the device sends on its own (`0xE2` status, `0xE5` profile), publishing the whole `DeviceState` when it changes.
- Workers talk to the app only through `DeviceUpdate`s on an `UpdateSink`; the `DeviceStore` entity applies them on the GPUI thread and emits `DeviceStoreEvent`s. Features subscribe to those events (`battery`, `osd`) instead of talking to devices.
- Commands go from the UI to a worker through the store (`DeviceStore::send`). A worker writes the change, then reads the setting back: the UI always shows the device's state, never an optimistic one.
- The first state of a device, or its state after it wakes up, is not a change: only a stage switch while online raises `DpiChanged`.
- Next steps, in order: background mode (tray icon, close to tray, single instance, start at login), keyboard support through Keychron's raw HID commands, the other mouse protocols (`0xFF0A`, `0x008C`), button remapping and macros.

### Dependencies

- Prefer existing crates over new ones; every new dependency must pass `mise run deny` (licenses allowed in `deny.toml`, no unknown registries or git sources).
- New tools go in `mise.toml` `[tools]` with an exact version, so Renovate can update them.

## Branching & PR Workflow

| Branch      | Purpose            | Merges To            |
| ----------- | ------------------ | -------------------- |
| `main`      | Stable releases    | -                    |
| `develop`   | Integration branch | `main` (via release) |
| `feature/*` | New features       | `develop`            |
| `fix/*`     | Bug fixes          | `develop`            |

### PR Requirements

1. Branch from `develop`.
2. Conventional commit messages (`feat:`, `fix:`, `docs:`, `ci(github):`, ...): they drive the version bump and the changelog.
3. `mise run ci` passes.
4. Update this file and `README.md` when behavior, tasks or structure change.

## CI/CD Pipeline

### On Push to `develop` / PR to non-main

File: `.github/workflows/develop.yaml`, Linux only (`ubuntu-24.04`):

```
commits (PRs only)
lint ─┬→ unit-tests (test:cov)
      └→ build
```

### On Push to `main` (Release)

Files: `.github/workflows/release-please.yaml`, `.github/workflows/build.yaml`

- release-please (`release-please-config.json`, `.release-please-manifest.json`) maintains the release PR: version in `Cargo.toml`/`Cargo.lock` and `CHANGELOG.md`. Tags are `vX.Y.Z`; before 1.0, breaking changes bump the minor version.
- When the release PR is merged: `build.yaml` builds and packages the six targets (macOS, Linux and Windows on x86_64 and arm64) and attaches the archives and checksums to the release; `backmerge` merges `main` into `develop`.
- `build.yaml` can also be dispatched manually: without a tag, it only uploads the archives as workflow artifacts.

## Definition of Done

- [ ] `mise run ci` passes
- [ ] New behavior is covered by a test that fails without the change
- [ ] UI changes checked by running the app (`mise run run`)
- [ ] No secrets committed
- [ ] `README.md` / `AGENTS.md` updated if behavior, tasks or structure changed
- [ ] Conventional commit messages
