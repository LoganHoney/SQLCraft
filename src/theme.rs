//! Visual Studio 2022 style colour themes (Dark, Light, Blue), as SSMS 22 offers.

use eframe::egui::{self, Color32, CornerRadius, Stroke};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeKind {
    #[default]
    Dark,
    Light,
    Blue,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 3] = [ThemeKind::Dark, ThemeKind::Light, ThemeKind::Blue];

    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Dark => "Dark",
            ThemeKind::Light => "Light",
            ThemeKind::Blue => "Blue",
        }
    }
}

/// Colours that egui's Visuals do not cover: the editor, the status bars and the grid.
#[derive(Clone, Copy)]
pub struct Palette {
    pub editor_bg: Color32,
    pub text: Color32,
    pub gutter: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub comment: Color32,
    pub function: Color32,
    pub system_object: Color32,
    pub system_proc: Color32,
    pub operator: Color32,
    pub number: Color32,
    pub accent: Color32,
    /// The "Ready" bar along the bottom of the window.
    pub status_bar: Color32,
    pub status_text: Color32,
    /// Default background of a query window's status bar.
    pub query_status: Color32,
    pub null_bg: Color32,
    pub grid_header: Color32,
    pub border: Color32,
}

fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub fn palette(kind: ThemeKind) -> Palette {
    match kind {
        ThemeKind::Dark => Palette {
            editor_bg: hex(0x1E1E1E),
            text: hex(0xDCDCDC),
            gutter: hex(0x2B91AF),
            keyword: hex(0x569CD6),
            string: hex(0xD69D85),
            comment: hex(0x57A64A),
            function: hex(0xC586C0),
            system_object: hex(0x4EC9B0),
            system_proc: hex(0xD7BA7D),
            operator: hex(0xB4B4B4),
            number: hex(0xB5CEA8),
            accent: hex(0x3794FF),
            status_bar: hex(0x007ACC),
            status_text: Color32::WHITE,
            query_status: hex(0x2D2D30),
            null_bg: hex(0x4A4528),
            grid_header: hex(0x2D2D30),
            border: hex(0x3F3F46),
        },
        ThemeKind::Light => Palette {
            editor_bg: Color32::WHITE,
            text: Color32::BLACK,
            gutter: hex(0x2B91AF),
            keyword: hex(0x0000FF),
            string: hex(0xFF0000),
            comment: hex(0x008000),
            function: hex(0xFF00FF),
            system_object: hex(0x008000),
            system_proc: hex(0x800000),
            operator: hex(0x808080),
            number: Color32::BLACK,
            accent: hex(0x007ACC),
            status_bar: hex(0x007ACC),
            status_text: Color32::WHITE,
            query_status: hex(0xFFF8D8),
            null_bg: hex(0xFFFFE1),
            grid_header: hex(0xF0F0F0),
            border: hex(0xCCCEDB),
        },
        ThemeKind::Blue => Palette {
            editor_bg: Color32::WHITE,
            text: Color32::BLACK,
            gutter: hex(0x2B91AF),
            keyword: hex(0x0000FF),
            string: hex(0xFF0000),
            comment: hex(0x008000),
            function: hex(0xFF00FF),
            system_object: hex(0x008000),
            system_proc: hex(0x800000),
            operator: hex(0x808080),
            number: Color32::BLACK,
            accent: hex(0x3399FF),
            status_bar: hex(0x0E639C),
            status_text: Color32::WHITE,
            query_status: hex(0xFFF29D),
            null_bg: hex(0xFFFFE1),
            grid_header: hex(0xE6EBF5),
            border: hex(0x8E9BBC),
        },
    }
}

/// Applies the theme to egui's widgets.
pub fn apply(ctx: &egui::Context, kind: ThemeKind) {
    let p = palette(kind);
    let mut v = match kind {
        ThemeKind::Dark => egui::Visuals::dark(),
        _ => egui::Visuals::light(),
    };
    let (panel, window, widget, hover, extreme, text) = match kind {
        ThemeKind::Dark => (hex(0x1F1F1F), hex(0x252526), hex(0x333337), hex(0x3E3E40), hex(0x1E1E1E), hex(0xF1F1F1)),
        ThemeKind::Light => (hex(0xEEEEF2), hex(0xF5F5F5), hex(0xFCFCFC), hex(0xC9DEF5), Color32::WHITE, hex(0x1E1E1E)),
        ThemeKind::Blue => (hex(0xD6DBE9), hex(0xEEF0F7), hex(0xF5F7FC), hex(0xFDF4BF), Color32::WHITE, hex(0x1B293E)),
    };
    v.panel_fill = panel;
    v.window_fill = window;
    v.extreme_bg_color = extreme;
    v.faint_bg_color = if kind == ThemeKind::Dark { hex(0x252526) } else { hex(0xF3F3F7) };
    v.code_bg_color = p.editor_bg;
    v.override_text_color = Some(text);
    v.selection.bg_fill = if kind == ThemeKind::Dark { hex(0x264F78) } else { hex(0xADD6FF) };
    v.selection.stroke = Stroke::new(1.0, if kind == ThemeKind::Dark { Color32::WHITE } else { Color32::BLACK });
    v.hyperlink_color = p.accent;
    v.window_stroke = Stroke::new(1.0, p.border);
    let square = CornerRadius::same(2);
    v.window_corner_radius = CornerRadius::same(3);
    v.menu_corner_radius = square;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = square;
    }
    v.widgets.noninteractive.bg_fill = panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.bg_fill = widget;
    v.widgets.inactive.weak_bg_fill = widget;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.hovered.bg_fill = hover;
    v.widgets.hovered.weak_bg_fill = hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.bg_fill = p.accent;
    v.widgets.active.weak_bg_fill = p.accent;
    v.widgets.open.bg_fill = hover;
    v.widgets.open.weak_bg_fill = hover;
    ctx.set_visuals_of(if kind == ThemeKind::Dark { egui::Theme::Dark } else { egui::Theme::Light }, v);
    ctx.set_theme(if kind == ThemeKind::Dark { egui::Theme::Dark } else { egui::Theme::Light });
}

/// Black or white, whichever reads better on `bg` (for custom status bar colours).
pub fn contrast(bg: Color32) -> Color32 {
    let l = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if l > 150.0 { Color32::BLACK } else { Color32::WHITE }
}
