//! Directory listing state: sort, filter, cursor, and which paths an action hits.
//! No UI and no filesystem calls.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Symlink,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub link_target: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Modified => "modified",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::Modified,
            Self::Modified => Self::Name,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Parent,
    Item(usize),
}

#[derive(Clone, Debug)]
pub struct RowInfo<'a> {
    pub name: &'a str,
    pub path: &'a Path,
    pub kind: Kind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub is_parent: bool,
}

#[derive(Clone, Debug)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub parent: Option<PathBuf>,
    pub show_hidden: bool,
    pub query: String,
    pub sort: SortKey,
    pub ascending: bool,
}

impl Listing {
    pub fn new(cwd: &Path) -> Self {
        let mut listing = Self {
            entries: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            parent: None,
            show_hidden: false,
            query: String::new(),
            sort: SortKey::Name,
            ascending: true,
        };
        listing.set_cwd(cwd);
        listing
    }

    pub fn set_cwd(&mut self, cwd: &Path) {
        self.parent = if cwd == Path::new("/") {
            None
        } else {
            cwd.parent().map(Path::to_path_buf)
        };
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Replace entries from a directory read.
    /// `select` wins over cursor preservation. `preserve` keeps the cursor on the
    /// same path when refreshing the directory that is already open.
    pub fn set_entries(&mut self, entries: Vec<Entry>, select: Option<&str>, preserve: bool) {
        let keep = if preserve && select.is_none() {
            self.row_path(self.cursor).map(Path::to_path_buf)
        } else {
            None
        };
        self.entries = entries;
        self.rows = self.compute_rows();
        self.cursor = 0;
        if let Some(name) = select {
            if let Some(ix) = self.index_of_name(name) {
                self.cursor = ix;
                return;
            }
        }
        if let Some(path) = keep.as_deref() {
            if let Some(ix) = self.index_of_path(path) {
                self.cursor = ix;
            }
        }
    }

    pub fn refilter(&mut self) {
        let keep = self.row_path(self.cursor).map(Path::to_path_buf);
        self.rows = self.compute_rows();
        self.cursor = keep
            .as_deref()
            .and_then(|path| self.index_of_path(path))
            .unwrap_or(0);
    }

    pub fn move_by(&mut self, delta: isize) {
        let len = self.rows.len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, len as isize - 1) as usize;
    }

    pub fn move_to(&mut self, ix: usize) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = ix.min(self.rows.len() - 1);
    }

    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.refilter();
    }

    pub fn toggle_direction(&mut self) {
        self.ascending = !self.ascending;
        self.refilter();
    }

    pub fn set_sort(&mut self, key: SortKey) {
        if self.sort == key {
            self.ascending = !self.ascending;
        } else {
            self.sort = key;
            self.ascending = true;
        }
        self.refilter();
    }

    pub fn info(&self, ix: usize) -> Option<RowInfo<'_>> {
        match self.rows.get(ix)? {
            Row::Parent => {
                let path = self.parent.as_deref()?;
                Some(RowInfo {
                    name: "..",
                    path,
                    kind: Kind::Dir,
                    size: None,
                    modified: None,
                    is_parent: true,
                })
            }
            Row::Item(index) => {
                let entry = self.entries.get(*index)?;
                Some(RowInfo {
                    name: &entry.name,
                    path: &entry.path,
                    kind: entry.kind,
                    size: entry.size,
                    modified: entry.modified,
                    is_parent: false,
                })
            }
        }
    }

    pub fn cursor_info(&self) -> Option<RowInfo<'_>> {
        self.info(self.cursor)
    }

    pub fn row_path(&self, ix: usize) -> Option<&Path> {
        self.info(ix).map(|info| info.path)
    }

    pub fn cursor_is_parent(&self) -> bool {
        matches!(self.rows.get(self.cursor), Some(Row::Parent))
    }

    pub fn hidden_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| is_hidden_name(&entry.name))
            .count()
    }

    fn compute_rows(&self) -> Vec<Row> {
        let mut items: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.show_hidden || !is_hidden_name(&entry.name))
            .map(|(index, _)| index)
            .collect();
        let query = self.query.to_lowercase();
        if !query.is_empty() {
            items.retain(|index| contains_ci(&self.entries[*index].name, &query));
        }
        items.sort_by(|a, b| {
            cmp_entries(
                &self.entries[*a],
                &self.entries[*b],
                self.sort,
                self.ascending,
            )
        });

        let mut rows = Vec::with_capacity(items.len() + 1);
        if self.parent.is_some() {
            rows.push(Row::Parent);
        }
        rows.extend(items.into_iter().map(Row::Item));
        rows
    }

    fn index_of_path(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|row| match row {
            Row::Parent => self.parent.as_deref() == Some(path),
            Row::Item(index) => self
                .entries
                .get(*index)
                .is_some_and(|entry| entry.path == path),
        })
    }

    fn index_of_name(&self, name: &str) -> Option<usize> {
        self.rows.iter().position(|row| match row {
            Row::Parent => name == "..",
            Row::Item(index) => self
                .entries
                .get(*index)
                .is_some_and(|entry| entry.name == name),
        })
    }
}

pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.') && name != "." && name != ".."
}

/// Paths an operation should touch. Marks win. The parent row is never included.
pub fn action_paths<'a>(listing: &'a Listing, marks: &'a BTreeSet<PathBuf>) -> Vec<&'a Path> {
    if !marks.is_empty() {
        return listing
            .entries
            .iter()
            .filter(|entry| marks.contains(&entry.path))
            .map(|entry| entry.path.as_path())
            .collect();
    }
    match listing.cursor_info() {
        Some(info) if !info.is_parent => vec![info.path],
        _ => Vec::new(),
    }
}

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ac = a.chars().peekable();
    let mut bc = b.chars().peekable();
    loop {
        match (ac.peek().copied(), bc.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let an = take_number(&mut ac);
                let bn = take_number(&mut bc);
                let ord = cmp_number(&an, &bn);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = fold(x).cmp(&fold(y));
                ac.next();
                bc.next();
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

fn fold(c: char) -> char {
    c.to_ascii_lowercase()
}

fn take_number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut raw = String::new();
    while let Some(c) = chars.peek().copied() {
        if c.is_ascii_digit() {
            raw.push(c);
            chars.next();
        } else {
            break;
        }
    }
    let trimmed = raw.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.to_string()
    }
}

fn cmp_number(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn contains_ci(hay: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    hay.to_lowercase().contains(needle_lower)
}

fn dir_rank(kind: Kind) -> u8 {
    if kind == Kind::Dir {
        0
    } else {
        1
    }
}

fn cmp_entries(a: &Entry, b: &Entry, key: SortKey, ascending: bool) -> Ordering {
    let rank = dir_rank(a.kind).cmp(&dir_rank(b.kind));
    if rank != Ordering::Equal {
        return rank;
    }
    let primary = match key {
        SortKey::Name => natural_cmp(&a.name, &b.name),
        SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
        SortKey::Modified => a.modified.cmp(&b.modified),
    };
    let primary = if ascending {
        primary
    } else {
        primary.reverse()
    };
    if primary != Ordering::Equal {
        return primary;
    }
    natural_cmp(&a.name, &b.name)
}

pub fn extension_cursor(name: &str) -> usize {
    if name.starts_with('.') && name.chars().filter(|c| *c == '.').count() == 1 {
        return name.len();
    }
    match name.rfind('.') {
        Some(index) if index > 0 => index,
        _ => name.len(),
    }
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("Name is empty".into());
    }
    if name.contains('/') || name.contains('\0') || name.chars().any(|c| c.is_control()) {
        return Err("Name cannot contain a path separator".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn entry(name: &str, kind: Kind, size: u64) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from("/demo").join(name),
            kind,
            size: if kind == Kind::Dir { None } else { Some(size) },
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(size)),
            link_target: None,
        }
    }

    fn listing_with(cwd: &str, entries: Vec<Entry>) -> Listing {
        let mut listing = Listing::new(Path::new(cwd));
        listing.show_hidden = true;
        listing.set_entries(entries, None, false);
        listing
    }

    #[test]
    fn natural_order_sorts_numbers_and_ignores_case() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("file10", "file2"), Ordering::Greater);
        assert_eq!(natural_cmp("File", "file"), Ordering::Equal);
        assert_eq!(natural_cmp("a01", "a1"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "b"), Ordering::Less);
    }

    #[test]
    fn dirs_stay_first_when_sorting_by_size_descending() {
        let mut listing = listing_with(
            "/demo",
            vec![
                entry("zeta", Kind::Dir, 1),
                entry("big", Kind::File, 50),
                entry("small", Kind::File, 2),
                entry("alpha", Kind::Dir, 9),
            ],
        );
        listing.set_sort(SortKey::Size);
        listing.set_sort(SortKey::Size);
        let names: Vec<_> = (0..listing.len())
            .filter_map(|ix| listing.info(ix).map(|info| info.name.to_string()))
            .collect();
        assert_eq!(names, ["..", "alpha", "zeta", "big", "small"]);
    }

    #[test]
    fn filter_keeps_parent_and_cursor_path() {
        let mut listing = listing_with(
            "/demo",
            vec![
                entry("notes.txt", Kind::File, 1),
                entry("photo.png", Kind::File, 2),
                entry(".secret", Kind::File, 3),
            ],
        );
        listing.cursor = listing.index_of_name("photo.png").unwrap();
        listing.query = "pho".into();
        listing.refilter();
        let names: Vec<_> = (0..listing.len())
            .filter_map(|ix| listing.info(ix).map(|info| info.name.to_string()))
            .collect();
        assert_eq!(names, ["..", "photo.png"]);
        assert_eq!(listing.cursor_info().unwrap().name, "photo.png");

        listing.query = "missing".into();
        listing.refilter();
        assert_eq!(listing.cursor_info().unwrap().name, "..");
    }

    #[test]
    fn hidden_files_drop_out_until_toggled() {
        let mut listing = listing_with(
            "/demo",
            vec![
                entry("notes.txt", Kind::File, 1),
                entry(".secret", Kind::File, 2),
            ],
        );
        listing.show_hidden = false;
        listing.refilter();
        assert!(listing.hidden_count() == 1);
        let names: Vec<_> = (0..listing.len())
            .filter_map(|ix| listing.info(ix).map(|info| info.name.to_string()))
            .collect();
        assert_eq!(names, ["..", "notes.txt"]);
    }

    #[test]
    fn root_has_no_parent_row() {
        let listing = listing_with("/", vec![entry("etc", Kind::Dir, 0)]);
        // entry() joins /demo, but parent logic uses cwd.
        assert!(listing.parent.is_none());
        assert!(!listing.rows.iter().any(|row| matches!(row, Row::Parent)));
    }

    #[test]
    fn select_name_after_load() {
        let mut listing = Listing::new(Path::new("/demo"));
        listing.set_entries(
            vec![entry("a.txt", Kind::File, 1), entry("b.txt", Kind::File, 2)],
            Some("b.txt"),
            false,
        );
        assert_eq!(listing.cursor_info().unwrap().name, "b.txt");
    }

    #[test]
    fn cursor_clamps() {
        let mut listing = listing_with("/demo", vec![entry("a.txt", Kind::File, 1)]);
        listing.move_by(10);
        assert_eq!(listing.cursor, listing.len() - 1);
        listing.move_by(-10);
        assert_eq!(listing.cursor, 0);
        listing.move_to(0);
        listing.move_by(-1);
        assert_eq!(listing.cursor, 0);
    }

    #[test]
    fn marks_override_cursor_and_skip_parent() {
        let mut listing = listing_with(
            "/demo",
            vec![entry("a.txt", Kind::File, 1), entry("b.txt", Kind::File, 2)],
        );
        listing.cursor = listing.index_of_name("a.txt").unwrap();
        assert_eq!(action_paths(&listing, &BTreeSet::new()).len(), 1);
        assert_eq!(
            action_paths(&listing, &BTreeSet::new())[0],
            Path::new("/demo/a.txt")
        );

        let mut on_parent = listing.clone();
        on_parent.cursor = 0;
        assert!(action_paths(&on_parent, &BTreeSet::new()).is_empty());

        let mut marks = BTreeSet::new();
        marks.insert(PathBuf::from("/demo/b.txt"));
        let paths = action_paths(&listing, &marks);
        assert_eq!(paths, vec![Path::new("/demo/b.txt")]);
    }

    #[test]
    fn names_and_extension_cursor() {
        assert!(validate_name("notes.txt").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\nb").is_err());
        assert_eq!(extension_cursor("notes.txt"), 5);
        assert_eq!(extension_cursor(".gitignore"), ".gitignore".len());
        assert_eq!(extension_cursor("Makefile"), "Makefile".len());
        assert_eq!(extension_cursor("archive.tar.gz"), "archive.tar".len());
    }
}
