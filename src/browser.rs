//! Keyboard-first file manager view.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{
    actions, anchored, div, prelude::*, px, rgb, rgba, uniform_list, App, Context, FocusHandle,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, ScrollStrategy,
    UniformListScrollHandle, Window,
};
use notify::Watcher;

use crate::format::{self, Crumb};
use crate::fsops::{self, Place};
use crate::model::{self, Kind, Listing, SortKey};
use crate::openers;
use crate::theme::{self, Palette};

actions!(
    beefile,
    [
        Quit,
        Open,
        GoParent,
        GoHome,
        GoBack,
        GoForward,
        MoveDown,
        MoveUp,
        MoveTop,
        MoveBottom,
        PageDown,
        PageUp,
        ToggleHidden,
        CycleSort,
        ToggleSortDir,
        CycleTheme,
        Refresh,
        Yank,
        Cut,
        Paste,
        CopyPath,
        TrashAction,
        DeletePermanent,
        RenameAction,
        NewFile,
        NewDir,
        GoTo,
        Filter,
        ToggleHelp,
        ToggleMark,
        MarkDown,
        MarkUp,
        MarkAll,
        Cancel,
    ]
);

const HELP: &[(&str, &str)] = &[
    ("j  k  ↑  ↓", "Move"),
    ("h  ←  Backspace", "Parent directory"),
    ("l  →  Enter", "Open"),
    ("g g    G", "Top / bottom"),
    ("PgUp  PgDn", "Page"),
    ("[  ]", "Back / forward"),
    ("~", "Home"),
    ("Enter on a file", "Linked app, or xdg-open"),
    ("Space", "Mark and move down"),
    ("Shift-J  Shift-K", "Mark a range"),
    ("Ctrl-A", "Mark or unmark visible files"),
    ("y", "Copy"),
    ("x", "Cut"),
    ("p", "Paste"),
    ("Drag", "Move onto a folder. Ctrl copies"),
    ("Right-click a folder", "Favorite, or open foot or Cursor"),
    ("Right-click a file", "Link its extension to an app"),
    ("Y  Ctrl-C", "Copy path"),
    ("d  Delete", "Move to trash"),
    ("Shift-D  Shift-Delete", "Delete permanently"),
    ("r  F2", "Rename"),
    ("n", "New file"),
    ("Shift-N", "New folder"),
    ("/", "Filter"),
    ("Ctrl-L", "Go to path"),
    (".", "Show hidden files"),
    ("s    S", "Cycle sort / reverse"),
    ("t", "Cycle color theme"),
    ("Ctrl-R  F5", "Refresh"),
    ("?", "Help"),
    ("Esc", "Cancel, then clear filter"),
    ("q    Ctrl-Q", "Quit"),
];

const ROW_H: f32 = 30.0;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("q", Quit, Some("BeeFile")),
        KeyBinding::new("enter", Open, Some("BeeFile")),
        KeyBinding::new("l", Open, Some("BeeFile")),
        KeyBinding::new("right", Open, Some("BeeFile")),
        KeyBinding::new("h", GoParent, Some("BeeFile")),
        KeyBinding::new("left", GoParent, Some("BeeFile")),
        KeyBinding::new("backspace", GoParent, Some("BeeFile")),
        KeyBinding::new("~", GoHome, Some("BeeFile")),
        KeyBinding::new("[", GoBack, Some("BeeFile")),
        KeyBinding::new("]", GoForward, Some("BeeFile")),
        KeyBinding::new("j", MoveDown, Some("BeeFile")),
        KeyBinding::new("down", MoveDown, Some("BeeFile")),
        KeyBinding::new("k", MoveUp, Some("BeeFile")),
        KeyBinding::new("up", MoveUp, Some("BeeFile")),
        KeyBinding::new("g g", MoveTop, Some("BeeFile")),
        KeyBinding::new("home", MoveTop, Some("BeeFile")),
        KeyBinding::new("shift-g", MoveBottom, Some("BeeFile")),
        KeyBinding::new("end", MoveBottom, Some("BeeFile")),
        KeyBinding::new("pagedown", PageDown, Some("BeeFile")),
        KeyBinding::new("ctrl-d", PageDown, Some("BeeFile")),
        KeyBinding::new("pageup", PageUp, Some("BeeFile")),
        KeyBinding::new("ctrl-u", PageUp, Some("BeeFile")),
        KeyBinding::new(".", ToggleHidden, Some("BeeFile")),
        KeyBinding::new("s", CycleSort, Some("BeeFile")),
        KeyBinding::new("shift-s", ToggleSortDir, Some("BeeFile")),
        KeyBinding::new("t", CycleTheme, Some("BeeFile")),
        KeyBinding::new("ctrl-r", Refresh, Some("BeeFile")),
        KeyBinding::new("f5", Refresh, Some("BeeFile")),
        KeyBinding::new("y", Yank, Some("BeeFile")),
        KeyBinding::new("x", Cut, Some("BeeFile")),
        KeyBinding::new("p", Paste, Some("BeeFile")),
        KeyBinding::new("shift-y", CopyPath, Some("BeeFile")),
        KeyBinding::new("ctrl-c", CopyPath, Some("BeeFile")),
        KeyBinding::new("d", TrashAction, Some("BeeFile")),
        KeyBinding::new("delete", TrashAction, Some("BeeFile")),
        KeyBinding::new("shift-d", DeletePermanent, Some("BeeFile")),
        KeyBinding::new("shift-delete", DeletePermanent, Some("BeeFile")),
        KeyBinding::new("r", RenameAction, Some("BeeFile")),
        KeyBinding::new("f2", RenameAction, Some("BeeFile")),
        KeyBinding::new("n", NewFile, Some("BeeFile")),
        KeyBinding::new("shift-n", NewDir, Some("BeeFile")),
        KeyBinding::new("ctrl-l", GoTo, Some("BeeFile")),
        KeyBinding::new("/", Filter, Some("BeeFile")),
        KeyBinding::new("?", ToggleHelp, Some("BeeFile")),
        KeyBinding::new("shift-/", ToggleHelp, Some("BeeFile")),
        KeyBinding::new("space", ToggleMark, Some("BeeFile")),
        KeyBinding::new("shift-j", MarkDown, Some("BeeFile")),
        KeyBinding::new("shift-k", MarkUp, Some("BeeFile")),
        KeyBinding::new("ctrl-a", MarkAll, Some("BeeFile")),
        KeyBinding::new("escape", Cancel, Some("BeeFile")),
    ]);
}

#[derive(Clone, Debug)]
enum Clip {
    Copy(Vec<PathBuf>),
    Cut(Vec<PathBuf>),
}

#[derive(Clone, Debug)]
enum PromptKind {
    Filter,
    GoTo,
    Rename { from: PathBuf },
    NewFile,
    NewDir,
    Trash { paths: Vec<PathBuf> },
    Delete { paths: Vec<PathBuf> },
    LinkOpen { path: PathBuf, ext: String },
}

impl PromptKind {
    fn is_confirm(&self) -> bool {
        matches!(self, Self::Trash { .. } | Self::Delete { .. })
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Filter => "Filter",
            Self::GoTo => "Go to",
            Self::Rename { .. } => "Rename",
            Self::NewFile => "New file",
            Self::NewDir => "New folder",
            Self::Trash { .. } => "Trash",
            Self::Delete { .. } => "Delete",
            Self::LinkOpen { .. } => "Open with",
        }
    }
}

#[derive(Clone, Debug)]
struct PromptState {
    kind: PromptKind,
    text: String,
    cursor: usize,
    saved_query: String,
}

impl PromptState {
    fn insert(&mut self, extra: &str) {
        if self.kind.is_confirm() || extra.is_empty() {
            return;
        }
        let cursor = self.cursor.min(self.text.len());
        self.text.insert_str(cursor, extra);
        self.cursor = cursor + extra.len();
    }

    fn backspace(&mut self) {
        if self.kind.is_confirm() || self.cursor == 0 {
            return;
        }
        let prev = prev_boundary(&self.text, self.cursor);
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    fn delete_forward(&mut self) {
        if self.kind.is_confirm() || self.cursor >= self.text.len() {
            return;
        }
        let next = next_boundary(&self.text, self.cursor);
        self.text.replace_range(self.cursor..next, "");
    }

    fn move_left(&mut self) {
        self.cursor = prev_boundary(&self.text, self.cursor);
    }

    fn move_right(&mut self) {
        self.cursor = next_boundary(&self.text, self.cursor);
    }

    fn kill_word(&mut self) {
        if self.kind.is_confirm() {
            return;
        }
        let start = simple_word_left(&self.text, self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    fn clear(&mut self) {
        if self.kind.is_confirm() {
            return;
        }
        self.text.clear();
        self.cursor = 0;
    }
}

pub struct Browser {
    focus: FocusHandle,
    prompt_focus: FocusHandle,
    scroll: UniformListScrollHandle,
    cwd: PathBuf,
    home: PathBuf,
    listing: Listing,
    places: Vec<Place>,
    media: Vec<Place>,
    favorites: Vec<PathBuf>,
    favorites_file: PathBuf,
    openers: BTreeMap<String, String>,
    openers_file: PathBuf,
    theme_id: theme::ThemeId,
    palette: Palette,
    theme_label: String,
    theme_file: PathBuf,
    /// Set after an Omarchy palette has been read. Theme switches delete
    /// `colors.toml` for a moment; the last good colors stay up through that.
    omarchy_live: bool,
    theme_watch_tx: async_channel::Sender<()>,
    theme_watcher: Option<notify::RecommendedWatcher>,
    menu: Option<ContextMenu>,
    marks: BTreeSet<PathBuf>,
    clipboard: Option<Clip>,
    prompt: Option<PromptState>,
    help: bool,
    error: Option<String>,
    note: Option<String>,
    busy: bool,
    free: Option<u64>,
    generation: u64,
    pending_select: Option<String>,
    last_click: Option<(usize, Instant)>,
    /// Set when a prompt key was handled, so the matching list action does not also run.
    ate_key: bool,
    watch_tx: async_channel::Sender<()>,
    watcher: Option<notify::RecommendedWatcher>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl Browser {
    pub fn new(
        cwd: PathBuf,
        select: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tx, rx) = async_channel::bounded(64);
        let (theme_tx, theme_rx) = async_channel::bounded(8);
        let theme_file = theme::theme_file();
        let theme_id = theme::initial_choice(&theme_file);
        let resolved = theme::resolve(theme_id);
        let focus = cx.focus_handle();
        let prompt_focus = cx.focus_handle();
        focus.focus(window);
        let listing = Listing::new(&cwd);
        let mut this = Self {
            focus,
            prompt_focus,
            scroll: UniformListScrollHandle::new(),
            home: fsops::home_dir(),
            cwd,
            listing,
            places: Vec::new(),
            media: Vec::new(),
            favorites: Vec::new(),
            favorites_file: fsops::favorites_file(),
            openers: BTreeMap::new(),
            openers_file: openers::openers_file(),
            theme_id,
            palette: resolved.palette,
            theme_label: resolved.label,
            theme_file,
            omarchy_live: theme_id == theme::ThemeId::Omarchy && resolved.available,
            theme_watch_tx: theme_tx,
            theme_watcher: None,
            menu: None,
            marks: BTreeSet::new(),
            clipboard: None,
            prompt: None,
            help: false,
            error: None,
            note: None,
            busy: false,
            free: None,
            generation: 0,
            pending_select: select,
            last_click: None,
            ate_key: false,
            watch_tx: tx,
            watcher: None,
            back: Vec::new(),
            forward: Vec::new(),
        };
        this.refresh_places();
        this.favorites = fsops::read_favorites(&this.favorites_file);
        this.openers = openers::read_openers(&this.openers_file);
        this.spawn_poll(rx, cx);
        this.spawn_theme_poll(theme_rx, cx);
        this.arm_watcher();
        this.arm_theme_watcher();
        this.set_title(window);
        this.reload(cx, false);
        this
    }

    fn typing(&self) -> bool {
        self.prompt.is_some() || self.ate_key
    }

    fn set_title(&self, window: &mut Window) {
        let shown = format::display_path(&self.cwd, &self.home);
        window.set_window_title(&format!("BeeFile {} — {shown}", crate::version::label()));
    }

    fn refresh_places(&mut self) {
        let (places, media) = fsops::places();
        self.places = places;
        self.media = media;
    }

    fn spawn_poll(&mut self, rx: async_channel::Receiver<()>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            if rx.recv().await.is_err() {
                break;
            }
            let executor = cx.background_executor().clone();
            executor
                .spawn(async { std::thread::sleep(Duration::from_millis(80)) })
                .await;
            while rx.try_recv().is_ok() {}
            let alive = this.update(cx, |this, cx| {
                this.reload(cx, true);
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    fn arm_watcher(&mut self) {
        let tx = self.watch_tx.clone();
        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if res.is_ok() {
                    let _ = tx.try_send(());
                }
            }) {
                Ok(watcher) => watcher,
                Err(_) => {
                    self.watcher = None;
                    return;
                }
            };
        if watcher
            .watch(&self.cwd, notify::RecursiveMode::NonRecursive)
            .is_err()
        {
            self.watcher = None;
            return;
        }
        self.watcher = Some(watcher);
    }

    fn spawn_theme_poll(&mut self, rx: async_channel::Receiver<()>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            if rx.recv().await.is_err() {
                break;
            }
            let executor = cx.background_executor().clone();
            executor
                .spawn(async { std::thread::sleep(Duration::from_millis(200)) })
                .await;
            while rx.try_recv().is_ok() {}
            let alive = this.update(cx, |this, cx| this.refresh_theme(cx, true));
            let Ok(retry) = alive else {
                break;
            };
            if !retry {
                continue;
            }
            let executor = cx.background_executor().clone();
            executor
                .spawn(async { std::thread::sleep(Duration::from_millis(200)) })
                .await;
            while rx.try_recv().is_ok() {}
            if this
                .update(cx, |this, cx| {
                    // The directory swap is over. Apply whatever is on disk now,
                    // including the Honey fallback when the palette is still gone.
                    this.omarchy_live = false;
                    this.refresh_theme(cx, true);
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    fn arm_theme_watcher(&mut self) {
        self.theme_watcher = None;
        if self.theme_id != theme::ThemeId::Omarchy {
            return;
        }
        let dir = theme::omarchy_current_dir();
        if !dir.is_dir() {
            return;
        }
        let tx = self.theme_watch_tx.clone();
        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if res.is_ok() {
                    let _ = tx.try_send(());
                }
            }) {
                Ok(watcher) => watcher,
                Err(_) => return,
            };
        if watcher
            .watch(&dir, notify::RecursiveMode::NonRecursive)
            .is_err()
        {
            return;
        }
        self.theme_watcher = Some(watcher);
    }

    /// Returns true when the Omarchy palette was missing and should be read again.
    fn refresh_theme(&mut self, cx: &mut Context<Self>, announce: bool) -> bool {
        let resolved = theme::resolve(self.theme_id);
        if self.theme_id == theme::ThemeId::Omarchy && !resolved.available && self.omarchy_live {
            return true;
        }
        let label_changed = self.theme_label != resolved.label;
        let changed = self.palette != resolved.palette || label_changed;
        self.omarchy_live = self.theme_id == theme::ThemeId::Omarchy && resolved.available;
        self.palette = resolved.palette;
        self.theme_label = resolved.label;
        if announce && label_changed {
            self.note = Some(format!("Theme: {}", self.theme_label));
        }
        if changed {
            cx.notify();
        }
        false
    }

    fn cycle_theme(&mut self, _: &CycleTheme, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.theme_id = self.theme_id.next();
        self.refresh_theme(cx, true);
        self.arm_theme_watcher();
        match theme::write_choice(&self.theme_file, self.theme_id) {
            Ok(()) => self.error = None,
            Err(err) => {
                self.error = Some(err);
                self.note = None;
            }
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>, preserve: bool) {
        self.generation += 1;
        let generation = self.generation;
        let cwd = self.cwd.clone();
        let select = self.pending_select.clone();
        let preserve = preserve && select.is_none();
        if !preserve {
            self.note = Some("Reading…".into());
        }
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let listed = executor.spawn(async move { fsops::list_dir(&cwd) }).await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match listed {
                    Ok(snap) => {
                        this.free = snap.free;
                        this.listing
                            .set_entries(snap.entries, select.as_deref(), preserve);
                        if select.is_some() {
                            this.pending_select = None;
                        }
                        this.follow_cursor();
                        if this.note.as_deref() == Some("Reading…") {
                            this.note = None;
                        }
                        this.error = None;
                    }
                    Err(err) => this.error = Some(err),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn navigate(
        &mut self,
        path: PathBuf,
        record: bool,
        select: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if !path.is_dir() {
            self.error = Some(format!("Not a directory: {}", path.display()));
            cx.notify();
            return;
        }
        if record && path != self.cwd {
            self.back.push(self.cwd.clone());
            if self.back.len() > 64 {
                self.back.remove(0);
            }
            self.forward.clear();
        }
        let changed = path != self.cwd;
        self.cwd = path;
        self.listing.set_cwd(&self.cwd);
        self.listing.query.clear();
        if changed {
            self.marks.clear();
            self.listing.entries.clear();
            self.listing.rows.clear();
            self.listing.cursor = 0;
        }
        self.pending_select = select;
        self.prompt = None;
        self.help = false;
        self.menu = None;
        self.error = None;
        self.focus.focus(window);
        self.refresh_places();
        self.arm_watcher();
        self.set_title(window);
        self.reload(cx, !changed);
        cx.notify();
    }

    fn follow_cursor(&self) {
        if self.listing.len() > 0 {
            self.scroll
                .scroll_to_item(self.listing.cursor, ScrollStrategy::Center);
        }
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        self.follow_cursor();
        cx.notify();
    }

    fn open_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.listing.cursor_info() else {
            return;
        };
        let path = info.path.to_path_buf();
        let open_dir = info.is_parent || info.kind == Kind::Dir || path.is_dir();
        if open_dir {
            self.navigate(path, true, None, window, cx);
            return;
        }
        self.open_file(&path, cx);
    }

    fn open_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        let spec = openers::file_extension(path).and_then(|ext| self.openers.get(&ext).cloned());
        let result = match &spec {
            Some(spec) => openers::launch_spec(spec, path),
            None => fsops::open_with_system(path),
        };
        self.menu = None;
        if let Err(err) = result {
            self.error = Some(err);
            self.note = None;
        }
        cx.notify();
    }

    fn action_owned(&self) -> Vec<PathBuf> {
        model::action_paths(&self.listing, &self.marks)
            .into_iter()
            .map(Path::to_path_buf)
            .collect()
    }

    fn require_paths(&mut self, cx: &mut Context<Self>) -> Option<Vec<PathBuf>> {
        let paths = self.action_owned();
        if paths.is_empty() {
            self.error = Some("Nothing selected".into());
            cx.notify();
            None
        } else {
            Some(paths)
        }
    }

    fn mark_cursor(&mut self) {
        if self.listing.cursor_is_parent() {
            return;
        }
        let Some(path) = self
            .listing
            .row_path(self.listing.cursor)
            .map(Path::to_path_buf)
        else {
            return;
        };
        self.marks.insert(path);
    }

    fn toggle_cursor_mark(&mut self) {
        if self.listing.cursor_is_parent() {
            return;
        }
        let Some(path) = self
            .listing
            .row_path(self.listing.cursor)
            .map(Path::to_path_buf)
        else {
            return;
        };
        if !self.marks.insert(path.clone()) {
            self.marks.remove(&path);
        }
    }

    fn open_prompt(
        &mut self,
        kind: PromptKind,
        text: String,
        cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved_query = self.listing.query.clone();
        self.prompt = Some(PromptState {
            kind,
            text,
            cursor,
            saved_query,
        });
        self.help = false;
        self.menu = None;
        self.prompt_focus.focus(window);
        cx.notify();
    }

    fn close_prompt(&mut self, restore_query: bool, window: &mut Window) {
        let saved = self
            .prompt
            .as_ref()
            .map(|prompt| prompt.saved_query.clone());
        let was_filter = self
            .prompt
            .as_ref()
            .is_some_and(|prompt| matches!(prompt.kind, PromptKind::Filter));
        self.prompt = None;
        if restore_query && was_filter {
            if let Some(saved) = saved {
                self.listing.query = saved;
                self.listing.refilter();
            }
        }
        self.focus.focus(window);
    }

    fn sync_filter(&mut self) {
        let Some(text) = self.prompt.as_ref().and_then(|prompt| {
            matches!(prompt.kind, PromptKind::Filter).then(|| prompt.text.clone())
        }) else {
            return;
        };
        self.listing.query = text;
        self.listing.refilter();
    }

    fn spawn_work<F>(&mut self, cx: &mut Context<Self>, label: &str, clear_cut: bool, work: F)
    where
        F: FnOnce() -> Result<String, String> + Send + 'static,
    {
        if self.busy {
            self.error = Some("Busy".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.note = Some(label.into());
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let result = executor.spawn(async move { work() }).await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(message) => {
                        this.note = Some(message);
                        this.error = None;
                        if clear_cut {
                            this.clipboard = None;
                        }
                        this.marks
                            .retain(|path| std::fs::symlink_metadata(path).is_ok());
                        let clipboard_empty = this.clipboard.as_mut().is_some_and(|clip| {
                            let paths = match clip {
                                Clip::Copy(paths) | Clip::Cut(paths) => paths,
                            };
                            paths.retain(|path| std::fs::symlink_metadata(path).is_ok());
                            paths.is_empty()
                        });
                        if clipboard_empty {
                            this.clipboard = None;
                        }
                        this.favorites = fsops::read_favorites(&this.favorites_file);
                    }
                    Err(err) => {
                        this.error = Some(err);
                        this.note = None;
                    }
                }
                this.reload(cx, true);
            })
            .ok();
        })
        .detach();
    }

    fn confirm_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        match prompt.kind {
            PromptKind::Filter => {
                self.close_prompt(false, window);
                cx.notify();
            }
            PromptKind::GoTo => {
                let path = fsops::expand_input(&prompt.text, &self.cwd, &self.home);
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                self.close_prompt(false, window);
                if path.is_dir() {
                    self.navigate(path, true, None, window, cx);
                } else if path.is_file() {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                    let parent = path
                        .parent()
                        .unwrap_or_else(|| Path::new("/"))
                        .to_path_buf();
                    self.navigate(parent, true, name, window, cx);
                } else {
                    self.error = Some(format!("No such path: {}", path.display()));
                    cx.notify();
                }
            }
            PromptKind::Rename { from } => {
                if let Err(err) = model::validate_name(&prompt.text) {
                    self.error = Some(err);
                    cx.notify();
                    return;
                }
                let parent = from
                    .parent()
                    .unwrap_or_else(|| Path::new("/"))
                    .to_path_buf();
                let dest = parent.join(&prompt.text);
                self.close_prompt(false, window);
                if dest == from {
                    cx.notify();
                    return;
                }
                self.marks.remove(&from);
                let favorites_file = self.favorites_file.clone();
                self.spawn_work(cx, "Renaming…", false, move || {
                    fsops::rename(&from, &dest)?;
                    let _ = fsops::retarget_favorite(&favorites_file, &from, &dest);
                    Ok(format!("Renamed to {}", prompt.text))
                });
            }
            PromptKind::NewFile => {
                if let Err(err) = model::validate_name(&prompt.text) {
                    self.error = Some(err);
                    cx.notify();
                    return;
                }
                let path = self.cwd.join(&prompt.text);
                let name = prompt.text.clone();
                self.close_prompt(false, window);
                self.pending_select = Some(name);
                self.spawn_work(cx, "Creating…", false, move || {
                    fsops::create_file(&path)?;
                    Ok("Created file".into())
                });
            }
            PromptKind::NewDir => {
                if let Err(err) = model::validate_name(&prompt.text) {
                    self.error = Some(err);
                    cx.notify();
                    return;
                }
                let path = self.cwd.join(&prompt.text);
                let name = prompt.text.clone();
                self.close_prompt(false, window);
                self.pending_select = Some(name);
                self.spawn_work(cx, "Creating…", false, move || {
                    fsops::create_dir(&path)?;
                    Ok("Created folder".into())
                });
            }
            PromptKind::Trash { paths } => {
                let count = paths.len();
                self.close_prompt(false, window);
                for path in &paths {
                    self.marks.remove(path);
                }
                self.spawn_work(cx, "Trashing…", false, move || {
                    for path in &paths {
                        fsops::trash(path)?;
                    }
                    Ok(format!("Trashed {count}"))
                });
            }
            PromptKind::Delete { paths } => {
                let count = paths.len();
                self.close_prompt(false, window);
                for path in &paths {
                    self.marks.remove(path);
                }
                self.spawn_work(cx, "Deleting…", false, move || {
                    for path in &paths {
                        fsops::delete_permanent(path)?;
                    }
                    Ok(format!("Deleted {count}"))
                });
            }
            PromptKind::LinkOpen { path, ext } => {
                let spec = prompt.text.trim().to_string();
                self.close_prompt(false, window);
                if spec.is_empty() {
                    self.unlink_extension(&ext, cx);
                    return;
                }
                self.use_opener(ext, spec.clone(), spec, path, false, cx);
            }
        }
    }

    fn handle_prompt_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.prompt.as_ref() else {
            return;
        };
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        let confirm_kind = prompt.kind.is_confirm();
        self.ate_key = true;

        if key == "escape" || (confirm_kind && key == "n" && !mods.control && !mods.alt) {
            self.close_prompt(true, window);
            self.error = None;
            cx.notify();
            return;
        }
        if key == "enter" || (confirm_kind && key == "y" && !mods.control && !mods.alt) {
            self.confirm_prompt(window, cx);
            return;
        }
        if confirm_kind {
            return;
        }
        if mods.control && !mods.alt {
            match key {
                "a" => {
                    if let Some(prompt) = self.prompt.as_mut() {
                        prompt.cursor = 0;
                    }
                }
                "e" => {
                    if let Some(prompt) = self.prompt.as_mut() {
                        let len = prompt.text.len();
                        prompt.cursor = len;
                    }
                }
                "u" => {
                    if let Some(prompt) = self.prompt.as_mut() {
                        prompt.clear();
                    }
                }
                "w" => {
                    if let Some(prompt) = self.prompt.as_mut() {
                        prompt.kill_word();
                    }
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        let flat = text.replace(['\n', '\r'], " ");
                        if let Some(prompt) = self.prompt.as_mut() {
                            prompt.insert(&flat);
                        }
                    }
                }
                _ => return,
            }
            self.sync_filter();
            self.reveal(cx);
            return;
        }
        if mods.control || mods.alt || mods.platform || mods.function {
            return;
        }
        match key {
            "backspace" => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.backspace();
                }
            }
            "delete" => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.delete_forward();
                }
            }
            "left" => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.move_left();
                }
            }
            "right" => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.move_right();
                }
            }
            "home" => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.cursor = 0;
                }
            }
            "end" => {
                if let Some(prompt) = self.prompt.as_mut() {
                    let len = prompt.text.len();
                    prompt.cursor = len;
                }
            }
            _ => {
                let typed = event
                    .keystroke
                    .key_char
                    .clone()
                    .filter(|text| !text.is_empty() && !text.chars().any(|c| c.is_control()));
                if let Some(typed) = typed {
                    if let Some(prompt) = self.prompt.as_mut() {
                        prompt.insert(&typed);
                    }
                } else {
                    return;
                }
            }
        }
        self.sync_filter();
        self.reveal(cx);
    }

    fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.open_cursor(window, cx);
    }

    fn go_parent(&mut self, _: &GoParent, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let Some(parent) = self.cwd.parent() else {
            return;
        };
        let name = self
            .cwd
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self.navigate(parent.to_path_buf(), true, name, window, cx);
    }

    fn go_home(&mut self, _: &GoHome, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.navigate(self.home.clone(), true, None, window, cx);
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let Some(prev) = self.back.pop() else {
            return;
        };
        self.forward.push(self.cwd.clone());
        self.navigate(prev, false, None, window, cx);
    }

    fn go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let Some(next) = self.forward.pop() else {
            return;
        };
        self.back.push(self.cwd.clone());
        self.navigate(next, false, None, window, cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.move_by(1);
        self.reveal(cx);
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.move_by(-1);
        self.reveal(cx);
    }

    fn move_top(&mut self, _: &MoveTop, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.move_to(0);
        self.reveal(cx);
    }

    fn move_bottom(&mut self, _: &MoveBottom, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.move_to(usize::MAX);
        self.reveal(cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.move_by(16);
        self.reveal(cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.move_by(-16);
        self.reveal(cx);
    }

    fn toggle_hidden(&mut self, _: &ToggleHidden, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.show_hidden = !self.listing.show_hidden;
        self.listing.refilter();
        self.reveal(cx);
    }

    fn cycle_sort(&mut self, _: &CycleSort, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.cycle_sort();
        self.reveal(cx);
    }

    fn toggle_sort_dir(&mut self, _: &ToggleSortDir, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.listing.toggle_direction();
        self.reveal(cx);
    }

    fn refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.refresh_places();
        self.reload(cx, true);
    }

    fn yank(&mut self, _: &Yank, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let Some(paths) = self.require_paths(cx) else {
            return;
        };
        let count = paths.len();
        self.clipboard = Some(Clip::Copy(paths));
        self.note = Some(format!("Copied {count}"));
        self.error = None;
        cx.notify();
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let Some(paths) = self.require_paths(cx) else {
            return;
        };
        let count = paths.len();
        self.clipboard = Some(Clip::Cut(paths));
        self.note = Some(format!("Cut {count}"));
        self.error = None;
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let Some(clip) = self.clipboard.clone() else {
            self.error = Some("Clipboard is empty".into());
            cx.notify();
            return;
        };
        let dest = self.cwd.clone();
        let cut = matches!(clip, Clip::Cut(_));
        let sources = match clip {
            Clip::Copy(paths) | Clip::Cut(paths) => paths,
        };
        self.spawn_work(cx, "Pasting…", cut, move || {
            let count = sources.len();
            for src in &sources {
                if cut {
                    fsops::move_into(src, &dest)?;
                } else {
                    fsops::copy_into(src, &dest)?;
                }
            }
            Ok(format!("Pasted {count}"))
        });
    }

    fn drag_paths_for(&self, path: &Path) -> Vec<PathBuf> {
        if !self.marks.contains(path) {
            return vec![path.to_path_buf()];
        }
        let mut paths: Vec<PathBuf> = self
            .listing
            .entries
            .iter()
            .filter(|entry| self.marks.contains(&entry.path))
            .map(|entry| entry.path.clone())
            .collect();
        if !paths.iter().any(|item| item == path) {
            paths.push(path.to_path_buf());
        }
        paths
    }

    fn accept_drop(
        &mut self,
        paths: Vec<PathBuf>,
        dest: PathBuf,
        copy: bool,
        cx: &mut Context<Self>,
    ) {
        self.spawn_work(
            cx,
            if copy { "Copying…" } else { "Moving…" },
            false,
            move || fsops::drop_into(&paths, &dest, copy),
        );
    }

    fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let Some(info) = self.listing.cursor_info() else {
            return;
        };
        let text = info.path.display().to_string();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.note = Some("Path copied".into());
        self.error = None;
        cx.notify();
    }

    fn trash_action(&mut self, _: &TrashAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let Some(paths) = self.require_paths(cx) else {
            return;
        };
        self.open_prompt(PromptKind::Trash { paths }, String::new(), 0, window, cx);
    }

    fn delete_permanent(
        &mut self,
        _: &DeletePermanent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.typing() {
            return;
        }
        let Some(paths) = self.require_paths(cx) else {
            return;
        };
        self.open_prompt(PromptKind::Delete { paths }, String::new(), 0, window, cx);
    }

    fn rename_action(&mut self, _: &RenameAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        if self.listing.cursor_is_parent() {
            self.error = Some("Cannot rename ..".into());
            cx.notify();
            return;
        }
        let Some(info) = self.listing.cursor_info() else {
            return;
        };
        let name = info.name.to_string();
        let from = info.path.to_path_buf();
        let cursor = model::extension_cursor(&name);
        self.open_prompt(PromptKind::Rename { from }, name, cursor, window, cx);
    }

    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.open_prompt(PromptKind::NewFile, String::new(), 0, window, cx);
    }

    fn new_dir(&mut self, _: &NewDir, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.open_prompt(PromptKind::NewDir, String::new(), 0, window, cx);
    }

    fn go_to(&mut self, _: &GoTo, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let text = self.cwd.display().to_string();
        let cursor = text.len();
        self.open_prompt(PromptKind::GoTo, text, cursor, window, cx);
    }

    fn filter(&mut self, _: &Filter, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.is_some() {
            return;
        }
        let text = self.listing.query.clone();
        let cursor = text.len();
        self.open_prompt(PromptKind::Filter, text, cursor, window, cx);
    }

    fn toggle_help(&mut self, _: &ToggleHelp, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.menu = None;
        self.help = !self.help;
        cx.notify();
    }

    fn toggle_mark(&mut self, _: &ToggleMark, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.toggle_cursor_mark();
        self.listing.move_by(1);
        self.reveal(cx);
    }

    fn mark_down(&mut self, _: &MarkDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.mark_cursor();
        self.listing.move_by(1);
        self.mark_cursor();
        self.reveal(cx);
    }

    fn mark_up(&mut self, _: &MarkUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        self.mark_cursor();
        self.listing.move_by(-1);
        self.mark_cursor();
        self.reveal(cx);
    }

    fn mark_all(&mut self, _: &MarkAll, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing() {
            return;
        }
        let paths: Vec<PathBuf> = self
            .listing
            .rows
            .iter()
            .filter_map(|row| match row {
                model::Row::Item(index) => self
                    .listing
                    .entries
                    .get(*index)
                    .map(|entry| entry.path.clone()),
                model::Row::Parent => None,
            })
            .collect();
        let all_marked = paths.iter().all(|path| self.marks.contains(path));
        if all_marked {
            for path in paths {
                self.marks.remove(&path);
            }
        } else {
            self.marks.extend(paths);
        }
        cx.notify();
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.ate_key {
            return;
        }
        if self.menu.take().is_some() {
            cx.notify();
            return;
        }
        if self.prompt.is_some() {
            self.close_prompt(true, window);
            cx.notify();
            return;
        }
        if !self.listing.query.is_empty() {
            self.listing.query.clear();
            self.listing.refilter();
            cx.notify();
            return;
        }
        if self.help {
            self.help = false;
            cx.notify();
            return;
        }
        if self.error.take().is_some() || self.note.take().is_some() {
            cx.notify();
        }
    }

    fn on_row_click(
        &mut self,
        ix: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.prompt.is_some() {
            self.close_prompt(false, window);
        }
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(prev, at)| {
            prev == ix && now.duration_since(at) < Duration::from_millis(350)
        });
        self.last_click = Some((ix, now));
        if event.modifiers.shift {
            let start = self.listing.cursor.min(ix);
            let end = self.listing.cursor.max(ix);
            for row in start..=end {
                self.listing.cursor = row;
                self.mark_cursor();
            }
        }
        self.listing.move_to(ix);
        self.focus.focus(window);
        if double && !event.modifiers.shift {
            self.open_cursor(window, cx);
        } else {
            self.reveal(cx);
        }
    }

    fn open_menu(
        &mut self,
        path: PathBuf,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.prompt.is_some() || self.help {
            return;
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let is_file = path.is_file();
        let is_dir = path.is_dir();
        let favorite = self.favorites.iter().any(|have| have == &path);
        if !is_file && !is_dir && !favorite {
            self.menu = None;
            cx.notify();
            return;
        }
        let ext = if is_file {
            openers::file_extension(&path)
        } else {
            None
        };
        let mut apps = ext
            .as_ref()
            .map(|ext| openers::apps_for_extension(ext))
            .unwrap_or_default();
        if let Some(ext) = &ext {
            if let Some(spec) = self.openers.get(ext) {
                if let Some(index) = apps.iter().position(|app| &app.id == spec) {
                    let app = apps.remove(index);
                    apps.insert(0, app);
                }
            }
        }
        apps.truncate(12);
        self.menu = Some(ContextMenu {
            path,
            position,
            ext,
            apps,
        });
        self.focus.focus(window);
        cx.notify();
    }

    fn use_opener(
        &mut self,
        ext: String,
        spec: String,
        display: String,
        path: PathBuf,
        already: bool,
        cx: &mut Context<Self>,
    ) {
        match openers::set_opener(&self.openers_file, &ext, &spec) {
            Ok(()) => {
                self.openers = openers::read_openers(&self.openers_file);
                match openers::launch_spec(&spec, &path) {
                    Ok(()) => {
                        self.error = None;
                        self.note = Some(if already {
                            format!("Opened with {display}")
                        } else {
                            format!("Linked .{ext} to {display}")
                        });
                    }
                    Err(err) => {
                        self.error = Some(err);
                        self.note = None;
                    }
                }
            }
            Err(err) => {
                self.error = Some(err);
                self.note = None;
            }
        }
        self.menu = None;
        cx.notify();
    }

    fn unlink_extension(&mut self, ext: &str, cx: &mut Context<Self>) {
        match openers::clear_opener(&self.openers_file, ext) {
            Ok(()) => {
                self.openers = openers::read_openers(&self.openers_file);
                self.error = None;
                self.note = Some(format!("Cleared .{ext}"));
            }
            Err(err) => {
                self.error = Some(err);
                self.note = None;
            }
        }
        self.menu = None;
        cx.notify();
    }

    fn prompt_opener(
        &mut self,
        path: PathBuf,
        ext: String,
        prefill: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cursor = prefill.len();
        self.open_prompt(
            PromptKind::LinkOpen { path, ext },
            prefill,
            cursor,
            window,
            cx,
        );
    }

    fn toggle_favorite(&mut self, path: &Path, cx: &mut Context<Self>) {
        match fsops::toggle_favorite(&self.favorites_file, path) {
            Ok(added) => {
                self.favorites = fsops::read_favorites(&self.favorites_file);
                self.error = None;
                let name = folder_label(path, &self.home);
                self.note = Some(if added {
                    format!("Added {name}")
                } else {
                    format!("Removed {name}")
                });
            }
            Err(err) => {
                self.error = Some(err);
                self.note = None;
            }
        }
        self.menu = None;
        cx.notify();
    }

    fn open_here(&mut self, app: fsops::HereApp, path: &Path, cx: &mut Context<Self>) {
        match fsops::open_here(app, path) {
            Ok(()) => {
                self.error = None;
                let name = folder_label(path, &self.home);
                self.note = Some(format!("{} in {name}", app.label()));
            }
            Err(err) => {
                self.error = Some(err);
                self.note = None;
            }
        }
        self.menu = None;
        cx.notify();
    }

    fn favorite_entries(&self) -> Vec<(String, PathBuf)> {
        let names: Vec<String> = self
            .favorites
            .iter()
            .map(|path| folder_label(path, &self.home))
            .collect();
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for name in &names {
            *counts.entry(name.clone()).or_insert(0) += 1;
        }
        self.favorites
            .iter()
            .zip(names)
            .map(|(path, name)| {
                let label = if counts[&name] > 1 {
                    match path.parent().and_then(|parent| parent.file_name()) {
                        Some(parent) => format!("{}/{}", parent.to_string_lossy(), name),
                        None => path.display().to_string(),
                    }
                } else {
                    name
                };
                (label, path.clone())
            })
            .collect()
    }

    fn sort_arrow(&self, key: SortKey) -> &'static str {
        if self.listing.sort != key {
            return "";
        }
        if self.listing.ascending {
            " ↑"
        } else {
            " ↓"
        }
    }
}

impl Render for Browser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ate_key = false;
        let crumbs = format::crumbs(&self.cwd, &self.home);
        let menu = self.menu.clone();
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(self.palette.bg))
            .text_color(rgb(self.palette.text))
            .text_size(px(14.))
            .on_modifiers_changed(cx.listener(
                |_this, _: &gpui::ModifiersChangedEvent, window, cx| {
                    if cx.has_active_drag() {
                        window.refresh();
                    }
                },
            ))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::go_parent))
            .on_action(cx.listener(Self::go_home))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_top))
            .on_action(cx.listener(Self::move_bottom))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::toggle_hidden))
            .on_action(cx.listener(Self::cycle_sort))
            .on_action(cx.listener(Self::toggle_sort_dir))
            .on_action(cx.listener(Self::cycle_theme))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::yank))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::trash_action))
            .on_action(cx.listener(Self::delete_permanent))
            .on_action(cx.listener(Self::rename_action))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::new_dir))
            .on_action(cx.listener(Self::go_to))
            .on_action(cx.listener(Self::filter))
            .on_action(cx.listener(Self::toggle_help))
            .on_action(cx.listener(Self::toggle_mark))
            .on_action(cx.listener(Self::mark_down))
            .on_action(cx.listener(Self::mark_up))
            .on_action(cx.listener(Self::mark_all))
            .on_action(cx.listener(Self::cancel))
            .child(self.render_path(crumbs, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(self.render_sidebar(cx))
                    .child(self.render_main(cx)),
            )
            .child(self.render_prompt(cx))
            .child(self.render_status());
        if let Some(menu) = menu {
            root = root.child(self.menu_layer(menu, cx));
        }
        root
    }
}

impl Browser {
    fn render_path(&self, crumbs: Vec<Crumb>, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.palette;
        let mut row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(2.))
            .px(px(12.))
            .py(px(8.))
            .bg(rgb(c.panel))
            .border_b_1()
            .border_color(rgb(c.line))
            .min_h(px(40.));
        for (index, crumb) in crumbs.into_iter().enumerate() {
            if index > 0 {
                row = row.child(div().text_color(rgb(c.dim)).child("/"));
            }
            let path = crumb.path.clone();
            let current = path == self.cwd;
            let click_path = path.clone();
            let menu_path = path.clone();
            row = row.child(
                attach_drop(
                    div()
                        .id(("crumb", index))
                        .px(px(4.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_color(rgb(if current { c.accent } else { c.text }))
                        .hover(|style| style.bg(rgb(c.elevated)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                                this.navigate(click_path.clone(), true, None, window, cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.open_menu(menu_path.clone(), event.position, window, cx);
                                cx.stop_propagation();
                            }),
                        ),
                    path,
                    true,
                    c.drop,
                    cx,
                )
                .child(crumb.label),
            );
        }
        row
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.palette;
        let mut side = div()
            .id("sidebar")
            .w(px(196.))
            .h_full()
            .flex()
            .flex_col()
            .py(px(8.))
            .bg(rgb(c.panel))
            .border_r_1()
            .border_color(rgb(c.line))
            .overflow_y_scroll()
            .child(section_label("Places", c.dim));
        for (index, place) in self.places.iter().enumerate() {
            side = side.child(self.place_row(
                ("place", index),
                place.label.clone(),
                place.path.clone(),
                cx,
            ));
        }
        let favorites = self.favorite_entries();
        if !favorites.is_empty() {
            side = side.child(section_label("Favorites", c.dim));
            for (index, (label, path)) in favorites.into_iter().enumerate() {
                side = side.child(self.place_row(("fav", index), label, path, cx));
            }
        }
        if !self.media.is_empty() {
            side = side.child(section_label("Media", c.dim));
            for (index, place) in self.media.iter().enumerate() {
                side = side.child(self.place_row(
                    ("media", index),
                    place.label.clone(),
                    place.path.clone(),
                    cx,
                ));
            }
        }
        side
    }

    fn place_row(
        &self,
        id: impl Into<gpui::ElementId>,
        label: String,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let c = self.palette;
        let active = path == self.cwd;
        let click_path = path.clone();
        let menu_path = path.clone();
        attach_drop(
            div()
                .id(id)
                .mx(px(8.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .bg(rgb(if active { c.select } else { c.panel }))
                .text_color(rgb(if active { c.accent } else { c.text }))
                .hover(|style| style.bg(rgb(c.elevated)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                        this.navigate(click_path.clone(), true, None, window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.open_menu(menu_path.clone(), event.position, window, cx);
                        cx.stop_propagation();
                    }),
                ),
            path,
            true,
            c.drop,
            cx,
        )
        .child(label)
    }

    fn render_main(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .key_context("BeeFile")
            .track_focus(&self.focus)
            .child(self.render_header(cx))
            .child(if self.help {
                self.render_help()
            } else {
                self.render_rows(cx)
            })
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.palette;
        div()
            .w_full()
            .flex()
            .items_center()
            .h(px(ROW_H))
            .px(px(8.))
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(rgb(c.muted))
            .border_b_1()
            .border_color(rgb(c.line))
            .child(div().w(px(18.)).flex_shrink_0())
            .child(div().w(px(14.)).flex_shrink_0())
            .child(self.header_cell("Name", SortKey::Name, true, cx))
            .child(self.header_cell("Size", SortKey::Size, false, cx))
            .child(self.header_cell("Modified", SortKey::Modified, false, cx))
    }

    fn header_cell(
        &self,
        title: &'static str,
        key: SortKey,
        grow: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label = format!("{title}{}", self.sort_arrow(key));
        let mut cell = div()
            .id(title)
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                    if this.typing() {
                        return;
                    }
                    this.listing.set_sort(key);
                    this.reveal(cx);
                }),
            )
            .child(label);
        if grow {
            cell = cell.flex_1().min_w(px(0.));
        } else if key == SortKey::Size {
            cell = cell.w(px(84.)).flex_shrink_0().flex().justify_end();
        } else {
            cell = cell.w(px(148.)).flex_shrink_0();
        }
        cell
    }

    fn render_rows(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        if self.listing.is_empty() {
            let message = if self.listing.query.is_empty() {
                "Empty directory"
            } else {
                "Nothing matches"
            };
            let cwd = self.cwd.clone();
            let c = self.palette;
            return attach_drop(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(c.muted)),
                cwd.clone(),
                false,
                c.drop,
                cx,
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_menu(cwd.clone(), event.position, window, cx);
                }),
            )
            .child(message);
        }
        let count = self.listing.len();
        let cwd = self.cwd.clone();
        let cwd_menu = cwd.clone();
        let list_drop = self.palette.drop;
        div().flex_1().min_h(px(0.)).child(
            attach_drop(
                uniform_list(
                    "rows",
                    count,
                    cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                        let mut items = Vec::new();
                        for ix in range {
                            let Some(info) = this.listing.info(ix) else {
                                continue;
                            };
                            let name = info.name.to_string();
                            let kind = info.kind;
                            let is_parent = info.is_parent;
                            let size = info.size;
                            let modified = info.modified;
                            let path = info.path.to_path_buf();
                            let marked = this.marks.contains(&path);
                            let selected = ix == this.listing.cursor;
                            let c = this.palette;
                            let name_color = if is_parent {
                                c.dim
                            } else {
                                match kind {
                                    Kind::Dir => c.accent,
                                    Kind::Symlink => c.link,
                                    Kind::File => c.text,
                                    Kind::Other => c.muted,
                                }
                            };
                            let size_text = if is_parent || size.is_none() {
                                "-".into()
                            } else {
                                format::format_size(size.unwrap_or(0))
                            };
                            let time_text = modified
                                .map(format::format_mtime)
                                .unwrap_or_else(|| "-".into());
                            let drop_here = is_parent
                                || kind == Kind::Dir
                                || (kind == Kind::Symlink && path.is_dir());
                            let menu_path = path.clone();
                            let mut row = div()
                                .id(("row", ix))
                                .w_full()
                                .flex()
                                .items_center()
                                .h(px(ROW_H))
                                .px(px(8.))
                                .gap(px(8.))
                                .bg(rgb(if selected { c.select } else { c.bg }))
                                .cursor_pointer()
                                .hover(|style| {
                                    if selected {
                                        style
                                    } else {
                                        style.bg(rgb(c.elevated))
                                    }
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        this.on_row_click(ix, event, window, cx);
                                    }),
                                )
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.listing.move_to(ix);
                                        this.focus.focus(window);
                                        this.open_menu(
                                            menu_path.clone(),
                                            event.position,
                                            window,
                                            cx,
                                        );
                                    }),
                                );
                            if !is_parent && this.prompt.is_none() {
                                let dragged = FileDrag {
                                    paths: this.drag_paths_for(&path),
                                };
                                let ghost = c;
                                row = row.on_drag(dragged, move |drag, _offset, _window, cx| {
                                    let label = drag_label(&drag.paths);
                                    cx.new(move |_| DragGhost {
                                        label,
                                        colors: ghost,
                                    })
                                });
                            }
                            row = if drop_here {
                                attach_drop(row, path.clone(), true, c.drop, cx)
                            } else {
                                swallow_drop(row, cx)
                            };
                            items.push(
                                row.child(
                                    div()
                                        .w(px(18.))
                                        .flex_shrink_0()
                                        .flex()
                                        .justify_center()
                                        .child(dot(c.accent, marked)),
                                )
                                .child(
                                    div()
                                        .w(px(14.))
                                        .flex_shrink_0()
                                        .flex()
                                        .justify_center()
                                        .child(kind_dot(kind, is_parent, c)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_color(rgb(name_color))
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .w(px(84.))
                                        .flex_shrink_0()
                                        .flex()
                                        .justify_end()
                                        .text_color(rgb(c.muted))
                                        .child(size_text),
                                )
                                .child(
                                    div()
                                        .w(px(148.))
                                        .flex_shrink_0()
                                        .text_color(rgb(c.muted))
                                        .child(time_text),
                                ),
                            );
                        }
                        items
                    }),
                )
                .track_scroll(self.scroll.clone())
                .size_full(),
                cwd,
                false,
                list_drop,
                cx,
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_menu(cwd_menu.clone(), event.position, window, cx);
                }),
            ),
        )
    }

    fn render_help(&self) -> gpui::Div {
        let c = self.palette;
        let mut list = div()
            .flex_1()
            .flex()
            .flex_col()
            .px(px(20.))
            .py(px(12.))
            .gap(px(4.))
            .overflow_hidden()
            .child(
                div()
                    .text_color(rgb(c.accent))
                    .pb(px(6.))
                    .child(format!("BeeFile {}", crate::version::label())),
            );
        for (keys, action) in HELP {
            list = list.child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(div().w(px(220.)).text_color(rgb(c.text)).child(*keys))
                    .child(div().text_color(rgb(c.muted)).child(*action)),
            );
        }
        list
    }

    fn render_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.palette;
        if let Some(prompt) = &self.prompt {
            let message = prompt_message(prompt);
            let danger = matches!(prompt.kind, PromptKind::Delete { .. });
            div()
                .key_context("Prompt")
                .track_focus(&self.prompt_focus)
                .on_key_down(cx.listener(Self::handle_prompt_key))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(36.))
                .px(px(12.))
                .bg(rgb(c.elevated))
                .border_t_1()
                .border_color(rgb(if danger { c.danger } else { c.accent }))
                .child(
                    div()
                        .text_color(rgb(if danger { c.danger } else { c.accent }))
                        .child(prompt.kind.label()),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(message),
                )
        } else {
            let hint = if self.listing.query.is_empty() {
                "Press  /  to filter     ?  help".to_string()
            } else {
                format!("Filter: {}     esc clears", self.listing.query)
            };
            div()
                .flex()
                .items_center()
                .h(px(28.))
                .px(px(12.))
                .text_size(px(12.))
                .text_color(rgb(c.dim))
                .bg(rgb(c.panel))
                .border_t_1()
                .border_color(rgb(c.line))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                        let text = this.listing.query.clone();
                        let cursor = text.len();
                        this.open_prompt(PromptKind::Filter, text, cursor, window, cx);
                    }),
                )
                .child(hint)
        }
    }

    fn render_status(&self) -> impl IntoElement {
        let visible = self
            .listing
            .rows
            .iter()
            .filter(|row| matches!(row, model::Row::Item(_)))
            .count();
        let mut parts = vec![format!("{visible} items")];
        if !self.listing.show_hidden {
            let hidden = self.listing.hidden_count();
            if hidden > 0 {
                parts.push(format!("{hidden} hidden"));
            }
        }
        if !self.marks.is_empty() {
            parts.push(format!("{} marked", self.marks.len()));
        }
        if let Some(free) = self.free {
            parts.push(format!("{} free", format::format_size(free)));
        }
        parts.push(format!(
            "sort {}",
            if self.listing.ascending {
                self.listing.sort.label()
            } else {
                match self.listing.sort {
                    SortKey::Name => "name desc",
                    SortKey::Size => "size desc",
                    SortKey::Modified => "modified desc",
                }
            }
        ));
        if let Some(clip) = &self.clipboard {
            let (word, count) = match clip {
                Clip::Copy(paths) => ("copy", paths.len()),
                Clip::Cut(paths) => ("cut", paths.len()),
            };
            parts.push(format!("{word} {count}"));
        }
        if self.busy {
            parts.push("busy".into());
        }
        let left = if let Some(info) = self.listing.cursor_info() {
            match &info.kind {
                Kind::Symlink => {
                    let target = self
                        .listing
                        .entries
                        .iter()
                        .find(|entry| entry.path == info.path)
                        .and_then(|entry| entry.link_target.as_ref())
                        .map(|target| target.display().to_string())
                        .unwrap_or_else(|| "link".into());
                    format!("{} → {target}", info.name)
                }
                _ => info.name.to_string(),
            }
        } else {
            String::new()
        };
        let c = self.palette;
        let theme_label = self.theme_label.clone();

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .h(px(28.))
            .px(px(12.))
            .bg(rgb(c.panel))
            .border_t_1()
            .border_color(rgb(c.line))
            .text_size(px(12.))
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(rgb(if self.error.is_some() {
                        c.danger
                    } else {
                        c.text
                    }))
                    .child(
                        self.error
                            .clone()
                            .or_else(|| self.note.clone())
                            .unwrap_or(left),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .flex_shrink_0()
                    .child(
                        div()
                            .text_color(rgb(if self.note.as_deref() == Some("Reading…") {
                                c.accent
                            } else if self
                                .clipboard
                                .as_ref()
                                .is_some_and(|clip| matches!(clip, Clip::Cut(_)))
                            {
                                c.ok
                            } else {
                                c.muted
                            }))
                            .whitespace_nowrap()
                            .child(parts.join("   ")),
                    )
                    .child(
                        div()
                            .text_color(rgb(c.dim))
                            .whitespace_nowrap()
                            .child(theme_label),
                    )
                    .child(
                        div()
                            .text_color(rgb(c.dim))
                            .whitespace_nowrap()
                            .child(crate::version::label()),
                    ),
            )
    }

    fn menu_layer(&self, menu: ContextMenu, cx: &mut Context<Self>) -> impl IntoElement {
        let path = menu.path.clone();
        let is_dir = path.is_dir();
        let is_file = path.is_file();
        let favorite = self.favorites.iter().any(|have| have == &path);
        let favorite_label = if favorite {
            "Remove from favorites".to_string()
        } else {
            "Add to favorites".to_string()
        };
        let title = format::display_path(&path, &self.home);
        let c = self.palette;
        let mut panel = div()
            .id("context-menu")
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.menu = None;
                cx.notify();
            }))
            .w(px(280.))
            .py(px(4.))
            .bg(rgb(c.elevated))
            .border_1()
            .border_color(rgb(c.line))
            .rounded(px(6.))
            .text_size(px(13.))
            .child(
                div()
                    .px(px(12.))
                    .pt(px(6.))
                    .pb(px(4.))
                    .text_size(px(12.))
                    .text_color(rgb(c.muted))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            );
        if is_file {
            panel = self.file_menu(panel, &menu, cx);
        } else if is_dir {
            let foot_path = path.clone();
            let cursor_path = path.clone();
            panel = panel
                .child(menu_action(
                    "context-menu-foot",
                    "Open foot here".to_string(),
                    c.select,
                    cx,
                    move |this, _window, cx| this.open_here(fsops::HereApp::Foot, &foot_path, cx),
                ))
                .child(menu_action(
                    "context-menu-cursor",
                    "Open Cursor here".to_string(),
                    c.select,
                    cx,
                    move |this, _window, cx| {
                        this.open_here(fsops::HereApp::Cursor, &cursor_path, cx);
                    },
                ));
        }
        if is_dir || favorite {
            if is_dir || is_file {
                panel = panel.child(menu_rule(c.line));
            }
            let fav_path = path.clone();
            panel = panel.child(menu_action(
                "context-menu-favorite",
                favorite_label,
                c.select,
                cx,
                move |this, _window, cx| this.toggle_favorite(&fav_path, cx),
            ));
        }
        anchored()
            .position(menu.position)
            .snap_to_window_with_margin(px(8.))
            .child(panel)
    }

    fn file_menu(
        &self,
        mut panel: gpui::Stateful<gpui::Div>,
        menu: &ContextMenu,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = self.palette;
        let Some(ext) = menu.ext.clone() else {
            let open_path = menu.path.clone();
            return panel.child(menu_action(
                "context-menu-open",
                "Open".to_string(),
                colors.select,
                cx,
                move |this, _window, cx| this.open_file(&open_path, cx),
            ));
        };
        let linked = self.openers.get(&ext).cloned();
        if let Some(spec) = linked.clone() {
            if !menu.apps.iter().any(|app| app.id == spec) {
                let open_path = menu.path.clone();
                let open_ext = ext.clone();
                let open_spec = spec.clone();
                let display = spec.clone();
                panel = panel.child(menu_action(
                    "context-menu-linked",
                    format!("{spec}  ·  linked"),
                    colors.select,
                    cx,
                    move |this, _window, cx| {
                        this.use_opener(
                            open_ext.clone(),
                            open_spec.clone(),
                            display.clone(),
                            open_path.clone(),
                            true,
                            cx,
                        );
                    },
                ));
            }
        }
        for (index, app) in menu.apps.iter().cloned().enumerate() {
            let marked = linked.as_deref() == Some(app.id.as_str());
            let name = app_label(&app, &menu.apps);
            let label = if marked {
                format!("{name}  ·  linked")
            } else {
                name.clone()
            };
            let open_path = menu.path.clone();
            let open_ext = ext.clone();
            let spec = app.id.clone();
            panel = panel.child(menu_action(
                ("opener", index),
                label,
                colors.select,
                cx,
                move |this, _window, cx| {
                    this.use_opener(
                        open_ext.clone(),
                        spec.clone(),
                        name.clone(),
                        open_path.clone(),
                        marked,
                        cx,
                    );
                },
            ));
        }
        panel = panel.child(menu_rule(colors.line));
        let prompt_path = menu.path.clone();
        let prompt_ext = ext.clone();
        let prefill = linked
            .clone()
            .filter(|spec| !openers::is_desktop_id(spec))
            .unwrap_or_default();
        panel = panel.child(menu_action(
            "context-menu-other",
            "Other command…".to_string(),
            colors.select,
            cx,
            move |this, window, cx| {
                this.prompt_opener(
                    prompt_path.clone(),
                    prompt_ext.clone(),
                    prefill.clone(),
                    window,
                    cx,
                );
            },
        ));
        if linked.is_some() {
            let clear_ext = ext;
            panel = panel.child(menu_action(
                "context-menu-clear",
                "Clear link".to_string(),
                colors.select,
                cx,
                move |this, _window, cx| this.unlink_extension(&clear_ext, cx),
            ));
        }
        panel
    }
}

#[derive(Clone)]
struct ContextMenu {
    path: PathBuf,
    position: Point<Pixels>,
    ext: Option<String>,
    apps: Vec<openers::AppChoice>,
}

fn app_label(app: &openers::AppChoice, apps: &[openers::AppChoice]) -> String {
    let duplicated = apps.iter().filter(|other| other.name == app.name).count() > 1;
    if duplicated {
        let id = app.id.trim_end_matches(".desktop");
        format!("{} ({id})", app.name)
    } else {
        app.name.clone()
    }
}

fn menu_action(
    id: impl Into<gpui::ElementId>,
    label: String,
    select: u32,
    cx: &mut Context<Browser>,
    on_press: impl Fn(&mut Browser, &mut Window, &mut Context<Browser>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(12.))
        .py(px(6.))
        .mx(px(4.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(select)))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                on_press(this, window, cx);
                cx.stop_propagation();
            }),
        )
        .child(label)
}

fn menu_rule(line: u32) -> gpui::Div {
    div().my(px(4.)).mx(px(8.)).h(px(1.)).bg(rgb(line))
}

fn folder_label(path: &Path, home: &Path) -> String {
    if path == Path::new("/") {
        return "/".into();
    }
    if path == home {
        return "Home".into();
    }
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

struct FileDrag {
    paths: Vec<PathBuf>,
}

struct DragGhost {
    label: String,
    colors: Palette,
}

impl Render for DragGhost {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let copy = window.modifiers().control;
        let verb = if copy { "Copy" } else { "Move" };
        let colors = self.colors;
        div()
            .px(px(10.))
            .py(px(6.))
            .bg(rgb(colors.elevated))
            .border_1()
            .border_color(rgb(if copy { colors.link } else { colors.accent }))
            .rounded(px(4.))
            .text_size(px(13.))
            .text_color(rgb(colors.text))
            .child(format!("{verb} {}", self.label))
    }
}

fn drag_label(paths: &[PathBuf]) -> String {
    match paths {
        [only] => only
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| only.display().to_string()),
        _ => format!("{} items", paths.len()),
    }
}

fn attach_drop<E>(
    element: E,
    dest: PathBuf,
    highlight: bool,
    drop_color: u32,
    cx: &mut Context<Browser>,
) -> E
where
    E: InteractiveElement,
{
    let external_dest = dest.clone();
    let external_color = drop_color;
    let mut element = element
        .on_drop(cx.listener(move |this, drag: &FileDrag, window, cx| {
            let copy = window.modifiers().control;
            this.accept_drop(drag.paths.clone(), dest.clone(), copy, cx);
        }))
        .on_drop(
            cx.listener(move |this, drag: &gpui::ExternalPaths, _window, cx| {
                this.accept_drop(drag.paths().to_vec(), external_dest.clone(), true, cx);
            }),
        );
    if highlight {
        element = element
            .drag_over::<FileDrag>(move |style, _, _, _| style.bg(rgb(drop_color)))
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.bg(rgb(external_color)));
    }
    element
}

fn swallow_drop<E>(element: E, cx: &mut Context<Browser>) -> E
where
    E: InteractiveElement,
{
    element
        .on_drop(
            cx.listener(
                |_: &mut Browser, _: &FileDrag, _: &mut Window, _: &mut Context<Browser>| {},
            ),
        )
        .on_drop(cx.listener(
            |_: &mut Browser, _: &gpui::ExternalPaths, _: &mut Window, _: &mut Context<Browser>| {},
        ))
}

fn section_label(text: &'static str, dim: u32) -> gpui::Div {
    div()
        .px(px(16.))
        .pt(px(8.))
        .pb(px(4.))
        .text_size(px(11.))
        .text_color(rgb(dim))
        .child(text)
}

fn dot(color: u32, on: bool) -> gpui::Div {
    div()
        .w(px(7.))
        .h(px(7.))
        .rounded(px(4.))
        .bg(if on { rgb(color) } else { rgba(0) })
}

fn kind_dot(kind: Kind, parent: bool, colors: Palette) -> gpui::Div {
    let color = if parent {
        colors.dim
    } else {
        match kind {
            Kind::Dir => colors.accent,
            Kind::Symlink => colors.link,
            Kind::File => colors.file,
            Kind::Other => colors.danger,
        }
    };
    div().w(px(8.)).h(px(8.)).rounded(px(2.)).bg(rgb(color))
}

fn prompt_message(prompt: &PromptState) -> String {
    match &prompt.kind {
        PromptKind::Trash { paths } => {
            format!("{}?   y confirm   n cancel", describe(paths, "Trash"))
        }
        PromptKind::Delete { paths } => {
            format!("{}?   y confirm   n cancel", describe(paths, "Delete"))
        }
        PromptKind::LinkOpen { ext, .. } if prompt.text.is_empty() => {
            format!("command for .{ext}|")
        }
        _ => {
            let cursor = prompt.cursor.min(prompt.text.len());
            let (head, tail) = prompt.text.split_at(cursor);
            format!("{head}|{tail}")
        }
    }
}

fn describe(paths: &[PathBuf], verb: &str) -> String {
    if paths.len() == 1 {
        let name = paths[0]
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| paths[0].display().to_string());
        format!("{verb} {name}")
    } else {
        format!("{verb} {} items", paths.len())
    }
}

fn prev_boundary(text: &str, cursor: usize) -> usize {
    text.char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index < cursor)
        .last()
        .unwrap_or(0)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text.char_indices()
        .map(|(index, _)| index)
        .find(|index| *index > cursor)
        .unwrap_or(text.len())
}

fn simple_word_left(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    let head = text[..cursor].trim_end();
    match head.rfind(char::is_whitespace) {
        Some(index) => {
            let width = head[index..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            index + width
        }
        None => 0,
    }
}
