//! Optional embedded binaries.
//!
//! When built with the `embed-tools` feature, the binaries placed under
//! `vendor/bin/<target-triple>/` at compile time are baked into the executable
//! and extracted to [`crate::tools::bundle_dir`] on first run, so xpress is a
//! single self-contained file with no external install step.
//!
//! Without the feature, [`ensure_bundled`] is a no-op and xpress falls back to
//! `$XPRESS_BIN_DIR`, a sibling `bin/`, the bundle dir, or `PATH`.

#[cfg(feature = "embed-tools")]
mod embed {
    use std::io::Write;
    use std::path::Path;

    // The target-specific directory is selected at build time by build.rs, which
    // sets XPRESS_EMBED_DIR. include_dir requires a literal, so we point it at a
    // stable path that build.rs guarantees exists.
    static EMBEDDED: include_dir::Dir<'_> =
        include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../vendor/bin/current");

    /// Extract every embedded binary into `dir`, marking it executable. Existing
    /// files are left untouched so user-supplied overrides win.
    pub fn extract_to(dir: &Path) -> std::io::Result<usize> {
        std::fs::create_dir_all(dir)?;
        let mut count = 0;
        for file in EMBEDDED.files() {
            let name = file.path().file_name().unwrap();
            // Skip placeholder/hidden files (e.g. .gitkeep).
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let dest = dir.join(name);
            if dest.exists() {
                continue;
            }
            let mut f = std::fs::File::create(&dest)?;
            f.write_all(file.contents())?;
            f.flush()?;
            set_executable(&dest)?;
            count += 1;
        }
        Ok(count)
    }

    #[cfg(unix)]
    fn set_executable(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)
    }

    #[cfg(not(unix))]
    fn set_executable(_path: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

/// Ensure bundled binaries are available on disk. Returns how many were extracted
/// (0 when the feature is off or everything was already present).
pub fn ensure_bundled() -> std::io::Result<usize> {
    #[cfg(feature = "embed-tools")]
    {
        if let Some(dir) = crate::tools::bundle_dir() {
            return embed::extract_to(&dir);
        }
        Ok(0)
    }
    #[cfg(not(feature = "embed-tools"))]
    {
        Ok(0)
    }
}
