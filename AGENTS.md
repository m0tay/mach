# AGENTS.md — AI Agent Pairing & Engineering Guide for Mach

Welcome, AI Coding Agent! This document is the definitive guide for developing, maintaining, and extending **Mach**. Read this thoroughly before planning, implementing, or running tests.

---

## 1. Project Vision and Verified Architecture

Mach provides Linux VMs and container integration for Apple Silicon through Rust and Objective-C bindings to Apple's Virtualization.framework. Low overhead, fast startup, and OrbStack interface parity are goals, not established measurements or completed feature claims. Do not publish numeric performance or competitor comparisons without reproducible measurements and sources.

Implemented components include VirtioFS host sharing, APFS clone attempts with copy fallbacks, SSH guest execution, a Docker socket tunnel, a background port forwarder and DNS/proxy services, and optional K3s. Their presence in source does not establish live correctness: verify the relevant behavior on real VMs.

Docker and K3s use the protected `mach-system` infrastructure VM, with separate dockerd and K3s containerd image stores. Docker builds are not automatically available to Kubernetes. The host bridge listens on `0.0.0.0:32222` and validates peers; it is not a localhost-only listener. Host `/Users` sharing exposes host files to guest commands.

## 2. Interface Contract

- `mach` opens a Linux shell or runs a guest command. Reserve only daily lifecycle verbs (`start`, `stop`, `restart`, `shell`, `create`, `delete`, `list`/`ls`) and `menu`, alongside help/version handling.
- Administration belongs to `machctl`. `mach docker ...` always means the guest Docker executable; `machctl docker ...` manages the infrastructure engine.
- Use host `docker --context mach ...` with `~/.mach/run/docker.sock` for the infrastructure Docker engine. Optional `machctl docker start --system` configures the legacy system socket and may require privileges.
- Local domains and published Docker ports depend on daemon, resolver, tunnel, and guest service health. Do not promise universal domain or port availability from configuration alone.

---

## 3. Workspace & Crate Structure

```
mach/
├── Cargo.toml                  # Workspace manifest
├── entitlements.plist          # macOS codesigning entitlements (Hypervisor + Virtualization)
├── install.sh                  # Build, install to ~/.cargo/bin, and codesign script
├── tests/
│   ├── test_cli_primitives.sh  # CLI regression assertions (inspect script for current scope)
│   └── test_real_deal.sh       # Live VM integration checks (boot, NAT, Rosetta, Docker)
├── assets/                     # Minimal Linux kernel, icons, & Alpine initrd assets
├── crates/
│   ├── mach-common/            # Shared types, machine metadata, config, APFS CoW utilities
│   │   ├── src/config.rs       # MachConfig (~/.mach/config.yaml; legacy JSON migration)
│   │   ├── src/machine.rs      # Machine struct, MachineState enum, persistence
│   │   └── src/error.rs        # Typed errors
│   ├── mach-vmm/               # Low-level macOS virtualization engine
│   │   ├── src/vz.m            # Objective-C Virtualization.framework implementation
│   │   ├── src/vz.rs           # FFI bridge to vz.m
│   │   └── src/vmm.rs          # VMM abstraction & hypervisor process lifecycle
│   └── mach-cli/               # Unified CLI binary (mach & machctl)
│       ├── src/main.rs         # Binary entrypoint & dual-mode dispatcher (mach vs machctl)
│       ├── src/cli.rs          # Clap CLI definition & subcommands
│       ├── src/commands/       # Subcommand handlers (create, list, start, stop, delete, run_cmd, etc.)
│       ├── src/ssh.rs          # Dedicated SSH key management (~/.mach/ssh/id_ed25519)
│       ├── src/cloud_init.rs   # NoCloud cidata.iso generator using native hdiutil
│       ├── src/images.rs       # Curated cloud image catalog & image download and decompression
│       ├── src/dns.rs          # Async UDP DNS responder on 127.0.0.1:5354 (*.mach.local)
│       ├── src/port_forward.rs # Async dynamic port forwarder for published container ports
│       ├── src/proxy.rs        # HTTP/HTTPS reverse proxy for container domain routing
│       ├── src/bridge.rs       # Host bridge daemon (mac open, mac pbcopy, mac pbpaste)
│       └── src/tls.rs          # Local wildcard TLS CA generator
```

---

## 4. Golden Rules for AI Agents

### Rule 1: Codesigning with Entitlements is Mandatory
Apple Silicon hypervisor frameworks strictly require specific entitlements. Running an unsigned or improperly signed `mach` binary will fail with OS signal `SIGKILL` or initialization failure `-2`.
- Whenever modifying Rust code in `crates/`, always recompile and install with:
  ```bash
  ./install.sh
  ```
- For focused local validation, manually sign the binary (the complete release still requires `./install.sh`):
  ```bash
  codesign --force --entitlements entitlements.plist --sign - target/release/mach
  ```

### Rule 2: Verify with Live Terminal Tests Before Declaring Done
Do **NOT** assume code works just because `cargo check` passes.
- Always execute and verify actual command exit codes and outputs.
- Run the real-deal integration test suite:
  ```bash
  ./tests/test_real_deal.sh
  ```
- Run the CLI primitives test suite:
  ```bash
  ./tests/test_cli_primitives.sh
  ```

### Rule 3: Keep Git Clean and Committed
- Do not leave dirty untracked files or abandoned test machines lying around.
- Regularly inspect `git status` and commit completed work with clean, conventional commit messages.

### Rule 4: Clean Up Only Task-Owned State
- Record pre-existing machines, services, and processes before live tests.
- Use unique test names and stop/delete only machines created by this task.
- Never use blanket `killall mach`, broad `pkill`, or delete user machines as test cleanup.
- Preserve logs and report failures before removing task-owned state. Restore services to their prior state where practical.

### Rule 5: Strict Separation of `mach` vs `machctl` (Commands in ONE Place Only — NEVER Duplicate!)
Commands must live in **ONE place and one place only**. Never duplicate commands across both `mach` and `machctl`.
- **`mach` is the daily driver CLI**:
  - Drops into the interactive Linux shell when run with 0 arguments (`mach`).
  - Transparently executes commands inside Linux (`mach uname -a`, `mach apt update`, `mach docker ps`, `mach -m debian -u root whoami`).
  - Intercepts ONLY basic daily machine lifecycle verbs: `start`, `stop`, `restart`, `shell`, `create`, `delete`, `list` / `ls`, `menu`.
  - **NEVER add administration, inspection, configuration, or management subcommands to `mach`!**
- **`machctl` is the single administration & control plane CLI** (mirroring `orbctl`):
  - All control plane, system diagnostics, inspection, and administration tools live exclusively here:
    `inspect`, `dump`, `info`, `top`, `doctor`, `config`, `daemon`, `dns`, `tls`, `k8s`, `docker`, `mount`, `code`, `link`, `debug`, `export`, `import`, `clone`, `rename`.
- **If a control plane command is invoked via `mach`** (e.g. `mach inspect` or `mach dump`):
  - `mach` must NOT execute the host administration operation. It should direct the user to `machctl <command>`. Guest Docker commands are always passed through to the selected guest.

### Rule 6: Strictly Follow SemVer & Display Version Discretely
- Every patch/change must increment the minor version (e.g. `0.1.0` -> `0.2.0` -> `0.3.0`).
- The delivered binary must display the version:
  - In CLI: `mach --version`, `machctl --version`, `mach version`, and in `machctl doctor`.
  - In macOS menu bar: Discretely displayed as tooltip `mach v<VERSION>` on hover, and as a disabled non-clickable menu item `mach v<VERSION>` directly above `Quit Mach` in the dropdown.
- Keep `Cargo.toml`, `Formula/mach.rb`, manpages (`mach.1`, `machctl.1`), and tests in sync.
- Automate distribution archive packaging and Homebrew tap synchronization via `./install.sh`.

### Rule 7: Mandatory Post-Implementation Release Pipeline
Whenever implementing a solution, bug fix, or feature in Mach, you MUST complete the full end-to-end release pipeline before declaring the task finished:

1. **Update Package & Bump Minor SemVer**:
   - Increment minor version in `Cargo.toml` (e.g. `0.14.0` -> `0.15.0`).
   - Sync `man/man1/mach.1` and `man/man1/machctl.1` headers.
   - Run `./install.sh` to compile, codesign with Apple Silicon entitlements, update `~/.cargo/bin`, package distribution archives (`dist/mach-<VERSION>-aarch64-apple-darwin.tar.gz`), and update `Formula/mach.rb` + `Casks/mach.rb`. Local Homebrew tap synchronization is an explicit `./install.sh --sync-tap` step; publishing the release archive is separate. Verify the published URL and checksum before reinstalling from it.

2. **Verify Tests**:
   - Ensure unit tests (`cargo test --workspace`) and CLI primitives (`./tests/test_cli_primitives.sh`) pass 100%.

3. **Clean Git Commit**:
   - Stage all modified crates, manpages, and the newly generated distribution archive.
   - Commit with a clean, conventional commit message:
     ```bash
     git add <reviewed-task-files> && git commit -m "fix: <description> (v<VERSION>)"
     ```

4. **Push to Remote**:
   - Push the reviewed release commit and release tag. Check the target branch and remote first; do not overwrite unrelated work:
     ```bash
     git push origin main
     git push origin v<VERSION>
     ```

5. **Brew Reinstall & System Refresh (CLI + Menu Bar)**:
   - Reinstall the Homebrew cask to refresh the primary CLI binaries (`/opt/homebrew/bin/mach` and `/opt/homebrew/bin/machctl`), shell completions, and man pages:
     ```bash
     brew reinstall --cask mach
     ```
   - If the menu bar was running, restart only its identified process and verify the replacement displays the release version. Do not start it on a previously headless installation.
   - Verify the installed version in terminal:
     ```bash
     mach --version && machctl --version
     ```

---

## 5. Key Technical Invariants & Gotchas

### 1. DHCP Lease Discovery in `/var/db/dhcpd_leases`
- macOS `bootpd` writes DHCP leases to `/var/db/dhcpd_leases`.
- **CRITICAL**: The `lease=0x...` value in this file is the **expiration timestamp** (~3600s in the future), NOT the creation timestamp!
- Always record `(prev_mac_ts, prev_name_ts)` *before* spawning the VMM child process using `get_machine_lease_timestamps()`.
- Match against the hardware MAC pattern first (`52:54:0:12:xx:yy`). Fall back to machine name only if MAC is missing.
- In `cloud-init.rs`, configure `dhcp-identifier: mac` in `network-config` and inject `bootcmd` setting `[DHCPv4] ClientIdentifier=mac` into `/etc/systemd/network/99-mach-mac.network`. This forces systemd-networkd to send the Ethernet MAC instead of RFC 4361 DUIDs.

### 2. Dedicated SSH Keys (`crates/mach-cli/src/ssh.rs`)
- Mach maintains a dedicated SSH key pair in `~/.mach/ssh/id_ed25519`.
- Mach internal commands (`mach run`, `mach shell`, `mach start`, Docker socket tunnel) must include `crate::ssh::mach_ssh_args()`, which passes `-i ~/.mach/ssh/id_ed25519`.
- Both Mach's dedicated public key and host public keys (`~/.ssh/*.pub`) are injected into the VM via cloud-init `authorized_keys`.
- This prevents "Too many authentication failures" when the user has multiple SSH keys loaded in `ssh-agent`.

### 3. Smart Machine Defaults
- Default Linux distro: `ubuntu` (if omitted in `mach create`).
- Auto-naming: If `name` is omitted, use the distro name. If taken, auto-increment (`fedora` -> `fedora-2` -> `fedora-3`).
- Auto-default tracking: The first created machine is automatically saved as `default_machine` in `~/.mach/config.yaml`.
- When a machine is deleted, if it was the default machine, clear `default_machine = None`.

### 4. Docker Socket & Forwarding
- Linux guest VM runs `dockerd` at `/var/run/docker.sock`.
- `machctl docker start` establishes the infrastructure Docker SSH background tunnel:
  ```bash
  ssh -f -nNT -L ~/.mach/run/docker.sock:/var/run/docker.sock root@<guest-ip>
  ```
- Docker CLI connects via `docker --context mach` pointing to `unix:///Users/<user>/.mach/run/docker.sock`.
- Dynamic port forwarder detects published container ports and binds `127.0.0.1:PORT` on macOS to forward connections to the guest VM.

### 5. Apple Silicon Rosetta 2 for Linux
- Handled natively via `VZLinuxRosettaDirectoryShare` mounted at tag `rosetta` in `vz.m`.
- Inside the guest VM, cloud-init / start logic mounts VirtioFS `rosetta` at `/media/rosetta` and registers the ELF translation handler in `/proc/sys/fs/binfmt_misc/register`.
- Allows transparent execution of `x86_64` binaries and Docker containers on Apple Silicon (`docker run --platform linux/amd64`).

---

## 6. Development Cheat Sheet

| Task | Command |
| :--- | :--- |
| **Fast Typecheck** | `cargo check --workspace` |
| **Build & Install Signed CLI** | `./install.sh` |
| **Run Real-Deal Virtualization Tests** | `./tests/test_real_deal.sh` |
| **Run CLI Primitives Tests** | `./tests/test_cli_primitives.sh` |
| **List Machines** | `mach ls` or `machctl list` |
| **Start / Stop Machine** | `mach start <name>` / `mach stop <name>` |
| **Open Linux Shell** | `mach` (default machine) or `mach shell <name>` |
| **Guest Command Execution** | `mach uname -a` or `mach -m <name> uname -a` |
| **Docker Context Setup** | `machctl docker start` |
| **Health Check** | `machctl doctor` |
| **Inspect Machine Specs (YAML/JSON)** | `machctl inspect <name>` / `machctl inspect <name> --json` |
| **Dump Raw Machine Config (YAML/JSON)** | `machctl dump <name>` / `machctl dump <name> --json` |

---

## 7. Guidelines for Parallel Subagent Workflows

When deploying subagents in parallel to execute sprints or fix issues:
1. **Isolated Responsibilities**: Scope subagents by clear functional boundaries (e.g., Subagent 1: Distro image catalog in `images.rs`; Subagent 2: Port forwarding & DNS in `port_forward.rs` and `dns.rs`).
2. **Synchronized Compilation**: Ensure subagents run `cargo check` within their crates, but only the main coordinator runs `./install.sh` and executes integration tests to prevent binary lock contention.
3. **Verify Before Reporting**: Subagents must include exact command outputs demonstrating verification of their work.
