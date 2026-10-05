# BeeFile

A keyboard-first file manager for [Omarchy](https://omarchy.org/). The window is drawn with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) 0.2.2 on Wayland, which is the compositor Hyprland uses. Directory reads, copies, and deletes run off the UI thread. The file list only builds the rows that are on screen.

`gpui_platform` from current Zed `main` is not published yet. BeeFile depends on the published `gpui` crate with the `wayland` feature and no X11.

## Scripts

```bash
scripts/dev.sh [PATH]       # debug build, then open PATH or the current directory
scripts/run.sh [PATH]       # release build, then open. Use this day to day.
scripts/test.sh             # unit tests, no window
scripts/smoke.sh            # build, open a temp folder on Wayland, expect it to stay up
scripts/install-omarchy.sh  # release build, install ~/.local/bin/beefile, enable the bar plugin
```

The first build compiles GPUI and takes a while. Later builds are incremental.

```bash
beefile                  # current directory
beefile ~/Downloads      # a directory
beefile ./notes.txt      # parent directory, with notes.txt selected
beefile --help
```

The Wayland app id is `BeeFile`, so a Hyprland rule can match it by that name.

`beefile --version` prints the version, such as `0.1.0+42`. That same string is at the right of the status bar and in the window title. The number after `+` goes up when you compile a change. It is stored in `target/beefile-build-number`.

## Keys

| Key | Action |
| --- | --- |
| `j` `k` or arrows | Move |
| `h` `←` Backspace | Parent |
| `l` `→` Enter | Open a folder, or open a file with its linked app. Otherwise `xdg-open` |
| `g g` / `G` | Top / bottom |
| `[` `]` | Back / forward |
| `~` | Home |
| Space | Mark, then move down |
| `y` `x` `p` | Copy, cut, paste |
| Drag | Move onto a folder, a place, or the path bar. Hold Ctrl to copy. A drop from another app copies |
| Right-click a folder | Add or remove a favorite, or open foot or Cursor there |
| Right-click a file | Link its extension to an app that can open it. Enter uses that link |
| `d` or Delete | Trash |
| Shift-D or Shift-Delete | Delete permanently, after confirm |
| `r` or F2 | Rename |
| `n` / Shift-N | New file / new folder |
| `/` | Filter |
| Ctrl-L | Go to a path |
| `.` | Show hidden files |
| `s` / `S` | Change sort column / reverse |
| `t` | Cycle color theme |
| `?` | Help inside the window |
| `q` or Ctrl-Q | Quit |

BeeFile will not trash or delete `/`, your home directory, or a top-level directory such as `/usr`. Paste does not overwrite an existing name. Trash follows the FreeDesktop trash spec: the home trash when the file is on that filesystem, otherwise the volume's `.Trash-$uid` directory.

`vendor/xattr` is xattr 0.2.3 with one Linux fix. GPUI 0.2.2 still depends on that release, and it does not compile against current libc (`ENOATTR` was removed; the missing-attribute errno on Linux is `ENODATA`).

## Layout

Places on the left include Home, the usual XDG folders that exist, `/`, and each mount under `/run/media/$USER`. Favorites are listed under Places once you add one. They are stored one path per line in `~/.config/beefile/favorites`, or under `$XDG_CONFIG_HOME` when that is set. Extension links live in `~/.config/beefile/openers`: one line is the extension, a tab, then a desktop id such as `imv.desktop` or a command. `%f` in a command is the file. The path bar is clickable. A dot marks a selected file. Folder names use the accent color. Symlinks use the link color.

## Themes

`t` cycles the color theme. **Honey** is BeeFile's own palette. **Omarchy** follows the theme selected on the desktop (`omarchy theme current`), and it updates when that theme changes while BeeFile is open.

The choice is one word in `~/.config/beefile/theme` (`honey` or `omarchy`), or under `$XDG_CONFIG_HOME` when that is set. With no file, BeeFile uses Omarchy when the desktop palette can be read from `$XDG_STATE_HOME/omarchy/current/theme/colors.toml`, which is `~/.local/state/omarchy/current/theme/colors.toml` by default.
