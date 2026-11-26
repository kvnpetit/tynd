//! `window.setEffects`: native window materials through `window-vibrancy`.
//!
//! The caller lists effects in order of preference; the first one the
//! running OS supports is applied, so one call covers every platform
//! (`["mica", "sidebar"]` → Mica on Windows 11, the sidebar material on
//! macOS). An empty list clears the effect. Linux has no compositor-neutral
//! material API: a non-empty list rejects there.
//!
//! | Effect | OS |
//! |---|---|
//! | `mica`, `tabbed` | Windows 11 |
//! | `acrylic` | Windows 10 1809+ / 11 |
//! | `blur` | Windows 10 / 11 |
//! | `titlebar`, `selection`, `menu`, `popover`, `sidebar`, `headerView`, `sheet`, `windowBackground`, `hudWindow`, `fullScreenUI`, `tooltip`, `contentBackground`, `underWindowBackground`, `underPageBackground` | macOS |
//!
//! The webview only shows the material where the page itself is
//! transparent (window `transparent: true` and a transparent background).

use serde_json::Value;
use tao::window::Window;

/// Longest accepted `effects` list.
const MAX_EFFECTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    Mica,
    Tabbed,
    Acrylic,
    Blur,
    Mac(MacMaterial),
}

/// `NSVisualEffectMaterial` values exposed to apps (the deprecated
/// light / dark / appearance-based ones are left out).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacMaterial {
    Titlebar,
    Selection,
    Menu,
    Popover,
    Sidebar,
    HeaderView,
    Sheet,
    WindowBackground,
    HudWindow,
    FullScreenUi,
    Tooltip,
    ContentBackground,
    UnderWindowBackground,
    UnderPageBackground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectState {
    FollowsWindowActiveState,
    Active,
    Inactive,
}

/// Parsed `setEffects` arguments.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EffectsSpec {
    pub(crate) effects: Vec<Effect>,
    pub(crate) state: Option<EffectState>,
    pub(crate) radius: Option<f64>,
    /// RGBA tint for `acrylic` / `blur`.
    pub(crate) color: Option<(u8, u8, u8, u8)>,
}

impl Effect {
    fn parse(name: &str) -> Option<Self> {
        use MacMaterial as M;
        Some(match name {
            "mica" => Self::Mica,
            "tabbed" => Self::Tabbed,
            "acrylic" => Self::Acrylic,
            "blur" => Self::Blur,
            "titlebar" => Self::Mac(M::Titlebar),
            "selection" => Self::Mac(M::Selection),
            "menu" => Self::Mac(M::Menu),
            "popover" => Self::Mac(M::Popover),
            "sidebar" => Self::Mac(M::Sidebar),
            "headerView" => Self::Mac(M::HeaderView),
            "sheet" => Self::Mac(M::Sheet),
            "windowBackground" => Self::Mac(M::WindowBackground),
            "hudWindow" => Self::Mac(M::HudWindow),
            "fullScreenUI" => Self::Mac(M::FullScreenUi),
            "tooltip" => Self::Mac(M::Tooltip),
            "contentBackground" => Self::Mac(M::ContentBackground),
            "underWindowBackground" => Self::Mac(M::UnderWindowBackground),
            "underPageBackground" => Self::Mac(M::UnderPageBackground),
            _ => return None,
        })
    }

    /// Whether the effect belongs to `os` (`std::env::consts::OS` values).
    fn runs_on(self, os: &str) -> bool {
        match self {
            Self::Mac(_) => os == "macos",
            Self::Mica | Self::Tabbed | Self::Acrylic | Self::Blur => os == "windows",
        }
    }
}

fn err(message: impl std::fmt::Display) -> String {
    format!("setEffects: {message}")
}

/// `"#rrggbb"` / `"#rrggbbaa"` → RGBA.
fn parse_color(raw: &str) -> Option<(u8, u8, u8, u8)> {
    let hex = raw.strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    let alpha = if hex.len() == 8 { byte(6)? } else { 255 };
    Some((byte(0)?, byte(2)?, byte(4)?, alpha))
}

pub(crate) fn parse(args: &Value) -> Result<EffectsSpec, String> {
    let list = args
        .get("effects")
        .and_then(Value::as_array)
        .ok_or_else(|| err("'effects' must be an array of effect names ([] clears)"))?;
    if list.len() > MAX_EFFECTS {
        return Err(err(format!("at most {MAX_EFFECTS} effects are accepted")));
    }
    let effects = list
        .iter()
        .map(|value| {
            let name = value
                .as_str()
                .ok_or_else(|| err("every effect must be a string"))?;
            Effect::parse(name).ok_or_else(|| err(format!("unknown effect '{name}'")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let state = match args.get("state").and_then(Value::as_str) {
        None => None,
        Some("followsWindowActiveState") => Some(EffectState::FollowsWindowActiveState),
        Some("active") => Some(EffectState::Active),
        Some("inactive") => Some(EffectState::Inactive),
        Some(other) => {
            return Err(err(format!(
                "unknown state '{other}'; expected \"followsWindowActiveState\", \"active\" or \"inactive\""
            )))
        },
    };
    let radius = match args.get("radius") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_f64() {
            Some(r) if r.is_finite() && (0.0..=1000.0).contains(&r) => Some(r),
            _ => return Err(err("'radius' must be a number between 0 and 1000")),
        },
    };
    let color = match args.get("color") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .and_then(parse_color)
                .ok_or_else(|| err("'color' must be \"#rrggbb\" or \"#rrggbbaa\""))?,
        ),
    };
    Ok(EffectsSpec {
        effects,
        state,
        radius,
        color,
    })
}

/// The effect to apply on `os`: `Ok(None)` clears, an error names the
/// requested effects none of which runs on `os`.
pub(crate) fn select(spec: &EffectsSpec, os: &str) -> Result<Option<Effect>, String> {
    if spec.effects.is_empty() {
        return Ok(None);
    }
    spec.effects
        .iter()
        .copied()
        .find(|effect| effect.runs_on(os))
        .map(Some)
        .ok_or_else(|| {
            err(format!(
                "not supported on {os}: none of the requested effects runs there (see the per-OS table in the window docs)"
            ))
        })
}

/// Parse, select and apply. Runs on the main thread (window command path).
pub(crate) fn set_effects(win: &Window, args: &Value) -> Result<(), String> {
    let spec = parse(args)?;
    let effect = select(&spec, std::env::consts::OS)?;
    imp::apply(win, effect, &spec)
}

#[cfg(target_os = "windows")]
mod imp {
    use tao::window::Window;

    use super::{err, Effect, EffectsSpec};

    pub(super) fn apply(
        win: &Window,
        effect: Option<Effect>,
        spec: &EffectsSpec,
    ) -> Result<(), String> {
        // Effects stack on Windows: clear every one first. Clearing one that
        // was never applied (or is unsupported on this build) is harmless.
        let _ = window_vibrancy::clear_mica(win);
        let _ = window_vibrancy::clear_tabbed(win);
        let _ = window_vibrancy::clear_acrylic(win);
        let _ = window_vibrancy::clear_blur(win);
        let result = match effect {
            // `select` never returns a macOS material on Windows.
            None | Some(Effect::Mac(_)) => return Ok(()),
            Some(Effect::Mica) => window_vibrancy::apply_mica(win, None),
            Some(Effect::Tabbed) => window_vibrancy::apply_tabbed(win, None),
            Some(Effect::Acrylic) => window_vibrancy::apply_acrylic(win, spec.color),
            Some(Effect::Blur) => window_vibrancy::apply_blur(win, spec.color),
        };
        result.map_err(err)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use tao::window::Window;
    use window_vibrancy::{NSVisualEffectMaterial as N, NSVisualEffectState as S};

    use super::{err, Effect, EffectState, EffectsSpec, MacMaterial as M};

    pub(super) fn apply(
        win: &Window,
        effect: Option<Effect>,
        spec: &EffectsSpec,
    ) -> Result<(), String> {
        let _ = window_vibrancy::clear_vibrancy(win);
        let Some(Effect::Mac(material)) = effect else {
            return Ok(());
        };
        let material = match material {
            M::Titlebar => N::Titlebar,
            M::Selection => N::Selection,
            M::Menu => N::Menu,
            M::Popover => N::Popover,
            M::Sidebar => N::Sidebar,
            M::HeaderView => N::HeaderView,
            M::Sheet => N::Sheet,
            M::WindowBackground => N::WindowBackground,
            M::HudWindow => N::HudWindow,
            M::FullScreenUi => N::FullScreenUI,
            M::Tooltip => N::Tooltip,
            M::ContentBackground => N::ContentBackground,
            M::UnderWindowBackground => N::UnderWindowBackground,
            M::UnderPageBackground => N::UnderPageBackground,
        };
        let state = spec.state.map(|state| match state {
            EffectState::FollowsWindowActiveState => S::FollowsWindowActiveState,
            EffectState::Active => S::Active,
            EffectState::Inactive => S::Inactive,
        });
        window_vibrancy::apply_vibrancy(win, material, state, spec.radius).map_err(err)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use tao::window::Window;

    use super::{Effect, EffectsSpec};

    pub(super) fn apply(
        _win: &Window,
        effect: Option<Effect>,
        _spec: &EffectsSpec,
    ) -> Result<(), String> {
        // `select` already rejected every non-empty list on Linux.
        debug_assert!(effect.is_none());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn picks_the_first_effect_of_the_running_os() {
        let spec = parse(&json!({ "effects": ["sidebar", "mica", "acrylic"] })).unwrap();
        assert_eq!(select(&spec, "windows").unwrap(), Some(Effect::Mica));
        assert_eq!(
            select(&spec, "macos").unwrap(),
            Some(Effect::Mac(MacMaterial::Sidebar))
        );
        assert!(select(&spec, "linux")
            .unwrap_err()
            .contains("not supported on linux"));
        let clear = parse(&json!({ "effects": [] })).unwrap();
        assert_eq!(select(&clear, "linux").unwrap(), None);
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(parse(&json!({})).unwrap_err().contains("'effects'"));
        assert!(parse(&json!({ "effects": ["glass"] }))
            .unwrap_err()
            .contains("unknown effect 'glass'"));
        assert!(parse(&json!({ "effects": [1] })).is_err());
        assert!(parse(&json!({ "effects": [], "state": "on" })).is_err());
        assert!(parse(&json!({ "effects": [], "radius": -1 })).is_err());
        assert!(parse(&json!({ "effects": [], "color": "red" })).is_err());
        let many: Vec<_> = (0..17).map(|_| json!("blur")).collect();
        assert!(parse(&json!({ "effects": many })).is_err());
    }

    #[test]
    fn parses_options() {
        let spec = parse(&json!({
            "effects": ["acrylic"],
            "state": "active",
            "radius": 12,
            "color": "#10203080",
        }))
        .unwrap();
        assert_eq!(spec.state, Some(EffectState::Active));
        assert_eq!(spec.radius, Some(12.0));
        assert_eq!(spec.color, Some((0x10, 0x20, 0x30, 0x80)));
        assert_eq!(parse_color("#abcdef"), Some((0xab, 0xcd, 0xef, 255)));
        assert_eq!(parse_color("#abcde"), None);
    }
}
