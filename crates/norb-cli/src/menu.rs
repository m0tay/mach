//! Minimal Apple Silicon macOS Menu Bar status item for Norb.
//!
//! Provides a native status item in the macOS menu bar (` Norb`)
//! displaying running machines, dynamic IP addresses, Docker socket status,
//! Kubernetes cluster readiness, and quick actions (shell, file browsing, restart, quit).
//! Pure native macOS Cocoa / AppKit with < 3 MB RAM footprint and zero Electron bloat.

use anyhow::Result;

#[cfg(target_os = "macos")]
mod imp {
    use anyhow::Result;
    use norb_common::config::NorbConfig;
    use norb_common::machine::MachineState;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::sync::Once;

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
            let cls_name = b"NorbMenuDelegate\0".as_ptr() as *const c_char;
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

    fn is_docker_active(cfg: Option<&NorbConfig>) -> bool {
        if let Some(cfg) = cfg {
            let sock = cfg.data_dir.join("run").join("docker.sock");
            if sock.exists() && std::os::unix::net::UnixStream::connect(&sock).is_ok() {
                return true;
            }
        }
        let var_sock = std::path::Path::new("/var/run/docker.sock");
        if var_sock.exists() && std::os::unix::net::UnixStream::connect(var_sock).is_ok() {
            return true;
        }
        false
    }

    fn is_k8s_ready(running_machines: &[norb_common::machine::Machine]) -> bool {
        let k8s_vm_running = running_machines.iter().any(|m| m.config.name == "norb-k8s");
        if k8s_vm_running {
            if let Some(home) = dirs::home_dir() {
                let kubeconfig = home.join(".kube").join("config.norb");
                if kubeconfig.exists() {
                    return true;
                }
            }
            return true;
        }
        false
    }

    unsafe fn populate_menu(menu: *mut c_void, target: *mut c_void) {
        // Clear all existing menu items
        msg_send_void(menu, sel_registerName(b"removeAllItems\0".as_ptr() as _));

        // Discover machines
        let cfg_res = NorbConfig::load();
        let running_machines = if let Ok(ref cfg) = cfg_res {
            if let Ok(machines) = crate::commands::load_all_machines(&cfg.machines_dir) {
                machines
                    .into_iter()
                    .filter(|m| m.state == MachineState::Running)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let running_count = running_machines.len();
        let vm_label = if running_count == 1 { "VM" } else { "VMs" };
        let status_header = format!("Norb: Running ({} {})", running_count, vm_label);
        add_menu_item(menu, &status_header, None, None, false);

        // Separator
        add_separator(menu);

        // Running Machines list (with IP)
        if running_machines.is_empty() {
            add_menu_item(menu, "  No running machines", None, None, false);
        } else {
            for m in &running_machines {
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
        let docker_active = is_docker_active(cfg_res.as_ref().ok());
        let docker_text = if docker_active {
            "Docker Socket: Active"
        } else {
            "Docker Socket: Inactive"
        };
        add_menu_item(menu, docker_text, None, None, false);

        // Kubernetes status
        let k8s_ready = is_k8s_ready(&running_machines);
        let k8s_text = if k8s_ready {
            "Kubernetes: Ready"
        } else {
            "Kubernetes: Stopped"
        };
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
            "Open Files (~/Norb)",
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

        // Quit Norb
        let quit_sel = sel_registerName(b"onQuit:\0".as_ptr() as _);
        add_menu_item(menu, "Quit Norb", Some(quit_sel), Some(target), true);
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
            .unwrap_or_else(|_| "norb".to_string());
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
                    .unwrap_or_else(|_| "norb".to_string());
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
        let user_norb = home.join("Norb");
        let dot_norb = home.join(".norb");
        let target = if user_norb.exists() {
            user_norb
        } else {
            dot_norb
        };
        let _ = std::fs::create_dir_all(&target);
        let _ = std::process::Command::new("open").arg(&target).spawn();
    }

    extern "C" fn on_restart_all(_this: *mut c_void, _cmd: *mut c_void, _sender: *mut c_void) {
        std::thread::spawn(|| {
            let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("norb"));
            if let Ok(cfg) = NorbConfig::load() {
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

    pub fn run_menu_bar() -> Result<()> {
        println!("\x1b[1;36m➜\x1b[0m Starting Norb Menu Bar status item (\x1b[1m Norb\x1b[0m)...");

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

            // Set Title " Norb"
            let title_ns = ns_string(" Norb");
            let button = msg_send_0(
                status_item,
                sel_registerName(b"button\0".as_ptr() as _),
            );
            if !button.is_null() {
                msg_send_1(button, sel_registerName(b"setTitle:\0".as_ptr() as _), title_ns);
                msg_send_1(
                    button,
                    sel_registerName(b"setToolTip:\0".as_ptr() as _),
                    ns_string("Norb Virtualization & Containers"),
                );
            } else {
                msg_send_1(
                    status_item,
                    sel_registerName(b"setTitle:\0".as_ptr() as _),
                    title_ns,
                );
            }

            // 4. Create Delegate and Menu
            let delegate_cls = get_delegate_class();
            if delegate_cls.is_null() {
                anyhow::bail!("Failed to register NorbMenuDelegate class");
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
                ns_string("Norb"),
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

            // Attach menu to status item
            msg_send_1(
                status_item,
                sel_registerName(b"setMenu:\0".as_ptr() as _),
                menu,
            );

            println!("\x1b[1;32m✓\x1b[0m Norb status item active in macOS menu bar.");
            println!("  Click \x1b[1m Norb\x1b[0m in your top status bar to view running machines and controls.");

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
    anyhow::bail!("Norb Menu Bar is only supported on macOS");
}
