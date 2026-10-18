//! Intelligent multi-region mirror selection and latency detection architecture for Linux distro cloud images.

use std::time::Duration;
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::{timeout, Instant};

/// Default TCP connection probe timeout in milliseconds.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_millis(350);

/// Metadata and endpoint configuration for a distro mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorEntry {
    /// Human-readable mirror name (e.g. "Cloudflare CDN", "Fastly CDN", "Kernel.org (US)").
    pub name: &'static str,
    /// Geographic region indicator (e.g. "Global", "US", "EU", "APAC", "SA").
    pub region: &'static str,
    /// Target hostname for TCP latency check (e.g. "cloud.debian.org", "deb.debian.org").
    pub host: &'static str,
    /// Port for TCP latency check (usually 443).
    pub port: u16,
    /// Full image download URL.
    pub url: &'static str,
}

// -----------------------------------------------------------------------------
// Debian 12 (Bookworm) Mirrors
// -----------------------------------------------------------------------------
pub const DEBIAN_MIRRORS: &[MirrorEntry] = &[
    MirrorEntry {
        name: "Debian Cloud CDN (Primary)",
        region: "Global",
        host: "cloud.debian.org",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "Debian CDImage Upstream",
        region: "Global",
        host: "cdimage.debian.org",
        port: 443,
        url: "https://cdimage.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "Academic Computer Club (Sweden)",
        region: "EU",
        host: "saimei.ftp.acc.umu.se",
        port: 443,
        url: "https://saimei.ftp.acc.umu.se/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "RWTH Aachen University (Germany)",
        region: "EU",
        host: "ftp.halifax.rwth-aachen.de",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "IRCAM Research Mirror (France)",
        region: "EU",
        host: "mirrors.ircam.fr",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "London Academic Mirror (UK)",
        region: "EU",
        host: "uk.alpinelinux.org",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "Princeton University (US East)",
        region: "US",
        host: "mirror.math.princeton.edu",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "Kernel.org Mirror (US West)",
        region: "US",
        host: "mirrors.edge.kernel.org",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "SJTU Linux Mirror (Shanghai)",
        region: "APAC",
        host: "mirror.sjtu.edu.cn",
        port: 443,
        url: "https://mirror.sjtu.edu.cn/debian-cdimage/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "JAIST Academic Mirror (Japan)",
        region: "APAC",
        host: "ftp.jaist.ac.jp",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "AARNet Academic Mirror (Australia)",
        region: "APAC",
        host: "mirror.aarnet.edu.au",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
    MirrorEntry {
        name: "UFSCar Mirror (Brazil)",
        region: "SA",
        host: "mirror.ufscar.br",
        port: 443,
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
    },
];

// -----------------------------------------------------------------------------
// Ubuntu 24.04 LTS (Noble) Mirrors
// -----------------------------------------------------------------------------
pub const UBUNTU_MIRRORS: &[MirrorEntry] = &[
    MirrorEntry {
        name: "Canonical Fastly CDN (Primary)",
        region: "Global",
        host: "cloud-images.ubuntu.com",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "Yandex Cloud Mirror (Europe)",
        region: "EU",
        host: "mirror.yandex.ru",
        port: 443,
        url: "https://mirror.yandex.ru/ubuntu-cloud-images/releases/24.04/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "RWTH Aachen University (Germany)",
        region: "EU",
        host: "ftp.halifax.rwth-aachen.de",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "IRCAM Mirror (France)",
        region: "EU",
        host: "mirrors.ircam.fr",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "London Academic Mirror (UK)",
        region: "EU",
        host: "uk.alpinelinux.org",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "Princeton University (US East)",
        region: "US",
        host: "mirror.math.princeton.edu",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "Kernel.org Mirror (US West)",
        region: "US",
        host: "mirrors.edge.kernel.org",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "JAIST Academic Mirror (Japan)",
        region: "APAC",
        host: "ftp.jaist.ac.jp",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "AARNet Academic Mirror (Australia)",
        region: "APAC",
        host: "mirror.aarnet.edu.au",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "Tsinghua University (Beijing)",
        region: "APAC",
        host: "mirrors.tuna.tsinghua.edu.cn",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
    MirrorEntry {
        name: "UFSCar Mirror (Brazil)",
        region: "SA",
        host: "mirror.ufscar.br",
        port: 443,
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
    },
];

// -----------------------------------------------------------------------------
// Fedora 40 Cloud Mirrors
// -----------------------------------------------------------------------------
pub const FEDORA_MIRRORS: &[MirrorEntry] = &[
    MirrorEntry {
        name: "Fedora Project Archive (Fastly CDN)",
        region: "Global",
        host: "archives.fedoraproject.org",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "Fedora Archive Secondary",
        region: "Global",
        host: "archive.fedoraproject.org",
        port: 443,
        url: "https://archive.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "Princeton University (US East)",
        region: "US",
        host: "mirror.math.princeton.edu",
        port: 443,
        url: "https://mirror.math.princeton.edu/pub/fedora-archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "Kernel.org Mirror (US West)",
        region: "US",
        host: "mirrors.edge.kernel.org",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "RWTH Aachen University (Germany)",
        region: "EU",
        host: "ftp.halifax.rwth-aachen.de",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "IRCAM Mirror (France)",
        region: "EU",
        host: "mirrors.ircam.fr",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "London Academic Mirror (UK)",
        region: "EU",
        host: "uk.alpinelinux.org",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "JAIST Academic Mirror (Japan)",
        region: "APAC",
        host: "ftp.jaist.ac.jp",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "AARNet Academic Mirror (Australia)",
        region: "APAC",
        host: "mirror.aarnet.edu.au",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "Tsinghua University (Beijing)",
        region: "APAC",
        host: "mirrors.tuna.tsinghua.edu.cn",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
    MirrorEntry {
        name: "UFSCar Mirror (Brazil)",
        region: "SA",
        host: "mirror.ufscar.br",
        port: 443,
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
    },
];

// -----------------------------------------------------------------------------
// Alpine Linux (v3.20) Mirrors
// -----------------------------------------------------------------------------
pub const ALPINE_MIRRORS: &[MirrorEntry] = &[
    MirrorEntry {
        name: "Cloudflare CDN (Alpine Global)",
        region: "Global",
        host: "dl-cdn.alpinelinux.org",
        port: 443,
        url: "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "LeaseWeb CDN (US East)",
        region: "US",
        host: "mirror.leaseweb.com",
        port: 443,
        url: "https://mirror.leaseweb.com/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "Kernel.org Mirror (US West)",
        region: "US",
        host: "mirrors.edge.kernel.org",
        port: 443,
        url: "https://mirrors.edge.kernel.org/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "RWTH Aachen University (Germany)",
        region: "EU",
        host: "ftp.halifax.rwth-aachen.de",
        port: 443,
        url: "https://ftp.halifax.rwth-aachen.de/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "IRCAM Mirror (France)",
        region: "EU",
        host: "mirrors.ircam.fr",
        port: 443,
        url: "https://mirrors.ircam.fr/pub/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "Alpine UK Mirror (London)",
        region: "EU",
        host: "uk.alpinelinux.org",
        port: 443,
        url: "https://uk.alpinelinux.org/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "Tsinghua University (Beijing)",
        region: "APAC",
        host: "mirrors.tuna.tsinghua.edu.cn",
        port: 443,
        url: "https://mirrors.tuna.tsinghua.edu.cn/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "AARNet Academic Mirror (Australia)",
        region: "APAC",
        host: "mirror.aarnet.edu.au",
        port: 443,
        url: "https://mirror.aarnet.edu.au/pub/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "JAIST Academic Mirror (Japan)",
        region: "APAC",
        host: "ftp.jaist.ac.jp",
        port: 443,
        url: "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
    MirrorEntry {
        name: "UEPG Mirror (Brazil)",
        region: "SA",
        host: "mirror.uepg.br",
        port: 443,
        url: "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/cloud/nocloud_alpine-3.20.0-aarch64-uefi-cloudinit-r0.qcow2",
    },
];

/// Returns the static mirror catalog for a given distribution name.
pub fn get_mirrors_for_distro(distro: &str) -> &'static [MirrorEntry] {
    match distro.to_lowercase().as_str() {
        "debian" => DEBIAN_MIRRORS,
        "ubuntu" => UBUNTU_MIRRORS,
        "fedora" => FEDORA_MIRRORS,
        "alpine" => ALPINE_MIRRORS,
        _ => &[],
    }
}

/// Probes all candidate mirrors concurrently using TCP connects and ranks them
/// by lowest round-trip latency. Unresponsive candidates are preserved at the end
/// in original priority order.
pub async fn rank_mirrors<'a>(mirrors: &'a [MirrorEntry]) -> Vec<(&'a MirrorEntry, Option<Duration>)> {
    if mirrors.is_empty() {
        return Vec::new();
    }

    let mut set = JoinSet::new();
    for (idx, mirror) in mirrors.iter().enumerate() {
        let host = mirror.host;
        let port = mirror.port;
        set.spawn(async move {
            let start = Instant::now();
            let connect_fut = TcpStream::connect((host, port));
            let res = timeout(DEFAULT_PROBE_TIMEOUT, connect_fut).await;
            let latency = match res {
                Ok(Ok(_stream)) => Some(start.elapsed()),
                _ => None,
            };
            (idx, latency)
        });
    }

    let mut latencies: Vec<Option<Duration>> = vec![None; mirrors.len()];
    while let Some(res) = set.join_next().await {
        if let Ok((idx, lat)) = res {
            latencies[idx] = lat;
        }
    }

    let mut responsive: Vec<(&'a MirrorEntry, Duration)> = Vec::new();
    let mut unresponsive: Vec<(&'a MirrorEntry, Option<Duration>)> = Vec::new();

    for (idx, mirror) in mirrors.iter().enumerate() {
        if let Some(lat) = latencies[idx] {
            responsive.push((mirror, lat));
        } else {
            unresponsive.push((mirror, None));
        }
    }

    // Sort responsive mirrors ascending by latency (fastest first)
    responsive.sort_by_key(|&(_, lat)| lat);

    let mut ranked = Vec::with_capacity(mirrors.len());
    for (mirror, lat) in responsive {
        ranked.push((mirror, Some(lat)));
    }
    ranked.extend(unresponsive);

    ranked
}

/// Selects the best mirror with the lowest round-trip latency, returning both
/// the mirror entry and measured latency. If all probes fail or timeout, gracefully
/// falls back to the primary/global mirror (the first entry in the catalog).
pub async fn select_best_mirror_with_latency<'a>(
    mirrors: &'a [MirrorEntry],
) -> (&'a MirrorEntry, Option<Duration>) {
    let ranked = rank_mirrors(mirrors).await;
    if let Some((mirror, lat)) = ranked.into_iter().next() {
        (mirror, lat)
    } else {
        panic!("mirrors slice must not be empty");
    }
}

/// Selects the best mirror with lowest latency, or falls back to primary/global mirror.
pub async fn select_best_mirror<'a>(mirrors: &'a [MirrorEntry]) -> &'a MirrorEntry {
    select_best_mirror_with_latency(mirrors).await.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mirrors_catalog_completeness() {
        let catalogs = [
            ("debian", DEBIAN_MIRRORS),
            ("ubuntu", UBUNTU_MIRRORS),
            ("fedora", FEDORA_MIRRORS),
            ("alpine", ALPINE_MIRRORS),
        ];

        for (name, mirrors) in catalogs {
            assert!(!mirrors.is_empty(), "mirror catalog for {name} should not be empty");
            // Verify all required regions exist
            let regions: Vec<&str> = mirrors.iter().map(|m| m.region).collect();
            assert!(regions.contains(&"Global"), "{name} missing Global region");
            assert!(regions.contains(&"US"), "{name} missing US region");
            assert!(regions.contains(&"EU"), "{name} missing EU region");
            assert!(regions.contains(&"APAC"), "{name} missing APAC region");
            assert!(regions.contains(&"SA"), "{name} missing SA region");

            // Verify structure fields
            for m in mirrors {
                assert!(!m.name.is_empty());
                assert!(!m.host.is_empty());
                assert!(m.port == 80 || m.port == 443);
                assert!(m.url.starts_with("http://") || m.url.starts_with("https://"));
            }

            // Verify helper lookup
            assert_eq!(get_mirrors_for_distro(name), mirrors);
        }

        assert!(get_mirrors_for_distro("unknown").is_empty());
    }

    #[tokio::test]
    async fn test_select_best_mirror_fallback_on_unreachable() {
        // Mirrors that will fail to connect immediately or timeout
        let bad_mirrors = [
            MirrorEntry {
                name: "Primary Global",
                region: "Global",
                host: "127.0.0.1",
                port: 65534, // Closed port
                url: "https://example.com/primary.raw",
            },
            MirrorEntry {
                name: "Secondary Regional",
                region: "US",
                host: "127.0.0.1",
                port: 65533, // Closed port
                url: "https://example.com/secondary.raw",
            },
        ];

        let (best, lat) = select_best_mirror_with_latency(&bad_mirrors).await;
        // Should gracefully fall back to the primary/first mirror
        assert_eq!(best.name, "Primary Global");
        assert!(lat.is_none());

        let best_ref = select_best_mirror(&bad_mirrors).await;
        assert_eq!(best_ref.name, "Primary Global");
    }

    #[tokio::test]
    async fn test_rank_mirrors_sorts_responsive_first() {
        // Start a mock local TCP listener to represent a responsive mirror
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_port = listener.local_addr().unwrap().port();

        // Spawn a background task to accept connections in a loop
        tokio::spawn(async move {
            loop {
                if listener.accept().await.is_err() {
                    break;
                }
            }
        });

        let test_mirrors = [
            MirrorEntry {
                name: "Unreachable Primary",
                region: "Global",
                host: "127.0.0.1",
                port: 65534,
                url: "https://example.com/unreachable.raw",
            },
            MirrorEntry {
                name: "Fast Local Mirror",
                region: "Local",
                host: "127.0.0.1",
                port: local_port,
                url: "https://example.com/fast.raw",
            },
        ];

        let ranked = rank_mirrors(&test_mirrors).await;
        assert_eq!(ranked.len(), 2);
        // Fast Local Mirror must be ranked #1
        assert_eq!(ranked[0].0.name, "Fast Local Mirror");
        assert!(ranked[0].1.is_some());

        // Unreachable mirror must be ranked #2 with None latency
        assert_eq!(ranked[1].0.name, "Unreachable Primary");
        assert!(ranked[1].1.is_none());

        let best = select_best_mirror(&test_mirrors).await;
        assert_eq!(best.name, "Fast Local Mirror");
    }
}
