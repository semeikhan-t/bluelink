# Contributing to BlueLink

Thanks for helping build a private, local-first Markdown workspace. Contributions of code, tests, documentation, platform packaging, and reproducible BLE reports are welcome.

## Before you start

- Search [open issues](https://github.com/semeikhan-t/bluelink/issues) before starting substantial work.
- Open a feature request before making a large UI, protocol, storage, or compatibility change.
- Keep pull requests focused. Unrelated refactors make protocol and vault changes harder to review safely.
- Never commit a real vault's `.bluelink/` directory; it contains private device identity and trust state.

## Development environment

### 1. Install prerequisites

Use Linux, Git, and the current stable Rust toolchain. On Ubuntu or Debian, the native `eframe` dependencies are:

```bash
sudo apt-get update
sudo apt-get install -y pkg-config libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

Install `bluez` when working on the real BLE transport:

```bash
sudo apt-get install -y bluez
```

Mermaid rendering is optional. Install `@mermaid-js/mermaid-cli` when working on diagram previews so the `mmdc` executable is available on `PATH`.

### 2. Clone and run

```bash
git clone https://github.com/semeikhan-t/bluelink.git
cd bluelink
mkdir -p "$HOME/BlueLinkDevVault"
cargo run -p desktop -- --vault "$HOME/BlueLinkDevVault"
```

For editor or sync-engine work that does not need Bluetooth, build without the default BlueZ feature:

```bash
cargo run -p desktop --no-default-features -- --vault "$HOME/BlueLinkDevVault"
```

### 3. Run the test suite

```bash
cargo test --workspace
```

Run focused tests while iterating:

```bash
cargo test -p bluelink_core
cargo test -p bluelink_sync
cargo test -p bluelink_ble
cargo test -p bluelink_ui
```

Format touched Rust files before requesting review:

```bash
cargo fmt --all
```

Do not weaken, skip, or delete a failing test to make a pull request green. Explain environment-dependent failures in the PR description.

## Repository map

| Path | Responsibility |
|---|---|
| `apps/desktop` | Linux entry point, file watcher, and sync runtime. |
| `crates/bluelink_core` | Vault, paths, Markdown, frontmatter, and wiki index. |
| `crates/bluelink_sync` | Pairing, trust, encryption, planning, and sync sessions. |
| `crates/bluelink_ble` | BlueZ GATT transport and fragmentation. |
| `crates/bluelink_ui` | Editor, workspace, graph, dialogs, and sync panels. |

## Branch naming

Use a short lowercase branch name with one of these prefixes:

| Prefix | Use for | Example |
|---|---|---|
| `feat/` | User-visible capability | `feat/android-central` |
| `fix/` | Bug fix | `fix/relative-wikilinks` |
| `docs/` | Documentation only | `docs/fedora-setup` |
| `test/` | Tests and fixtures | `test/conflict-matrix` |
| `refactor/` | Behavior-preserving code change | `refactor/sync-events` |
| `chore/` | Tooling or maintenance | `chore/release-workflow` |

## Commit messages

Use an imperative subject and keep each commit coherent. Conventional Commit prefixes are recommended:

```text
feat(sync): add Android central handshake
fix(wiki): preserve heading aliases on rename
docs(linux): add Fedora build dependencies
test(core): cover duplicate frontmatter aliases
```

Reference an issue when one exists, but make the subject understandable without opening the issue.

## Pull request process

1. Rebase or merge the latest `main` before requesting review.
2. Describe the user problem, the chosen approach, and important trade-offs.
3. List the exact commands you used to verify the change.
4. Add or update tests for changed behavior.
5. Include screenshots or a short recording for visible UI changes.
6. Call out changes to the vault format, sync protocol, cryptography, or trust boundary explicitly.
7. Respond to review comments with a new commit or a short explanation.

At least one maintainer approval is expected before merge. Reviewers will check correctness, compatibility with plain Markdown, path safety, protocol behavior, test coverage, and whether the change stays within the PR's stated scope.

## Issue labels

### `good first issue`

A small, well-scoped task with a clear expected result, relevant file pointers, and no requirement to redesign the sync protocol or storage model. Examples include documentation improvements, isolated parser fixtures, accessibility labels, or small UI polish.

### `help wanted`

A task where maintainer guidance is available but the work may cross crates, require design investigation, need specific Linux/Bluetooth hardware, or involve a larger test matrix. It is not necessarily beginner-friendly.

If an issue has both labels, the task should still include enough context for a new contributor to begin without reverse-engineering the entire repository.

## Code review expectations

Review is collaborative and evidence-based. Expect questions about:

- data loss and conflict behavior;
- untrusted paths or peer-provided input;
- backward compatibility of protocol and on-disk state;
- UI responsiveness and blocking I/O;
- tests that demonstrate the reported bug and the fix.

Security-sensitive changes may require extra review or a smaller follow-up PR before they can merge.

## Reporting security concerns

Do not open a public issue containing device keys, private vault content, or an exploitable security report. Follow the private disclosure process in [SECURITY.md](./SECURITY.md).
