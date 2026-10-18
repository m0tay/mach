//! Minimal Apple Silicon macOS Menu Bar status item for mach.
//!
//! Provides a native status item in the macOS menu bar (` mach`)
//! displaying three dedicated, actionable tiers:
//! 1. Virtual Machines (names, states, IPs, shell/file/stop actions)
//! 2. Containers (Docker names, status, ports, domains, browser/log/exec actions)
//! 3. Kubernetes (Services/NodePorts, Pods/Workloads, logs/exec actions)
//! Followed by system utilities (Finder, Dashboard, Settings, Quit).
//!
//! Sub-millisecond UI latency powered by background state caching.
//! Pure native macOS Cocoa / AppKit with < 5 MB RAM footprint and zero Electron bloat.

use anyhow::Result;

#[cfg(target_os = "macos")]
mod imp {
    use anyhow::Result;
    use mach_common::config::MachConfig;
    use mach_common::machine::MachineState;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::sync::{Once, RwLock};

    #[link(name = "AppKit", kind = "framework")]
    #[link(name = "Foundation", kind = "framework")]
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
        fn objc_allocateClassPair(
            superclass: *mut c_void,
            name: *const c_char,
            extra_bytes: usize,
        ) -> *mut c_void;
        fn objc_registerClassPair(cls: *mut c_void);
        fn class_addMethod(
            cls: *mut c_void,
            name: *mut c_void,
            imp: *const c_void,
            types: *const c_char,
        ) -> i8;
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    #[inline]
    unsafe fn msg_send_0(receiver: *mut c_void, sel: *mut c_void) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel)
    }

    #[inline]
    unsafe fn msg_send_1(receiver: *mut c_void, sel: *mut c_void, a1: *mut c_void) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, a1)
    }

    #[inline]
    unsafe fn msg_send_3(
        receiver: *mut c_void,
        sel: *mut c_void,
        a1: *mut c_void,
        a2: *mut c_void,
        a3: *mut c_void,
    ) -> *mut c_void {
        let f: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ) -> *mut c_void = std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, a1, a2, a3)
    }

    #[inline]
    unsafe fn msg_send_f64(receiver: *mut c_void, sel: *mut c_void, val: f64) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, f64) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, val)
    }

    #[inline]
    unsafe fn msg_send_isize(receiver: *mut c_void, sel: *mut c_void, val: isize) -> isize {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> isize =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, val)
    }

    #[inline]
    unsafe fn msg_send_bool(receiver: *mut c_void, sel: *mut c_void, val: i8) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, i8) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, val)
    }

    #[inline]
    unsafe fn msg_send_void(receiver: *mut c_void, sel: *mut c_void) {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel);
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct NSSize {
        width: f64,
        height: f64,
    }

    #[inline]
    unsafe fn msg_send_size(receiver: *mut c_void, sel: *mut c_void, size: NSSize) {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, NSSize) =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, size);
    }

    static EMBEDDED_MENU_ICON: &[u8] = include_bytes!("../../../assets/mach_menu_icon@2x.png");

    unsafe fn ns_string(s: &str) -> *mut c_void {
        let cls = objc_getClass(b"NSString\0".as_ptr() as _);
        let sel = sel_registerName(b"stringWithUTF8String:\0".as_ptr() as _);
        let c_str = CString::new(s).unwrap_or_default();
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        f(cls, sel, c_str.as_ptr())
    }

    unsafe fn from_ns_string(ns_str: *mut c_void) -> String {
        if ns_str.is_null() {
            return String::new();
        }
        let sel = sel_registerName(b"UTF8String\0".as_ptr() as _);
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *const c_char =
            std::mem::transmute(objc_msgSend as *const ());
        let ptr = f(ns_str, sel);
        if ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }

    fn copy_to_clipboard(text: &str) {
        use std::io::Write;
        if let Ok(mut child) = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Snapshot Data Structures (Cached for Sub-Millisecond Menu Rendering)
    // ─────────────────────────────────────────────────────────────────────────────

    #[derive(Clone, Debug)]
    pub struct VmMenuItem {
        pub name: String,
        pub state: MachineState,
        pub ip: Option<String>,
    }

    impl Default for VmMenuItem {
        fn default() -> Self {
            Self {
                name: String::new(),
                state: MachineState::Stopped,
                ip: None,
            }
        }
    }

    #[allow(dead_code)]
    #[derive(Clone, Default, Debug)]
    pub struct ContainerMenuItem {
        pub id: String,
        pub name: String,
        pub image: String,
        pub state: String,     // "running", "exited", etc.
        pub status: String,    // "Up 53 minutes"
        pub created: i64,
        pub public_port: Option<u16>,
        pub private_port: Option<u16>,
        pub ip: Option<String>,
        pub domain: String,
    }

    #[allow(dead_code)]
    #[derive(Clone, Default, Debug)]
    pub struct K8sServiceMenuItem {
        pub name: String,
        pub namespace: String,
        pub service_type: String, // "ClusterIP", "NodePort", etc.
        pub port: u16,
        pub node_port: Option<u16>,
    }

    #[derive(Clone, Default, Debug)]
    pub struct K8sPodMenuItem {
        pub name: String,
        pub namespace: String,
        pub phase: String,        // "Running", "Pending", etc.
        pub ready_summary: String, // "1/1"
    }

    #[derive(Clone, Default, Debug)]
    pub struct MenuSnapshot {
        pub vms: Vec<VmMenuItem>,
        pub docker_ready: bool,
        pub docker_status_text: String,
        pub containers: Vec<ContainerMenuItem>,
        pub k8s_ready: bool,
        pub k8s_status_text: String,
        pub k8s_services: Vec<K8sServiceMenuItem>,
        pub k8s_pods: Vec<K8sPodMenuItem>,
    }

    #[derive(Clone, Debug)]
    pub struct ActiveOp {
        pub description: String,
        pub started_at: std::time::Instant,
    }

    static ACTIVE_OP: RwLock<Option<ActiveOp>> = RwLock::new(None);

    fn set_active_op(desc: &str) {
        if let Ok(mut lock) = ACTIVE_OP.write() {
            *lock = Some(ActiveOp {
                description: desc.to_string(),
                started_at: std::time::Instant::now(),
            });
        }
        unsafe {
            update_status_item_state();
        }
    }

    fn clear_active_op() {
        if let Ok(mut lock) = ACTIVE_OP.write() {
            *lock = None;
        }
        unsafe {
            update_status_item_state();
        }
    }

    fn send_macos_notification(title: &str, subtitle: &str, message: &str) {
        let script = format!(
            r#"display notification "{}" with title "{}" subtitle "{}""#,
            message.replace('"', "\\\""),
            title.replace('"', "\\\""),
            subtitle.replace('"', "\\\""),
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }

    static CACHED_SNAPSHOT: RwLock<Option<MenuSnapshot>> = RwLock::new(None);

    fn is_system_machine(m: &mach_common::machine::Machine) -> bool {
        m.is_system()
            || m.config.name == "mach-system"
            || m.config.name == "mach-docker"
            || m.config.name == "mach-kubernetes"
            || m.config.name == "mach-k8s"
    }

    pub(crate) fn is_docker_socket_responding(cfg: Option<&MachConfig>) -> bool {
        let mut candidates = Vec::new();
        if let Some(cfg) = cfg {
            candidates.push(cfg.data_dir.join("run").join("docker.sock"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".mach").join("run").join("docker.sock"));
        }
        candidates.push(std::path::PathBuf::from("/var/run/docker.sock"));

        for sock_path in candidates {
            if !sock_path.exists() {
                continue;
            }
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(200)));
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(200)));
                use std::io::{Read, Write};
                if stream
                    .write_all(b"GET /_ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .is_ok()
                {
                    let mut buf = [0u8; 128];
                    if let Ok(n) = stream.read(&mut buf) {
                        if n > 0 {
                            let resp = String::from_utf8_lossy(&buf[..n]);
                            if resp.starts_with("HTTP/") || resp.contains("200 OK") || resp.contains("OK") {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    pub(crate) fn is_k8s_api_responding() -> bool {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 26443));
        let timeout = std::time::Duration::from_millis(200);
        std::net::TcpStream::connect_timeout(&addr, timeout).is_ok()
    }

    fn fetch_docker_containers(sock_path: &std::path::Path) -> Vec<ContainerMenuItem> {
        if !sock_path.exists() {
            return Vec::new();
        }
        use std::io::{Read, Write};
        let mut stream = match std::os::unix::net::UnixStream::connect(sock_path) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(400)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(400)));

        let req = b"GET /containers/json?all=1 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        if stream.write_all(req).is_err() {
            return Vec::new();
        }

        let mut buf = Vec::new();
        if stream.read_to_end(&mut buf).is_err() {
            return Vec::new();
        }

        let mut split_pos = None;
        for i in 0..buf.len().saturating_sub(3) {
            if &buf[i..i + 4] == b"\r\n\r\n" {
                split_pos = Some(i + 4);
                break;
            }
        }

        let body = match split_pos {
            Some(pos) => &buf[pos..],
            None => return Vec::new(),
        };

        let decoded_body = crate::port_forward::decode_chunked_body(body);
        let raw_json: serde_json::Value = match serde_json::from_slice(&decoded_body) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let mut result = Vec::new();
        if let Some(arr) = raw_json.as_array() {
            for item in arr {
                let id = item
                    .get("Id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .chars()
                    .take(12)
                    .collect::<String>();

                let name = item
                    .get("Names")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim_start_matches('/'))
                    .unwrap_or("")
                    .to_string();

                if name.is_empty() || name.starts_with("k8s_") {
                    continue;
                }

                // Filter out Kubernetes internal/managed containers from the Docker CONTAINERS list
                // Kubernetes workloads are already displayed in their own KUBERNETES section!
                if let Some(labels) = item.get("Labels").and_then(|l| l.as_object()) {
                    if labels.contains_key("io.kubernetes.pod.name")
                        || labels.contains_key("io.kubernetes.docker.type")
                        || labels.contains_key("io.cri-containerd.kind")
                    {
                        continue;
                    }
                }

                let image = item.get("Image").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if image.starts_with("rancher/mirrored-pause") || image.contains("/pause") {
                    continue;
                }
                let state = item.get("State").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let status = item.get("Status").and_then(|v| v.as_str()).unwrap_or("").to_string();

                let mut public_port = None;
                let mut private_port = None;
                if let Some(ports) = item.get("Ports").and_then(|p| p.as_array()) {
                    for p in ports {
                        if let Some(pub_p) = p.get("PublicPort").and_then(|v| v.as_u64()) {
                            if public_port.is_none() {
                                public_port = Some(pub_p as u16);
                                if let Some(priv_p) = p.get("PrivatePort").and_then(|v| v.as_u64()) {
                                    private_port = Some(priv_p as u16);
                                }
                            }
                        }
                    }
                }

                let mut ip = None;
                if let Some(networks) = item
                    .get("NetworkSettings")
                    .and_then(|n| n.get("Networks"))
                    .and_then(|n| n.as_object())
                {
                    for (_net_name, net_val) in networks {
                        if let Some(ip_addr) = net_val.get("IPAddress").and_then(|v| v.as_str()) {
                            if !ip_addr.is_empty() {
                                ip = Some(ip_addr.to_string());
                                break;
                            }
                        }
                    }
                }

                let domain = format!("{}.mach.local", name);
                let created = item.get("Created").and_then(|v| v.as_i64()).unwrap_or(0);

                result.push(ContainerMenuItem {
                    id,
                    name,
                    image,
                    state,
                    status,
                    created,
                    public_port,
                    private_port,
                    ip,
                    domain,
                });
            }
        }
        result
    }

    fn fetch_k8s_workloads() -> (Vec<K8sServiceMenuItem>, Vec<K8sPodMenuItem>) {
        let mut services = Vec::new();
        let mut pods = Vec::new();

        let home = dirs::home_dir().unwrap_or_default();
        let conf = home.join(".kube").join("config.mach");
        if !conf.exists() {
            return (services, pods);
        }

        let out = match std::process::Command::new("kubectl")
            .args([
                "--kubeconfig",
                conf.to_str().unwrap_or(""),
                "--request-timeout=1.2s",
                "get",
                "svc,pods",
                "-A",
                "-o",
                "json",
            ])
            .output()
        {
            Ok(o) => o,
            Err(_) => return (services, pods),
        };

        if !out.status.success() {
            return (services, pods);
        }

        let parsed: serde_json::Value = match serde_json::from_slice(&out.stdout) {
            Ok(v) => v,
            Err(_) => return (services, pods),
        };

        if let Some(items) = parsed.get("items").and_then(|i| i.as_array()) {
            for item in items {
                let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                let metadata = match item.get("metadata") {
                    Some(m) => m,
                    None => continue,
                };
                let name = metadata.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let namespace = metadata
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();

                // Suppress internal system noise
                if namespace == "kube-system" {
                    continue;
                }

                if kind == "Service" {
                    if name == "kubernetes" && namespace == "default" {
                        continue;
                    }
                    let spec = match item.get("spec") {
                        Some(s) => s,
                        None => continue,
                    };
                    let svc_type = spec
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("ClusterIP")
                        .to_string();
                    let mut port = 80;
                    let mut node_port = None;

                    if let Some(ports) = spec.get("ports").and_then(|p| p.as_array()) {
                        if let Some(first_p) = ports.first() {
                            if let Some(p) = first_p.get("port").and_then(|v| v.as_u64()) {
                                port = p as u16;
                            }
                            if let Some(np) = first_p.get("nodePort").and_then(|v| v.as_u64()) {
                                node_port = Some(np as u16);
                            }
                        }
                    }

                    services.push(K8sServiceMenuItem {
                        name,
                        namespace,
                        service_type: svc_type,
                        port,
                        node_port,
                    });
                } else if kind == "Pod" {
                    let status = match item.get("status") {
                        Some(s) => s,
                        None => continue,
                    };
                    let phase = status
                        .get("phase")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .to_string();

                    let mut ready_count = 0;
                    let mut total_count = 0;
                    if let Some(c_statuses) =
                        status.get("containerStatuses").and_then(|c| c.as_array())
                    {
                        total_count = c_statuses.len();
                        for cs in c_statuses {
                            if cs.get("ready").and_then(|v| v.as_bool()).unwrap_or(false) {
                                ready_count += 1;
                            }
                        }
                    }
                    let ready_summary = if total_count > 0 {
                        format!("{}/{}", ready_count, total_count)
                    } else {
                        phase.clone()
                    };

                    pods.push(K8sPodMenuItem {
                        name,
                        namespace,
                        phase,
                        ready_summary,
                    });
                }
            }
        }

        (services, pods)
    }

    fn poll_menu_snapshot() -> MenuSnapshot {
        let cfg_res = MachConfig::load();
        let all_machines = if let Ok(ref cfg) = cfg_res {
            crate::commands::load_all_machines(&cfg.machines_dir).unwrap_or_default()
        } else {
            Vec::new()
        };

        // 1. Virtual Machines (filtering system machines)
        let vms: Vec<VmMenuItem> = all_machines
            .iter()
            .filter(|m| !is_system_machine(m))
            .map(|m| VmMenuItem {
                name: m.config.name.clone(),
                state: m.state.clone(),
                ip: m.ip_address.map(|ip| ip.to_string()),
            })
            .collect();

        // 2. Docker Status & Containers
        let cfg_ref = cfg_res.as_ref().ok();
        let docker_sock_path = cfg_ref
            .map(|c| c.data_dir.join("run").join("docker.sock"))
            .or_else(|| dirs::home_dir().map(|h| h.join(".mach/run/docker.sock")))
            .unwrap_or_else(|| std::path::PathBuf::from("/Users/Shared/.mach/run/docker.sock"));

        let docker_ready = is_docker_socket_responding(cfg_ref);
        let docker_status_text = if docker_ready {
            "Docker: Ready".to_string()
        } else {
            let docker_vm = all_machines
                .iter()
                .find(|m| m.config.name == "mach-system" || m.config.name == "mach-docker");
            match docker_vm {
                None => "Docker: Stopped".to_string(),
                Some(m) => match &m.state {
                    MachineState::Error(_) => "Docker: Error".to_string(),
                    MachineState::Stopped | MachineState::Stopping => "Docker: Stopped".to_string(),
                    MachineState::Creating | MachineState::Starting => "Docker: Starting...".to_string(),
                    MachineState::Running => "Docker: Starting...".to_string(),
                },
            }
        };

        let containers = if docker_ready {
            fetch_docker_containers(&docker_sock_path)
        } else {
            Vec::new()
        };

        // 3. Kubernetes Status & Workloads
        let k8s_ready = is_k8s_api_responding();
        let k8s_status_text = if k8s_ready {
            "Kubernetes: Ready".to_string()
        } else {
            let infra_vm = all_machines.iter().find(|m| {
                m.config.name == "mach-system"
                    || m.config.name == "mach-docker"
                    || m.config.name == "mach-kubernetes"
                    || m.config.name == "mach-k8s"
            });
            match infra_vm {
                None => "Kubernetes: Stopped".to_string(),
                Some(m) => match &m.state {
                    MachineState::Error(_) => "Kubernetes: Error".to_string(),
                    MachineState::Stopped | MachineState::Stopping => {
                        "Kubernetes: Stopped".to_string()
                    }
                    MachineState::Creating | MachineState::Starting => {
                        "Kubernetes: Starting...".to_string()
                    }
                    MachineState::Running => {
                        if let Some(c) = cfg_ref {
                            if c.k8s.was_running {
                                "Kubernetes: Starting...".to_string()
                            } else {
                                "Kubernetes: Stopped".to_string()
                            }
                        } else {
                            "Kubernetes: Stopped".to_string()
                        }
                    }
                },
            }
        };

        let (k8s_services, k8s_pods) = if k8s_ready {
            fetch_k8s_workloads()
        } else {
            (Vec::new(), Vec::new())
        };

        MenuSnapshot {
            vms,
            docker_ready,
            docker_status_text,
            containers,
            k8s_ready,
            k8s_status_text,
            k8s_services,
            k8s_pods,
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Cocoa Menu Construction & Delegation
    // ─────────────────────────────────────────────────────────────────────────────

    static INIT_DELEGATE: Once = Once::new();
    static mut DELEGATE_CLASS: *mut c_void = std::ptr::null_mut();

    unsafe fn get_delegate_class() -> *mut c_void {
        INIT_DELEGATE.call_once(|| {
            let superclass = objc_getClass(b"NSObject\0".as_ptr() as _);
            let cls_name = b"MachMenuDelegate\0".as_ptr() as *const c_char;
            let mut cls = objc_getClass(cls_name);
            if cls.is_null() {
                cls = objc_allocateClassPair(superclass, cls_name, 0);
                if !cls.is_null() {
                    let types = b"v@:@\0".as_ptr() as *const c_char;

                    let methods: &[(&[u8], *const c_void)] = &[
                        (b"menuNeedsUpdate:\0", menu_needs_update as _),
                        (b"onOpenShell:\0", on_open_shell as _),
                        (b"onOpenMachineShell:\0", on_open_machine_shell as _),
                        (b"onOpenFiles:\0", on_open_files as _),
                        (b"onOpenMachineFiles:\0", on_open_machine_files as _),
                        (b"onCopyText:\0", on_copy_text as _),
                        (b"onOpenUrl:\0", on_open_url as _),
                        (b"onStartMachine:\0", on_start_machine as _),
                        (b"onStopMachine:\0", on_stop_machine as _),
                        (b"onRestartMachine:\0", on_restart_machine as _),
                        (b"onOpenContainerLogs:\0", on_open_container_logs as _),
                        (b"onOpenContainerShell:\0", on_open_container_shell as _),
                        (b"onStopContainer:\0", on_stop_container as _),
                        (b"onRestartContainer:\0", on_restart_container as _),
                        (b"onStartContainer:\0", on_start_container as _),
                        (b"onStartDocker:\0", on_start_docker as _),
                        (b"onStopDocker:\0", on_stop_docker as _),
                        (b"onRestartDocker:\0", on_restart_docker as _),
                        (b"onOpenDockerLogs:\0", on_open_docker_logs as _),
                        (b"onStartK8s:\0", on_start_k8s as _),
                        (b"onStopK8s:\0", on_stop_k8s as _),
                        (b"onRestartK8s:\0", on_restart_k8s as _),
                        (b"onOpenK8sClusterLogs:\0", on_open_k8s_cluster_logs as _),
                        (b"onOpenK8sLogs:\0", on_open_k8s_logs as _),
                        (b"onOpenK8sShell:\0", on_open_k8s_shell as _),
                        (b"onOpenK8sPortForward:\0", on_open_k8s_port_forward as _),
                        (b"onOpenDashboard:\0", on_open_dashboard as _),
                        (b"onRestartAll:\0", on_restart_all as _),
                        (b"onQuit:\0", on_quit as _),
                        (b"checkRunningState:\0", check_running_state as _),
                    ];

                    for (sel_name, imp) in methods {
                        class_addMethod(cls, sel_registerName(sel_name.as_ptr() as _), *imp, types);
                    }

                    objc_registerClassPair(cls);
                }
            }
            DELEGATE_CLASS = cls;
        });
        DELEGATE_CLASS
    }

    unsafe fn add_menu_item(
        menu: *mut c_void,
        title: &str,
        action: Option<*mut c_void>,
        target: Option<*mut c_void>,
        enabled: bool,
    ) -> *mut c_void {
        add_menu_item_with_obj(menu, title, action, target, enabled, None)
    }

    unsafe fn add_menu_item_with_obj(
        menu: *mut c_void,
        title: &str,
        action: Option<*mut c_void>,
        target: Option<*mut c_void>,
        enabled: bool,
        represented_obj: Option<&str>,
    ) -> *mut c_void {
        let title_ns = ns_string(title);
        let empty_str = ns_string("");
        let act = action.unwrap_or(std::ptr::null_mut());
        let item = msg_send_3(
            menu,
            sel_registerName(b"addItemWithTitle:action:keyEquivalent:\0".as_ptr() as _),
            title_ns,
            act,
            empty_str,
        );
        if let Some(t) = target {
            msg_send_1(item, sel_registerName(b"setTarget:\0".as_ptr() as _), t);
        }
        msg_send_bool(
            item,
            sel_registerName(b"setEnabled:\0".as_ptr() as _),
            if enabled { 1 } else { 0 },
        );
        if let Some(obj) = represented_obj {
            let obj_ns = ns_string(obj);
            msg_send_1(
                item,
                sel_registerName(b"setRepresentedObject:\0".as_ptr() as _),
                obj_ns,
            );
        }
        item
    }

    unsafe fn add_separator(menu: *mut c_void) {
        let item_cls = objc_getClass(b"NSMenuItem\0".as_ptr() as _);
        let sep = msg_send_0(
            item_cls,
            sel_registerName(b"separatorItem\0".as_ptr() as _),
        );
        msg_send_1(menu, sel_registerName(b"addItem:\0".as_ptr() as _), sep);
    }

    unsafe fn create_submenu(title: &str) -> *mut c_void {
        let menu_cls = objc_getClass(b"NSMenu\0".as_ptr() as _);
        let alloc_sel = sel_registerName(b"alloc\0".as_ptr() as _);
        let init_sel = sel_registerName(b"initWithTitle:\0".as_ptr() as _);
        let title_ns = ns_string(title);
        let sub = msg_send_1(msg_send_0(menu_cls, alloc_sel), init_sel, title_ns);
        msg_send_0(sub, sel_registerName(b"retain\0".as_ptr() as _));
        msg_send_bool(sub, sel_registerName(b"setAutoenablesItems:\0".as_ptr() as _), 0);
        sub
    }

    unsafe fn attach_submenu(parent_item: *mut c_void, submenu: *mut c_void) {
        let sel = sel_registerName(b"setSubmenu:\0".as_ptr() as _);
        msg_send_1(parent_item, sel, submenu);
    }

    fn truncate(s: &str, max_len: usize) -> String {
        if s.chars().count() > max_len {
            let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
            format!("{}…", truncated)
        } else {
            s.to_string()
        }
    }

    unsafe fn populate_menu(menu: *mut c_void, target: *mut c_void) {
        msg_send_void(menu, sel_registerName(b"removeAllItems\0".as_ptr() as _));

        let active_op = {
            let mut lock = ACTIVE_OP.write().unwrap();
            if let Some(ref op) = *lock {
                if op.started_at.elapsed() > std::time::Duration::from_secs(90) {
                    *lock = None;
                    None
                } else {
                    Some(op.clone())
                }
            } else {
                None
            }
        };

        if let Some(ref op) = active_op {
            let banner = format!("  ⟳ Working: {}...", op.description);
            add_menu_item(menu, &banner, None, None, false);
            add_separator(menu);
        }

        let snapshot = {
            let lock = CACHED_SNAPSHOT.read().unwrap();
            lock.clone().unwrap_or_default()
        };

        // ═════════════════════════════════════════════════════════════════════════
        // 1. VIRTUAL MACHINES (Top 5)
        // ═════════════════════════════════════════════════════════════════════════
        add_menu_item(menu, "VIRTUAL MACHINES", None, None, false);

        let mut sorted_vms = snapshot.vms.clone();
        sorted_vms.sort_by_key(|m| if m.state == MachineState::Running { 0 } else { 1 });
        let total_vms = sorted_vms.len();

        if sorted_vms.is_empty() {
            add_menu_item(menu, "  None", None, None, false);
        } else {
            for vm in sorted_vms.iter().take(5) {
                let is_target_of_op = active_op
                    .as_ref()
                    .map(|o| o.description.contains(&vm.name))
                    .unwrap_or(false);

                let bullet = if is_target_of_op {
                    "◐"
                } else {
                    match vm.state {
                        MachineState::Running => "●",
                        MachineState::Starting | MachineState::Creating | MachineState::Stopping => "◐",
                        MachineState::Stopped => "○",
                        MachineState::Error(_) => "⚠️",
                    }
                };

                let name = truncate(&vm.name, 12);
                let title = if is_target_of_op {
                    format!("  {} {} (working...)", bullet, name)
                } else if vm.state == MachineState::Running {
                    if let Some(ref ip) = vm.ip {
                        format!("  {} {} {}", bullet, name, ip)
                    } else {
                        format!("  {} {}", bullet, name)
                    }
                } else if matches!(vm.state, MachineState::Starting | MachineState::Creating) {
                    format!("  {} {} (starting)", bullet, name)
                } else {
                    format!("  {} {}", bullet, name)
                };

                let parent_item = add_menu_item(menu, &title, None, None, true);
                let sub = create_submenu(&vm.name);

                if vm.state == MachineState::Running {
                    let open_shell_sel = sel_registerName(b"onOpenMachineShell:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Shell",
                        Some(open_shell_sel),
                        Some(target),
                        true,
                        Some(&vm.name),
                    );

                    let stop_sel = sel_registerName(b"onStopMachine:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Stop",
                        Some(stop_sel),
                        Some(target),
                        true,
                        Some(&vm.name),
                    );

                    let restart_sel = sel_registerName(b"onRestartMachine:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Restart",
                        Some(restart_sel),
                        Some(target),
                        true,
                        Some(&vm.name),
                    );

                    add_separator(sub);

                    if let Some(ref ip) = vm.ip {
                        let copy_ip_sel = sel_registerName(b"onCopyText:\0".as_ptr() as _);
                        add_menu_item_with_obj(
                            sub,
                            "Copy IP",
                            Some(copy_ip_sel),
                            Some(target),
                            true,
                            Some(ip),
                        );
                    }

                    let open_files_sel = sel_registerName(b"onOpenMachineFiles:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Files",
                        Some(open_files_sel),
                        Some(target),
                        true,
                        Some(&vm.name),
                    );
                } else {
                    let start_sel = sel_registerName(b"onStartMachine:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Start",
                        Some(start_sel),
                        Some(target),
                        !is_target_of_op,
                        Some(&vm.name),
                    );

                    let open_files_sel = sel_registerName(b"onOpenMachineFiles:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Files",
                        Some(open_files_sel),
                        Some(target),
                        true,
                        Some(&vm.name),
                    );
                }

                attach_submenu(parent_item, sub);
            }

            if total_vms > 5 {
                let more_title = format!("  + {} more...", total_vms - 5);
                let dash_sel = sel_registerName(b"onOpenDashboard:\0".as_ptr() as _);
                add_menu_item(menu, &more_title, Some(dash_sel), Some(target), true);
            }
        }

        add_separator(menu);

        // ═════════════════════════════════════════════════════════════════════════
        // 2. CONTAINERS (Top 5 Recent)
        // ═════════════════════════════════════════════════════════════════════════
        add_menu_item(menu, "CONTAINERS", None, None, false);

        let is_starting_docker = active_op
            .as_ref()
            .map(|o| o.description.contains("Docker"))
            .unwrap_or(false)
            || snapshot.docker_status_text.contains("Starting");

        if !snapshot.docker_ready {
            let docker_status_label = if is_starting_docker {
                "Docker: Starting...".to_string()
            } else {
                snapshot.docker_status_text.clone()
            };
            let parent_item = add_menu_item(
                menu,
                &format!("  {}", docker_status_label),
                None,
                None,
                true,
            );
            let sub = create_submenu("Docker");
            let start_docker_sel = sel_registerName(b"onStartDocker:\0".as_ptr() as _);
            add_menu_item(
                sub,
                "Start Docker",
                Some(start_docker_sel),
                Some(target),
                !is_starting_docker,
            );
            let logs_sel = sel_registerName(b"onOpenDockerLogs:\0".as_ptr() as _);
            add_menu_item(
                sub,
                "View Logs",
                Some(logs_sel),
                Some(target),
                true,
            );
            attach_submenu(parent_item, sub);
        } else {
            let parent_item = add_menu_item(menu, "  ● Docker Engine", None, None, true);
            let sub = create_submenu("Docker");
            let logs_sel = sel_registerName(b"onOpenDockerLogs:\0".as_ptr() as _);
            add_menu_item(sub, "View Logs", Some(logs_sel), Some(target), true);
            let restart_sel = sel_registerName(b"onRestartDocker:\0".as_ptr() as _);
            add_menu_item(sub, "Restart", Some(restart_sel), Some(target), !is_starting_docker);
            let stop_sel = sel_registerName(b"onStopDocker:\0".as_ptr() as _);
            add_menu_item(sub, "Stop", Some(stop_sel), Some(target), !is_starting_docker);
            attach_submenu(parent_item, sub);

            if snapshot.containers.is_empty() {
                add_menu_item(menu, "  (no containers running)", None, None, false);
            } else {
                let mut sorted = snapshot.containers.clone();
                sorted.sort_by(|a, b| {
                    let a_running = a.state == "running";
                    let b_running = b.state == "running";
                    b_running
                        .cmp(&a_running)
                        .then_with(|| b.created.cmp(&a.created))
                });
                let total_containers = sorted.len();

                for c in sorted.iter().take(5) {
                    let is_up = c.state == "running";
                    let bullet = if is_up { "●" } else { "○" };
                    let name = truncate(&c.name, 14);

                    let title = if is_up {
                        if let Some(pub_p) = c.public_port {
                            format!("  {} {} :{}", bullet, name, pub_p)
                        } else if let Some(ref ip) = c.ip {
                            format!("  {} {} {}", bullet, name, ip)
                        } else {
                            format!("  {} {}", bullet, name)
                        }
                    } else {
                        format!("  {} {}", bullet, name)
                    };

                    let parent_item = add_menu_item(menu, &title, None, None, true);
                    let sub = create_submenu(&c.name);

                    if is_up {
                        let open_url_sel = sel_registerName(b"onOpenUrl:\0".as_ptr() as _);
                        let copy_sel = sel_registerName(b"onCopyText:\0".as_ptr() as _);
                        let domain_url = format!("http://{}", c.domain);

                        if let Some(pub_p) = c.public_port {
                            let port_url = format!("http://localhost:{}", pub_p);

                            add_menu_item_with_obj(
                                sub,
                                &format!("Open :{}", pub_p),
                                Some(open_url_sel),
                                Some(target),
                                true,
                                Some(&port_url),
                            );
                            add_menu_item_with_obj(
                                sub,
                                &format!("Open {}", c.domain),
                                Some(open_url_sel),
                                Some(target),
                                true,
                                Some(&domain_url),
                            );
                            add_menu_item_with_obj(
                                sub,
                                &format!("Copy URL (localhost:{})", pub_p),
                                Some(copy_sel),
                                Some(target),
                                true,
                                Some(&port_url),
                            );
                            add_menu_item_with_obj(
                                sub,
                                &format!("Copy URL ({})", c.domain),
                                Some(copy_sel),
                                Some(target),
                                true,
                                Some(&domain_url),
                            );
                            add_separator(sub);
                        } else {
                            add_menu_item_with_obj(
                                sub,
                                &format!("Open {}", c.domain),
                                Some(open_url_sel),
                                Some(target),
                                true,
                                Some(&domain_url),
                            );
                            add_menu_item_with_obj(
                                sub,
                                &format!("Copy URL ({})", c.domain),
                                Some(copy_sel),
                                Some(target),
                                true,
                                Some(&domain_url),
                            );
                            add_separator(sub);
                        }

                    let shell_sel = sel_registerName(b"onOpenContainerShell:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Shell",
                        Some(shell_sel),
                        Some(target),
                        true,
                        Some(&c.name),
                    );

                    let logs_sel = sel_registerName(b"onOpenContainerLogs:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Logs",
                        Some(logs_sel),
                        Some(target),
                        true,
                        Some(&c.name),
                    );

                    add_separator(sub);

                    let stop_sel = sel_registerName(b"onStopContainer:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Stop",
                        Some(stop_sel),
                        Some(target),
                        true,
                        Some(&c.name),
                    );

                    let restart_sel = sel_registerName(b"onRestartContainer:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Restart",
                        Some(restart_sel),
                        Some(target),
                        true,
                        Some(&c.name),
                    );
                } else {
                    let start_sel = sel_registerName(b"onStartContainer:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Start",
                        Some(start_sel),
                        Some(target),
                        true,
                        Some(&c.name),
                    );

                    let logs_sel = sel_registerName(b"onOpenContainerLogs:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Logs",
                        Some(logs_sel),
                        Some(target),
                        true,
                        Some(&c.name),
                    );
                }

                attach_submenu(parent_item, sub);
            }

            if total_containers > 5 {
                let more_title = format!("  + {} more...", total_containers - 5);
                let dash_sel = sel_registerName(b"onOpenDashboard:\0".as_ptr() as _);
                add_menu_item(menu, &more_title, Some(dash_sel), Some(target), true);
            }
        }
    }

    add_separator(menu);

        // ═════════════════════════════════════════════════════════════════════════
        // 3. KUBERNETES (Top 5 Workloads & Services)
        // ═════════════════════════════════════════════════════════════════════════
        add_menu_item(menu, "KUBERNETES", None, None, false);

        let is_starting_k8s = active_op
            .as_ref()
            .map(|o| o.description.contains("Kubernetes"))
            .unwrap_or(false)
            || snapshot.k8s_status_text.contains("Starting");

        if !snapshot.k8s_ready {
            let k8s_status_label = if is_starting_k8s {
                "Kubernetes: Starting...".to_string()
            } else {
                snapshot.k8s_status_text.clone()
            };
            let parent_item = add_menu_item(
                menu,
                &format!("  {}", k8s_status_label),
                None,
                None,
                true,
            );
            let sub = create_submenu("Kubernetes");
            let start_k8s_sel = sel_registerName(b"onStartK8s:\0".as_ptr() as _);
            add_menu_item(
                sub,
                "Start Kubernetes",
                Some(start_k8s_sel),
                Some(target),
                !is_starting_k8s,
            );
            let logs_sel = sel_registerName(b"onOpenK8sClusterLogs:\0".as_ptr() as _);
            add_menu_item(
                sub,
                "View Cluster Logs",
                Some(logs_sel),
                Some(target),
                true,
            );
            attach_submenu(parent_item, sub);
        } else {
            let parent_item = add_menu_item(menu, "  ● k3s cluster", None, None, true);
            let sub = create_submenu("Kubernetes");
            let logs_sel = sel_registerName(b"onOpenK8sClusterLogs:\0".as_ptr() as _);
            add_menu_item(sub, "View Cluster Logs", Some(logs_sel), Some(target), true);
            let restart_sel = sel_registerName(b"onRestartK8s:\0".as_ptr() as _);
            add_menu_item(sub, "Restart", Some(restart_sel), Some(target), !is_starting_k8s);
            let stop_sel = sel_registerName(b"onStopK8s:\0".as_ptr() as _);
            add_menu_item(sub, "Stop", Some(stop_sel), Some(target), !is_starting_k8s);
            attach_submenu(parent_item, sub);

            if snapshot.k8s_services.is_empty() && snapshot.k8s_pods.is_empty() {
                add_menu_item(menu, "  (no workloads running)", None, None, false);
            } else {
                let total_pods = snapshot.k8s_pods.len();
                for pod in snapshot.k8s_pods.iter().take(5) {
                    let bullet = match pod.phase.as_str() {
                        "Running" | "Succeeded" => "●",
                        "Pending" => "◐",
                        _ => "⚠️",
                    };
                    let name = truncate(&pod.name, 14);
                    let title = format!("  {} {} {}", bullet, name, pod.ready_summary);
                    let parent_item = add_menu_item(menu, &title, None, None, true);
                    let sub = create_submenu(&pod.name);

                    let target_str = format!("{}/{}", pod.namespace, pod.name);
                    let logs_sel = sel_registerName(b"onOpenK8sLogs:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Logs",
                        Some(logs_sel),
                        Some(target),
                        true,
                        Some(&target_str),
                    );

                    let shell_sel = sel_registerName(b"onOpenK8sShell:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Shell",
                        Some(shell_sel),
                        Some(target),
                        true,
                        Some(&target_str),
                    );

                    attach_submenu(parent_item, sub);
                }
                if total_pods > 5 {
                    let more_title = format!("  + {} more pods...", total_pods - 5);
                    let dash_sel = sel_registerName(b"onOpenDashboard:\0".as_ptr() as _);
                    add_menu_item(menu, &more_title, Some(dash_sel), Some(target), true);
                }

                let total_svcs = snapshot.k8s_services.len();
                for svc in snapshot.k8s_services.iter().take(5) {
                    let name = truncate(&svc.name, 14);
                    let title = if let Some(np) = svc.node_port {
                        format!("  ● {} :{} ➔ :{}", name, svc.port, np)
                    } else {
                        format!("  ● {} :{}", name, svc.port)
                    };

                    let parent_item = add_menu_item(menu, &title, None, None, true);
                    let sub = create_submenu(&svc.name);

                    if let Some(np) = svc.node_port {
                        let url = format!("http://localhost:{}", np);
                        let open_url_sel = sel_registerName(b"onOpenUrl:\0".as_ptr() as _);
                        let copy_sel = sel_registerName(b"onCopyText:\0".as_ptr() as _);

                        add_menu_item_with_obj(
                            sub,
                            &format!("Open :{}", np),
                            Some(open_url_sel),
                            Some(target),
                            true,
                            Some(&url),
                        );
                        add_menu_item_with_obj(
                            sub,
                            "Copy URL",
                            Some(copy_sel),
                            Some(target),
                            true,
                            Some(&url),
                        );
                        add_separator(sub);
                    }

                    let pf_target = format!("{}/{}", svc.name, svc.port);
                    let pf_sel = sel_registerName(b"onOpenK8sPortForward:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Port Forward",
                        Some(pf_sel),
                        Some(target),
                        true,
                        Some(&pf_target),
                    );

                    let pf_cmd = format!(
                        "kubectl port-forward svc/{} {}:{}",
                        svc.name, svc.port, svc.port
                    );
                    let copy_sel = sel_registerName(b"onCopyText:\0".as_ptr() as _);
                    add_menu_item_with_obj(
                        sub,
                        "Copy Cmd",
                        Some(copy_sel),
                        Some(target),
                        true,
                        Some(&pf_cmd),
                    );

                    attach_submenu(parent_item, sub);
                }
                if total_svcs > 5 {
                    let more_title = format!("  + {} more services...", total_svcs - 5);
                    let dash_sel = sel_registerName(b"onOpenDashboard:\0".as_ptr() as _);
                    add_menu_item(menu, &more_title, Some(dash_sel), Some(target), true);
                }
            }
        }

        add_separator(menu);

        // ═════════════════════════════════════════════════════════════════════════
        // 4. FOOTER
        // ═════════════════════════════════════════════════════════════════════════
        let dash_sel = sel_registerName(b"onOpenDashboard:\0".as_ptr() as _);
        add_menu_item(
            menu,
            "Dashboard",
            Some(dash_sel),
            Some(target),
            true,
        );

        let open_files_sel = sel_registerName(b"onOpenFiles:\0".as_ptr() as _);
        add_menu_item(
            menu,
            "Files",
            Some(open_files_sel),
            Some(target),
            true,
        );

        let restart_sel = sel_registerName(b"onRestartAll:\0".as_ptr() as _);
        add_menu_item(
            menu,
            "Restart All",
            Some(restart_sel),
            Some(target),
            true,
        );

        add_separator(menu);

        let ver_title = format!("mach v{}", env!("CARGO_PKG_VERSION"));
        add_menu_item(menu, &ver_title, None, None, false);
        let quit_sel = sel_registerName(b"onQuit:\0".as_ptr() as _);
        add_menu_item(menu, "Quit Mach", Some(quit_sel), Some(target), true);
    }

    extern "C" fn menu_needs_update(this: *mut c_void, _cmd: *mut c_void, menu: *mut c_void) {
        unsafe {
            let pool = objc_autoreleasePoolPush();
            populate_menu(menu, this);
            objc_autoreleasePoolPop(pool);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Action Callbacks
    // ─────────────────────────────────────────────────────────────────────────────

    extern "C" fn on_open_shell(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "mach".to_string());
        let script = format!(
            r#"tell application "Terminal"
    do script "{}"
    activate
end tell"#,
            exe
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }

    extern "C" fn on_open_machine_shell(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let machine_name = from_ns_string(obj);
            if !machine_name.is_empty() {
                let exe = std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "mach".to_string());
                let script = format!(
                    r#"tell application "Terminal"
    do script "{} shell {}"
    activate
end tell"#,
                    exe, machine_name
                );
                let _ = std::process::Command::new("osascript")
                    .arg("-e")
                    .arg(&script)
                    .spawn();
            }
        }
    }

    extern "C" fn on_open_files(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/Users"));
        let user_mach = home.join("Mach");
        let dot_mach = home.join(".mach");
        let target = if user_mach.exists() {
            user_mach
        } else {
            dot_mach
        };
        let _ = std::fs::create_dir_all(&target);
        let _ = std::process::Command::new("open").arg(&target).spawn();
    }

    extern "C" fn on_open_machine_files(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let machine_name = from_ns_string(obj);
            if !machine_name.is_empty() {
                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/Users"));
                let user_mach = home.join("Mach").join(&machine_name);
                let dot_mach = home.join(".mach").join("machines").join(&machine_name);
                let target = if user_mach.exists() {
                    user_mach
                } else if dot_mach.exists() {
                    dot_mach
                } else {
                    home.join("Mach")
                };
                let _ = std::fs::create_dir_all(&target);
                let _ = std::process::Command::new("open").arg(&target).spawn();
            }
        }
    }

    extern "C" fn on_copy_text(_this: *mut c_void, _cmd: *mut c_void, sender: *mut c_void) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let text = from_ns_string(obj);
            if !text.is_empty() {
                copy_to_clipboard(&text);
            }
        }
    }

    extern "C" fn on_open_url(_this: *mut c_void, _cmd: *mut c_void, sender: *mut c_void) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let url = from_ns_string(obj);
            if !url.is_empty() {
                let _ = std::process::Command::new("open").arg(&url).spawn();
            }
        }
    }

    extern "C" fn on_start_machine(_this: *mut c_void, _cmd: *mut c_void, sender: *mut c_void) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let name = from_ns_string(obj);
            if !name.is_empty() {
                set_active_op(&format!("Starting {name}"));
                std::thread::spawn(move || {
                    let exe = std::env::current_exe()
                        .unwrap_or_else(|_| std::path::PathBuf::from("mach"));
                    let st = std::process::Command::new(exe).args(["start", &name]).status();
                    if st.map(|s| s.success()).unwrap_or(false) {
                        send_macos_notification("Mach", &format!("Machine '{name}'"), "Virtual machine is now running and ready.");
                    } else {
                        send_macos_notification("Mach", &format!("Machine '{name}'"), "Failed to start virtual machine.");
                    }
                    clear_active_op();
                });
            }
        }
    }

    extern "C" fn on_stop_machine(_this: *mut c_void, _cmd: *mut c_void, sender: *mut c_void) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let name = from_ns_string(obj);
            if !name.is_empty() {
                set_active_op(&format!("Stopping {name}"));
                std::thread::spawn(move || {
                    let exe = std::env::current_exe()
                        .unwrap_or_else(|_| std::path::PathBuf::from("mach"));
                    let _ = std::process::Command::new(exe).args(["stop", &name]).status();
                    send_macos_notification("Mach", &format!("Machine '{name}'"), "Virtual machine stopped.");
                    clear_active_op();
                });
            }
        }
    }

    extern "C" fn on_restart_machine(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let name = from_ns_string(obj);
            if !name.is_empty() {
                set_active_op(&format!("Restarting {name}"));
                std::thread::spawn(move || {
                    let exe = std::env::current_exe()
                        .unwrap_or_else(|_| std::path::PathBuf::from("mach"));
                    let st = std::process::Command::new(exe)
                        .args(["restart", &name])
                        .status();
                    if st.map(|s| s.success()).unwrap_or(false) {
                        send_macos_notification("Mach", &format!("Machine '{name}'"), "Virtual machine restarted and ready.");
                    } else {
                        send_macos_notification("Mach", &format!("Machine '{name}'"), "Restart failed.");
                    }
                    clear_active_op();
                });
            }
        }
    }

    extern "C" fn on_open_container_logs(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let name = from_ns_string(obj);
            if !name.is_empty() {
                let script = format!(
                    r#"tell application "Terminal"
    do script "docker logs -f {}"
    activate
end tell"#,
                    name
                );
                let _ = std::process::Command::new("osascript")
                    .arg("-e")
                    .arg(&script)
                    .spawn();
            }
        }
    }

    extern "C" fn on_open_container_shell(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let name = from_ns_string(obj);
            if !name.is_empty() {
                let script = format!(
                    r#"tell application "Terminal"
    do script "docker exec -it {} sh"
    activate
end tell"#,
                    name
                );
                let _ = std::process::Command::new("osascript")
                    .arg("-e")
                    .arg(&script)
                    .spawn();
            }
        }
    }

    extern "C" fn on_stop_container(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let name = from_ns_string(obj);
            if !name.is_empty() {
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("docker")
                        .args(["stop", &name])
                        .status();
                });
            }
        }
    }

    extern "C" fn on_start_container(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let name = from_ns_string(obj);
            if !name.is_empty() {
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("docker")
                        .args(["start", &name])
                        .status();
                });
            }
        }
    }

    extern "C" fn on_restart_container(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let name = from_ns_string(obj);
            if !name.is_empty() {
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("docker")
                        .args(["restart", &name])
                        .status();
                });
            }
        }
    }

    fn resolve_machctl_bin() -> String {
        if let Ok(exe_path) = std::env::current_exe() {
            let sibling = exe_path.with_file_name("machctl");
            if sibling.exists() {
                return sibling.to_string_lossy().into_owned();
            }
        }
        "machctl".to_string()
    }

    extern "C" fn on_start_docker(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        set_active_op("Starting Docker Engine");
        std::thread::spawn(|| {
            let machctl_bin = resolve_machctl_bin();
            let st = std::process::Command::new(machctl_bin)
                .args(["docker", "start"])
                .status();
            if st.map(|s| s.success()).unwrap_or(false) {
                send_macos_notification("Mach", "Docker Engine", "Docker Engine is ready! Container runtime is active.");
            } else {
                send_macos_notification("Mach", "Docker Engine", "Failed to start Docker Engine. Check 'machctl docker logs'.");
            }
            clear_active_op();
        });
    }

    extern "C" fn on_stop_docker(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        set_active_op("Stopping Docker Engine");
        std::thread::spawn(|| {
            let machctl_bin = resolve_machctl_bin();
            let _ = std::process::Command::new(machctl_bin)
                .args(["docker", "stop"])
                .status();
            send_macos_notification("Mach", "Docker Engine", "Docker Engine stopped.");
            clear_active_op();
        });
    }

    extern "C" fn on_restart_docker(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        set_active_op("Restarting Docker Engine");
        std::thread::spawn(|| {
            let machctl_bin = resolve_machctl_bin();
            let st = std::process::Command::new(machctl_bin)
                .args(["docker", "restart"])
                .status();
            if st.map(|s| s.success()).unwrap_or(false) {
                send_macos_notification("Mach", "Docker Engine", "Docker Engine restarted and ready.");
            } else {
                send_macos_notification("Mach", "Docker Engine", "Docker restart encountered an error.");
            }
            clear_active_op();
        });
    }

    extern "C" fn on_open_docker_logs(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        let machctl_bin = resolve_machctl_bin();
        let script = format!(
            r#"tell application "Terminal"
    do script "{} docker logs -f"
    activate
end tell"#,
            machctl_bin
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }

    extern "C" fn on_start_k8s(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        set_active_op("Starting Kubernetes");
        std::thread::spawn(|| {
            let machctl_bin = resolve_machctl_bin();
            let st = std::process::Command::new(machctl_bin)
                .args(["k8s", "start"])
                .status();
            if st.map(|s| s.success()).unwrap_or(false) {
                send_macos_notification("Mach", "Kubernetes", "K3s cluster is active and ready for kubectl.");
            } else {
                send_macos_notification("Mach", "Kubernetes", "Failed to start Kubernetes cluster. Check 'machctl k8s logs'.");
            }
            clear_active_op();
        });
    }

    extern "C" fn on_stop_k8s(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        set_active_op("Stopping Kubernetes");
        std::thread::spawn(|| {
            let machctl_bin = resolve_machctl_bin();
            let _ = std::process::Command::new(machctl_bin)
                .args(["k8s", "stop"])
                .status();
            send_macos_notification("Mach", "Kubernetes", "Kubernetes cluster stopped.");
            clear_active_op();
        });
    }

    extern "C" fn on_restart_k8s(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        set_active_op("Restarting Kubernetes");
        std::thread::spawn(|| {
            let machctl_bin = resolve_machctl_bin();
            let st = std::process::Command::new(machctl_bin)
                .args(["k8s", "restart"])
                .status();
            if st.map(|s| s.success()).unwrap_or(false) {
                send_macos_notification("Mach", "Kubernetes", "Kubernetes cluster restarted and ready.");
            } else {
                send_macos_notification("Mach", "Kubernetes", "Kubernetes restart encountered an error.");
            }
            clear_active_op();
        });
    }

    extern "C" fn on_open_k8s_cluster_logs(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        let machctl_bin = resolve_machctl_bin();
        let script = format!(
            r#"tell application "Terminal"
    do script "{} k8s logs -f"
    activate
end tell"#,
            machctl_bin
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }

    extern "C" fn on_open_k8s_logs(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let target = from_ns_string(obj);
            if !target.is_empty() {
                let parts: Vec<&str> = target.split('/').collect();
                let (ns, pod) = if parts.len() == 2 {
                    (parts[0], parts[1])
                } else {
                    ("default", target.as_str())
                };
                let home = dirs::home_dir().unwrap_or_default();
                let conf = home.join(".kube").join("config.mach");
                let kube_flag = if conf.exists() {
                    format!("--kubeconfig {}", conf.display())
                } else {
                    String::new()
                };
                let script = format!(
                    r#"tell application "Terminal"
    do script "kubectl {} logs -f {} -n {}"
    activate
end tell"#,
                    kube_flag, pod, ns
                );
                let _ = std::process::Command::new("osascript")
                    .arg("-e")
                    .arg(&script)
                    .spawn();
            }
        }
    }

    extern "C" fn on_open_k8s_shell(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let target = from_ns_string(obj);
            if !target.is_empty() {
                let parts: Vec<&str> = target.split('/').collect();
                let (ns, pod) = if parts.len() == 2 {
                    (parts[0], parts[1])
                } else {
                    ("default", target.as_str())
                };
                let home = dirs::home_dir().unwrap_or_default();
                let conf = home.join(".kube").join("config.mach");
                let kube_flag = if conf.exists() {
                    format!("--kubeconfig {}", conf.display())
                } else {
                    String::new()
                };
                let script = format!(
                    r#"tell application "Terminal"
    do script "kubectl {} exec -it {} -n {} -- sh"
    activate
end tell"#,
                    kube_flag, pod, ns
                );
                let _ = std::process::Command::new("osascript")
                    .arg("-e")
                    .arg(&script)
                    .spawn();
            }
        }
    }

    extern "C" fn on_open_k8s_port_forward(
        _this: *mut c_void,
        _cmd: *mut c_void,
        sender: *mut c_void,
    ) {
        unsafe {
            let obj = msg_send_0(
                sender,
                sel_registerName(b"representedObject\0".as_ptr() as _),
            );
            let target = from_ns_string(obj);
            if !target.is_empty() {
                let parts: Vec<&str> = target.split('/').collect();
                if parts.len() == 2 {
                    let (svc, port) = (parts[0], parts[1]);
                    let home = dirs::home_dir().unwrap_or_default();
                    let conf = home.join(".kube").join("config.mach");
                    let kube_flag = if conf.exists() {
                        format!("--kubeconfig {}", conf.display())
                    } else {
                        String::new()
                    };
                    let script = format!(
                        r#"tell application "Terminal"
    do script "kubectl {} port-forward svc/{} {}:{}"
    activate
end tell"#,
                        kube_flag, svc, port, port
                    );
                    let _ = std::process::Command::new("osascript")
                        .arg("-e")
                        .arg(&script)
                        .spawn();
                }
            }
        }
    }

    extern "C" fn on_open_dashboard(
        _this: *mut c_void,
        _cmd: *mut c_void,
        _sender: *mut c_void,
    ) {
        let machctl_bin = resolve_machctl_bin();
        let script = format!(
            r#"tell application "Terminal"
    do script "{} top"
    activate
end tell"#,
            machctl_bin
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }

    extern "C" fn on_restart_all(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        set_active_op("Restarting All");
        std::thread::spawn(|| {
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("mach"));
            if let Ok(cfg) = MachConfig::load() {
                if let Ok(machines) = crate::commands::load_all_machines(&cfg.machines_dir) {
                    let running: Vec<_> = machines
                        .into_iter()
                        .filter(|m| m.state == MachineState::Running)
                        .collect();
                    if running.is_empty() {
                        let _ = std::process::Command::new(&exe).arg("restart").status();
                    } else {
                        for m in running {
                            let _ = std::process::Command::new(&exe)
                                .args(["restart", &m.config.name])
                                .status();
                        }
                    }
                    send_macos_notification("Mach", "All Services", "Restart completed.");
                    clear_active_op();
                    return;
                }
            }
            let _ = std::process::Command::new(&exe).arg("restart").status();
            send_macos_notification("Mach", "All Services", "Restart completed.");
            clear_active_op();
        });
    }

    extern "C" fn on_quit(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        // 1. Record running state for services and cleanly stop all machines
        if let Ok(mut cfg) = MachConfig::load() {
            if let Ok(machines) = crate::commands::load_all_machines(&cfg.machines_dir) {
                let docker_running = machines.iter().any(|m| {
                    (m.config.name == crate::commands::docker::SYSTEM_VM_NAME
                        || m.config.name == "mach-docker")
                        && (m.state == MachineState::Running
                            || crate::commands::find_running_vm_pid(&m.config.name).is_some())
                });
                let k8s_running = machines.iter().any(|m| {
                    (m.config.name == crate::commands::k8s::K8S_VM_NAME
                        || m.config.name == crate::commands::k8s::ALT_K8S_VM_NAME
                        || m.config.name == crate::commands::docker::SYSTEM_VM_NAME)
                        && (m.state == MachineState::Running
                            || crate::commands::find_running_vm_pid(&m.config.name).is_some())
                });
                cfg.docker.was_running = docker_running;
                cfg.k8s.was_running = k8s_running;
                let _ = cfg.save();

                for mut m in machines {
                    let _ = crate::commands::stop::stop_single_machine(&mut m, &cfg, true);
                }
            }
        }

        // 2. Terminate any remaining VM hypervisor processes
        crate::commands::kill_running_vm_processes(None);

        // 3. Terminate background tunnels and clean up socket files
        crate::commands::docker::terminate_docker_ssh_tunnel();
        crate::commands::k8s::terminate_k8s_ssh_tunnel();
        if let Ok(cfg) = MachConfig::load() {
            let _ = std::fs::remove_file(cfg.data_dir.join("run").join("docker.sock"));
            let _ = std::fs::remove_file(cfg.data_dir.join("run").join("menu.pid"));
        }

        // 4. Terminate NSApplication
        unsafe {
            let app = msg_send_0(
                objc_getClass(b"NSApplication\0".as_ptr() as _),
                sel_registerName(b"sharedApplication\0".as_ptr() as _),
            );
            if !app.is_null() {
                msg_send_1(
                    app,
                    sel_registerName(b"terminate:\0".as_ptr() as _),
                    std::ptr::null_mut(),
                );
            }
        }
    }

    static STATUS_BUTTON: std::sync::atomic::AtomicPtr<c_void> =
        std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

    unsafe fn update_status_item_state() {
        let button = STATUS_BUTTON.load(std::sync::atomic::Ordering::SeqCst);
        if button.is_null() {
            return;
        }

        let active_op = {
            let mut lock = ACTIVE_OP.write().unwrap();
            if let Some(ref op) = *lock {
                if op.started_at.elapsed() > std::time::Duration::from_secs(90) {
                    *lock = None;
                    None
                } else {
                    Some(op.clone())
                }
            } else {
                None
            }
        };

        let snapshot = {
            let lock = CACHED_SNAPSHOT.read().unwrap();
            lock.clone().unwrap_or_default()
        };

        let starting = snapshot.vms.iter().find(|m| {
            matches!(m.state, MachineState::Starting | MachineState::Creating)
        });

        let is_working = active_op.is_some()
            || starting.is_some()
            || snapshot.docker_status_text.contains("Starting")
            || snapshot.k8s_status_text.contains("Starting");

        let title = if is_working {
            " ⟳"
        } else {
            ""
        };

        let tooltip = if let Some(ref op) = active_op {
            format!("{}...", op.description)
        } else if let Some(m) = starting {
            format!("Starting {}...", m.name)
        } else if snapshot.docker_status_text.contains("Starting") {
            "Starting Docker Engine...".to_string()
        } else if snapshot.k8s_status_text.contains("Starting") {
            "Starting Kubernetes...".to_string()
        } else {
            format!("mach v{}", env!("CARGO_PKG_VERSION"))
        };

        msg_send_1(
            button,
            sel_registerName(b"setTitle:\0".as_ptr() as _),
            ns_string(title),
        );

        msg_send_1(
            button,
            sel_registerName(b"setToolTip:\0".as_ptr() as _),
            ns_string(&tooltip),
        );
    }

    extern "C" fn check_running_state(_this: *mut c_void, _cmd: *mut c_void, _timer: *mut c_void) {
        unsafe {
            update_status_item_state();
        }
    }

    fn find_menu_icon_path() -> Option<std::path::PathBuf> {
        for name in &[
            "assets/mach_menu_icon@2x.png",
            "assets/mach_menu_icon.png",
        ] {
            let p = std::path::PathBuf::from(name);
            if p.exists() {
                return Some(p);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let user_asset = home.join(".mach/assets/mach_menu_icon@2x.png");
            if user_asset.exists() {
                return Some(user_asset);
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(prefix) = exe.parent().and_then(|p| p.parent()) {
                let share_icon = prefix.join("share/mach/assets/mach_menu_icon@2x.png");
                if share_icon.exists() {
                    return Some(share_icon);
                }
            }
            if let Some(parent) = exe.parent() {
                let local_icon = parent.join("assets/mach_menu_icon@2x.png");
                if local_icon.exists() {
                    return Some(local_icon);
                }
            }
        }
        None
    }

    pub fn run_menu_bar() -> Result<()> {
        println!("\x1b[1;36m➜\x1b[0m Starting mach menu bar status item (\x1b[1mmach\x1b[0m)...");

        // 1. Initial snapshot polling
        let initial_snapshot = poll_menu_snapshot();
        if let Ok(mut lock) = CACHED_SNAPSHOT.write() {
            *lock = Some(initial_snapshot);
        }

        // 2. Background polling worker (refreshes snapshot every 2.0s without blocking UI)
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(2000));
                let snapshot = poll_menu_snapshot();
                if let Ok(mut lock) = CACHED_SNAPSHOT.write() {
                    *lock = Some(snapshot);
                }
            }
        });

        // 3. Restore Docker/Kubernetes if they were running in the previous session
        if let Ok(cfg) = MachConfig::load() {
            if cfg.docker.was_running || cfg.k8s.was_running {
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    if let Ok(rt) = rt {
                        let _ = rt.block_on(crate::commands::restore_services_if_needed(&cfg));
                    }
                });
            }
        }

        unsafe {
            let pool = objc_autoreleasePoolPush();

            // Initialize NSApplication
            let app_cls = objc_getClass(b"NSApplication\0".as_ptr() as _);
            if app_cls.is_null() {
                anyhow::bail!("NSApplication class unavailable");
            }
            let app = msg_send_0(
                app_cls,
                sel_registerName(b"sharedApplication\0".as_ptr() as _),
            );
            if app.is_null() {
                anyhow::bail!("Failed to get NSApplication shared instance");
            }

            // NSApplicationActivationPolicyAccessory = 1 (Runs in menu bar without Dock presence)
            msg_send_isize(
                app,
                sel_registerName(b"setActivationPolicy:\0".as_ptr() as _),
                1,
            );

            // Obtain NSStatusBar
            let status_bar_cls = objc_getClass(b"NSStatusBar\0".as_ptr() as _);
            if status_bar_cls.is_null() {
                anyhow::bail!("NSStatusBar class unavailable");
            }
            let status_bar = msg_send_0(
                status_bar_cls,
                sel_registerName(b"systemStatusBar\0".as_ptr() as _),
            );
            if status_bar.is_null() {
                anyhow::bail!("Failed to get systemStatusBar");
            }

            // Create NSStatusItem with length = -1.0 (NSVariableStatusItemLength)
            let status_item = msg_send_f64(
                status_bar,
                sel_registerName(b"statusItemWithLength:\0".as_ptr() as _),
                -1.0,
            );
            if status_item.is_null() {
                anyhow::bail!("Failed to allocate NSStatusItem");
            }
            msg_send_0(status_item, sel_registerName(b"retain\0".as_ptr() as _));

            // Set Title & Icon
            let button = msg_send_0(
                status_item,
                sel_registerName(b"button\0".as_ptr() as _),
            );
            if !button.is_null() {
                STATUS_BUTTON.store(button, std::sync::atomic::Ordering::SeqCst);
                let img_cls = objc_getClass(b"NSImage\0".as_ptr() as _);
                let mut image: *mut c_void = std::ptr::null_mut();

                if !img_cls.is_null() {
                    if let Some(icon_path) = find_menu_icon_path() {
                        if let Some(path_str) = icon_path.to_str() {
                            let path_ns = ns_string(path_str);
                            let img_alloc = msg_send_0(
                                img_cls,
                                sel_registerName(b"alloc\0".as_ptr() as _),
                            );
                            image = msg_send_1(
                                img_alloc,
                                sel_registerName(b"initWithContentsOfFile:\0".as_ptr() as _),
                                path_ns,
                            );
                        }
                    }

                    if image.is_null() && !EMBEDDED_MENU_ICON.is_empty() {
                        let data_cls = objc_getClass(b"NSData\0".as_ptr() as _);
                        if !data_cls.is_null() {
                            let sel_data =
                                sel_registerName(b"dataWithBytes:length:\0".as_ptr() as _);
                            let f_data: unsafe extern "C" fn(
                                *mut c_void,
                                *mut c_void,
                                *const u8,
                                usize,
                            ) -> *mut c_void =
                                std::mem::transmute(objc_msgSend as *const ());
                            let ns_data = f_data(
                                data_cls,
                                sel_data,
                                EMBEDDED_MENU_ICON.as_ptr(),
                                EMBEDDED_MENU_ICON.len(),
                            );
                            if !ns_data.is_null() {
                                let img_alloc = msg_send_0(
                                    img_cls,
                                    sel_registerName(b"alloc\0".as_ptr() as _),
                                );
                                image = msg_send_1(
                                    img_alloc,
                                    sel_registerName(b"initWithData:\0".as_ptr() as _),
                                    ns_data,
                                );
                            }
                        }
                    }
                }

                if !image.is_null() {
                    msg_send_size(
                        image,
                        sel_registerName(b"setSize:\0".as_ptr() as _),
                        NSSize {
                            width: 18.0,
                            height: 18.0,
                        },
                    );
                    msg_send_bool(
                        image,
                        sel_registerName(b"setTemplate:\0".as_ptr() as _),
                        1,
                    );
                    msg_send_1(
                        button,
                        sel_registerName(b"setImage:\0".as_ptr() as _),
                        image,
                    );
                    msg_send_1(
                        button,
                        sel_registerName(b"setTitle:\0".as_ptr() as _),
                        ns_string(""),
                    );
                } else {
                    msg_send_1(
                        button,
                        sel_registerName(b"setTitle:\0".as_ptr() as _),
                        ns_string("⚡️"),
                    );
                }
                let tip = format!("mach v{}", env!("CARGO_PKG_VERSION"));
                msg_send_1(
                    button,
                    sel_registerName(b"setToolTip:\0".as_ptr() as _),
                    ns_string(&tip),
                );
            }

            // Create Delegate and Menu
            let delegate_cls = get_delegate_class();
            if delegate_cls.is_null() {
                anyhow::bail!("Failed to register MachMenuDelegate class");
            }
            let delegate = msg_send_0(
                delegate_cls,
                sel_registerName(b"new\0".as_ptr() as _),
            );
            msg_send_0(delegate, sel_registerName(b"retain\0".as_ptr() as _));
            update_status_item_state();

            let menu_cls = objc_getClass(b"NSMenu\0".as_ptr() as _);
            if menu_cls.is_null() {
                anyhow::bail!("NSMenu class unavailable");
            }
            let menu = msg_send_0(
                menu_cls,
                sel_registerName(b"alloc\0".as_ptr() as _),
            );
            let menu = msg_send_1(
                menu,
                sel_registerName(b"initWithTitle:\0".as_ptr() as _),
                ns_string("mach"),
            );
            msg_send_0(menu, sel_registerName(b"retain\0".as_ptr() as _));

            msg_send_bool(
                menu,
                sel_registerName(b"setAutoenablesItems:\0".as_ptr() as _),
                0,
            );

            msg_send_1(
                menu,
                sel_registerName(b"setDelegate:\0".as_ptr() as _),
                delegate,
            );

            // Initial populate
            populate_menu(menu, delegate);

            // Periodic status update check
            let timer_cls = objc_getClass(b"NSTimer\0".as_ptr() as _);
            if !timer_cls.is_null() {
                let sel_timer = sel_registerName(
                    b"scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:\0".as_ptr()
                        as _,
                );
                let f_timer: unsafe extern "C" fn(
                    *mut c_void,
                    *mut c_void,
                    f64,
                    *mut c_void,
                    *mut c_void,
                    *mut c_void,
                    i8,
                ) -> *mut c_void = std::mem::transmute(objc_msgSend as *const ());
                f_timer(
                    timer_cls,
                    sel_timer,
                    1.5,
                    delegate,
                    sel_registerName(b"checkRunningState:\0".as_ptr() as _),
                    std::ptr::null_mut(),
                    1,
                );
            }

            // Attach menu to status item
            msg_send_1(
                status_item,
                sel_registerName(b"setMenu:\0".as_ptr() as _),
                menu,
            );

            println!("\x1b[1;32m✓\x1b[0m mach status item active in macOS menu bar.");
            println!("  Click \x1b[1mmach\x1b[0m in your top status bar to view running machines and controls.");

            objc_autoreleasePoolPop(pool);

            // Start Cocoa RunLoop
            msg_send_void(app, sel_registerName(b"run\0".as_ptr() as _));
        }

        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub fn run_menu_bar() -> Result<()> {
    imp::run_menu_bar()
}

#[cfg(target_os = "macos")]
pub(crate) use imp::{is_docker_socket_responding, is_k8s_api_responding};

#[cfg(not(target_os = "macos"))]
pub fn run_menu_bar() -> Result<()> {
    anyhow::bail!("mach Menu Bar is only supported on macOS");
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn is_docker_socket_responding(_cfg: Option<&mach_common::config::MachConfig>) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn is_k8s_api_responding() -> bool {
    false
}

