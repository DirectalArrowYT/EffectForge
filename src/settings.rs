//! The window's own choices (theme, auto save), kept in `forge_data/settings.json`.

use eframe::egui::{self, Color32, Stroke, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    Dark,
    Ember,
}

impl Theme {
    pub const ALL: [Theme; 2] = [Theme::Dark, Theme::Ember];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Ember => "Ember",
        }
    }

    /// What the preview clears to behind the effect.
    pub fn backdrop(self) -> [f64; 3] {
        match self {
            Theme::Dark => [0.013, 0.012, 0.024],
            Theme::Ember => [0.009, 0.005, 0.016],
        }
    }

    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(egui::Theme::Dark);
        ctx.set_visuals_of(egui::Theme::Dark, self.visuals());
    }

    fn visuals(self) -> Visuals {
        let mut v = Visuals::dark();
        // the text of picked items and menus: white reads on any highlight
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        if self == Theme::Dark {
            return v;
        }
        // dark purple with orange accents
        let bg = Color32::from_rgb(0x17, 0x0e, 0x22);
        let panel = Color32::from_rgb(0x1f, 0x13, 0x2e);
        let widget = Color32::from_rgb(0x2e, 0x1c, 0x44);
        let hover = Color32::from_rgb(0x44, 0x2a, 0x63);
        let stripe = Color32::from_rgb(0x26, 0x18, 0x38);
        let text = Color32::from_rgb(0xe9, 0xdf, 0xf5);
        let orange = Color32::from_rgb(0xff, 0x8a, 0x1f);
        let orange_dim = Color32::from_rgb(0xb8, 0x5c, 0x10);
        v.panel_fill = panel;
        v.window_fill = panel;
        v.window_stroke = Stroke::new(1.0, widget);
        v.extreme_bg_color = bg;
        v.faint_bg_color = stripe;
        v.code_bg_color = stripe;
        v.hyperlink_color = orange;
        v.selection.bg_fill = orange_dim;
        v.slider_trailing_fill = true;
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = panel;
        w.noninteractive.weak_bg_fill = panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, widget);
        w.noninteractive.fg_stroke = Stroke::new(1.0, text);
        w.inactive.bg_fill = widget;
        w.inactive.weak_bg_fill = widget;
        w.inactive.fg_stroke = Stroke::new(1.0, text);
        w.hovered.bg_fill = hover;
        w.hovered.weak_bg_fill = hover;
        w.hovered.bg_stroke = Stroke::new(1.0, orange);
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.bg_fill = orange_dim;
        w.active.weak_bg_fill = orange_dim;
        w.active.bg_stroke = Stroke::new(1.0, orange);
        w.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
        w.open.bg_fill = hover;
        w.open.weak_bg_fill = hover;
        w.open.fg_stroke = Stroke::new(1.0, text);
        v
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: Theme,
    /// Save a few seconds after the last change.
    #[serde(default = "yes")]
    pub auto_save: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings { theme: Theme::default(), auto_save: true }
    }
}

fn path() -> std::path::PathBuf {
    crate::scratch_dirs::app_storage_root().join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path(), text);
        }
    }
}
