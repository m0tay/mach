//! Kubernetes (K3s) integration for Norb.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::commands::find_machine;

pub const K8S_VM_NAME: &str = "norb-k8s";

/// Enable or start the local Kubernetes cluster.
pub async fn enable() -> Result<()> {
    let cfg = NorbConfig::load()?;

    println!("\x1b[1;36m➜\x1b[0m Checking Norb Kubernetes cluster...");

    let machine = find_machine(&cfg.machines_dir, K8S_VM_NAME)?;

    if machine.is_none() {
        println!("  Creating dedicated Kubernetes node \x1b[1m{}\x1b[0m...", K8S_VM_NAME);
        crate::commands::create::run(
            "debian",
            Some(K8S_VM_NAME),
            2,
            3072,
            20,
            "aarch64",
        )
        .await?;
    }

    let m = find_machine(&cfg.machines_dir, K8S_VM_NAME)?.unwrap();
    if m.state != MachineState::Running {
        crate::commands::start::run(Some(K8S_VM_NAME)).await?;
    }

    let m = find_machine(&cfg.machines_dir, K8S_VM_NAME)?.unwrap();
    let ip = m.ip_address.context("Kubernetes VM has no IP address")?;

    println!("  \x1b[1;36m➜\x1b[0m Installing and configuring K3s inside {}...", K8S_VM_NAME);

    // Install k3s if not present
    let check_k3s = Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null",
            "-o", "LogLevel=ERROR",
            &format!("root@{ip}"),
            "which k3s >/dev/null 2>&1 || curl -sfL https://get.k3s.io | sh -s - --disable traefik",
        ])
        .status()?;

    if !check_k3s.success() {
        bail!("Failed to initialize K3s inside Kubernetes VM");
    }

    // Retrieve kubeconfig
    println!("  \x1b[1;36m➜\x1b[0m Fetching cluster kubeconfig...");
    let get_conf = Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null",
            "-o", "LogLevel=ERROR",
            &format!("root@{ip}"),
            "cat /etc/rancher/k3s/k3s.yaml",
        ])
        .output()?;

    if !get_conf.status.success() {
        bail!("Failed to read /etc/rancher/k3s/k3s.yaml from cluster VM");
    }

    let raw_yaml = String::from_utf8_lossy(&get_conf.stdout);
    let patched_yaml = raw_yaml
        .replace("127.0.0.1", &ip.to_string())
        .replace("default", "norb");

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/Users/Shared"));
    let kube_dir = home.join(".kube");
    fs::create_dir_all(&kube_dir)?;

    let norb_kubeconfig = kube_dir.join("config.norb");
    fs::write(&norb_kubeconfig, &patched_yaml)?;

    // Set or merge into ~/.kube/config
    let main_config = kube_dir.join("config");
    if !main_config.exists() {
        fs::write(&main_config, &patched_yaml)?;
    } else {
        // Merge or set KUBECONFIG
        let _ = Command::new("kubectl")
            .args(["config", "set-cluster", "norb", &format!("--server=https://{}:6443", ip)])
            .output();
    }

    println!("\x1b[1;32m✓\x1b[0m Kubernetes cluster \x1b[1mnorb\x1b[0m is UP and READY!");
    println!("  Server:     \x1b[1;34mhttps://{}:6443\x1b[0m", ip);
    println!("  Kubeconfig: {}", norb_kubeconfig.display());
    println!("\nTest your cluster:");
    println!("  \x1b[1;32mkubectl --kubeconfig {} get nodes\x1b[0m", norb_kubeconfig.display());

    Ok(())
}

/// Stop the Kubernetes cluster node.
pub async fn disable() -> Result<()> {
    let cfg = NorbConfig::load()?;
    if find_machine(&cfg.machines_dir, K8S_VM_NAME)?.is_some() {
        println!("  \x1b[1;36m➜\x1b[0m Stopping Kubernetes node {}...", K8S_VM_NAME);
        crate::commands::stop::run(Some(K8S_VM_NAME), false).await?;
        println!("\x1b[1;32m✓\x1b[0m Kubernetes cluster disabled.");
    } else {
        println!("Kubernetes cluster is not currently installed.");
    }
    Ok(())
}

/// Show status of the Kubernetes cluster.
pub async fn status() -> Result<()> {
    let cfg = NorbConfig::load()?;
    let machine = find_machine(&cfg.machines_dir, K8S_VM_NAME)?;

    match machine {
        Some(m) if m.state == MachineState::Running => {
            let ip = m.ip_address.unwrap_or_else(|| std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
            println!("\x1b[1;32m●\x1b[0m Kubernetes is \x1b[1;32mrunning\x1b[0m on {} (IP: {})", K8S_VM_NAME, ip);
            let home = dirs::home_dir().unwrap_or_default();
            let conf = home.join(".kube").join("config.norb");
            if conf.exists() {
                let _ = Command::new("kubectl")
                    .args(["--kubeconfig", conf.to_str().unwrap(), "get", "nodes"])
                    .status();
            }
        }
        Some(m) => {
            println!("\x1b[1;33m○\x1b[0m Kubernetes node {} is \x1b[1m{}\x1b[0m.", K8S_VM_NAME, m.state);
            println!("  Run \x1b[1;34mnorbctl k8s enable\x1b[0m to start it.");
        }
        None => {
            println!("\x1b[1;31m○\x1b[0m Kubernetes is not installed.");
            println!("  Run \x1b[1;34mnorbctl k8s enable\x1b[0m to provision a lightweight local cluster.");
        }
    }

    Ok(())
}
