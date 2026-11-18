//! How a picked colour is written when it is copied: `#rrggbb` (default), `rgb()` or `hsl()`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorFormat {
    #[default]
    Hex,
    Rgb,
    Hsl,
}

impl ColorFormat {
    pub const ALL: [ColorFormat; 3] = [ColorFormat::Hex, ColorFormat::Rgb, ColorFormat::Hsl];

    /// What the user reads in Preferences.
    pub fn name(self) -> &'static str {
        match self {
            ColorFormat::Hex => "#rrggbb (web, design apps)",
            ColorFormat::Rgb => "rgb(r, g, b) (CSS)",
            ColorFormat::Hsl => "hsl(h, s%, l%) (CSS)",
        }
    }
}

/// `rgb` written as asked.
pub fn format(rgb: [u8; 3], how: ColorFormat) -> String {
    let [r, g, b] = rgb;
    match how {
        ColorFormat::Hex => format!("#{r:02x}{g:02x}{b:02x}"),
        ColorFormat::Rgb => format!("rgb({r}, {g}, {b})"),
        ColorFormat::Hsl => {
            let (h, s, l) = hsl(rgb);
            format!("hsl({h}, {s}%, {l}%)")
        }
    }
}

/// Hue in degrees (0-359), saturation and lightness in percent, rounded.
pub fn hsl([r, g, b]: [u8; 3]) -> (u32, u32, u32) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0, 0, (l * 100.0).round() as u32);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    ((h.round() as u32) % 360, (s * 100.0).round() as u32, (l * 100.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_written_in_each_format() {
        assert_eq!(format([255, 136, 0], ColorFormat::Hex), "#ff8800");
        assert_eq!(format([255, 136, 0], ColorFormat::Rgb), "rgb(255, 136, 0)");
        assert_eq!(format([255, 0, 0], ColorFormat::Hsl), "hsl(0, 100%, 50%)");
        assert_eq!(format([0, 128, 0], ColorFormat::Hsl), "hsl(120, 100%, 25%)");
        assert_eq!(format([0, 0, 255], ColorFormat::Hsl), "hsl(240, 100%, 50%)");
        assert_eq!(format([128, 128, 128], ColorFormat::Hsl), "hsl(0, 0%, 50%)");
        assert_eq!(format([53, 132, 228], ColorFormat::Hsl), "hsl(213, 76%, 55%)");
        assert_eq!(format([255, 0, 128], ColorFormat::Hsl), "hsl(330, 100%, 50%)");
    }

    #[test]
    fn the_setting_reads_back() {
        assert_eq!(serde_json::to_string(&ColorFormat::Rgb).unwrap(), "\"rgb\"");
        assert_eq!(serde_json::from_str::<ColorFormat>("\"hsl\"").unwrap(), ColorFormat::Hsl);
    }
}
