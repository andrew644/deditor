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
macro_rules! for_each_theme_color {
    ($mac:ident) => {
        $mac! {
            [fg, "#c8d3f5", 0xFFC8D3F5],
            [bg, "#222436", 0xFF222436],
            [gutter_fg, "#545c7e", 0xFF545C7E],
            [selection, "#2d3f76", 0xFF2D3F76],
            [cursorline, "#2f334d", 0xFF2F334D],
            [tab_active, "#3b4261", 0xFF3B4261],
            [tab_inactive, "#1e2030", 0xFF1E2030],
            [status_bg, "#1e2030", 0xFF1E2030],
            [accent, "#82aaff", 0xFF82AAFF],
            [error, "#ff757f", 0xFFFF757F],
            [warn, "#ffc777", 0xFFFFC777],
            [sign_add, "#c3e88d", 0xFFC3E88D],
            [sign_change, "#ffc777", 0xFFFFC777],
            [sign_delete, "#ff757f", 0xFFFF757F],
            [popup_bg, "#262a3f", 0xFF262A3F],
            [popup_fg, "#c8d3f5", 0xFFC8D3F5],
            [popup_sel, "#2d3f76", 0xFF2D3F76]
        }
    };
}

macro_rules! theme_items {
    ($([$n:ident, $h:literal, $r:literal]),*) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct Theme {
            $(pub $n: String,)*
        }

        impl Default for Theme {
            fn default() -> Self {
                Self { $($n: $h.into(),)* }
            }
        }
    };
}

for_each_theme_color!(theme_items);

fn flip_hex(s: &str) -> String {
    let Some(v) = parse_hex(s) else {
        return s.to_string();
    };
    let (r, g, b) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
    let v = (255 - r) << 16 | (255 - g) << 8 | (255 - b);
    format!("#{v:06x}")
}

macro_rules! theme_functions {
    ($([$n:ident, $h:literal, $r:literal]),*) => {
        pub struct RgbTheme {
            $(pub $n: u32,)*
        }

        pub const PALETTE: &[&'static str] = &[$($h,)*];

        /// Inverts every channel so the palette swaps between dark and light. Hues
        /// rotate but keep their contrast, and the transform is its own inverse.
        pub fn flip_lightness(t: &Theme) -> Theme {
            Theme { $($n: flip_hex(&t.$n),)* }
        }

        impl RgbTheme {
            pub fn from_theme_checked(t: &Theme) -> (Self, Vec<String>) {
                let mut w = Vec::new();
                let s = Self {
                    $($n: hex_checked(&t.$n, stringify!($n), $r, &mut w),)*
                };
                (s, w)
            }
        }

        pub fn theme_color<'a>(t: &'a Theme, name: &str) -> Option<&'a str> {
            Some(match name {
                $(stringify!($n) => &t.$n,)*
                _ => return None,
            })
        }

        pub fn theme_color_mut<'a>(t: &'a mut Theme, name: &str) -> Option<&'a mut String> {
            Some(match name {
                $(stringify!($n) => &mut t.$n,)*
                _ => return None,
            })
        }
    };
}

for_each_theme_color!(theme_functions);

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
    pub git_signs: bool,
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
            git_signs: true,
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

fn hex_checked(raw: &str, name: &str, def: u32, warnings: &mut Vec<String>) -> u32 {
    match parse_hex(raw) {
        Some(v) => v,
        None => {
            warnings.push(format!("invalid theme.{name} color {raw:?}, using default"));
            def
        }
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

macro_rules! theme_settings {
    ($([$n:ident, $h:literal, $r:literal]),*) => {
        pub const SETTINGS: [&str; 25] = [
            "font.family",
            "font.size",
            "tab_width",
            "line_numbers",
            "clipboard",
            "show_whitespace",
            "spellcheck",
            "git_signs",
            $(concat!("theme.", stringify!($n)),)*
        ];
    };
}

for_each_theme_color!(theme_settings);

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
        "git_signs" => format!("{}", cfg.git_signs),
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
        "git_signs" => cfg.git_signs = !cfg.git_signs,
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
        assert_eq!(rgb.fg, 0xFFC8D3F5);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("theme.fg"));
    }

    #[test]
    fn cursorline_is_editable_and_checked() {
        let mut c = Config::default();
        assert_eq!(setting_value(&c, "theme.cursorline"), "#2f334d");
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
        assert_eq!(rgb.cursorline, 0xFF2F334D);
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
        assert_eq!(light.bg, "#dddbc9");
        assert_eq!(light.fg, "#372c0a");
        let lum = |s: &str| {
            let v = parse_hex(s).unwrap();
            ((v >> 16) & 0xFF) + ((v >> 8) & 0xFF) + (v & 0xFF)
        };
        assert!(lum(&light.bg) > lum(&light.fg));
        assert!(lum(&dark.bg) < lum(&dark.fg));
        assert!(lum(&light.accent) < lum(&light.bg));
        assert_eq!(flip_lightness(&light), dark);
    }

    #[test]
    fn flip_hex_inverts_every_channel() {
        assert_eq!(flip_hex("#82aaff"), "#7d5500");
        assert_eq!(flip_hex("#222436"), "#dddbc9");
        assert_eq!(flip_hex("nonsense"), "nonsense");
        assert_eq!(flip_hex("#12"), "#12");
    }
}
