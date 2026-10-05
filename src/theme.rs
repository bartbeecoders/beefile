//! Color themes. Honey is BeeFile's own palette. Omarchy follows the desktop
//! theme in `~/.local/state/omarchy/current/theme/colors.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::fsops;

/// Which palette BeeFile paints with. Stored in `~/.config/beefile/theme`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeId {
    Honey,
    Omarchy,
}

impl ThemeId {
    pub fn next(self) -> Self {
        match self {
            Self::Honey => Self::Omarchy,
            Self::Omarchy => Self::Honey,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Honey => "honey",
            Self::Omarchy => "omarchy",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "honey" => Some(Self::Honey),
            "omarchy" => Some(Self::Omarchy),
            _ => None,
        }
    }
}

/// Colors the file list paints with. Values are `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: u32,
    pub panel: u32,
    pub elevated: u32,
    pub line: u32,
    pub text: u32,
    pub muted: u32,
    pub dim: u32,
    pub accent: u32,
    pub select: u32,
    pub drop: u32,
    pub link: u32,
    pub danger: u32,
    pub ok: u32,
    pub file: u32,
}

/// BeeFile's own palette. Kept as the default look when Omarchy colors are absent.
pub const HONEY: Palette = Palette {
    bg: 0x161410,
    panel: 0x201e1a,
    elevated: 0x2a2722,
    line: 0x3a342c,
    text: 0xf4efe6,
    muted: 0xa39886,
    dim: 0x6f675c,
    accent: 0xe2b657,
    select: 0x3c3424,
    drop: 0x4a3c22,
    link: 0x8fb7d6,
    danger: 0xe07a5f,
    ok: 0x9cba7a,
    file: 0x8a8175,
};

pub struct Resolved {
    pub palette: Palette,
    pub label: String,
    /// False when Omarchy was requested and `colors.toml` could not be read.
    pub available: bool,
}

/// `~/.config/beefile/theme`, or under `$XDG_CONFIG_HOME` when that is set.
pub fn theme_file() -> PathBuf {
    fsops::config_dir().join("theme")
}

/// Directory Omarchy rewrites when `omarchy theme set` runs.
/// `$XDG_STATE_HOME/omarchy/current`, or `~/.local/state/omarchy/current`.
pub fn omarchy_current_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| fsops::home_dir().join(".local").join("state"));
    base.join("omarchy").join("current")
}

pub fn read_choice(file: &Path) -> Option<ThemeId> {
    let text = std::fs::read_to_string(file).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(id) = ThemeId::parse(line) {
            return Some(id);
        }
    }
    None
}

pub fn write_choice(file: &Path, id: ThemeId) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Saving theme {}: {err}", file.display()))?;
    }
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, format!("{}\n", id.as_str()))
        .map_err(|err| format!("Saving theme {}: {err}", file.display()))?;
    std::fs::rename(&tmp, file).map_err(|err| format!("Saving theme {}: {err}", file.display()))
}

/// Saved choice, or Omarchy when the desktop palette is on disk.
pub fn initial_choice(file: &Path) -> ThemeId {
    if let Some(id) = read_choice(file) {
        return id;
    }
    let colors = omarchy_current_dir().join("theme").join("colors.toml");
    if colors.is_file() {
        ThemeId::Omarchy
    } else {
        ThemeId::Honey
    }
}

pub fn resolve(id: ThemeId) -> Resolved {
    match id {
        ThemeId::Honey => Resolved {
            palette: HONEY,
            label: "Honey".into(),
            available: true,
        },
        ThemeId::Omarchy => {
            let dir = omarchy_current_dir();
            let colors = std::fs::read_to_string(dir.join("theme").join("colors.toml")).ok();
            let name = std::fs::read_to_string(dir.join("theme.name")).ok();
            resolve_omarchy(name.as_deref(), colors.as_deref())
        }
    }
}

pub fn resolve_omarchy(name: Option<&str>, colors: Option<&str>) -> Resolved {
    match colors.and_then(palette_from_omarchy) {
        Some(palette) => Resolved {
            palette,
            label: omarchy_label(name),
            available: true,
        },
        None => Resolved {
            palette: HONEY,
            label: "Omarchy".into(),
            available: false,
        },
    }
}

fn omarchy_label(name: Option<&str>) -> String {
    match name.map(str::trim).filter(|text| !text.is_empty()) {
        Some(slug) => format!("Omarchy · {}", display_theme_name(slug)),
        None => "Omarchy".into(),
    }
}

/// `tokyo-night` becomes `Tokyo Night`, matching `omarchy theme current`.
pub fn display_theme_name(slug: &str) -> String {
    let slug = slug.trim();
    if slug.is_empty() {
        return "Omarchy".into();
    }
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Map an Omarchy `colors.toml` onto BeeFile's roles.
///
/// Omarchy names `dark_foreground` after a terminal slot. On a light theme that
/// slot is often a pale gray, so text roles are chosen by contrast with the
/// background instead of by the name.
pub fn palette_from_omarchy(text: &str) -> Option<Palette> {
    let map = parse_map(text);
    let bg = hex(map.get("background")?)?;
    let foreground = hex(map.get("foreground")?)?;
    let get = |key: &str| map.get(key).and_then(|value| hex(value));

    let text_color = ink(foreground, bg);
    let muted = text_step(
        &[
            get("light_foreground"),
            get("dark_foreground"),
            get("muted"),
            get("bright_foreground"),
        ],
        bg,
        text_color,
        &[text_color],
        3.1,
        0.4,
    );
    let mut dim = text_step(
        &[get("dark_foreground"), get("muted")],
        bg,
        text_color,
        &[text_color, muted],
        2.4,
        0.62,
    );
    if dim == muted || dim == text_color {
        dim = text_step(&[], bg, muted, &[text_color, muted], 2.3, 0.42);
    }

    let panel = surface(
        &[
            get("dark_background"),
            get("darker_background"),
            get("lighter_background"),
        ],
        bg,
        text_color,
        1.05,
        1.85,
        0.07,
    );
    let mut elevated = surface(
        &[
            get("lighter_background"),
            get("selection"),
            get("dark_background"),
        ],
        bg,
        text_color,
        1.12,
        2.15,
        0.14,
    );
    elevated = different(elevated, panel, text_color);
    let line = surface(
        &[get("muted"), get("lighter_background"), get("selection")],
        bg,
        text_color,
        1.12,
        2.05,
        0.2,
    );
    let select = different(
        surface(
            &[get("selection"), get("lighter_background"), Some(elevated)],
            bg,
            text_color,
            1.12,
            2.5,
            0.18,
        ),
        bg,
        text_color,
    );
    let accent = accent_color(
        &[get("accent"), get("yellow"), get("blue"), get("cyan")],
        bg,
        text_color,
    );
    let link = link_color(get("blue"), get("cyan"), accent, bg);
    let danger = force_contrast(get("red").unwrap_or(0xe07a5f), bg, text_color, 2.6);
    let ok = force_contrast(get("green").unwrap_or(0x9cba7a), bg, text_color, 2.6);
    let file = text_step(&[], bg, text_color, &[], 2.1, 0.48);
    let drop = drop_color(select, accent, bg);

    Some(Palette {
        bg,
        panel,
        elevated,
        line,
        text: text_color,
        muted,
        dim,
        accent,
        select,
        drop,
        link,
        danger,
        ok,
        file,
    })
}

fn parse_map(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = value_of(value);
        if value.is_empty() {
            continue;
        }
        map.insert(key.to_string(), value.to_string());
    }
    map
}

fn value_of(raw: &str) -> &str {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('"') {
        return rest.split('"').next().unwrap_or("");
    }
    if let Some(rest) = raw.strip_prefix('\'') {
        return rest.split('\'').next().unwrap_or("");
    }
    raw.split_whitespace().next().unwrap_or("")
}

fn hex(value: &str) -> Option<u32> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(value, 16).ok()
}

fn channel(hex: u32, shift: u32) -> u32 {
    (hex >> shift) & 0xff
}

fn pack(r: u32, g: u32, b: u32) -> u32 {
    (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
}

fn mix(from: u32, to: u32, toward: f32) -> u32 {
    let toward = toward.clamp(0.0, 1.0);
    let blend = |shift: u32| {
        let start = channel(from, shift) as f32;
        let end = channel(to, shift) as f32;
        (start + (end - start) * toward).round() as u32
    };
    pack(blend(16), blend(8), blend(0))
}

fn rel_luminance(color: u32) -> f32 {
    fn chan(value: u32) -> f32 {
        let s = value as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * chan(channel(color, 16))
        + 0.7152 * chan(channel(color, 8))
        + 0.0722 * chan(channel(color, 0))
}

fn contrast(a: u32, b: u32) -> f32 {
    let left = rel_luminance(a);
    let right = rel_luminance(b);
    let (hi, lo) = if left > right {
        (left, right)
    } else {
        (right, left)
    };
    (hi + 0.05) / (lo + 0.05)
}

fn channel_distance(a: u32, b: u32) -> u32 {
    channel(a, 16).abs_diff(channel(b, 16))
        + channel(a, 8).abs_diff(channel(b, 8))
        + channel(a, 0).abs_diff(channel(b, 0))
}

fn ink(foreground: u32, bg: u32) -> u32 {
    let fallback = if rel_luminance(bg) > 0.5 {
        0x000000
    } else {
        0xffffff
    };
    force_contrast(foreground, bg, fallback, 4.5)
}

fn force_contrast(color: u32, bg: u32, anchor: u32, min: f32) -> u32 {
    if contrast(color, bg) >= min {
        return color;
    }
    let ink = if contrast(anchor, bg) >= min {
        anchor
    } else if rel_luminance(bg) > 0.5 {
        0x000000
    } else {
        0xffffff
    };
    let mut best = ink;
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..14 {
        let mid = (lo + hi) / 2.0;
        let mixed = mix(color, ink, mid);
        if contrast(mixed, bg) >= min {
            best = mixed;
            hi = mid;
        } else {
            lo = mid;
        }
    }
    best
}

fn text_step(
    candidates: &[Option<u32>],
    bg: u32,
    anchor: u32,
    avoid: &[u32],
    min: f32,
    toward_bg: f32,
) -> u32 {
    for color in candidates.iter().flatten().copied() {
        if avoid.contains(&color) {
            continue;
        }
        if contrast(color, bg) >= min {
            return color;
        }
    }
    let mut color = force_contrast(mix(anchor, bg, toward_bg), bg, anchor, min);
    if avoid.contains(&color) {
        color = force_contrast(mix(color, anchor, 0.35), bg, anchor, min);
    }
    color
}

fn surface(
    candidates: &[Option<u32>],
    bg: u32,
    text: u32,
    min: f32,
    max: f32,
    fallback_t: f32,
) -> u32 {
    for color in candidates.iter().flatten().copied() {
        if color == bg {
            continue;
        }
        let ratio = contrast(color, bg);
        if ratio >= min && ratio <= max {
            return color;
        }
    }
    let mut t = fallback_t;
    let mut color = mix(bg, text, t);
    while contrast(color, bg) < min && t < 0.55 {
        t += 0.03;
        color = mix(bg, text, t);
    }
    while contrast(color, bg) > max && t > 0.02 {
        t -= 0.02;
        color = mix(bg, text, t);
    }
    if color == bg {
        mix(bg, text, 0.08)
    } else {
        color
    }
}

fn different(color: u32, other: u32, toward: u32) -> u32 {
    if color != other {
        color
    } else {
        mix(color, toward, 0.16)
    }
}

fn accent_color(candidates: &[Option<u32>], bg: u32, text: u32) -> u32 {
    for color in candidates.iter().flatten().copied() {
        if color != text && contrast(color, bg) >= 2.6 {
            return color;
        }
    }
    let fallback = if rel_luminance(bg) > 0.5 {
        0x000000
    } else {
        0xffffff
    };
    force_contrast(
        candidates.iter().flatten().copied().next().unwrap_or(text),
        bg,
        fallback,
        2.6,
    )
}

fn link_color(blue: Option<u32>, cyan: Option<u32>, accent: u32, bg: u32) -> u32 {
    for color in [blue, cyan].into_iter().flatten() {
        if contrast(color, bg) >= 2.6 && channel_distance(color, accent) > 24 {
            return color;
        }
    }
    force_contrast(blue.unwrap_or(accent), bg, accent, 2.6)
}

fn drop_color(select: u32, accent: u32, bg: u32) -> u32 {
    let mut t = 0.42;
    let mut color = mix(select, accent, t);
    while contrast(color, bg) > 3.2 && t > 0.12 {
        t -= 0.06;
        color = mix(select, accent, t);
    }
    if color == select {
        mix(select, accent, 0.55)
    } else {
        color
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORD: &str = r##"
mode = "dark"
accent = "#81a1c1"
selection = "#434c5e"
muted = "#4c566a"
background = "#2e3440"
dark_background = "#222730"
darker_background = "#191c23"
lighter_background = "#3b4252"
foreground = "#d8dee9"
dark_foreground = "#667080"
light_foreground = "#adb5c4"
red = "#bf616a"
yellow = "#ebcb8b"
green = "#a3be8c"
cyan = "#88c0d0"
blue = "#81a1c1"
"##;

    const WHITE: &str = r##"
mode = "light"
accent = "#6e6e6e"
selection = "#c0c0c0"
muted = "#808080"
background = "#ffffff"
dark_background = "#f5f5f5"
darker_background = "#e8e8e8"
lighter_background = "#c0c0c0"
foreground = "#000000"
dark_foreground = "#c0c0c0"
light_foreground = "#000000"
red = "#2a2a2a"
yellow = "#4a4a4a"
green = "#3a3a3a"
cyan = "#3e3e3e"
blue = "#1a1a1a"
"##;

    const DAWN: &str = r##"
mode = "light"
accent = "#56949f"
selection = "#dfdad9"
muted = "#cecacd"
background = "#faf4ed"
dark_background = "#ede7e1"
lighter_background = "#f2e9e1"
foreground = "#575279"
dark_foreground = "#9893a5"
light_foreground = "#6e6a86"
red = "#b4637a"
yellow = "#ea9d34"
green = "#286983"
cyan = "#56949f"
blue = "#286983"
"##;

    fn assert_readable(palette: Palette) {
        assert!(
            contrast(palette.text, palette.bg) >= 4.5,
            "text {:06x} on {:06x}",
            palette.text,
            palette.bg
        );
        assert!(contrast(palette.muted, palette.bg) >= 3.0);
        assert!(contrast(palette.dim, palette.bg) >= 2.3);
        assert!(contrast(palette.accent, palette.bg) >= 2.6);
        assert!(contrast(palette.link, palette.bg) >= 2.6);
        assert!(contrast(palette.danger, palette.bg) >= 2.6);
        assert!(contrast(palette.ok, palette.bg) >= 2.6);
        assert_ne!(palette.text, palette.muted);
        assert_ne!(palette.muted, palette.dim);
        assert_ne!(palette.panel, palette.bg);
        assert_ne!(palette.select, palette.bg);
        assert_ne!(palette.line, palette.bg);
        assert!(contrast(palette.panel, palette.bg) < 2.2);
        assert!(contrast(palette.select, palette.bg) < 2.8);
        assert!(contrast(palette.elevated, palette.bg) < 2.5);
    }

    #[test]
    fn honey_matches_the_original_palette() {
        assert_eq!(HONEY.bg, 0x161410);
        assert_eq!(HONEY.accent, 0xe2b657);
        assert_eq!(HONEY.link, 0x8fb7d6);
        assert_eq!(HONEY.file, 0x8a8175);
        assert_readable(HONEY);
    }

    #[test]
    fn nord_uses_the_omarchy_background_and_a_distinct_link() {
        let palette = palette_from_omarchy(NORD).unwrap();
        assert_eq!(palette.bg, 0x2e3440);
        assert_eq!(palette.text, 0xd8dee9);
        assert_eq!(palette.accent, 0x81a1c1);
        assert_eq!(palette.link, 0x88c0d0);
        assert_eq!(palette.danger, 0xbf616a);
        assert_eq!(palette.ok, 0xa3be8c);
        assert_eq!(palette.panel, 0x222730);
        assert_eq!(palette.select, 0x434c5e);
        assert_readable(palette);
    }

    #[test]
    fn light_themes_do_not_use_pale_slots_as_text() {
        for fixture in [WHITE, DAWN] {
            let palette = palette_from_omarchy(fixture).unwrap();
            assert_ne!(palette.dim, 0xc0c0c0);
            assert_ne!(palette.muted, palette.bg);
            assert_readable(palette);
        }
        let white = palette_from_omarchy(WHITE).unwrap();
        assert_eq!(white.bg, 0xffffff);
        assert_eq!(white.text, 0x000000);
        assert_eq!(white.muted, 0x808080);
    }

    #[test]
    fn comments_quotes_and_missing_accent_still_parse() {
        let text = r##"
# comment
background = '#112233'
foreground = "#eeeeee"
blue = "#6688aa"
"##;
        let palette = palette_from_omarchy(text).unwrap();
        assert_eq!(palette.bg, 0x112233);
        assert_eq!(palette.text, 0xeeeeee);
        assert_eq!(palette.accent, 0x6688aa);
        assert_readable(palette);
    }

    #[test]
    fn broken_colors_fall_back() {
        assert!(palette_from_omarchy("accent = \"#fff\"").is_none());
        let resolved = resolve_omarchy(Some("nord"), Some("nope"));
        assert!(!resolved.available);
        assert_eq!(resolved.palette, HONEY);
        assert_eq!(resolved.label, "Omarchy");
    }

    #[test]
    fn omarchy_label_title_cases_the_slug() {
        assert_eq!(display_theme_name("nord"), "Nord");
        assert_eq!(display_theme_name("tokyo-night"), "Tokyo Night");
        assert_eq!(
            resolve_omarchy(Some("catppuccin-latte\n"), Some(DAWN)).label,
            "Omarchy · Catppuccin Latte"
        );
        assert_eq!(resolve_omarchy(None, Some(NORD)).label, "Omarchy");
    }

    #[test]
    fn choice_round_trip_and_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("theme");
        assert!(read_choice(&file).is_none());
        write_choice(&file, ThemeId::Honey).unwrap();
        assert_eq!(read_choice(&file), Some(ThemeId::Honey));
        write_choice(&file, ThemeId::Omarchy).unwrap();
        assert_eq!(read_choice(&file), Some(ThemeId::Omarchy));
        assert_eq!(ThemeId::Honey.next(), ThemeId::Omarchy);
        assert_eq!(ThemeId::Omarchy.next().next(), ThemeId::Omarchy);
        assert_eq!(initial_choice(&file), ThemeId::Omarchy);
    }

    #[test]
    fn unknown_choice_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("theme");
        std::fs::write(&file, "# note\n\nnope\n").unwrap();
        assert_eq!(read_choice(&file), None);
    }

    #[test]
    fn stock_omarchy_themes_stay_readable() {
        let root = Path::new("/usr/share/omarchy/themes");
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        let mut seen = 0;
        for entry in entries.flatten() {
            let path = entry.path().join("colors.toml");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let palette = palette_from_omarchy(&text).unwrap_or_else(|| {
                panic!("could not parse {}", path.display());
            });
            assert_readable(palette);
            seen += 1;
        }
        if root.is_dir() {
            assert!(seen > 0, "expected stock theme colors");
        }
    }

    #[test]
    fn live_omarchy_theme_matches_colors_file() {
        let path = omarchy_current_dir().join("theme").join("colors.toml");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let expected = palette_from_omarchy(&text).expect("colors.toml");
        let resolved = resolve(ThemeId::Omarchy);
        assert!(resolved.available);
        assert_eq!(resolved.palette, expected);
        assert!(resolved.label.starts_with("Omarchy"));
        assert_readable(resolved.palette);
    }
}
