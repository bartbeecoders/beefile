//! Display helpers that do not touch the filesystem except for local time.

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

pub fn format_mtime(time: SystemTime) -> String {
    let Ok(dur) = time.duration_since(UNIX_EPOCH) else {
        return "-".into();
    };
    let secs = dur.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ptr = unsafe { libc::localtime_r(&secs, &mut tm) };
    if ptr.is_null() {
        return "-".into();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

pub fn display_path(path: &Path, home: &Path) -> String {
    if path == home {
        return "~".into();
    }
    if let Ok(rest) = path.strip_prefix(home) {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub path: PathBuf,
}

/// Breadcrumb segments. Under `$HOME` the first crumb is `~`.
pub fn crumbs(path: &Path, home: &Path) -> Vec<Crumb> {
    let under_home = path.starts_with(home);
    let mut out = Vec::new();
    let mut acc = if under_home {
        out.push(Crumb {
            label: "~".into(),
            path: home.to_path_buf(),
        });
        home.to_path_buf()
    } else {
        out.push(Crumb {
            label: "/".into(),
            path: PathBuf::from("/"),
        });
        PathBuf::from("/")
    };

    let relative = if under_home {
        path.strip_prefix(home).unwrap_or(path)
    } else {
        path
    };

    for component in relative.components() {
        if let Component::Normal(name) = component {
            acc.push(name);
            out.push(Crumb {
                label: name.to_string_lossy().into_owned(),
                path: acc.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sizes_use_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn mtime_shape() {
        let text = format_mtime(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        assert_eq!(text.len(), 16, "{text}");
        assert_eq!(&text[4..5], "-");
        assert_eq!(&text[10..11], " ");
    }

    #[test]
    fn paths_and_crumbs_fold_home() {
        let home = Path::new("/home/bart");
        assert_eq!(display_path(home, home), "~");
        assert_eq!(
            display_path(Path::new("/home/bart/Documents"), home),
            "~/Documents"
        );
        assert_eq!(display_path(Path::new("/etc"), home), "/etc");

        let home_crumbs = crumbs(Path::new("/home/bart/Documents/notes"), home);
        let labels: Vec<_> = home_crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["~", "Documents", "notes"]);
        assert_eq!(home_crumbs[1].path, Path::new("/home/bart/Documents"));

        let root = crumbs(Path::new("/run/media"), home);
        let labels: Vec<_> = root.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["/", "run", "media"]);
    }
}
