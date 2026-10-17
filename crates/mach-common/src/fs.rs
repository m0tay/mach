//! Filesystem utilities, including APFS Copy-on-Write (CoW) cloning.

use std::path::Path;

/// Clones a file using macOS APFS Copy-on-Write (`clonefile`).
///
/// If `clonefile` is unavailable or fails (e.g., cross-device copy or unsupported filesystem),
/// automatically falls back to standard file copy (`std::fs::copy`).
pub fn apfs_clone(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let src_c = CString::new(src.as_os_str().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let dst_c = CString::new(dst.as_os_str().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let ret = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
        if ret == 0 {
            return Ok(());
        }
    }

    // Fallback to standard copy
    std::fs::copy(src, dst).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apfs_clone() {
        let temp_dir = std::env::temp_dir().join(format!("mach_test_apfs_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let src = temp_dir.join("test_src.txt");
        let dst = temp_dir.join("test_dst.txt");

        std::fs::write(&src, b"hello mach apfs").unwrap();
        apfs_clone(&src, &dst).unwrap();

        let content = std::fs::read(&dst).unwrap();
        assert_eq!(content, b"hello mach apfs");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
