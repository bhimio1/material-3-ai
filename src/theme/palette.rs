use egui::Color32;
use material_colors::color::Argb;
use material_colors::scheme::Scheme;
use material_colors::theme::ThemeBuilder;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

/// Declares the palette struct and the per-role mapping from a
/// `material_colors` scheme in one place so the role list can't drift.
macro_rules! palette_roles {
    ($($role:ident),* $(,)?) => {
        /// The Material 3 color roles the UI paints with.
        #[derive(Clone, Debug, PartialEq)]
        pub struct M3Palette {
            $(pub $role: Color32,)*
        }

        impl M3Palette {
            fn from_scheme(s: &Scheme) -> Self {
                Self { $($role: argb(s.$role),)* }
            }

            /// Override roles present in a matugen `theme.json` map.
            fn apply_overrides(&mut self, map: &HashMap<String, String>) {
                $(
                    if let Some(c) = map.get(stringify!($role)).and_then(|h| parse_hex(h)) {
                        self.$role = c;
                    }
                )*
            }
        }
    };
}

palette_roles!(
    primary,
    on_primary,
    primary_container,
    on_primary_container,
    secondary,
    on_secondary,
    secondary_container,
    on_secondary_container,
    tertiary,
    on_tertiary,
    tertiary_container,
    on_tertiary_container,
    error,
    on_error,
    error_container,
    on_error_container,
    background,
    on_background,
    surface,
    on_surface,
    surface_variant,
    on_surface_variant,
    outline,
    outline_variant,
    shadow,
    scrim,
    inverse_surface,
    inverse_on_surface,
    inverse_primary,
    surface_container_lowest,
    surface_container_low,
    surface_container,
    surface_container_high,
    surface_container_highest,
);

pub const DEFAULT_SEED: &str = "#4285F4";

fn argb(c: Argb) -> Color32 {
    Color32::from_rgba_unmultiplied(c.red, c.green, c.blue, c.alpha)
}

pub fn parse_hex(hex: &str) -> Option<Color32> {
    let hex = hex.trim().trim_start_matches('#');
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)),
        8 => Some(Color32::from_rgba_unmultiplied(
            (v >> 24) as u8,
            (v >> 16) as u8,
            (v >> 8) as u8,
            v as u8,
        )),
        _ => None,
    }
}

pub fn to_hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

fn relative_luminance(c: Color32) -> f32 {
    let f = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
}

impl M3Palette {
    /// A full tonal scheme generated from a seed color.
    pub fn from_seed(seed: &str, dark: bool) -> Self {
        let source = Argb::from_str(seed.trim())
            .unwrap_or_else(|_| Argb::from_str(DEFAULT_SEED).expect("valid default seed"));
        let theme = ThemeBuilder::with_source(source).build();
        let scheme = if dark {
            &theme.schemes.dark
        } else {
            &theme.schemes.light
        };
        Self::from_scheme(scheme)
    }

    /// Load a matugen `theme.json`. Roles missing from the file are filled
    /// in from a scheme generated off the file's own primary color, so a
    /// partial file still produces a coherent palette.
    pub fn from_theme_json(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let map: HashMap<String, String> =
            serde_json::from_str::<HashMap<String, serde_json::Value>>(&contents)?
                .into_iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                .collect();

        let background = map.get("background").and_then(|h| parse_hex(h));
        let dark = background
            .map(|c| relative_luminance(c) < 0.2)
            .unwrap_or(true);
        let seed = map
            .get("primary")
            .filter(|h| parse_hex(h).is_some())
            .cloned()
            .unwrap_or_else(|| DEFAULT_SEED.to_string());

        let mut palette = Self::from_seed(&seed, dark);
        palette.apply_overrides(&map);
        Ok(palette)
    }

    pub fn is_dark(&self) -> bool {
        relative_luminance(self.background) < 0.2
    }
}

/// Mix `a` towards `b` by `t` (0..=1), used for M3 state layers.
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#FF0000"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse_hex("00ff00"), Some(Color32::from_rgb(0, 255, 0)));
        assert_eq!(parse_hex("#12"), None);
        assert_eq!(to_hex(Color32::from_rgb(1, 2, 255)), "#0102FF");
    }

    #[test]
    fn seed_palettes_have_expected_brightness() {
        assert!(M3Palette::from_seed(DEFAULT_SEED, true).is_dark());
        assert!(!M3Palette::from_seed(DEFAULT_SEED, false).is_dark());
        // Garbage seed falls back instead of panicking.
        let _ = M3Palette::from_seed("not a color", false);
    }

    #[test]
    fn theme_json_overrides_and_fills_gaps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.json");
        std::fs::write(
            &path,
            r##"{"primary": "#ff0000", "background": "#101010", "bogus": 3}"##,
        )
        .unwrap();
        let p = M3Palette::from_theme_json(&path).unwrap();
        assert_eq!(p.primary, Color32::from_rgb(255, 0, 0));
        assert_eq!(p.background, Color32::from_rgb(16, 16, 16));
        assert!(p.is_dark());
    }
}
