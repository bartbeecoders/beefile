//! Filesystem operations. Directory reads stay off the UI thread; callers decide when.

use crate::model::{Entry, Kind};
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn launch_target(arg: Option<PathBuf>) -> (PathBuf, Option<String>) {
    let home = home_dir();
    let raw = arg.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| home.clone()));
    let expanded = expand_input(
        &raw.to_string_lossy(),
        &std::env::current_dir().unwrap_or_else(|_| home.clone()),
        &home,
    );
    match resolve_start(&expanded) {
        Ok(target) => target,
        Err(_) => (home, None),
    }
}

pub fn resolve_start(path: &Path) -> io::Result<(PathBuf, Option<String>)> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        let canon = fs::canonicalize(path)?;
        return resolve_start(&canon);
    }
    if meta.is_file() {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("/"));
        let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        return Ok((parent, Some(name)));
    }
    if meta.is_dir() {
        let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        return Ok((canon, None));
    }
    Err(io::Error::new(
        ErrorKind::InvalidInput,
        "not a file or directory",
    ))
}

pub fn expand_input(text: &str, cwd: &Path, home: &Path) -> PathBuf {
    let text = text.trim();
    if text.is_empty() {
        return cwd.to_path_buf();
    }
    let path = if text == "~" {
        home.to_path_buf()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else {
        let typed = PathBuf::from(text);
        if typed.is_absolute() {
            typed
        } else {
            cwd.join(typed)
        }
    };
    normalize(&path)
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => out.push(name),
            Component::Prefix(_) => {}
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

/// Refuse to destroy `/`, the home directory, or any top-level directory.
pub fn is_protected(path: &Path) -> bool {
    let candidate = fs::canonicalize(path).unwrap_or_else(|_| normalize(path));
    if candidate == Path::new("/") {
        return true;
    }
    let home = home_dir();
    if candidate == home {
        return true;
    }
    candidate.parent() == Some(Path::new("/"))
}

pub fn ensure_mutable(path: &Path) -> Result<(), String> {
    if is_protected(path) {
        Err(format!("Refusing to change {}", path.display()))
    } else {
        Ok(())
    }
}

pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub free: Option<u64>,
}

pub fn list_dir(cwd: &Path) -> Result<Snapshot, String> {
    let dir = fs::read_dir(cwd).map_err(|err| io_msg("Reading", cwd, err))?;
    let mut entries = Vec::new();
    for item in dir {
        let item = match item {
            Ok(item) => item,
            Err(err) => return Err(io_msg("Reading", cwd, err)),
        };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => {
                entries.push(Entry {
                    name,
                    path,
                    kind: Kind::Other,
                    size: None,
                    modified: None,
                    link_target: None,
                });
                continue;
            }
        };
        let file_type = meta.file_type();
        let (kind, link_target) = if file_type.is_symlink() {
            (Kind::Symlink, fs::read_link(&path).ok())
        } else if file_type.is_dir() {
            (Kind::Dir, None)
        } else if file_type.is_file() {
            (Kind::File, None)
        } else {
            (Kind::Other, None)
        };
        let size = if kind == Kind::Dir {
            None
        } else {
            Some(meta.len())
        };
        let modified = meta.modified().ok();
        entries.push(Entry {
            name,
            path,
            kind,
            size,
            modified,
            link_target,
        });
    }
    Ok(Snapshot {
        entries,
        free: free_space(cwd),
    })
}

pub fn free_space(path: &Path) -> Option<u64> {
    let text = path.as_os_str().to_string_lossy();
    let c = std::ffi::CString::new(text.as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

pub fn create_file(path: &Path) -> Result<(), String> {
    ensure_mutable(path)?;
    if exists(path) {
        return Err(format!("{} already exists", path.display()));
    }
    File::create(path)
        .map(|_| ())
        .map_err(|err| io_msg("Creating", path, err))
}

pub fn create_dir(path: &Path) -> Result<(), String> {
    ensure_mutable(path)?;
    if exists(path) {
        return Err(format!("{} already exists", path.display()));
    }
    fs::create_dir(path).map_err(|err| io_msg("Creating", path, err))
}

pub fn rename(from: &Path, to: &Path) -> Result<(), String> {
    ensure_mutable(from)?;
    ensure_mutable(to)?;
    if exists(to) {
        return Err(format!("{} already exists", to.display()));
    }
    fs::rename(from, to).map_err(|err| io_msg("Renaming", from, err))
}

/// Move `sources` into `dest`, or copy them when `copy` is set.
/// A move onto the folder that already contains the file is skipped.
pub fn drop_into(sources: &[PathBuf], dest: &Path, copy: bool) -> Result<String, String> {
    ensure_mutable(dest)?;
    if !dest.is_dir() {
        return Err(format!("Not a folder: {}", dest.display()));
    }
    let mut done = 0usize;
    for src in sources {
        if !exists(src) {
            return Err(format!("Missing {}", src.display()));
        }
        if src == dest || is_inside(src, dest) {
            return Err("Cannot drop a folder into itself".into());
        }
        if !copy && src.parent() == Some(dest) {
            continue;
        }
        if copy {
            copy_into(src, dest)?;
        } else {
            move_into(src, dest)?;
        }
        done += 1;
    }
    if done == 0 {
        Err("Already in that folder".into())
    } else if copy {
        Ok(format!("Copied {done}"))
    } else {
        Ok(format!("Moved {done}"))
    }
}

pub fn copy_into(src: &Path, dest_dir: &Path) -> Result<(), String> {
    let name = file_name(src)?;
    let dest = dest_dir.join(name);
    copy_path(src, &dest)
}

pub fn move_into(src: &Path, dest_dir: &Path) -> Result<(), String> {
    ensure_mutable(src)?;
    let name = file_name(src)?;
    let dest = dest_dir.join(name);
    ensure_mutable(&dest)?;
    if exists(&dest) {
        return Err(format!("{} already exists", dest.display()));
    }
    if is_inside(src, dest_dir) {
        return Err("Cannot move a directory into itself".into());
    }
    match fs::rename(src, &dest) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            copy_path(src, &dest)?;
            delete_permanent(src)
        }
        Err(err) => Err(io_msg("Moving", src, err)),
    }
}

pub fn copy_path(src: &Path, dest: &Path) -> Result<(), String> {
    if exists(dest) {
        return Err(format!("{} already exists", dest.display()));
    }
    if is_inside(src, dest) {
        return Err("Cannot copy a directory into itself".into());
    }
    copy_rec(src, dest)
}

fn copy_rec(src: &Path, dest: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(src).map_err(|err| io_msg("Copying", src, err))?;
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        let target = fs::read_link(src).map_err(|err| io_msg("Copying", src, err))?;
        symlink(&target, dest).map_err(|err| io_msg("Copying", dest, err))?;
        return Ok(());
    }
    if file_type.is_dir() {
        fs::create_dir(dest).map_err(|err| io_msg("Copying", dest, err))?;
        let dir = fs::read_dir(src).map_err(|err| io_msg("Copying", src, err))?;
        for item in dir {
            let item = item.map_err(|err| io_msg("Copying", src, err))?;
            copy_rec(&item.path(), &dest.join(item.file_name()))?;
        }
        return Ok(());
    }
    fs::copy(src, dest)
        .map(|_| ())
        .map_err(|err| io_msg("Copying", src, err))
}

/// Move `path` into the FreeDesktop trash. Same-filesystem files use
/// `$XDG_DATA_HOME/Trash` (or `~/.local/share/Trash`). Other volumes use
/// `$mount/.Trash/$uid` when that directory is sticky, otherwise `$mount/.Trash-$uid`.
pub fn trash(path: &Path) -> Result<(), String> {
    ensure_mutable(path)?;
    if !exists(path) {
        return Err(format!("Trashing {}: not found", path.display()));
    }
    let abs = absolute_path(path);
    let dir = trash_dir_for(&abs)?;
    let unique = unique_trash_name(&dir, file_name(&abs)?)?;
    let info_path = dir.join("info").join(format!("{unique}.trashinfo"));
    let files_path = dir.join("files").join(&unique);
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(&abs),
        deletion_stamp()
    );
    {
        let mut info =
            File::create(&info_path).map_err(|err| io_msg("Trashing", &info_path, err))?;
        info.write_all(body.as_bytes())
            .map_err(|err| io_msg("Trashing", &info_path, err))?;
    }
    match fs::rename(&abs, &files_path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&info_path);
            if err.raw_os_error() == Some(libc::EXDEV) {
                Err("Cannot trash across drives. Delete permanently instead.".into())
            } else {
                Err(io_msg("Trashing", &abs, err))
            }
        }
    }
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize(path)
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        normalize(&cwd.join(path))
    }
}

fn home_trash_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(xdg);
        if path.is_absolute() {
            return path.join("Trash");
        }
    }
    home_dir().join(".local/share/Trash")
}

fn prepare_trash_dir(dir: &Path) -> Result<(), String> {
    for child in ["files", "info"] {
        let path = dir.join(child);
        if !path.is_dir() {
            fs::create_dir_all(&path).map_err(|err| io_msg("Creating", &path, err))?;
        }
    }
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    Ok(())
}

fn trash_dir_for(path: &Path) -> Result<PathBuf, String> {
    let dev = fs::symlink_metadata(path)
        .map_err(|err| io_msg("Trashing", path, err))?
        .dev();
    let home = home_trash_path();
    if fs::create_dir_all(&home).is_ok() && fs::metadata(&home).is_ok_and(|meta| meta.dev() == dev)
    {
        prepare_trash_dir(&home)?;
        return Ok(home);
    }
    volume_trash_dir(&mount_point(path)?)
}

fn mount_point(path: &Path) -> Result<PathBuf, String> {
    let anchor = existing_ancestor(path);
    let dev = fs::symlink_metadata(&anchor)
        .map_err(|err| io_msg("Trashing", &anchor, err))?
        .dev();
    let mut current = anchor;
    loop {
        let Some(parent) = current.parent() else {
            break;
        };
        if parent.as_os_str().is_empty() {
            break;
        }
        match fs::metadata(parent) {
            Ok(meta) if meta.dev() == dev => current = parent.to_path_buf(),
            _ => break,
        }
    }
    if current.as_os_str().is_empty() {
        Ok(PathBuf::from("/"))
    } else {
        Ok(current)
    }
}

fn existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    loop {
        if fs::symlink_metadata(&current).is_ok() {
            return current;
        }
        match current.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => current = parent.to_path_buf(),
            _ => return PathBuf::from("/"),
        }
    }
}

fn volume_trash_dir(top: &Path) -> Result<PathBuf, String> {
    let uid = unsafe { libc::getuid() };
    let shared = top.join(".Trash");
    if let Ok(meta) = fs::symlink_metadata(&shared) {
        let sticky = meta.permissions().mode() & libc::S_ISVTX != 0;
        if meta.is_dir() && !meta.file_type().is_symlink() && sticky {
            let dest = shared.join(uid.to_string());
            if prepare_trash_dir(&dest).is_ok() {
                return Ok(dest);
            }
        }
    }
    let dest = top.join(format!(".Trash-{uid}"));
    prepare_trash_dir(&dest)?;
    Ok(dest)
}

fn unique_trash_name(dir: &Path, name: &std::ffi::OsStr) -> Result<String, String> {
    let base = name.to_string_lossy();
    if base.is_empty() || base.contains('/') || base.contains('\0') {
        return Err(format!("Refusing trash name {base}"));
    }
    for n in 0..10_000 {
        let candidate = if n == 0 {
            base.to_string()
        } else {
            format!("{base}_{n}")
        };
        let in_files = dir.join("files").join(&candidate);
        let in_info = dir.join("info").join(format!("{candidate}.trashinfo"));
        if !exists(&in_files) && !exists(&in_info) {
            return Ok(candidate);
        }
    }
    Err(format!("No free trash name for {base}"))
}

fn percent_encode(path: &Path) -> String {
    let mut out = String::new();
    for &byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn deletion_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return "1970-01-01T00:00:00".into();
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

pub fn delete_permanent(path: &Path) -> Result<(), String> {
    ensure_mutable(path)?;
    let meta = fs::symlink_metadata(path).map_err(|err| io_msg("Deleting", path, err))?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|err| io_msg("Deleting", path, err))
    } else {
        fs::remove_file(path).map_err(|err| io_msg("Deleting", path, err))
    }
}

pub fn places() -> (Vec<Place>, Vec<Place>) {
    let home = home_dir();
    let mut main = vec![Place {
        label: "Home".into(),
        path: home.clone(),
    }];
    for (label, folder) in [
        ("Documents", "Documents"),
        ("Downloads", "Downloads"),
        ("Pictures", "Pictures"),
        ("Videos", "Videos"),
        ("Music", "Music"),
        ("Desktop", "Desktop"),
    ] {
        let path = home.join(folder);
        if path.is_dir() {
            main.push(Place {
                label: label.into(),
                path,
            });
        }
    }
    main.push(Place {
        label: "Root".into(),
        path: PathBuf::from("/"),
    });

    let mut media = Vec::new();
    if let Some(user) = std::env::var_os("USER") {
        let root = PathBuf::from("/run/media").join(user);
        if let Ok(dir) = fs::read_dir(&root) {
            let mut found: Vec<Place> = dir
                .filter_map(|item| item.ok())
                .filter(|item| item.path().is_dir())
                .map(|item| Place {
                    label: item.file_name().to_string_lossy().into_owned(),
                    path: item.path(),
                })
                .collect();
            found.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
            media = found;
        }
    }
    (main, media)
}

/// `~/.config/beefile`, or under `$XDG_CONFIG_HOME` when that is set.
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".config"));
    base.join("beefile")
}

/// `~/.config/beefile/favorites`, or under `$XDG_CONFIG_HOME` when that is set.
pub fn favorites_file() -> PathBuf {
    config_dir().join("favorites")
}

/// One absolute path per line. Blank lines and `#` comments are ignored.
/// Missing paths are kept so an unmounted drive does not drop a favorite.
pub fn read_favorites(file: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(file) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let path = PathBuf::from(line);
        let path = fs::canonicalize(&path).unwrap_or(path);
        if !out.iter().any(|have| have == &path) {
            out.push(path);
        }
    }
    out
}

pub fn write_favorites(file: &Path, paths: &[PathBuf]) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|err| io_msg("Saving favorites", parent, err))?;
    }
    let mut body = String::new();
    for path in paths {
        body.push_str(&path.display().to_string());
        body.push('\n');
    }
    let tmp = file.with_extension("tmp");
    fs::write(&tmp, body).map_err(|err| io_msg("Saving favorites", file, err))?;
    fs::rename(&tmp, file).map_err(|err| io_msg("Saving favorites", file, err))
}

/// Add `path` when it is a directory and not already listed. Returns whether it was added.
pub fn toggle_favorite(file: &Path, path: &Path) -> Result<bool, String> {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut paths = read_favorites(file);
    if let Some(index) = paths.iter().position(|have| have == &path) {
        paths.remove(index);
        write_favorites(file, &paths)?;
        return Ok(false);
    }
    if !path.is_dir() {
        return Err("Not a folder".into());
    }
    paths.push(path);
    write_favorites(file, &paths)?;
    Ok(true)
}

/// Point favorites that lived under `from` at `to` after a rename.
pub fn retarget_favorite(file: &Path, from: &Path, to: &Path) -> Result<(), String> {
    let mut paths = read_favorites(file);
    let mut changed = false;
    for path in &mut paths {
        if let Ok(rest) = path.strip_prefix(from) {
            *path = if rest.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(rest)
            };
            changed = true;
        }
    }
    if changed {
        write_favorites(file, &paths)?;
    }
    Ok(())
}

pub fn open_with_system(path: &Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Opening {}: {err}", path.display()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HereApp {
    Foot,
    Cursor,
}

impl HereApp {
    pub fn label(self) -> &'static str {
        match self {
            HereApp::Foot => "Foot",
            HereApp::Cursor => "Cursor",
        }
    }
}

/// `setsid uwsm-app -- …` so the new window joins the Omarchy session
/// and is not a child of BeeFile. The directory is an argument because
/// the app daemon does not keep BeeFile's working directory.
pub fn here_command(app: HereApp, dir: &Path) -> (&'static str, Vec<String>) {
    let dir = dir.display().to_string();
    let args = match app {
        HereApp::Foot => vec![
            "uwsm-app".into(),
            "--".into(),
            "foot".into(),
            format!("--working-directory={dir}"),
        ],
        HereApp::Cursor => vec!["uwsm-app".into(), "--".into(), "cursor".into(), dir],
    };
    ("setsid", args)
}

pub fn open_here(app: HereApp, dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err("Not a folder".into());
    }
    let (program, args) = here_command(app, dir);
    let mut child = std::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| format!("Opening {} in {}: {err}", app.label(), dir.display()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn file_name(path: &Path) -> Result<&std::ffi::OsStr, String> {
    path.file_name()
        .ok_or_else(|| format!("Missing file name for {}", path.display()))
}

fn is_inside(parent: &Path, child: &Path) -> bool {
    let Some(parent) = fs::canonicalize(parent).ok() else {
        return false;
    };
    let child = fs::canonicalize(child).unwrap_or_else(|_| normalize(child));
    child.starts_with(&parent) && child != parent
}

fn io_msg(action: &str, path: &Path, err: io::Error) -> String {
    format!("{action} {}: {err}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;
    use std::os::unix::fs::symlink;

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn lists_kinds_including_hidden_and_links() {
        let dir = scratch();
        fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        fs::write(dir.path().join(".hidden"), b"x").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        symlink("a.txt", dir.path().join("link")).unwrap();

        let snap = list_dir(dir.path()).unwrap();
        let mut names: Vec<_> = snap.entries.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, [".hidden", "a.txt", "link", "sub"]);
        let link = snap.entries.iter().find(|e| e.name == "link").unwrap();
        assert_eq!(link.kind, Kind::Symlink);
        assert_eq!(link.link_target.as_deref(), Some(Path::new("a.txt")));
        let sub = snap.entries.iter().find(|e| e.name == "sub").unwrap();
        assert_eq!(sub.kind, Kind::Dir);
        assert!(sub.size.is_none());
        assert!(snap.free.is_some());
    }

    #[test]
    fn create_rename_copy_move_and_delete() {
        let dir = scratch();
        let file = dir.path().join("note.txt");
        create_file(&file).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "");

        let renamed = dir.path().join("renamed.txt");
        rename(&file, &renamed).unwrap();
        assert!(!exists(&file));
        assert!(exists(&renamed));

        let nested = dir.path().join("folder");
        create_dir(&nested).unwrap();
        fs::write(nested.join("inside.txt"), b"data").unwrap();
        symlink("inside.txt", nested.join("inside-link")).unwrap();

        let dest_root = dir.path().join("copied");
        fs::create_dir(&dest_root).unwrap();
        copy_into(&nested, &dest_root).unwrap();
        assert_eq!(
            fs::read_to_string(dest_root.join("folder/inside.txt")).unwrap(),
            "data"
        );
        assert!(fs::symlink_metadata(dest_root.join("folder/inside-link"))
            .unwrap()
            .file_type()
            .is_symlink());

        let move_root = dir.path().join("moved");
        fs::create_dir(&move_root).unwrap();
        move_into(&renamed, &move_root).unwrap();
        assert!(exists(&move_root.join("renamed.txt")));
        assert!(!exists(&renamed));

        delete_permanent(&dest_root.join("folder")).unwrap();
        assert!(!exists(&dest_root.join("folder")));
    }

    #[test]
    fn refuses_overwrite_self_copy_and_protected_paths() {
        let dir = scratch();
        let file = dir.path().join("a.txt");
        create_file(&file).unwrap();
        let err = copy_into(&file, dir.path()).unwrap_err();
        assert!(err.contains("already exists"), "{err}");

        let nested = dir.path().join("dir");
        create_dir(&nested).unwrap();
        let err = copy_path(&nested, &nested.join("dir")).unwrap_err();
        assert!(err.contains("into itself"), "{err}");

        assert!(is_protected(Path::new("/")));
        assert!(is_protected(Path::new("/usr")));
        assert!(is_protected(&home_dir()));
        assert!(!is_protected(&file));
        assert!(delete_permanent(Path::new("/usr")).is_err());
        assert!(trash(&home_dir()).is_err());
        assert!(exists(Path::new("/usr")));
    }

    #[test]
    fn drop_moves_and_ctrl_would_copy() {
        let dir = scratch();
        let src_dir = dir.path().join("src");
        let dest_dir = dir.path().join("dest");
        fs::create_dir(&src_dir).unwrap();
        fs::create_dir(&dest_dir).unwrap();
        let file = src_dir.join("note.txt");
        fs::write(&file, b"hello").unwrap();

        assert_eq!(
            drop_into(&[file.clone()], &dest_dir, false).unwrap(),
            "Moved 1"
        );
        assert!(!exists(&file));
        let moved = dest_dir.join("note.txt");
        assert_eq!(fs::read(&moved).unwrap(), b"hello");

        let err = drop_into(&[moved.clone()], &dest_dir, false).unwrap_err();
        assert!(err.contains("Already"), "{err}");

        assert_eq!(
            drop_into(&[moved.clone()], &src_dir, true).unwrap(),
            "Copied 1"
        );
        assert!(exists(&moved));
        assert_eq!(fs::read(src_dir.join("note.txt")).unwrap(), b"hello");

        let folder = dir.path().join("folder");
        create_dir(&folder).unwrap();
        let err = drop_into(&[folder.clone()], &folder, false).unwrap_err();
        assert!(err.contains("into itself"), "{err}");
        assert!(drop_into(&[moved], Path::new("/usr"), false).is_err());
        assert!(exists(Path::new("/usr")));
    }

    #[test]
    fn favorites_add_remove_and_follow_rename() {
        let dir = scratch();
        let file = dir.path().join("favorites");
        let folder = dir.path().join("keep");
        let nested = folder.join("inside");
        fs::create_dir(&folder).unwrap();
        fs::create_dir(&nested).unwrap();
        let note = dir.path().join("note.txt");
        fs::write(&note, b"x").unwrap();

        assert!(read_favorites(&file).is_empty());
        assert!(toggle_favorite(&file, &folder).unwrap());
        assert!(toggle_favorite(&file, &nested).unwrap());
        let canon = fs::canonicalize(&folder).unwrap();
        assert_eq!(
            read_favorites(&file),
            vec![canon.clone(), fs::canonicalize(&nested).unwrap()]
        );
        let err = toggle_favorite(&file, &note).unwrap_err();
        assert!(err.contains("folder"), "{err}");

        fs::write(
            &file,
            format!(
                "# saved folders\n\n{}\n{}\n",
                canon.display(),
                canon.display()
            ),
        )
        .unwrap();
        assert_eq!(read_favorites(&file), vec![canon]);

        assert!(!toggle_favorite(&file, &folder).unwrap());
        assert!(read_favorites(&file).is_empty());

        write_favorites(&file, &[folder.clone(), nested.clone()]).unwrap();
        let renamed = dir.path().join("renamed");
        fs::rename(&folder, &renamed).unwrap();
        retarget_favorite(&file, &folder, &renamed).unwrap();
        assert_eq!(
            read_favorites(&file),
            vec![
                fs::canonicalize(&renamed).unwrap(),
                fs::canonicalize(&renamed.join("inside")).unwrap()
            ]
        );
        assert!(favorites_file().ends_with("beefile/favorites"));
    }

    #[test]
    fn here_commands_name_the_folder() {
        let dir = Path::new("/tmp/keep me");
        let (program, args) = here_command(HereApp::Foot, dir);
        assert_eq!(program, "setsid");
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        assert_eq!(
            args,
            ["uwsm-app", "--", "foot", "--working-directory=/tmp/keep me",]
        );
        let (_, args) = here_command(HereApp::Cursor, dir);
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        assert_eq!(args, ["uwsm-app", "--", "cursor", "/tmp/keep me"]);

        let missing = Path::new("/tmp/beefile-missing-folder");
        let err = open_here(HereApp::Foot, missing).unwrap_err();
        assert!(err.contains("folder"), "{err}");
        assert!(open_here(HereApp::Cursor, missing).is_err());
    }

    #[test]
    fn expand_and_resolve_start() {
        let dir = scratch();
        let home = dir.path().join("home");
        fs::create_dir(&home).unwrap();
        let file = dir.path().join("pic.png");
        fs::write(&file, b"img").unwrap();

        assert_eq!(expand_input("~/docs", dir.path(), &home), home.join("docs"));
        assert_eq!(expand_input("~", dir.path(), &home), home);
        assert_eq!(
            expand_input("pic.png", dir.path(), &home),
            dir.path().join("pic.png")
        );
        assert_eq!(
            expand_input("/etc/../tmp", dir.path(), &home),
            Path::new("/tmp")
        );

        let (parent, name) = resolve_start(&file).unwrap();
        assert_eq!(parent, fs::canonicalize(dir.path()).unwrap());
        assert_eq!(name.as_deref(), Some("pic.png"));
        let (folder, none) = resolve_start(dir.path()).unwrap();
        assert!(none.is_none());
        assert_eq!(folder, fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn trash_removes_the_original_path() {
        let dir = scratch();
        let file = dir.path().join("trash-me.txt");
        fs::write(&file, b"bye").unwrap();
        let abs = absolute_path(&file);
        trash(&file).unwrap();
        assert!(!exists(&file));

        let stored = find_trashed(&abs).expect("trashed copy");
        assert_eq!(fs::read(&stored).unwrap(), b"bye");
        let info = stored
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("info")
            .join(format!(
                "{}.trashinfo",
                stored.file_name().unwrap().to_string_lossy()
            ));
        let text = fs::read_to_string(&info).unwrap();
        assert!(text.starts_with("[Trash Info]\n"), "{text}");
        assert!(
            text.contains(&format!("Path={}\n", percent_encode(&abs))),
            "{text}"
        );
        assert!(text.contains("DeletionDate="), "{text}");
        fs::remove_file(&stored).unwrap();
        fs::remove_file(&info).unwrap();
    }

    fn find_trashed(original: &Path) -> Option<PathBuf> {
        let needle = format!("Path={}\n", percent_encode(original));
        let mut dirs = vec![home_trash_path()];
        if let Ok(top) = mount_point(original) {
            let uid = unsafe { libc::getuid() };
            dirs.push(top.join(".Trash").join(uid.to_string()));
            dirs.push(top.join(format!(".Trash-{uid}")));
        }
        for dir in dirs {
            let Ok(entries) = fs::read_dir(dir.join("info")) else {
                continue;
            };
            for entry in entries.flatten() {
                let info = entry.path();
                let Ok(text) = fs::read_to_string(&info) else {
                    continue;
                };
                if text.contains(&needle) {
                    let name = info.file_stem()?.to_os_string();
                    return Some(dir.join("files").join(name));
                }
            }
        }
        None
    }
}
