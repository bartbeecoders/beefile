//! Extension → application links, and the desktop apps that can open a file.
//!
//! Links live in `~/.config/beefile/openers` (or `$XDG_CONFIG_HOME`). One line
//! is `ext<TAB>spec`. `spec` is a desktop id such as `imv.desktop`, or a
//! command. `%f` in a command is the file; when it is absent the path is
//! appended. Commands are split on whitespace, not run through a shell.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::fsops;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppChoice {
    pub id: String,
    pub name: String,
}

pub fn openers_file() -> PathBuf {
    fsops::config_dir().join("openers")
}

/// Last suffix, lowercased, without the dot. `archive.tar.gz` is `gz`.
/// `Makefile` and `.gitignore` have none.
pub fn file_extension(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    if name.starts_with('.') && !name[1..].contains('.') {
        return None;
    }
    let (_, ext) = name.rsplit_once('.')?;
    normalize_extension(ext).ok()
}

pub fn read_openers(file: &Path) -> BTreeMap<String, String> {
    let Ok(text) = fs::read_to_string(file) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((ext, spec)) = line.split_once('\t') else {
            continue;
        };
        let Ok(ext) = normalize_extension(ext) else {
            continue;
        };
        let spec = spec.trim();
        if spec.is_empty() || spec.contains('\n') {
            continue;
        }
        out.insert(ext, spec.to_string());
    }
    out
}

pub fn set_opener(file: &Path, ext: &str, spec: &str) -> Result<(), String> {
    let ext = normalize_extension(ext)?;
    let spec = spec.trim();
    if spec.is_empty() || spec.contains('\n') || spec.len() > 512 {
        return Err("Not a command".into());
    }
    if !is_desktop_id(spec) {
        split_command(spec)?;
    }
    let mut map = read_openers(file);
    map.insert(ext, spec.to_string());
    write_openers(file, &map)
}

pub fn clear_opener(file: &Path, ext: &str) -> Result<(), String> {
    let ext = normalize_extension(ext)?;
    let mut map = read_openers(file);
    if map.remove(&ext).is_some() {
        write_openers(file, &map)?;
    }
    Ok(())
}

pub fn is_desktop_id(spec: &str) -> bool {
    let spec = spec.trim();
    spec.ends_with(".desktop")
        && !spec.contains('/')
        && !spec.contains('\\')
        && !spec.contains(' ')
        && spec.chars().all(|c| c.is_ascii_graphic() && c != ';')
}

/// Argv for the application itself, without the session wrapper.
pub fn launch_argv(spec: &str, path: &Path) -> Result<Vec<String>, String> {
    let spec = spec.trim();
    let file = path.display().to_string();
    if is_desktop_id(spec) {
        return Ok(vec!["gtk-launch".into(), spec.to_string(), file]);
    }
    let args = split_command(spec)?;
    let (mut args, saw_file) = expand_fields(args, &file);
    if !saw_file {
        args.push(file);
    }
    if args.is_empty() {
        return Err("Empty command".into());
    }
    Ok(args)
}

pub fn launch_spec(spec: &str, path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("Not a file: {}", path.display()));
    }
    let args = launch_argv(spec, path)?;
    let mut child = std::process::Command::new("setsid")
        .arg("uwsm-app")
        .arg("--")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| format!("Opening {}: {err}", path.display()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Apps from the FreeDesktop mime database that claim this extension.
/// The user's default, then added associations, then the cache. Removed
/// associations are left out. Hidden desktop entries are left out.
pub fn apps_for_extension(ext: &str) -> Vec<AppChoice> {
    let Ok(ext) = normalize_extension(ext) else {
        return Vec::new();
    };
    apps_in_roots(&ext, &data_dirs(), &mimeapps_paths())
}

fn apps_in_roots(ext: &str, data_dirs: &[PathBuf], mimeapps: &[PathBuf]) -> Vec<AppChoice> {
    let mut globs = Vec::new();
    for dir in data_dirs {
        if let Ok(text) = fs::read_to_string(dir.join("mime/globs2")) {
            globs.push(text);
        }
    }
    let Some(mime) = best_mime(&globs, ext) else {
        return Vec::new();
    };
    let mut cache_ids = Vec::new();
    for dir in data_dirs {
        if let Ok(text) = fs::read_to_string(dir.join("applications/mimeinfo.cache")) {
            cache_ids.extend(desktop_ids_for_mime(&text, &mime));
        }
    }
    let mut default = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for path in mimeapps {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let parsed = parse_mimeapps(&text, &mime);
        if default.is_empty() {
            default = parsed.default;
        }
        added.extend(parsed.added);
        removed.extend(parsed.removed);
    }
    merge_apps(&default, &added, &removed, &cache_ids)
        .into_iter()
        .filter_map(|id| load_desktop(data_dirs, &id))
        .collect()
}

fn normalize_extension(ext: &str) -> Result<String, String> {
    let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if ext.is_empty()
        || ext.len() > 32
        || !ext
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_'))
    {
        return Err("Not a file extension".into());
    }
    Ok(ext)
}

fn write_openers(file: &Path, map: &BTreeMap<String, String>) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("Saving openers: {err}"))?;
    }
    let mut body = String::from("# extension, then a tab, then a desktop id or a command\n");
    for (ext, spec) in map {
        body.push_str(ext);
        body.push('\t');
        body.push_str(spec);
        body.push('\n');
    }
    let tmp = file.with_extension("tmp");
    fs::write(&tmp, body).map_err(|err| format!("Saving openers: {err}"))?;
    fs::rename(&tmp, file).map_err(|err| format!("Saving openers: {err}"))
}

fn best_mime(files: &[String], ext: &str) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    for text in files {
        if let Some((weight, mime)) = mime_in_globs(text, ext) {
            if best.as_ref().is_none_or(|(have, _)| weight > *have) {
                best = Some((weight, mime));
            }
        }
    }
    best.map(|(_, mime)| mime)
}

fn mime_in_globs(globs2: &str, ext: &str) -> Option<(u32, String)> {
    let mut best: Option<(u32, String)> = None;
    for line in globs2.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, ':');
        let Some(weight) = parts.next().and_then(|weight| weight.parse::<u32>().ok()) else {
            continue;
        };
        let Some(mime) = parts.next() else {
            continue;
        };
        let Some(pattern) = parts.next() else {
            continue;
        };
        let Some(pattern_ext) = pattern.strip_prefix("*.") else {
            continue;
        };
        if pattern_ext.contains('*')
            || pattern_ext.contains('?')
            || !pattern_ext.eq_ignore_ascii_case(ext)
        {
            continue;
        }
        if best.as_ref().is_none_or(|(have, _)| weight > *have) {
            best = Some((weight, mime.to_string()));
        }
    }
    best
}

fn desktop_ids_for_mime(cache: &str, mime: &str) -> Vec<String> {
    let mut in_cache = false;
    for line in cache.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_cache = line == "[MIME Cache]";
            continue;
        }
        if !in_cache || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == mime {
            return split_ids(value);
        }
    }
    Vec::new()
}

struct MimeLists {
    default: Vec<String>,
    added: Vec<String>,
    removed: Vec<String>,
}

fn parse_mimeapps(text: &str, mime: &str) -> MimeLists {
    let mut section = "";
    let mut default = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = match name {
                "Default Applications" => "default",
                "Added Associations" => "added",
                "Removed Associations" => "removed",
                _ => "",
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key != mime {
            continue;
        }
        let ids = split_ids(value);
        match section {
            "default" => default = ids,
            "added" => added = ids,
            "removed" => removed = ids,
            _ => {}
        }
    }
    MimeLists {
        default,
        added,
        removed,
    }
}

fn split_ids(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|id| is_desktop_id(id))
        .map(str::to_string)
        .collect()
}

fn merge_apps(
    default: &[String],
    added: &[String],
    removed: &[String],
    cache: &[String],
) -> Vec<String> {
    let removed: BTreeSet<&str> = removed.iter().map(String::as_str).collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for id in default.iter().chain(added).chain(cache) {
        if removed.contains(id.as_str()) || !seen.insert(id.as_str()) {
            continue;
        }
        out.push(id.clone());
    }
    out
}

fn load_desktop(dirs: &[PathBuf], id: &str) -> Option<AppChoice> {
    if !is_desktop_id(id) {
        return None;
    }
    for dir in dirs {
        let Ok(text) = fs::read_to_string(dir.join("applications").join(id)) else {
            continue;
        };
        let name = desktop_entry_name(&text)?;
        return Some(AppChoice {
            id: id.to_string(),
            name,
        });
    }
    None
}

fn desktop_entry_name(text: &str) -> Option<String> {
    let mut in_entry = false;
    let mut name = None;
    let mut hidden = false;
    let mut application = true;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if in_entry {
                break;
            }
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Name" if name.is_none() => name = Some(value.to_string()),
            "Hidden" if value.eq_ignore_ascii_case("true") => hidden = true,
            "Type" if value != "Application" => application = false,
            _ => {}
        }
    }
    if hidden || !application {
        return None;
    }
    name.filter(|name| !name.is_empty())
}

fn split_command(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut chars = input.chars().peekable();
    let mut in_quote = false;
    let mut started = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                started = true;
                match chars.next() {
                    Some(next) => cur.push(next),
                    None => cur.push('\\'),
                }
            }
            '"' => {
                started = true;
                in_quote = !in_quote;
            }
            c if c.is_whitespace() && !in_quote => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                started = true;
                cur.push(c);
            }
        }
    }
    if in_quote {
        return Err("Unclosed quote".into());
    }
    if started {
        args.push(cur);
    }
    if args.is_empty() {
        return Err("Empty command".into());
    }
    Ok(args)
}

fn expand_fields(args: Vec<String>, file: &str) -> (Vec<String>, bool) {
    let mut saw_file = false;
    let mut out = Vec::new();
    for arg in args {
        let mut expanded = String::new();
        let mut chars = arg.chars().peekable();
        let mut keep = false;
        while let Some(c) = chars.next() {
            if c != '%' {
                expanded.push(c);
                keep = true;
                continue;
            }
            match chars.next() {
                Some('%') => {
                    expanded.push('%');
                    keep = true;
                }
                Some('f' | 'F' | 'u' | 'U') => {
                    expanded.push_str(file);
                    saw_file = true;
                    keep = true;
                }
                Some('c' | 'k' | 'i') => {}
                Some(other) => {
                    expanded.push('%');
                    expanded.push(other);
                    keep = true;
                }
                None => {
                    expanded.push('%');
                    keep = true;
                }
            }
        }
        if keep {
            out.push(expanded);
        }
    }
    (out, saw_file)
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| fsops::home_dir().join(".local/share"));
    dirs.push(home);
    let rest =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for part in rest.split(':') {
        if !part.is_empty() {
            dirs.push(PathBuf::from(part));
        }
    }
    dirs
}

fn mimeapps_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| fsops::home_dir().join(".config"));
    paths.push(config.join("mimeapps.list"));
    let config_dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
    for part in config_dirs.split(':') {
        if !part.is_empty() {
            paths.push(PathBuf::from(part).join("mimeapps.list"));
        }
    }
    for dir in data_dirs() {
        paths.push(dir.join("applications/mimeapps.list"));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn extension_is_the_last_suffix() {
        assert_eq!(
            file_extension(Path::new("/tmp/Photo.PNG")).as_deref(),
            Some("png")
        );
        assert_eq!(
            file_extension(Path::new("archive.tar.gz")).as_deref(),
            Some("gz")
        );
        assert_eq!(
            file_extension(Path::new(".config.json")).as_deref(),
            Some("json")
        );
        assert_eq!(file_extension(Path::new("Makefile")), None);
        assert_eq!(file_extension(Path::new(".gitignore")), None);
        assert_eq!(file_extension(Path::new("trailing.")), None);
    }

    #[test]
    fn openers_round_trip_and_reject_a_bad_command() {
        let dir = scratch();
        let file = dir.path().join("openers");
        assert!(read_openers(&file).is_empty());
        set_opener(&file, "PNG", "imv.desktop").unwrap();
        set_opener(&file, ".md", "foot -e nvim %f").unwrap();
        assert_eq!(
            read_openers(&file).get("png").map(String::as_str),
            Some("imv.desktop")
        );
        assert_eq!(
            read_openers(&file).get("md").map(String::as_str),
            Some("foot -e nvim %f")
        );
        let err = set_opener(&file, "../x", "imv").unwrap_err();
        assert!(err.contains("extension"), "{err}");
        let err = set_opener(&file, "log", "\"nvim").unwrap_err();
        assert!(err.contains("quote"), "{err}");
        clear_opener(&file, "png").unwrap();
        assert!(!read_openers(&file).contains_key("png"));
        assert!(read_openers(&file).contains_key("md"));
        assert!(openers_file().ends_with("beefile/openers"));
    }

    #[test]
    fn globs_cache_and_desktop_name() {
        let globs = "\
# sample
10:text/plain:*.txt
50:text/x-rust:*.rs
80:text/plain:*.txt
50:image/png:*.PNG
";
        assert_eq!(mime_in_globs(globs, "txt").unwrap().1, "text/plain");
        assert_eq!(mime_in_globs(globs, "txt").unwrap().0, 80);
        assert_eq!(mime_in_globs(globs, "png").unwrap().1, "image/png");
        assert_eq!(mime_in_globs(globs, "rs").unwrap().1, "text/x-rust");

        let cache = "\
[MIME Cache]
text/plain=hidden.desktop;writer.desktop;not a desktop;
image/png=imv.desktop;
";
        let ids = desktop_ids_for_mime(cache, "text/plain");
        let ids: Vec<_> = ids.iter().map(String::as_str).collect();
        assert_eq!(ids, ["hidden.desktop", "writer.desktop"]);

        let mimeapps = "\
[Default Applications]
text/plain=writer.desktop
[Removed Associations]
text/plain=hidden.desktop
[Added Associations]
text/plain=extra.desktop
";
        let parsed = parse_mimeapps(mimeapps, "text/plain");
        let merged = merge_apps(
            &parsed.default,
            &parsed.added,
            &parsed.removed,
            &desktop_ids_for_mime(cache, "text/plain"),
        );
        let merged: Vec<_> = merged.iter().map(String::as_str).collect();
        assert_eq!(merged, ["writer.desktop", "extra.desktop"]);

        assert_eq!(
            desktop_entry_name("[Desktop Entry]\nType=Application\nName=Writer\n").as_deref(),
            Some("Writer")
        );
        assert_eq!(
            desktop_entry_name("[Desktop Entry]\nType=Application\nHidden=true\nName=Nope\n"),
            None
        );
        assert_eq!(
            desktop_entry_name("[Desktop Entry]\nType=Link\nName=Nope\n"),
            None
        );
    }

    #[test]
    fn commands_keep_the_file_once() {
        let path = Path::new("/tmp/keep me.png");
        let argv = |spec| launch_argv(spec, path).unwrap().join("\n");
        assert_eq!(
            argv("imv.desktop"),
            "gtk-launch\nimv.desktop\n/tmp/keep me.png"
        );
        assert_eq!(argv("imv"), "imv\n/tmp/keep me.png");
        assert_eq!(argv("foot -e nvim %f"), "foot\n-e\nnvim\n/tmp/keep me.png");
        assert_eq!(argv("echo a%%b"), "echo\na%b\n/tmp/keep me.png");
        let err = launch_spec("imv", Path::new("/tmp/beefile-no-such-file")).unwrap_err();
        assert!(err.contains("file"), "{err}");
    }

    #[test]
    fn roots_list_the_default_app_and_skip_hidden() {
        let dir = scratch();
        let data = dir.path().join("share");
        let apps = data.join("applications");
        fs::create_dir_all(data.join("mime")).unwrap();
        fs::create_dir_all(&apps).unwrap();
        fs::write(data.join("mime/globs2"), "50:image/png:*.png\n").unwrap();
        fs::write(
            apps.join("mimeinfo.cache"),
            "[MIME Cache]\nimage/png=hidden.desktop;imv.desktop;\n",
        )
        .unwrap();
        fs::write(
            apps.join("imv.desktop"),
            "[Desktop Entry]\nType=Application\nName=imv\nExec=imv %F\n",
        )
        .unwrap();
        fs::write(
            apps.join("hidden.desktop"),
            "[Desktop Entry]\nType=Application\nHidden=true\nName=Hidden\nExec=hidden %f\n",
        )
        .unwrap();
        fs::write(
            apps.join("writer.desktop"),
            "[Desktop Entry]\nType=Application\nName=Writer\nExec=writer %f\n",
        )
        .unwrap();
        let mimeapps = dir.path().join("mimeapps.list");
        fs::write(
            &mimeapps,
            "[Default Applications]\nimage/png=writer.desktop\n",
        )
        .unwrap();
        let found = apps_in_roots("png", &[data], &[mimeapps]);
        let names: Vec<_> = found.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(names, ["Writer", "imv"]);
    }
}
