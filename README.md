<div align="center">
  <br />
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/bluelink-logo-white.png" />
    <source media="(prefers-color-scheme: light)" srcset="./assets/bluelink-logo-color.png" />
    <img src="./assets/bluelink-logo-color.png" alt="BlueLink" width="360" />
  </picture>

  <p><strong>Own your Markdown. Link your thinking.</strong></p>
  <p>
    A native, local-first knowledge workspace for Linux.<br />
    Encrypted Bluetooth LE sync is in progress.
  </p>

  <p>
    <code>plain .md</code>&nbsp; → &nbsp;<code>[[wikilinks]]</code>&nbsp; → &nbsp;<code>local graph</code>&nbsp; → &nbsp;<code>trusted devices</code>
  </p>

  <!-- Badges: License | Stars | Release | CI Status | Contributors | Platform -->
  <a href="https://github.com/semeikhan-t/bluelink/releases"><img alt="Release: pre-alpha" src="https://img.shields.io/badge/release-pre--alpha-2563eb?style=flat-square" /></a>
  <img alt="Platform: Linux" src="https://img.shields.io/badge/platform-Linux-0f172a?style=flat-square&logo=linux&logoColor=white" />
  <a href="https://github.com/semeikhan-t/bluelink/actions/workflows/ci.yml"><img alt="CI status" src="https://github.com/semeikhan-t/bluelink/actions/workflows/ci.yml/badge.svg" /></a>
  <a href="./LICENSE"><img alt="License: MIT" src="https://img.shields.io/github/license/semeikhan-t/bluelink?style=flat-square" /></a>
  <a href="https://github.com/semeikhan-t/bluelink/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/semeikhan-t/bluelink?style=flat-square" /></a>
  <a href="https://github.com/semeikhan-t/bluelink/graphs/contributors"><img alt="Contributors" src="https://img.shields.io/github/contributors/semeikhan-t/bluelink?style=flat-square" /></a>

  <br /><br />

  <a href="#quick-start"><strong>Get started</strong></a>
  &nbsp;·&nbsp;
  <a href="#documentation">Documentation</a>
  &nbsp;·&nbsp;
  <a href="https://github.com/semeikhan-t/bluelink/issues/new?template=bug_report.md">Report a bug</a>
  &nbsp;·&nbsp;
  <a href="https://github.com/semeikhan-t/bluelink/issues/new?template=feature_request.md">Request a feature</a>
  <br /><br />
</div>

<p align="center">

https://github.com/user-attachments/assets/c61eebca-5d8b-48fa-a7f3-9a7a608e1d60

</p>

> [!IMPORTANT]
> BlueLink is pre-alpha software. The Linux editor, vault engine, secure sync protocol, TCP demo, and BlueZ BLE host are implemented. A production Android BLE client and packaged releases are still on the roadmap.

## What is BlueLink?

Your notes should remain useful even if an account disappears, a subscription changes, or a sync service is unavailable. BlueLink keeps the source of truth in an ordinary folder of Markdown files, so the vault can be searched, backed up, versioned, or opened with other compatible tools.

Many cross-device note workflows eventually depend on a vendor cloud, a third-party storage provider, or a server you must maintain. BlueLink takes a proximity-first approach: trusted devices pair explicitly and exchange vault changes directly, with no hosted backend in the middle.

The project combines an Obsidian-friendly editing model with a native Rust desktop application and a transport-independent sync engine. Bluetooth LE is the target transport; the same pairing and synchronization flow can also run over localhost TCP for development and testing without BLE hardware.

## ✨ Features

- 🗂️ **Plain Markdown vaults** — your notes stay as portable `.md` files in folders you control.
- 🔗 **Connected knowledge** — use `[[wikilinks]]`, aliases, headings, backlinks, an outline, and a local graph.
- ✍️ **Rich native editing** — switch between Live, Reading, and Source modes with tables, math, optional Mermaid rendering, embeds, footnotes, and YAML properties.
- ⚡ **Fast desktop workflow** — tabs, quick switcher, full-vault search, history navigation, one-second autosave, and external-file change detection.
- 🔐 **Secure device pairing** — a six-digit SPAKE2 flow establishes trust, then Noise XX authenticates and encrypts sync sessions.
- 🔄 **Conflict-aware synchronization** — three-way comparison propagates edits and deletions while preserving concurrent changes as conflict siblings.
- 🦀 **Native Rust stack** — `eframe`/`egui`, BlueZ, and Tokio deliver a Linux app without Electron or a WebView.

<a id="quick-start"></a>

## 🚀 Quick Start

The shortest path to a local vault requires Git, a current stable Rust toolchain, and Linux:

```bash
git clone https://github.com/semeikhan-t/bluelink.git
```

```bash
cd bluelink
```

```bash
mkdir -p "$HOME/BlueLinkVault"
```

```bash
cargo run -p desktop -- --vault "$HOME/BlueLinkVault"
```

BlueLink should open an empty vault with controls for creating a note or folder. Bluetooth is initialized lazily only when you open the sync UI.

## 📦 Installation

### Requirements

- Linux desktop with X11 or Wayland
- [Rust](https://www.rust-lang.org/tools/install) stable and Cargo
- Git
- BlueZ 5.60+ and a powered Bluetooth adapter for the implemented BLE host and protocol testing

On Ubuntu or Debian, install the native build dependencies used by `eframe`:

```bash
sudo apt-get update
```

```bash
sudo apt-get install -y pkg-config libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

For BLE development, also install the BlueZ tools:

```bash
sudo apt-get install -y bluez
```

Mermaid diagram rendering is optional and uses the external [`mmdc`](https://github.com/mermaid-js/mermaid-cli) executable. Install it with Node.js and npm when you need rendered Mermaid blocks:

```bash
npm install -g @mermaid-js/mermaid-cli
```

```bash
mmdc -h
```

Without `mmdc`, BlueLink keeps the diagram source intact and shows a renderer status instead of the generated diagram.

### Build from source

Source builds are currently the only supported installation method:

```bash
cargo build -p desktop --release
```

```bash
mkdir -p "$HOME/BlueLinkVault"
```

```bash
./target/release/bluelink --vault "$HOME/BlueLinkVault"
```

There are no official crates.io, Homebrew, Flatpak, AppImage, or distro packages yet. Follow [Releases](https://github.com/semeikhan-t/bluelink/releases) for the first downloadable build.

### Troubleshooting

#### Native Linux dependencies are missing

If linking fails around X11, XCB, OpenSSL, or `xkbcommon`, install the Ubuntu/Debian packages above or their equivalents for your distribution, then rebuild.

#### BlueLink reports that the BLE adapter is unavailable

Verify that BlueZ is running and the adapter is powered:

```bash
systemctl status bluetooth
```

```bash
bluetoothctl power on
```

You may also need the appropriate Bluetooth/D-Bus permissions for your distribution. To work on the editor or test sync without BLE, use the TCP demo below.

#### The vault cannot be opened

`--vault` must point to an existing, readable, and writable directory. Create it first with `mkdir -p`, and do not point BlueLink at an individual Markdown file.

#### Mermaid blocks do not render

Run `mmdc -h` to verify that Mermaid CLI is installed and available on `PATH`. BlueLink stops a render after 10 seconds, so also test the same diagram directly with `mmdc` if a complex block times out.

## 💡 Usage

### 1. Create connected notes

Create `projects/bluelink.md`, then link it from another note using portable Markdown:

```markdown
# Project index

Continue with [[projects/bluelink|BlueLink]].
Jump directly to [[projects/bluelink#Roadmap]].
```

Renaming a note or folder rewrites resolvable wikilinks. Backlinks, the outline, and the local graph update from the same vault index.

### 2. Navigate and edit quickly

| Shortcut | Action |
|---|---|
| `Ctrl+P` | Open the quick switcher |
| `Ctrl+F` | Find in the active note |
| `Ctrl+G` | Toggle the local graph |
| `Ctrl+E` | Toggle Reading mode |
| `Ctrl+Shift+E` | Toggle Source mode |
| `Ctrl+S` | Save immediately |
| `Ctrl+T` / `Ctrl+W` | Open / close a tab |

### 3. Test pairing and sync without Bluetooth

Create two vaults and start a host in the first terminal:

```bash
mkdir -p "$HOME/BlueLinkHost" "$HOME/BlueLinkPeer"
```

```bash
cargo run -p desktop --no-default-features -- --vault "$HOME/BlueLinkHost" --sync-tcp 127.0.0.1:7878
```

Start the initiator in a second terminal:

```bash
cargo run -p desktop --no-default-features -- --vault "$HOME/BlueLinkPeer" --initiator --connect 127.0.0.1:7878
```

On the host, open **Devices → Show code**. On the initiator, enter that code under **Devices → Join**, edit either vault, then choose **Sync → Synchronize**.

See [Documentation](#documentation) for the architecture and protocol notes.

## ⚙️ Configuration

BlueLink needs no hand-written config file. Startup behavior is controlled by CLI arguments, while device identity, trust, and sync baselines are stored under `<vault>/.bluelink/`.

| Parameter | Type | Default | Description |
|---|---|---|---|
| `--vault PATH` | path | none | Open an existing vault directory at startup. |
| `--sync-tcp ADDR` | socket address | none | Run as a TCP sync host, for example `127.0.0.1:7878`. |
| `--initiator` | flag | `false` | Run the TCP side that initiates pairing and sync. |
| `--connect ADDR` | socket address | none | Connect an initiator to a TCP host; requires `--initiator`. |
| `--help` | flag | `false` | Print the complete CLI usage. |

Example development configuration:

```bash
cargo run -p desktop --no-default-features -- \
  --vault "$HOME/BlueLinkPeer" \
  --initiator \
  --connect 127.0.0.1:7878
```

> [!WARNING]
> `.bluelink/` contains private device identity and trust state. Do not publish it or copy it between devices. BlueLink excludes it from synchronization.

<a id="documentation"></a>

## 📖 Documentation

| Document | What it covers |
|---|---|
| [Product and protocol specification](./docs/%D0%A2%D0%97_BlueLink.md) | Original Russian product requirements, vault layout, pairing, and sync protocol. Some platform items remain roadmap work. |
| [Editor parity plan](./docs/plan_editor_obsidian_parity.md) | Detailed implementation plan for Obsidian-compatible editor behavior. |
| [`bluelink_core`](./crates/bluelink_core) | Vault I/O, Markdown parsing, paths, frontmatter, and wikilinks. |
| [`bluelink_sync`](./crates/bluelink_sync) | Pairing, trust, Noise sessions, manifests, conflicts, and transports. |
| [`bluelink_ble`](./crates/bluelink_ble) | BlueZ GATT peripheral and BLE fragmentation. |
| [`bluelink_ui`](./crates/bluelink_ui) | Native editor, workspace, graph, panels, and sync UI. |

## 🗺️ Roadmap

- [ ] Ship an Android BLE central client and mobile UI.
- [ ] Publish signed Linux binaries and reproducible packages.
- [ ] Add end-to-end sync tests against real BLE hardware.
- [ ] Add automatic sync when a trusted nearby device is detected.
- [ ] Expand platform support after the Linux/Android path is stable.

Track progress in [GitHub Issues](https://github.com/semeikhan-t/bluelink/issues). Feature proposals are welcome through the [feature request template](https://github.com/semeikhan-t/bluelink/issues/new?template=feature_request.md).

## 🤝 Contributing

Contributions are welcome, whether you are improving Rust internals, Linux packaging, BLE interoperability, documentation, or the editor experience. Read [CONTRIBUTING.md](./CONTRIBUTING.md) before opening a pull request.

The short version:

1. Fork the repository.
2. Create a focused branch such as `feat/android-central` or `fix/wiki-resolution`.
3. Make and test one coherent change.
4. Use a descriptive commit message.
5. Open a pull request with the motivation and verification steps.

Good first issue candidates include **documenting distro-specific build packages**, **adding a searchable keyboard-shortcut reference**, and **expanding Markdown parser fixtures**. Larger tasks such as Android BLE transport or cross-device hardware tests are better suited to the `help wanted` label.

⭐ **Star the repo to show your support — it helps a lot!**

## 📄 License

BlueLink is available under the [MIT License](./LICENSE).

## 💬 Support / Community

- ⭐ [Star BlueLink on GitHub](https://github.com/semeikhan-t/bluelink) to help more local-first developers discover it.
- 🐛 Use [Issues](https://github.com/semeikhan-t/bluelink/issues) for bug reports, questions, and feature requests.
- 🔐 Report vulnerabilities using the private process in [SECURITY.md](./SECURITY.md), not a public bug report.

There is no official Discord or Telegram community yet. If the project grows, GitHub Discussions is the recommended next community channel.
