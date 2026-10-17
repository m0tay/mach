<div align="center">
  <img src="assets/mach_icon_512.png" width="120" height="120" alt="Mach Logo" />
  <h1>Mach</h1>
  <p><strong>Supersonic, zero-overhead Linux & container engine for Apple Silicon Macs.</strong></p>
  <p>Sub-second boots • Native Docker & Kubernetes • APFS Copy-on-Write • 15 MB RAM</p>
</div>

---

### Installation

No building from source required. Just install via Homebrew:

```bash
brew tap douglaslobo/mach
brew install mach
```

Everything (Linux environment, Docker context, Apple menu bar icon, and dedicated SSH keys) is automatically prepared.

---

### Quickstart

```bash
# 1. Drop into an interactive Linux shell
mach

# 2. Run Linux commands directly from macOS
mach uname -a
mach cat /etc/os-release

# 3. Use standard macOS Docker CLI with zero background bloat
docker ps
docker run --rm -it -p 8080:80 nginx:alpine
# Open http://localhost:8080 or http://nginx.mach.local
```

---

### Why Mach?

| Feature | Mach | OrbStack | Docker Desktop |
| :--- | :--- | :--- | :--- |
| **Footprint** | **< 15 MB RAM** | ~50 MB GUI | 500 MB – 2 GB |
| **Startup Time** | **< 1 second** | ~2 seconds | 20–40 seconds |
| **Architecture** | **100% Rust** + Apple Silicon native | Swift + C++ (Closed) | Electron + Go |
| **File Sharing** | **VirtioFS** (`/Users` 1:1 parity) | Custom VirtioFS | gRPC-FUSE |
| **Local Domains** | `*.mach.local` | `*.orb.local` | `host.docker.internal` |
| **Disk Snapshots** | **Sub-ms APFS CoW** (`/bin/cp -c`) | APFS CoW | Manual export |
| **Docker Engine** | Standard `~/.mach/run/docker.sock` | Proprietary socket | VM socket |
| **Rosetta 2** | Native `VZLinuxRosettaDirectoryShare` | Custom Rosetta | QEMU / Rosetta |

---

### CLI Reference

Mach provides two commands matching developer muscle memory:

#### Daily Driver: `mach`
- `mach` — Opens default Linux shell.
- `mach <cmd>` — Runs command in Linux (`mach git status`, `mach cargo build`).
- `mach -m <machine> -u root <cmd>` — Specify machine or user.

#### Management Plane: `machctl`
```bash
machctl list                  # List all machines, IPs, and resource usage
machctl create [name]         # Create new VM (Debian, Ubuntu, Fedora, Alpine)
machctl clone <src> <dst>     # Instant APFS CoW clone (< 0.01s, 0 extra bytes)
machctl start <name>          # Start a machine
machctl stop <name>           # Gracefully stop a machine
machctl top                   # Live activity & resource monitor
machctl k8s enable            # Instant local Kubernetes (K3s) node
machctl docker setup          # Activate Docker CLI context
machctl doctor                # System health & entitlement diagnostics
```

---

### Manual Pages

Full Unix documentation is available:
```bash
man mach
man machctl
```

---

### License

MIT License. Designed with speed and simplicity for Apple Silicon.
