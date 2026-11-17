use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(0xFF00_0000 | v)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub fg: String,
    pub bg: String,
    pub gutter_fg: String,
    pub selection: String,
    pub cursorline: String,
    pub tab_active: String,
    pub tab_inactive: String,
    pub status_bg: String,
    pub accent: String,
    pub error: String,
    pub warn: String,
    pub popup_bg: String,
    pub popup_fg: String,
    pub popup_sel: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            fg: "#d8d8da".into(),
            bg: "#1c1c20".into(),
            gutter_fg: "#6a6a72".into(),
            cursorline: "#2a2b33".into(),
            selection: "#3a3d4a".into(),
            tab_active: "#2f3240".into(),
            tab_inactive: "#22232a".into(),
            status_bg: "#2a2b33".into(),
            accent: "#7aa2f7".into(),
            error: "#f7768e".into(),
            warn: "#e0af68".into(),
            popup_bg: "#26262e".into(),
            popup_fg: "#d8d8da".into(),
            popup_sel: "#3f4a5f".into(),
        }
    }
}

fn flip_hex(s: &str) -> String {
    let Some(v) = parse_hex(s) else {
        return s.to_string();
    };
    let (r, g, b) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    if mx - mn > 32 {
        return s.to_string();
    }
    let v = (255 - r) << 16 | (255 - g) << 8 | (255 - b);
    format!("#{v:06x}")
}

/// Mirrors every near-neutral color around mid gray so the palette swaps
/// between dark and light. Saturated accents keep their identity, and the
/// transform is its own inverse.
pub fn flip_lightness(t: &Theme) -> Theme {
    Theme {
        fg: flip_hex(&t.fg),
        bg: flip_hex(&t.bg),
        gutter_fg: flip_hex(&t.gutter_fg),
        selection: flip_hex(&t.selection),
        cursorline: flip_hex(&t.cursorline),
        tab_active: flip_hex(&t.tab_active),
        tab_inactive: flip_hex(&t.tab_inactive),
        status_bg: flip_hex(&t.status_bg),
        accent: flip_hex(&t.accent),
        error: flip_hex(&t.error),
        warn: flip_hex(&t.warn),
        popup_bg: flip_hex(&t.popup_bg),
        popup_fg: flip_hex(&t.popup_fg),
        popup_sel: flip_hex(&t.popup_sel),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontCfg {
    pub family: String,
    pub size: f32,
}

impl Default for FontCfg {
    fn default() -> Self {
        Self {
            family: "monospace".into(),
            size: 14.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font: FontCfg,
    pub theme: Theme,
    pub tab_width: u32,
    pub line_numbers: bool,
    pub clipboard: bool,
    pub show_whitespace: bool,
    pub spellcheck: bool,
    pub spell_dict: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontCfg::default(),
            theme: Theme::default(),
            tab_width: 4,
            line_numbers: true,
            clipboard: true,
            show_whitespace: false,
            spellcheck: false,
            spell_dict: String::new(),
        }
    }
}

impl Config {
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_yaml_ng::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save_default(&self) -> Result<PathBuf, String> {
        let p = config_path();
        self.save(&p)?;
        Ok(p)
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("DEDITOR_CONFIG") {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(not(windows))]
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".config")
        }
    };
    base.join("deditor").join("config.yaml")
}

pub fn load() -> (Config, Vec<String>) {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_yaml_ng::from_str::<Config>(&text) {
            Ok(c) => {
                let (_, warnings) = RgbTheme::from_theme_checked(&c.theme);
                (c, warnings)
            }
            Err(e) => (
                Config::default(),
                vec![format!("invalid config {}: {e}", path.display())],
            ),
        },
        Err(_) => (Config::default(), Vec::new()),
    }
}

pub struct RgbTheme {
    pub fg: u32,
    pub bg: u32,
    pub gutter_fg: u32,
    pub selection: u32,
    pub cursorline: u32,
    pub tab_active: u32,
    pub tab_inactive: u32,
    pub status_bg: u32,
    pub accent: u32,
    pub error: u32,
    pub warn: u32,
    pub popup_bg: u32,
    pub popup_fg: u32,
    pub popup_sel: u32,
}

fn hex_checked(raw: &str, name: &str, def: u32, warnings: &mut Vec<String>) -> u32 {
    match parse_hex(raw) {
        Some(v) => v,
        None => {
            warnings.push(format!("invalid theme.{name} color {raw:?}, using default"));
            def
        }
    }
}

impl RgbTheme {
    pub fn from_theme_checked(t: &Theme) -> (Self, Vec<String>) {
        let mut w = Vec::new();
        let s = Self {
            fg: hex_checked(&t.fg, "fg", 0xFFD8D8DA, &mut w),
            bg: hex_checked(&t.bg, "bg", 0xFF1C1C20, &mut w),
            gutter_fg: hex_checked(&t.gutter_fg, "gutter_fg", 0xFF6A6A72, &mut w),
            cursorline: hex_checked(&t.cursorline, "cursorline", 0xFF2A2B33, &mut w),
            selection: hex_checked(&t.selection, "selection", 0xFF3A3D4A, &mut w),
            tab_active: hex_checked(&t.tab_active, "tab_active", 0xFF2F3240, &mut w),
            tab_inactive: hex_checked(&t.tab_inactive, "tab_inactive", 0xFF22232A, &mut w),
            status_bg: hex_checked(&t.status_bg, "status_bg", 0xFF2A2B33, &mut w),
            accent: hex_checked(&t.accent, "accent", 0xFF7AA2F7, &mut w),
            error: hex_checked(&t.error, "error", 0xFFF7768E, &mut w),
            warn: hex_checked(&t.warn, "warn", 0xFFE0AF68, &mut w),
            popup_bg: hex_checked(&t.popup_bg, "popup_bg", 0xFF26262E, &mut w),
            popup_fg: hex_checked(&t.popup_fg, "popup_fg", 0xFFD8D8DA, &mut w),
            popup_sel: hex_checked(&t.popup_sel, "popup_sel", 0xFF3F4A5F, &mut w),
        };
        (s, w)
    }
}

pub const FONT_SIZES: [f32; 6] = [10.0, 12.0, 14.0, 16.0, 20.0, 24.0];
pub const TAB_WIDTHS: [u32; 3] = [2, 4, 8];
pub const FONT_FAMILIES: [&str; 4] = [
    "monospace",
    "DejaVu Sans Mono",
    "JetBrains Mono",
    "Fira Code",
];
pub const PALETTE: [&str; 12] = [
    "#d8d8da", "#1c1c20", "#6a6a72", "#3a3d4a", "#2f3240", "#22232a", "#2a2b33", "#7aa2f7",
    "#f7768e", "#e0af68", "#26262e", "#3f4a5f",
];

pub const SETTINGS: [&str; 21] = [
    "font.family",
    "font.size",
    "tab_width",
    "line_numbers",
    "clipboard",
    "show_whitespace",
    "spellcheck",
    "theme.fg",
    "theme.bg",
    "theme.gutter_fg",
    "theme.selection",
    "theme.cursorline",
    "theme.tab_active",
    "theme.tab_inactive",
    "theme.status_bg",
    "theme.accent",
    "theme.error",
    "theme.warn",
    "theme.popup_bg",
    "theme.popup_fg",
    "theme.popup_sel",
];

pub fn theme_color<'a>(t: &'a Theme, name: &str) -> Option<&'a str> {
    Some(match name {
        "fg" => &t.fg,
        "bg" => &t.bg,
        "gutter_fg" => &t.gutter_fg,
        "selection" => &t.selection,
        "cursorline" => &t.cursorline,
        "tab_active" => &t.tab_active,
        "tab_inactive" => &t.tab_inactive,
        "status_bg" => &t.status_bg,
        "accent" => &t.accent,
        "error" => &t.error,
        "warn" => &t.warn,
        "popup_bg" => &t.popup_bg,
        "popup_fg" => &t.popup_fg,
        "popup_sel" => &t.popup_sel,
        _ => return None,
    })
}

pub fn theme_color_mut<'a>(t: &'a mut Theme, name: &str) -> Option<&'a mut String> {
    Some(match name {
        "fg" => &mut t.fg,
        "bg" => &mut t.bg,
        "gutter_fg" => &mut t.gutter_fg,
        "selection" => &mut t.selection,
        "cursorline" => &mut t.cursorline,
        "tab_active" => &mut t.tab_active,
        "tab_inactive" => &mut t.tab_inactive,
        "status_bg" => &mut t.status_bg,
        "accent" => &mut t.accent,
        "error" => &mut t.error,
        "warn" => &mut t.warn,
        "popup_bg" => &mut t.popup_bg,
        "popup_fg" => &mut t.popup_fg,
        "popup_sel" => &mut t.popup_sel,
        _ => return None,
    })
}

fn next_in<T: PartialEq + Clone>(list: &[T], cur: &T) -> T {
    match list.iter().position(|v| v == cur) {
        Some(i) => list[(i + 1) % list.len()].clone(),
        None => list[0].clone(),
    }
}

pub fn setting_value(cfg: &Config, key: &str) -> String {
    match key {
        "font.family" => cfg.font.family.clone(),
        "font.size" => format!("{}", cfg.font.size),
        "tab_width" => format!("{}", cfg.tab_width),
        "line_numbers" => format!("{}", cfg.line_numbers),
        "clipboard" => format!("{}", cfg.clipboard),
        "show_whitespace" => format!("{}", cfg.show_whitespace),
        "spellcheck" => format!("{}", cfg.spellcheck),
        k if k.starts_with("theme.") => theme_color(&cfg.theme, &k["theme.".len()..])
            .unwrap_or("?")
            .to_string(),
        _ => String::new(),
    }
}

pub fn setting_activate(cfg: &mut Config, key: &str) {
    match key {
        "font.family" => {
            let fams: Vec<String> = FONT_FAMILIES.iter().map(|s| s.to_string()).collect();
            cfg.font.family = next_in(&fams, &cfg.font.family);
        }
        "font.size" => cfg.font.size = next_in(&FONT_SIZES, &cfg.font.size),
        "tab_width" => cfg.tab_width = next_in(&TAB_WIDTHS, &cfg.tab_width),
        "line_numbers" => cfg.line_numbers = !cfg.line_numbers,
        "clipboard" => cfg.clipboard = !cfg.clipboard,
        "show_whitespace" => cfg.show_whitespace = !cfg.show_whitespace,
        "spellcheck" => cfg.spellcheck = !cfg.spellcheck,
        k if k.starts_with("theme.") => {
            let name = &k["theme.".len()..];
            let cur = theme_color(&cfg.theme, name).map(|s| s.to_string());
            if let Some(cur) = cur {
                let pal: Vec<String> = PALETTE.iter().map(|s| s.to_string()).collect();
                let n = next_in(&pal, &cur);
                if let Some(slot) = theme_color_mut(&mut cfg.theme, name) {
                    *slot = n;
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_yaml_keeps_defaults() {
        let c: Config = serde_yaml_ng::from_str("tab_width: 2\n").unwrap();
        assert_eq!(c.tab_width, 2);
        assert_eq!(c.font, FontCfg::default());
        assert_eq!(c.theme, Theme::default());
        assert!(c.line_numbers);
    }

    #[test]
    fn bad_color_warns_and_keeps_default() {
        let t = Theme {
            fg: "blorbe".into(),
            ..Theme::default()
        };
        let (rgb, w) = RgbTheme::from_theme_checked(&t);
        assert_eq!(rgb.fg, 0xFFD8D8DA);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("theme.fg"));
    }

    #[test]
    fn cursorline_is_editable_and_checked() {
        let mut c = Config::default();
        assert_eq!(setting_value(&c, "theme.cursorline"), "#2a2b33");
        assert!(SETTINGS.contains(&"theme.cursorline"));
        setting_activate(&mut c, "theme.cursorline");
        assert_ne!(c.theme.cursorline, Theme::default().cursorline);
        let (rgb, w) = RgbTheme::from_theme_checked(&c.theme);
        assert!(w.is_empty());
        assert_eq!(
            rgb.cursorline,
            parse_hex(&c.theme.cursorline).unwrap_or_default()
        );
        let bad = Theme {
            cursorline: "not-a-color".into(),
            ..Theme::default()
        };
        let (rgb, w) = RgbTheme::from_theme_checked(&bad);
        assert_eq!(rgb.cursorline, 0xFF2A2B33);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("theme.cursorline"));
    }

    #[test]
    fn save_load_roundtrip() {
        let mut c = Config::default();
        c.font.size = 20.0;
        c.theme.accent = "#123456".into();
        let mut path = std::env::temp_dir();
        path.push(format!("deditor-cfg-test-{}.yaml", std::process::id()));
        c.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let back: Config = serde_yaml_ng::from_str(&text).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(back, c);
    }

    #[test]
    fn activate_bool_size_color() {
        let mut c = Config::default();
        setting_activate(&mut c, "line_numbers");
        assert!(!c.line_numbers);
        setting_activate(&mut c, "line_numbers");
        assert!(c.line_numbers);
        assert_eq!(setting_value(&c, "clipboard"), "true");
        setting_activate(&mut c, "clipboard");
        assert!(!c.clipboard);
        assert!(SETTINGS.contains(&"clipboard"));
        setting_activate(&mut c, "font.size");
        assert_eq!(c.font.size, 16.0);
        setting_activate(&mut c, "theme.accent");
        assert!(parse_hex(&c.theme.accent).is_some());
        assert_ne!(c.theme.accent, Theme::default().accent);
        assert_eq!(setting_value(&c, "tab_width"), "4");
    }

    #[test]
    fn show_whitespace_round_trips() {
        let mut c = Config::default();
        assert!(!c.show_whitespace);
        assert_eq!(setting_value(&c, "show_whitespace"), "false");
        assert!(SETTINGS.contains(&"show_whitespace"));
        setting_activate(&mut c, "show_whitespace");
        assert!(c.show_whitespace);
        assert_eq!(setting_value(&c, "show_whitespace"), "true");
    }

    #[test]
    fn spellcheck_round_trips() {
        let mut c = Config::default();
        assert!(!c.spellcheck);
        assert!(c.spell_dict.is_empty());
        assert_eq!(setting_value(&c, "spellcheck"), "false");
        assert!(SETTINGS.contains(&"spellcheck"));
        setting_activate(&mut c, "spellcheck");
        assert!(c.spellcheck);
        assert_eq!(setting_value(&c, "spellcheck"), "true");
    }

    #[test]
    fn example_yaml_parses() {
        let text = include_str!("../config.example.yaml");
        let c: Config = serde_yaml_ng::from_str(text).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn lightness_flip_swaps_dark_and_light_exactly() {
        let dark = Theme::default();
        let light = flip_lightness(&dark);
        assert_eq!(light.bg, "#e3e3df");
        assert_eq!(light.fg, "#272725");
        let lum = |s: &str| {
            let v = parse_hex(s).unwrap();
            ((v >> 16) & 0xFF) + ((v >> 8) & 0xFF) + (v & 0xFF)
        };
        assert!(lum(&light.bg) > lum(&light.fg));
        assert_eq!(light.accent, dark.accent);
        assert_eq!(light.error, dark.error);
        assert_eq!(light.warn, dark.warn);
        assert_eq!(flip_lightness(&light), dark);
    }

    #[test]
    fn flip_hex_keeps_saturated_and_broken_colors() {
        assert_eq!(flip_hex("#7aa2f7"), "#7aa2f7");
        assert_eq!(flip_hex("#3a3d4a"), "#c5c2b5");
        assert_eq!(flip_hex("nonsense"), "nonsense");
        assert_eq!(flip_hex("#12"), "#12");
    }
}
