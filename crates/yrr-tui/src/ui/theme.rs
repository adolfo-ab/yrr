//! Catppuccin Mocha color theme for the TUI.

#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};

// ── Background ──────────────────────────────────────────────────────────────

pub const BG: Color = Color::Rgb(0x1e, 0x1e, 0x2e); // Base
pub const BG_HIGHLIGHT: Color = Color::Rgb(0x31, 0x32, 0x44); // Surface0
pub const BG_SURFACE: Color = Color::Rgb(0x45, 0x47, 0x5a); // Surface1

// ── Foreground ──────────────────────────────────────────────────────────────

pub const FG: Color = Color::Rgb(0xcd, 0xd6, 0xf4); // Text
pub const FG_DIM: Color = Color::Rgb(0xa6, 0xad, 0xc8); // Subtext0
pub const FG_DARK: Color = Color::Rgb(0x6c, 0x70, 0x86); // Overlay0

// ── Borders ─────────────────────────────────────────────────────────────────

pub const BORDER: Color = Color::Rgb(0x45, 0x47, 0x5a); // Surface1
pub const BORDER_HIGHLIGHT: Color = Color::Rgb(0x93, 0x99, 0xb2); // Overlay2

// ── Accent colors ───────────────────────────────────────────────────────────

pub const RED: Color = Color::Rgb(0xf3, 0x8b, 0xa8); // Red
pub const GREEN: Color = Color::Rgb(0xa6, 0xe3, 0xa1); // Green
pub const YELLOW: Color = Color::Rgb(0xf9, 0xe2, 0xaf); // Yellow
pub const BLUE: Color = Color::Rgb(0x89, 0xb4, 0xfa); // Blue
pub const AQUA: Color = Color::Rgb(0x94, 0xe2, 0xd5); // Teal
pub const ORANGE: Color = Color::Rgb(0xfa, 0xb3, 0x87); // Peach
pub const VIOLET: Color = Color::Rgb(0xcb, 0xa6, 0xf7); // Mauve
pub const PINK: Color = Color::Rgb(0xf5, 0xc2, 0xe7); // Pink
pub const ASH: Color = Color::Rgb(0x7f, 0x84, 0x9c); // Overlay1
pub const TEAL: Color = Color::Rgb(0x89, 0xdc, 0xeb); // Sky

// ── Node status colors ─────────────────────────────────────────────────────

pub const NODE_IDLE: Color = GREEN;
pub const NODE_BUSY: Color = ORANGE;
pub const NODE_STOPPED: Color = FG_DARK;
pub const NODE_PENDING: Color = FG_DIM;
pub const NODE_SELECTED: Color = BLUE;
pub const NODE_STEER: Color = VIOLET;

// ── Status bar ──────────────────────────────────────────────────────────────

pub const STATUS_PREVIEW_BG: Color = BG_SURFACE;
pub const STATUS_PREVIEW_FG: Color = FG;
pub const STATUS_RUNNING_BG: Color = Color::Rgb(0x31, 0x32, 0x44); // Surface0
pub const STATUS_RUNNING_FG: Color = FG;
pub const STATUS_FINISHED_BG: Color = BG_HIGHLIGHT;
pub const STATUS_FINISHED_FG: Color = FG_DIM;

// ── Convenience styles ──────────────────────────────────────────────────────

pub fn label() -> Style {
    Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
}

pub fn value() -> Style {
    Style::default().fg(FG)
}

pub fn dim() -> Style {
    Style::default().fg(FG_DARK)
}

pub fn signal() -> Style {
    Style::default().fg(AQUA)
}

pub fn border() -> Style {
    Style::default().fg(BORDER)
}
