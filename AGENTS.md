# AGENTS.md — AI Agent Pairing & Engineering Guide for Norb

Welcome, AI Coding Agent! This document is the definitive guide for developing, maintaining, and extending **Norb**. Read this thoroughly before planning, implementing, or running tests.

---

## 1. Project Vision & Core Philosophy

**Norb** is a lightweight, zero-overhead macOS virtualization and container engine written in **Rust**, designed as a fast, native, open-source replacement for **OrbStack** and Docker Desktop on Apple Silicon.

### Core Tenets:
1. **Pure CLI-First & Zero Bloat**: No heavy Electron GUI, no bloated background runtimes. Ultra-minimal memory footprint (< 15 MB) and sub-second startup times.
2. **Apple Silicon Native**: Powered by Apple `Virtualization.framework` (`VZVirtualMachine`), APFS Copy-on-Write (CoW) disk cloning, and Apple Silicon Rosetta 2 for Linux (`VZLinuxRosettaDirectoryShare`).
3. **OrbStack Feature & CLI Parity**:
   - `norb` (short daily driver command): Drops into interactive shell, executes transparent commands on Linux (`norb uname -a`), or manages machines.
   - `norbctl` (administration CLI): Comprehensive control plane mirroring `orbctl`.
   - Standard macOS `docker` CLI drop-in context `norb` pointing to `~/.norb/run/docker.sock`.
4. **Real-Deal Verification (Zero Mocking / No Superficial Tests)**: Never claim a feature is done without booting real VMs, pinging NAT, checking dynamic port forwarding, and observing exit codes in the terminal.

---

## 2. Modern Architectural Stance vs. Legacy Approaches (Feature Parity + Common Interface)

A guiding directive for Norb is: **Achieve 100% feature and developer interface parity with OrbStack, but do not copy OrbStack's internal implementation if a cleaner, more modern, or more native approach exists.**

### The Modern vs. Legacy Architectural Matrix

| Capability | Legacy / OrbStack Approach | Norb Modern Native Approach | Why Norb's Approach is Better |
| :--- | :--- | :--- | :--- |
| **Virtualization Framework** | Proprietary closed-source Swift/C++ hypervisor wrapper | Native Apple `Virtualization.framework` (`VZVirtualMachine`) in Rust/Obj-C | 100% open-source, official Apple Silicon support, zero proprietary lock-in, ultra-low overhead (<15 MB footprint). |
| **Linux Containers on macOS** | Proprietary VM running closed Linux system runtime | Standard Linux VM kernel running OCI runtime (`containerd` / `dockerd`) | True Linux container semantics (cgroups v2, namespaces, overlayfs) managed through standard OCI tools rather than black-box daemons. |
| **x86_64 Emulation (Rosetta 2)** | Custom binfmt wrappers / QEMU translation | First-party `VZLinuxRosettaDirectoryShare` mounted via VirtioFS with kernel `binfmt_misc` | Hardware-accelerated memory model (TSO) on Apple Silicon; near-native x86_64 performance inside Linux Docker containers. |
| **Disk Storage & Snapshotting** | Custom sparse bundle formats or block copies | Native **APFS Copy-on-Write (CoW)** via `clonefile` / `/bin/cp -c` | Sub-millisecond instant cloning without duplicating disk bytes; shared blocks until written; native APFS compression. |
| **Host Directory Sharing** | Custom 9P / proprietary VirtioFS server | First-party Apple VirtioFS (`VZVirtioFileSystemDeviceConfiguration`) with 1:1 `/Users` path parity | Native Darwin kernel page caching; zero path translation overhead; seamless editing in macOS IDEs. |
| **Local DNS & Domains** | Proprietary system DNS hijacking | Async Tokio UDP responder on `127.0.0.1:5354` + native `/etc/resolver/norb.local` | Leverages macOS native resolver framework without modifying `/etc/hosts` or interfering with system DNS daemons. |
| **Dynamic Port Forwarding** | Proprietary background daemon | Async Tokio port forwarder polling Docker socket events and binding `127.0.0.1:PORT` | Zero root/sudo daemon requirements; dynamically binds and unbinds container ports in user space. |
| **Host Command Bridge** | Custom IPC bridge | Native bidirectional TCP bridge on localhost (port 32222) handling `mac open`, `mac pbcopy`, `mac pbpaste` | Lightweight, robust, standard protocol easily scriptable from any Linux shell. |
| **System Overhead** | Background menu bar GUI app (~50–100 MB RAM) | Pure CLI-first binary + headless Tokio background daemon (<15 MB RAM) | Starts instantly (<0.5s), consumes negligible battery, operates seamlessly in headless/CI and SSH terminal environments. |

### Preserving the Common Interface (Developer Ergonomics)

While the internals are modern, native, and open-source, the external user experience matches developer muscle memory 1:1:
1. **Short Daily Driver (`norb`)**:
   - `norb` $\rightarrow$ Interactive shell in default machine.
   - `norb uname -a` $\rightarrow$ Transparent command execution in Linux.
   - `norb -m <machine> -u root <cmd>` $\rightarrow$ Machine and user flags.
2. **Management Plane (`norbctl`)**:
   - Complete subcommands matching `orbctl`: `create`, `list`, `start`, `stop`, `delete`, `clone`, `rename`, `info`, `top`, `debug`, `export`, `import`, `push`, `pull`, `doctor`, `k8s`.
3. **Standard Docker Context**:
   - `docker --context norb ps` / `docker context use norb` pointing to `~/.norb/run/docker.sock`.
   - Optional legacy symlink at `/var/run/docker.sock` for VS Code Dev Containers and Testcontainers via `norbctl docker setup --system`.
4. **Networking**:
   - Container domain routing: `http://<machine>.norb.local` and `http://<container>.norb.local`.
   - Direct published ports: `curl http://localhost:<port>`.

---

## 3. Workspace & Crate Structure

```
norb/
├── Cargo.toml                  # Workspace manifest
├── entitlements.plist          # macOS codesigning entitlements (Hypervisor + Virtualization)
├── install.sh                  # Build, install to ~/.cargo/bin, and codesign script
├── tests/
│   ├── test_cli_primitives.sh  # 32 CLI assertions (lifecycle, config, export/import, push/pull)
│   └── test_real_deal.sh       # 17 live virtualization integration tests (boot, NAT, Rosetta, Docker)
├── assets/                     # Minimal Linux kernel & Alpine initrd assets
├── crates/
│   ├── norb-common/            # Shared types, machine metadata, config, APFS CoW utilities
│   │   ├── src/config.rs       # NorbConfig (~/.norb/config.json)
│   │   ├── src/machine.rs      # Machine struct, MachineState enum, persistence
│   │   └── src/error.rs        # Typed errors
│   ├── norb-vmm/               # Low-level macOS virtualization engine
│   │   ├── src/vz.m            # Objective-C Virtualization.framework implementation
│   │   ├── src/vz.rs           # FFI bridge to vz.m
│   │   └── src/vmm.rs          # VMM abstraction & hypervisor process lifecycle
│   └── norb-cli/               # Unified CLI binary (norb & norbctl)
│       ├── src/main.rs         # Binary entrypoint & dual-mode dispatcher (norb vs norbctl)
│       ├── src/cli.rs          # Clap CLI definition & subcommands
│       ├── src/commands/       # Subcommand handlers (create, list, start, stop, delete, run_cmd, etc.)
│       ├── src/ssh.rs          # Dedicated SSH key management (~/.norb/ssh/id_ed25519)
│       ├── src/cloud_init.rs   # NoCloud cidata.iso generator using native hdiutil
│       ├── src/images.rs       # Curated cloud image catalog & streaming decompressor
│       ├── src/dns.rs          # Async UDP DNS responder on 127.0.0.1:5354 (*.norb.local)
│       ├── src/port_forward.rs # Async dynamic port forwarder for published container ports
│       ├── src/proxy.rs        # HTTP/HTTPS reverse proxy for container domain routing
│       ├── src/bridge.rs       # Host bridge daemon (mac open, mac pbcopy, mac pbpaste)
│       └── src/tls.rs          # Local wildcard TLS CA generator
```

---

## 4. Golden Rules for AI Agents

### Rule 1: Codesigning with Entitlements is Mandatory
Apple Silicon hypervisor frameworks strictly require specific entitlements. Running an unsigned or improperly signed `norb` binary will fail with OS signal `SIGKILL` or initialization failure `-2`.
- Whenever modifying Rust code in `crates/`, always recompile and install with:
  ```bash
  ./install.sh
  ```
- Or manually sign the binary:
  ```bash
  codesign --force --entitlements entitlements.plist --sign - target/release/norb
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

### Rule 4: Clean Up Test State
- Never leave orphan hypervisor processes running. Clean up using:
  ```bash
  norb stop --force >/dev/null 2>&1 || true
  killall -9 norb 2>/dev/null || true
  ```
- Ensure test machines (`real-alp`, `real-deb`, `test-deb`, etc.) are deleted after test suites complete.

---

## 5. Key Technical Invariants & Gotchas

### 1. DHCP Lease Discovery in `/var/db/dhcpd_leases`
- macOS `bootpd` writes DHCP leases to `/var/db/dhcpd_leases`.
- **CRITICAL**: The `lease=0x...` value in this file is the **expiration timestamp** (~3600s in the future), NOT the creation timestamp!
- Always record `(prev_mac_ts, prev_name_ts)` *before* spawning the VMM child process using `get_machine_lease_timestamps()`.
- Match against the hardware MAC pattern first (`52:54:0:12:xx:yy`). Fall back to machine name only if MAC is missing.
- In `cloud-init.rs`, configure `dhcp-identifier: mac` in `network-config` and inject `bootcmd` setting `[DHCPv4] ClientIdentifier=mac` into `/etc/systemd/network/99-norb-mac.network`. This forces systemd-networkd to send the Ethernet MAC instead of RFC 4361 DUIDs.

### 2. Dedicated SSH Keys (`crates/norb-cli/src/ssh.rs`)
- Norb maintains a dedicated SSH key pair in `~/.norb/ssh/id_ed25519`.
- Norb internal commands (`norb run`, `norb shell`, `norb start`, Docker socket tunnel) must include `crate::ssh::norb_ssh_args()`, which passes `-i ~/.norb/ssh/id_ed25519`.
- Both Norb's dedicated public key and host public keys (`~/.ssh/*.pub`) are injected into the VM via cloud-init `authorized_keys`.
- This prevents "Too many authentication failures" when the user has multiple SSH keys loaded in `ssh-agent`.

### 3. Smart Machine Defaults
- Default Linux distro: `ubuntu` (if omitted in `norb create`).
- Auto-naming: If `name` is omitted, use the distro name. If taken, auto-increment (`fedora` -> `fedora-2` -> `fedora-3`).
- Auto-default tracking: The first created machine is automatically saved as `default_machine` in `~/.norb/config.json`.
- When a machine is deleted, if it was the default machine, clear `default_machine = None`.

### 4. Docker Socket & Forwarding
- Linux guest VM runs `dockerd` at `/var/run/docker.sock`.
- `norb start` and `norb docker setup` establish an SSH background tunnel:
  ```bash
  ssh -f -nNT -L ~/.norb/run/docker.sock:/var/run/docker.sock root@<guest-ip>
  ```
- Docker CLI connects via `docker --context norb` pointing to `unix:///Users/<user>/.norb/run/docker.sock`.
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
| **List Machines** | `norb ls` or `norbctl list` |
| **Start / Stop Machine** | `norb start <name>` / `norb stop <name>` |
| **Open Linux Shell** | `norb` (default machine) or `norb shell <name>` |
| **Transparent Command Execution** | `norb uname -a` or `norb run <name> uname -a` |
| **Docker Context Setup** | `norb docker setup` |
| **Health Check** | `norbctl doctor` |

---

## 7. Guidelines for Parallel Subagent Workflows

When deploying subagents in parallel to execute sprints or fix issues:
1. **Isolated Responsibilities**: Scope subagents by clear functional boundaries (e.g., Subagent 1: Distro image catalog in `images.rs`; Subagent 2: Port forwarding & DNS in `port_forward.rs` and `dns.rs`).
2. **Synchronized Compilation**: Ensure subagents run `cargo check` within their crates, but only the main coordinator runs `./install.sh` and executes integration tests to prevent binary lock contention.
3. **Verify Before Reporting**: Subagents must include exact command outputs demonstrating verification of their work.
