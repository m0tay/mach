//! Minimal Apple Silicon macOS Menu Bar status item for mach.
//!
//! Provides a native status item in the macOS menu bar (` mach`)
//! displaying running machines, dynamic IP addresses, Docker socket status,
//! Kubernetes cluster readiness, and quick actions (shell, file browsing, restart, quit).
//! Pure native macOS Cocoa / AppKit with < 3 MB RAM footprint and zero Electron bloat.

use anyhow::Result;

#[cfg(target_os = "macos")]
mod imp {
    use anyhow::Result;
    use mach_common::config::MachConfig;
    use mach_common::machine::MachineState;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::sync::{Once, OnceLock};

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
                    class_addMethod(
                        cls,
                        sel_registerName(b"menuNeedsUpdate:\0".as_ptr() as _),
                        menu_needs_update as *const c_void,
                        types,
                    );
                    class_addMethod(
                        cls,
                        sel_registerName(b"onOpenShell:\0".as_ptr() as _),
                        on_open_shell as *const c_void,
                        types,
                    );
                    class_addMethod(
                        cls,
                        sel_registerName(b"onOpenMachineShell:\0".as_ptr() as _),
                        on_open_machine_shell as *const c_void,
                        types,
                    );
                    class_addMethod(
                        cls,
                        sel_registerName(b"onOpenFiles:\0".as_ptr() as _),
                        on_open_files as *const c_void,
                        types,
                    );
                    class_addMethod(
                        cls,
                        sel_registerName(b"onRestartAll:\0".as_ptr() as _),
                        on_restart_all as *const c_void,
                        types,
                    );
                    class_addMethod(
                        cls,
                        sel_registerName(b"onQuit:\0".as_ptr() as _),
                        on_quit as *const c_void,
                        types,
                    );
                    class_addMethod(
                        cls,
                        sel_registerName(b"checkRunningState:\0".as_ptr() as _),
                        check_running_state as *const c_void,
                        types,
                    );
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

    fn is_system_machine(m: &mach_common::machine::Machine) -> bool {
        m.is_system()
            || m.config.name == "mach-docker"
            || m.config.name == "mach-kubernetes"
            || m.config.name == "mach-k8s"
    }

    fn is_docker_socket_responding(cfg: Option<&MachConfig>) -> bool {
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
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(250)));
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(250)));
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

    fn is_k8s_api_responding() -> bool {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 26443));
        let timeout = std::time::Duration::from_millis(250);
        std::net::TcpStream::connect_timeout(&addr, timeout).is_ok()
    }

    fn get_docker_status(
        all_machines: &[mach_common::machine::Machine],
        cfg: Option<&MachConfig>,
    ) -> &'static str {
        let docker_vm = all_machines.iter().find(|m| m.config.name == "mach-docker");
        match docker_vm {
            None => "Docker: Not installed",
            Some(m) => match &m.state {
                MachineState::Error(_) => "Docker: Error",
                MachineState::Stopped | MachineState::Stopping => "Docker: Stopped",
                MachineState::Creating | MachineState::Starting => "Docker: Starting...",
                MachineState::Running => {
                    if is_docker_socket_responding(cfg) {
                        "Docker: Ready"
                    } else {
                        "Docker: Starting..."
                    }
                }
            },
        }
    }

    fn get_k8s_status(all_machines: &[mach_common::machine::Machine]) -> &'static str {
        let k8s_vm = all_machines.iter().find(|m| {
            m.config.name == "mach-kubernetes"
                || m.config.name == "mach-k8s"
        });
        match k8s_vm {
            None => "Kubernetes: Not installed",
            Some(m) => match &m.state {
                MachineState::Error(_) => "Kubernetes: Error",
                MachineState::Stopped | MachineState::Stopping => "Kubernetes: Stopped",
                MachineState::Creating | MachineState::Starting => "Kubernetes: Starting...",
                MachineState::Running => {
                    if is_k8s_api_responding() {
                        "Kubernetes: Ready"
                    } else {
                        "Kubernetes: Starting..."
                    }
                }
            },
        }
    }

    unsafe fn populate_menu(menu: *mut c_void, target: *mut c_void) {
        // Clear all existing menu items
        msg_send_void(menu, sel_registerName(b"removeAllItems\0".as_ptr() as _));

        // Discover machines
        let cfg_res = MachConfig::load();
        let all_machines = if let Ok(ref cfg) = cfg_res {
            crate::commands::load_all_machines(&cfg.machines_dir).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Filter out any system infrastructure machines (mach-docker, mach-kubernetes, is_system())
        let user_machines: Vec<_> = all_machines
            .iter()
            .filter(|m| !is_system_machine(m) && m.state == MachineState::Running)
            .collect();

        let running_count = user_machines.len();
        let vm_label = if running_count == 1 { "VM" } else { "VMs" };
        let status_header = format!("Mach: Running ({} {})", running_count, vm_label);
        add_menu_item(menu, &status_header, None, None, false);

        // Separator
        add_separator(menu);

        // Running User Machines list (with IP)
        if user_machines.is_empty() {
            add_menu_item(menu, "  No running user machines", None, None, false);
        } else {
            for m in &user_machines {
                let ip_str = m
                    .ip_address
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "no IP".to_string());
                let title = format!("  ● {} ({})", m.config.name, ip_str);
                let action = sel_registerName(b"onOpenMachineShell:\0".as_ptr() as _);
                add_menu_item_with_obj(
                    menu,
                    &title,
                    Some(action),
                    Some(target),
                    true,
                    Some(&m.config.name),
                );
            }
        }

        // Docker Socket status
        let docker_text = get_docker_status(&all_machines, cfg_res.as_ref().ok());
        add_menu_item(menu, docker_text, None, None, false);

        // Kubernetes status
        let k8s_text = get_k8s_status(&all_machines);
        add_menu_item(menu, k8s_text, None, None, false);

        // Separator
        add_separator(menu);

        // Quick Actions
        let open_shell_sel = sel_registerName(b"onOpenShell:\0".as_ptr() as _);
        add_menu_item(
            menu,
            "Open Shell (Default VM)",
            Some(open_shell_sel),
            Some(target),
            true,
        );

        let open_files_sel = sel_registerName(b"onOpenFiles:\0".as_ptr() as _);
        add_menu_item(
            menu,
            "Open Files (~/Mach)",
            Some(open_files_sel),
            Some(target),
            true,
        );

        let restart_sel = sel_registerName(b"onRestartAll:\0".as_ptr() as _);
        add_menu_item(
            menu,
            "Restart All Machines",
            Some(restart_sel),
            Some(target),
            true,
        );

        // Separator
        add_separator(menu);

        // Discrete version display (non-clickable)
        let version_label = format!("mach v{}", env!("CARGO_PKG_VERSION"));
        add_menu_item(menu, &version_label, None, None, false);

        // Quit Mach
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

    extern "C" fn on_restart_all(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        std::thread::spawn(|| {
            let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("mach"));
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
                    return;
                }
            }
            let _ = std::process::Command::new(&exe).arg("restart").status();
        });
    }

    extern "C" fn on_quit(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        // 1. Cleanly stop all machines (both user and system)
        if let Ok(cfg) = MachConfig::load() {
            if let Ok(machines) = crate::commands::load_all_machines(&cfg.machines_dir) {
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

    static MENU_START_TIME: OnceLock<std::time::Instant> = OnceLock::new();

    extern "C" fn check_running_state(_this: *mut c_void, _cmd: *mut c_void, _timer: *mut c_void) {
        let start = MENU_START_TIME.get_or_init(std::time::Instant::now);
        // 3-second grace period on launch
        if start.elapsed().as_millis() < 3000 {
            return;
        }

        if let Ok(cfg) = MachConfig::load() {
            let has_running = crate::commands::load_all_machines(&cfg.machines_dir)
                .map(|ms| {
                    ms.iter().any(|m| {
                        m.state == MachineState::Running
                            || m.state == MachineState::Starting
                            || crate::commands::find_running_vm_pid(&m.config.name).is_some()
                    })
                })
                .unwrap_or(false);

            if !has_running {
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
        }
    }

    fn find_menu_icon_path() -> Option<std::path::PathBuf> {
        // 1. Current working directory or dev repo
        for name in &["assets/mach_menu_icon@2x.png", "assets/mach_menu_icon.png"] {
            let p = std::path::PathBuf::from(name);
            if p.exists() {
                return Some(p);
            }
        }
        // 2. ~/.mach/assets
        if let Some(home) = dirs::home_dir() {
            let user_asset = home.join(".mach/assets/mach_menu_icon@2x.png");
            if user_asset.exists() {
                return Some(user_asset);
            }
        }
        // 3. Executable relative (Homebrew /opt/homebrew/share/mach or /usr/local/share/mach)
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

        unsafe {
            let pool = objc_autoreleasePoolPush();

            // 1. Initialize NSApplication
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

            // 2. Obtain NSStatusBar
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

            // 3. Create NSStatusItem with length = -1.0 (NSVariableStatusItemLength)
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
                let img_cls = objc_getClass(b"NSImage\0".as_ptr() as _);
                let mut image: *mut c_void = std::ptr::null_mut();

                if !img_cls.is_null() {
                    // 1. Try file path if available
                    if let Some(icon_path) = find_menu_icon_path() {
                        if let Some(path_str) = icon_path.to_str() {
                            let path_ns = ns_string(path_str);
                            let img_alloc = msg_send_0(img_cls, sel_registerName(b"alloc\0".as_ptr() as _));
                            image = msg_send_1(img_alloc, sel_registerName(b"initWithContentsOfFile:\0".as_ptr() as _), path_ns);
                        }
                    }

                    // 2. Fall back to embedded high-resolution retina icon
                    if image.is_null() && !EMBEDDED_MENU_ICON.is_empty() {
                        let data_cls = objc_getClass(b"NSData\0".as_ptr() as _);
                        if !data_cls.is_null() {
                            let sel_data = sel_registerName(b"dataWithBytes:length:\0".as_ptr() as _);
                            let f_data: unsafe extern "C" fn(*mut c_void, *mut c_void, *const u8, usize) -> *mut c_void =
                                std::mem::transmute(objc_msgSend as *const ());
                            let ns_data = f_data(data_cls, sel_data, EMBEDDED_MENU_ICON.as_ptr(), EMBEDDED_MENU_ICON.len());
                            if !ns_data.is_null() {
                                let img_alloc = msg_send_0(img_cls, sel_registerName(b"alloc\0".as_ptr() as _));
                                image = msg_send_1(img_alloc, sel_registerName(b"initWithData:\0".as_ptr() as _), ns_data);
                            }
                        }
                    }
                }

                if !image.is_null() {
                    // Set explicit logical point size for macOS status item (18.0 x 18.0 pt)
                    msg_send_size(image, sel_registerName(b"setSize:\0".as_ptr() as _), NSSize { width: 18.0, height: 18.0 });
                    // setTemplate:YES allows macOS to adapt the monochrome stencil for dark/light mode
                    msg_send_bool(image, sel_registerName(b"setTemplate:\0".as_ptr() as _), 1);
                    msg_send_1(button, sel_registerName(b"setImage:\0".as_ptr() as _), image);
                    msg_send_1(button, sel_registerName(b"setTitle:\0".as_ptr() as _), ns_string(""));
                } else {
                    msg_send_1(button, sel_registerName(b"setTitle:\0".as_ptr() as _), ns_string("⚡️"));
                }
                let tip = format!("mach v{} (Virtualization & Containers)", env!("CARGO_PKG_VERSION"));
                msg_send_1(
                    button,
                    sel_registerName(b"setToolTip:\0".as_ptr() as _),
                    ns_string(&tip),
                );
            } else {
                msg_send_1(
                    status_item,
                    sel_registerName(b"setTitle:\0".as_ptr() as _),
                    ns_string("mach"),
                );
            }

            // 4. Create Delegate and Menu
            let delegate_cls = get_delegate_class();
            if delegate_cls.is_null() {
                anyhow::bail!("Failed to register MachMenuDelegate class");
            }
            let delegate = msg_send_0(delegate_cls, sel_registerName(b"new\0".as_ptr() as _));
            msg_send_0(delegate, sel_registerName(b"retain\0".as_ptr() as _));

            let menu_cls = objc_getClass(b"NSMenu\0".as_ptr() as _);
            if menu_cls.is_null() {
                anyhow::bail!("NSMenu class unavailable");
            }
            let menu = msg_send_0(menu_cls, sel_registerName(b"alloc\0".as_ptr() as _));
            let menu = msg_send_1(
                menu,
                sel_registerName(b"initWithTitle:\0".as_ptr() as _),
                ns_string("mach"),
            );
            msg_send_0(menu, sel_registerName(b"retain\0".as_ptr() as _));

            // Disable autoenablesItems so enabled state is respected
            msg_send_bool(
                menu,
                sel_registerName(b"setAutoenablesItems:\0".as_ptr() as _),
                0,
            );

            // Wire delegate for dynamic menu updates
            msg_send_1(
                menu,
                sel_registerName(b"setDelegate:\0".as_ptr() as _),
                delegate,
            );

            // Initial populate
            populate_menu(menu, delegate);

            // Schedule periodic auto-terminate check (exits if all machines are stopped)
            let timer_cls = objc_getClass(b"NSTimer\0".as_ptr() as _);
            if !timer_cls.is_null() {
                let sel_timer = sel_registerName(
                    b"scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:\0".as_ptr() as _,
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
                    1.5, // Check every 1.5s
                    delegate,
                    sel_registerName(b"checkRunningState:\0".as_ptr() as _),
                    std::ptr::null_mut(),
                    1, // repeats: YES
                );
            }

            // Attach menu to status item
            msg_send_1(
                status_item,
                sel_registerName(b"setMenu:\0".as_ptr() as _),
                menu,
            );

            println!("\x1b[1;32m✓\x1b[0m mach status item active in macOS menu bar.");
            println!("  Click \x1b[1m mach\x1b[0m in your top status bar to view running machines and controls.");

            objc_autoreleasePoolPop(pool);

            // 5. Start Cocoa RunLoop
            msg_send_void(app, sel_registerName(b"run\0".as_ptr() as _));
        }

        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub fn run_menu_bar() -> Result<()> {
    imp::run_menu_bar()
}

#[cfg(not(target_os = "macos"))]
pub fn run_menu_bar() -> Result<()> {
    anyhow::bail!("mach Menu Bar is only supported on macOS");
}
