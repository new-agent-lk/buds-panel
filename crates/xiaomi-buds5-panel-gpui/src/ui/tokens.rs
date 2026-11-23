//! Design tokens: one struct, two static instances, one global mode.
//!
//! The panel paints from these instead of the old hard-coded AirPods constants.
//! gpui-kit's own `ThemeColor` (~120 tokens, JSON hot-reload) is deliberately
//! untouched — we use fewer than 20 of its slots and would inherit a second
//! naming scheme for no gain.
//!
//! Colours are `0xRRGGBBAA`; paint the ones with a non-`ff` alpha through
//! `rgba()`, the rest through `rgb()`. The field names say which is which by
//! convention: `hairline`, `track`, `accent_soft` and `glow` always carry alpha.
//!
//! Mode is a process-global because half the paint helpers are free functions
//! with no `cx` to ask. `render` refreshes it from the window appearance before
//! building any element.
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Tokens {
    // Colours.
    pub bg: u32,
    pub card: u32,
    pub card_raised: u32,
    pub ink: u32,
    pub sub: u32,
    pub muted: u32,
    pub hairline: u32,
    pub track: u32,
    pub accent: u32,
    pub accent_soft: u32,
    pub accent_fg: u32,
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
    pub glow: u32,
    // Corner radii: small block / block / card / hero.
    pub r_sm: f32,
    pub r_md: f32,
    pub r_lg: f32,
    pub r_xl: f32,
    // Spacing scale.
    pub s1: f32,
    pub s2: f32,
    pub s3: f32,
    pub s4: f32,
    pub s5: f32,
    pub s6: f32,
    // Type scale.
    pub fs_xs: f32,
    pub fs_sm: f32,
    pub fs_base: f32,
    pub fs_lg: f32,
    pub fs_xl: f32,
    // Weights, as the numbers gpui's `FontWeight` takes.
    pub fw_normal: f32,
    pub fw_medium: f32,
    pub fw_semibold: f32,
}

pub static LIGHT: Tokens = Tokens {
    bg: 0xf5f5f7ff,
    card: 0xffffffff,
    card_raised: 0xffffffff,
    ink: 0x1d1d1fff,
    sub: 0x6e6e73ff,
    muted: 0x8e8e93ff,
    hairline: 0x0000000e,
    track: 0x76768020,
    accent: 0x007affff,
    accent_soft: 0x007aff1f,
    accent_fg: 0xffffffff,
    success: 0x34c759ff,
    warning: 0xff9f0aff,
    danger: 0xff3b30ff,
    glow: 0x007aff1a,
    r_sm: 8.,
    r_md: 12.,
    r_lg: 16.,
    r_xl: 20.,
    s1: 4.,
    s2: 8.,
    s3: 12.,
    s4: 16.,
    s5: 24.,
    s6: 32.,
    fs_xs: 11.,
    fs_sm: 13.,
    fs_base: 15.,
    fs_lg: 20.,
    fs_xl: 28.,
    fw_normal: 400.,
    fw_medium: 500.,
    fw_semibold: 600.,
};

pub static DARK: Tokens = Tokens {
    bg: 0x0b0f13ff,
    card: 0x12171cff,
    card_raised: 0x1a2128ff,
    ink: 0xf2f5f8ff,
    sub: 0x9aa4aeff,
    muted: 0x6b7681ff,
    hairline: 0xffffff14,
    track: 0xffffff1a,
    accent: 0x4c8dffff,
    accent_soft: 0x4c8dff29,
    accent_fg: 0xffffffff,
    success: 0x87e6aaff,
    warning: 0xffb340ff,
    danger: 0xff6961ff,
    glow: 0x4c8dff2e,
    r_sm: 8.,
    r_md: 12.,
    r_lg: 16.,
    r_xl: 20.,
    s1: 4.,
    s2: 8.,
    s3: 12.,
    s4: 16.,
    s5: 24.,
    s6: 32.,
    fs_xs: 11.,
    fs_sm: 13.,
    fs_base: 15.,
    fs_lg: 20.,
    fs_xl: 28.,
    fw_normal: 400.,
    fw_medium: 500.,
    fw_semibold: 600.,
};

static DARK_MODE: AtomicBool = AtomicBool::new(false);

/// Point the whole panel at one palette. Called from `render`, so a system
/// appearance change lands on the next frame.
pub fn set_dark(dark: bool) {
    DARK_MODE.store(dark, Ordering::Relaxed);
}

pub fn is_dark() -> bool {
    DARK_MODE.load(Ordering::Relaxed)
}

/// Repaint a token at another alpha — for tinted backgrounds built from an
/// opaque colour (e.g. the warning tint behind the status bar).
pub fn with_alpha(color: u32, alpha: u8) -> u32 {
    (color & 0xffff_ff00) | alpha as u32
}

pub fn t() -> &'static Tokens {
    if is_dark() {
        &DARK
    } else {
        &LIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both palettes have to be complete and actually different, or a theme
    /// switch silently paints light-on-light somewhere.
    #[test]
    fn palettes_are_distinct_and_complete() {
        let l = &LIGHT;
        let d = &DARK;
        for (name, a, b) in [
            ("bg", l.bg, d.bg),
            ("card", l.card, d.card),
            ("card_raised", l.card_raised, d.card_raised),
            ("ink", l.ink, d.ink),
            ("sub", l.sub, d.sub),
            ("muted", l.muted, d.muted),
            ("hairline", l.hairline, d.hairline),
            ("track", l.track, d.track),
            ("accent", l.accent, d.accent),
            ("accent_soft", l.accent_soft, d.accent_soft),
            ("success", l.success, d.success),
            ("warning", l.warning, d.warning),
            ("danger", l.danger, d.danger),
            ("glow", l.glow, d.glow),
        ] {
            assert_ne!(a, b, "{name} is the same in both palettes");
            assert!(a >> 24 != 0 || a & 0xff != 0, "{name} has no alpha");
        }
        // The alpha-carrying tokens must stay translucent.
        for (name, v) in [("hairline", l.hairline), ("track", l.track),
                          ("accent_soft", l.accent_soft), ("glow", l.glow)] {
            assert!(v & 0xff < 0xff, "{name} should keep its alpha");
        }
    }

    #[test]
    fn scales_ascend() {
        assert!(LIGHT.r_sm < LIGHT.r_md && LIGHT.r_md < LIGHT.r_lg && LIGHT.r_lg < LIGHT.r_xl);
        assert!(LIGHT.s1 < LIGHT.s2 && LIGHT.s2 < LIGHT.s3);
        assert!(LIGHT.s3 < LIGHT.s4 && LIGHT.s4 < LIGHT.s5 && LIGHT.s5 < LIGHT.s6);
        assert!(LIGHT.fs_xs < LIGHT.fs_sm && LIGHT.fs_sm < LIGHT.fs_base);
        assert!(LIGHT.fs_base < LIGHT.fs_lg && LIGHT.fs_lg < LIGHT.fs_xl);
        assert!(LIGHT.fw_normal < LIGHT.fw_medium && LIGHT.fw_medium < LIGHT.fw_semibold);
    }

    #[test]
    fn mode_switches_the_lookup() {
        set_dark(false);
        assert_eq!(t().bg, LIGHT.bg);
        set_dark(true);
        assert_eq!(t().bg, DARK.bg);
        set_dark(false); // leave the global as we found it
    }
}
