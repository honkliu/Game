//! The complete keyboard map (§3.5), resolved from raw Windows virtual-key
//! messages on the UI thread.
//!
//! Why not `RegisterHotKey`: global hotkeys are exclusive and steal the key
//! from every other app. Why not tao's keyboard events: child webviews are
//! native windows whose keystrokes may never reach tao's window proc, and the
//! design's R2 explicitly calls for a process-level hook. This module is the
//! *decision* half of that hook: the message hook in `app.rs` feeds
//! `resolve()` and forwards the result to the shell as `shell.shortcut`.
//!
//! Kept pure so the whole map is unit-testable without a window.

use serde::Serialize;

/// Virtual-key codes we name (Win32 values, stable ABI).
pub mod vk {
    pub const TAB: u32 = 0x09;
    pub const ESCAPE: u32 = 0x1B;
    pub const SPACE: u32 = 0x20;
    pub const PRIOR: u32 = 0x21;
    pub const NEXT: u32 = 0x22;
    pub const HOME: u32 = 0x24;
    pub const LEFT: u32 = 0x25;
    pub const RIGHT: u32 = 0x27;
    pub const F3: u32 = 0x72;
    pub const F5: u32 = 0x74;
    pub const F6: u32 = 0x75;
    pub const F12: u32 = 0x7B;
    pub const NUMPAD_PLUS: u32 = 0x6B;
    pub const NUMPAD_MINUS: u32 = 0x6D;
    pub const OEM_PLUS: u32 = 0xBB;
    pub const OEM_MINUS: u32 = 0xBD;
    pub const COMMA: u32 = 0xBC;
    pub const BACKSLASH: u32 = 0xDC;
    pub const N0: u32 = 0x30;
    pub const N9: u32 = 0x39;
    pub const A: u32 = 0x41;
    pub const D: u32 = 0x44;
    pub const E: u32 = 0x45;
    pub const F: u32 = 0x46;
    pub const G: u32 = 0x47;
    pub const H: u32 = 0x48;
    pub const J: u32 = 0x4A;
    pub const K: u32 = 0x4B;
    pub const L: u32 = 0x4C;
    pub const P: u32 = 0x50;
    pub const R: u32 = 0x52;
    pub const T: u32 = 0x54;
    pub const U: u32 = 0x55;
    pub const W: u32 = 0x57;
    pub const C: u32 = 0x43;
    pub const V: u32 = 0x56;
    pub const X: u32 = 0x58;
    pub const Z: u32 = 0x5A;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
    };
    pub const CTRL: Mods = Mods {
        ctrl: true,
        alt: false,
        shift: false,
    };
    pub const CTRL_SHIFT: Mods = Mods {
        ctrl: true,
        alt: false,
        shift: true,
    };
    pub const ALT: Mods = Mods {
        ctrl: false,
        alt: true,
        shift: false,
    };
    pub const CTRL_ALT: Mods = Mods {
        ctrl: true,
        alt: true,
        shift: false,
    };
    pub const SHIFT: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: true,
    };
}

/// A resolved shell action, serialized as `{name, args}` for `shell.shortcut`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "name", content = "args", rename_all = "snake_case")]
pub enum Shortcut {
    NewTab,
    CloseTab,
    CloseWindow,
    NextTab,
    PrevTab,
    RapidNextTab,
    RapidPrevTab,
    /// 1-based position; `9` means "last tab" (§3.5 `Ctrl+9`).
    TabIndex(u8),
    ReopenClosed,
    FocusOmnibox,
    CycleFocus,
    OpenPalette,
    ClosePalette,
    Find,
    FindNext,
    FindPrev,
    OpenDownloads,
    OpenHistory { today: bool },
    PageBack,
    PageForward,
    Home,
    Reload,
    ReloadIgnoringCache,
    Bookmark,
    BookmarkAllTabs,
    ViewSource,
    OpenDevtools,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    OpenSettings,
    Escape,
    /// `Space` with the rail focused — the item menu (§3.5).
    ItemMenu,
    ToggleRail,
    TogglePin,
    /// Open the AI composer on the current page.
    AskAi,
    /// `Ctrl+W` from a focused text input: the shell deletes a word when the
    /// field is non-empty and falls through to close-tab when it is empty
    /// (`data-intercept-ctrl-w="word"`, §3.5).
    CloseTabOrWord,
}

/// What the hook should do with the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Swallow the key; the shell handles `Shortcut`.
    Consume(Shortcut),
    /// Let the key through to the focused webview.
    Pass,
}

/// Resolve a keydown.
///
/// `focused_is_input` is true when the shell's focused element is a text field
/// (omnibox, find bar, history search). It suppresses Alt-navigation and turns
/// `Ctrl+W` into `CloseTabOrWord`.
pub fn resolve(vk: u32, mods: Mods, focused_is_input: bool) -> Outcome {
    use Shortcut as S;

    // --- no modifiers -----------------------------------------------------
    if !mods.ctrl && !mods.alt && !mods.shift {
        return match vk {
            vk::F3 => Outcome::Consume(S::FindNext),
            vk::F5 => Outcome::Consume(S::Reload),
            vk::F6 => Outcome::Consume(S::CycleFocus),
            vk::F12 => Outcome::Consume(S::OpenDevtools),
            vk::ESCAPE => Outcome::Consume(S::Escape),
            vk::SPACE if !focused_is_input => Outcome::Consume(S::ItemMenu),
            _ => Outcome::Pass,
        };
    }

    // --- shift only -------------------------------------------------------
    if !mods.ctrl && !mods.alt && mods.shift {
        return match vk {
            vk::F3 => Outcome::Consume(S::FindPrev),
            vk::F5 => Outcome::Consume(S::ReloadIgnoringCache),
            _ => Outcome::Pass,
        };
    }

    // --- alt only (suppressed while a shell input has focus) --------------
    if mods.alt && !mods.ctrl && !mods.shift && !focused_is_input {
        return match vk {
            vk::LEFT => Outcome::Consume(S::PageBack),
            vk::RIGHT => Outcome::Consume(S::PageForward),
            vk::HOME => Outcome::Consume(S::Home),
            vk::D => Outcome::Consume(S::FocusOmnibox),
            _ => Outcome::Pass,
        };
    }

    // --- ctrl+alt: the page-swallow-proof tab switch ----------------------
    if mods.ctrl && mods.alt && !mods.shift {
        return match vk {
            vk::LEFT => Outcome::Consume(S::RapidPrevTab),
            vk::RIGHT => Outcome::Consume(S::RapidNextTab),
            _ => Outcome::Pass,
        };
    }

    if !mods.ctrl || mods.alt {
        return Outcome::Pass;
    }

    // --- ctrl+shift -------------------------------------------------------
    if mods.shift {
        return match vk {
            vk::TAB => Outcome::Consume(S::PrevTab),
            vk::T => Outcome::Consume(S::ReopenClosed),
            vk::W => Outcome::Consume(S::CloseWindow),
            vk::K => Outcome::Consume(S::ClosePalette),
            vk::H => Outcome::Consume(S::OpenHistory { today: true }),
            vk::G => Outcome::Consume(S::FindPrev),
            vk::D => Outcome::Consume(S::BookmarkAllTabs),
            vk::R => Outcome::Consume(S::ReloadIgnoringCache),
            vk::E => Outcome::Consume(S::ToggleRail),
            vk::P => Outcome::Consume(S::TogglePin),
            vk::A => Outcome::Consume(S::AskAi),
            vk::OEM_PLUS | vk::NUMPAD_PLUS => Outcome::Consume(S::ZoomIn),
            // Ctrl+Shift+1..9 is reserved for group switching (§3.5), unbound.
            _ => Outcome::Pass,
        };
    }

    // --- ctrl only --------------------------------------------------------
    match vk {
        vk::T => Outcome::Consume(S::NewTab),
        vk::W => Outcome::Consume(if focused_is_input {
            S::CloseTabOrWord
        } else {
            S::CloseTab
        }),
        vk::TAB => Outcome::Consume(S::NextTab),
        // §3.5: Ctrl+1..8 jump to tabs, Ctrl+9 to the last tab, Ctrl+0 zoom reset.
        vk::N0 => Outcome::Consume(S::ZoomReset),
        v @ vk::N0..=vk::N9 => Outcome::Consume(S::TabIndex((v - vk::N0) as u8)),
        vk::PRIOR => Outcome::Consume(S::NextTab),
        vk::NEXT => Outcome::Consume(S::PrevTab),
        vk::L => Outcome::Consume(S::FocusOmnibox),
        vk::K => Outcome::Consume(S::OpenPalette),
        vk::F => Outcome::Consume(S::Find),
        vk::G => Outcome::Consume(S::FindNext),
        vk::J => Outcome::Consume(S::OpenDownloads),
        vk::H => Outcome::Consume(S::OpenHistory { today: false }),
        vk::R => Outcome::Consume(S::Reload),
        vk::D => Outcome::Consume(S::Bookmark),
        vk::U => Outcome::Consume(S::ViewSource),
        vk::COMMA => Outcome::Consume(S::OpenSettings),
        vk::OEM_PLUS | vk::NUMPAD_PLUS => Outcome::Consume(S::ZoomIn),
        vk::OEM_MINUS | vk::NUMPAD_MINUS => Outcome::Consume(S::ZoomOut),
        // Reserved-but-unbound: Ctrl+\ split view.
        _ => Outcome::Pass,
    }
}

/// Native accelerators cannot inspect DOM focus. Space and Escape belong to
/// the focused control (text entry, menus, find and palette dismissal).
pub fn resolve_native(vk: u32, mods: Mods) -> Outcome {
    match resolve(vk, mods, false) {
        Outcome::Consume(Shortcut::ItemMenu | Shortcut::Escape) => Outcome::Pass,
        outcome => outcome,
    }
}

/// Translate AppKit key events without assuming a US keyboard layout for letters.
pub fn macos_key(code: u16, characters: &str) -> u32 {
    match code {
        48 => vk::TAB,
        53 => vk::ESCAPE,
        96 => vk::F5,
        97 => vk::F6,
        99 => vk::F3,
        111 => vk::F12,
        116 => vk::PRIOR,
        121 => vk::NEXT,
        123 => vk::LEFT,
        124 => vk::RIGHT,
        _ => match characters.chars().next() {
            Some('+' | '=') => vk::OEM_PLUS,
            Some('-') => vk::OEM_MINUS,
            Some(',') => vk::COMMA,
            Some('[' | '{') => 0xDB,
            Some(']' | '}') => 0xDD,
            Some(c) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u32,
            _ => 0,
        },
    }
}

pub fn resolve_macos(key: u32, command: bool, control: bool, option: bool, shift: bool) -> Outcome {
    use Shortcut as S;
    if key == vk::ESCAPE {
        return Outcome::Pass;
    }
    if control && !command && !option && key == vk::TAB {
        return Outcome::Consume(if shift { S::PrevTab } else { S::NextTab });
    }
    if control {
        return Outcome::Pass;
    }
    if command && option {
        return match key {
            vk::LEFT => Outcome::Consume(S::PrevTab),
            vk::RIGHT => Outcome::Consume(S::NextTab),
            _ => Outcome::Pass,
        };
    }
    // Leave Option text entry, system shortcuts and standard editing to AppKit.
    if option || (command && matches!(key, vk::H | vk::TAB)) {
        return Outcome::Pass;
    }
    if !command && !matches!(key, vk::ESCAPE | vk::F3 | vk::F5 | vk::F6 | vk::F12) {
        return Outcome::Pass;
    }
    if command && key == b'Y' as u32 {
        return Outcome::Consume(S::OpenHistory { today: false });
    }
    if command && matches!(key, vk::OEM_PLUS | vk::OEM_MINUS) {
        return resolve(key, Mods::CTRL, false);
    }
    if command && key == vk::J && shift {
        return Outcome::Consume(S::OpenDownloads);
    }
    if command && matches!(key, 0xDB | 0xDD) {
        return Outcome::Consume(match (key, shift) {
            (0xDB, false) => S::PageBack,
            (0xDD, false) => S::PageForward,
            (0xDB, true) => S::PrevTab,
            _ => S::NextTab,
        });
    }
    resolve(key, Mods { ctrl: command, alt: false, shift }, false)
}

#[cfg(test)]
mod tests {
    use super::vk::*;
    use super::*;

    #[test]
    fn f12_opens_the_page_inspector_without_stealing_modified_keys() {
        assert_eq!(resolve(F12, Mods::NONE, true), Outcome::Consume(Shortcut::OpenDevtools));
        assert_eq!(resolve(F12, Mods::CTRL, false), Outcome::Pass);
        assert_eq!(resolve(F12, Mods::SHIFT, false), Outcome::Pass);
        assert_eq!(macos_key(111, ""), F12);
        assert_eq!(resolve_macos(F12, false, false, false, false), Outcome::Consume(Shortcut::OpenDevtools));
    }

    #[test]
    fn mac_shortcuts_preserve_editing_and_system_commands() {
        assert_eq!(resolve_macos(T, true, false, false, false), Outcome::Consume(Shortcut::NewTab));
        assert_eq!(resolve_macos(T, true, false, false, true), Outcome::Consume(Shortcut::ReopenClosed));
        assert_eq!(resolve_macos(W, true, false, false, false), Outcome::Consume(Shortcut::CloseTab));
        assert_eq!(resolve_macos(W, false, true, false, false), Outcome::Pass);
        assert_eq!(resolve_macos(L, true, false, false, false), Outcome::Consume(Shortcut::FocusOmnibox));
        assert_eq!(resolve_macos(TAB, false, true, false, false), Outcome::Consume(Shortcut::NextTab));
        assert_eq!(resolve_macos(TAB, false, true, false, true), Outcome::Consume(Shortcut::PrevTab));
        assert_eq!(resolve_macos(LEFT, true, false, true, false), Outcome::Consume(Shortcut::PrevTab));
        for key in [A, C, V, X, H, TAB, SPACE] {
            assert_eq!(resolve_macos(key, true, false, false, false), Outcome::Pass);
        }
        assert_eq!(resolve_macos(SPACE, false, false, false, false), Outcome::Pass);
        assert_eq!(resolve_macos(N0 + 2, false, false, true, false), Outcome::Pass);
        assert_eq!(macos_key(12, "a"), A);
        assert_eq!(macos_key(48, "\t"), TAB);
        assert_eq!(macos_key(24, "="), OEM_PLUS);
        assert_eq!(resolve_macos(OEM_PLUS, true, false, false, true), Outcome::Consume(Shortcut::ZoomIn));
        assert_eq!(resolve_macos(macos_key(33, "["), true, false, false, false), Outcome::Consume(Shortcut::PageBack));
        assert_eq!(resolve_macos(macos_key(30, "}"), true, false, false, true), Outcome::Consume(Shortcut::NextTab));
    }

    fn sc(vk_code: u32, m: Mods) -> Option<Shortcut> {
        match resolve(vk_code, m, false) {
            Outcome::Consume(s) => Some(s),
            Outcome::Pass => None,
        }
    }

    #[test]
    fn tab_management() {
        assert_eq!(sc(T, Mods::CTRL), Some(Shortcut::NewTab));
        assert_eq!(sc(W, Mods::CTRL), Some(Shortcut::CloseTab));
        assert_eq!(sc(W, Mods::CTRL_SHIFT), Some(Shortcut::CloseWindow));
        assert_eq!(sc(TAB, Mods::CTRL), Some(Shortcut::NextTab));
        assert_eq!(sc(TAB, Mods::CTRL_SHIFT), Some(Shortcut::PrevTab));
        assert_eq!(sc(T, Mods::CTRL_SHIFT), Some(Shortcut::ReopenClosed));
        assert_eq!(sc(PRIOR, Mods::CTRL), Some(Shortcut::NextTab));
        assert_eq!(sc(NEXT, Mods::CTRL), Some(Shortcut::PrevTab));
    }

    #[test]
    fn rapid_switch_is_the_page_safe_fallback() {
        assert_eq!(sc(LEFT, Mods::CTRL_ALT), Some(Shortcut::RapidPrevTab));
        assert_eq!(sc(RIGHT, Mods::CTRL_ALT), Some(Shortcut::RapidNextTab));
        // Ctrl+Alt+Tab is not bound.
        assert_eq!(sc(TAB, Mods::CTRL_ALT), None);
    }

    #[test]
    fn tab_index_digits_and_ctrl_zero() {
        assert_eq!(sc(N0 + 1, Mods::CTRL), Some(Shortcut::TabIndex(1)));
        assert_eq!(sc(N0 + 8, Mods::CTRL), Some(Shortcut::TabIndex(8)));
        assert_eq!(sc(N9, Mods::CTRL), Some(Shortcut::TabIndex(9)));
        assert_eq!(sc(N0, Mods::CTRL), Some(Shortcut::ZoomReset));
    }

    #[test]
    fn navigation_and_focus() {
        assert_eq!(sc(LEFT, Mods::ALT), Some(Shortcut::PageBack));
        assert_eq!(sc(RIGHT, Mods::ALT), Some(Shortcut::PageForward));
        assert_eq!(sc(HOME, Mods::ALT), Some(Shortcut::Home));
        assert_eq!(sc(L, Mods::CTRL), Some(Shortcut::FocusOmnibox));
        assert_eq!(sc(D, Mods::ALT), Some(Shortcut::FocusOmnibox));
        assert_eq!(sc(F6, Mods::NONE), Some(Shortcut::CycleFocus));
    }

    #[test]
    fn find_bindings() {
        assert_eq!(sc(F, Mods::CTRL), Some(Shortcut::Find));
        assert_eq!(sc(F3, Mods::NONE), Some(Shortcut::FindNext));
        assert_eq!(sc(F3, Mods::SHIFT), Some(Shortcut::FindPrev));
        assert_eq!(sc(G, Mods::CTRL), Some(Shortcut::FindNext));
        assert_eq!(sc(G, Mods::CTRL_SHIFT), Some(Shortcut::FindPrev));
        assert_eq!(sc(A, Mods::ALT), None);
        assert_eq!(sc(R, Mods::ALT), None);
    }

    #[test]
    fn panels_reload_zoom_settings() {
        assert_eq!(sc(J, Mods::CTRL), Some(Shortcut::OpenDownloads));
        assert_eq!(sc(H, Mods::CTRL), Some(Shortcut::OpenHistory { today: false }));
        assert_eq!(sc(H, Mods::CTRL_SHIFT), Some(Shortcut::OpenHistory { today: true }));
        assert_eq!(sc(R, Mods::CTRL), Some(Shortcut::Reload));
        assert_eq!(sc(F5, Mods::NONE), Some(Shortcut::Reload));
        assert_eq!(sc(R, Mods::CTRL_SHIFT), Some(Shortcut::ReloadIgnoringCache));
        assert_eq!(sc(F5, Mods::SHIFT), Some(Shortcut::ReloadIgnoringCache));
        assert_eq!(sc(OEM_PLUS, Mods::CTRL), Some(Shortcut::ZoomIn));
        assert_eq!(sc(NUMPAD_PLUS, Mods::CTRL), Some(Shortcut::ZoomIn));
        assert_eq!(sc(OEM_PLUS, Mods::CTRL_SHIFT), Some(Shortcut::ZoomIn));
        assert_eq!(sc(OEM_MINUS, Mods::CTRL), Some(Shortcut::ZoomOut));
        assert_eq!(sc(NUMPAD_MINUS, Mods::CTRL), Some(Shortcut::ZoomOut));
        assert_eq!(sc(COMMA, Mods::CTRL), Some(Shortcut::OpenSettings));
    }

    #[test]
    fn rail_palette_bookmark() {
        assert_eq!(sc(E, Mods::CTRL_SHIFT), Some(Shortcut::ToggleRail));
        assert_eq!(sc(P, Mods::CTRL_SHIFT), Some(Shortcut::TogglePin));
        assert_eq!(sc(K, Mods::CTRL), Some(Shortcut::OpenPalette));
        assert_eq!(sc(K, Mods::CTRL_SHIFT), Some(Shortcut::ClosePalette));
        assert_eq!(sc(D, Mods::CTRL), Some(Shortcut::Bookmark));
        assert_eq!(sc(D, Mods::CTRL_SHIFT), Some(Shortcut::BookmarkAllTabs));
        assert_eq!(sc(U, Mods::CTRL), Some(Shortcut::ViewSource));
        assert_eq!(sc(SPACE, Mods::NONE), Some(Shortcut::ItemMenu));
    }

    #[test]
    fn ctrl_w_in_an_input_is_close_or_word() {
        assert_eq!(
            resolve(W, Mods::CTRL, true),
            Outcome::Consume(Shortcut::CloseTabOrWord)
        );
        assert_eq!(resolve(W, Mods::CTRL, false), Outcome::Consume(Shortcut::CloseTab));
    }

    #[test]
    fn text_editing_shortcuts_pass_through() {
        for code in [A, C, V, X, Z, vk::L] {
            // Ctrl+L is a shell binding even in an input (re-focus selects all);
            // the rest must reach the text field.
            if code == vk::L {
                continue;
            }
            assert_eq!(resolve(code, Mods::CTRL, true), Outcome::Pass, "vk {code:#x}");
        }
    }

    #[test]
    fn native_shortcuts_preserve_typing_but_close_tabs_from_inputs() {
        for key in [SPACE, ESCAPE, A, C, V, X, Z] {
            assert_eq!(resolve_native(key, Mods::NONE), Outcome::Pass);
            assert_eq!(resolve_native(key, Mods::CTRL), Outcome::Pass);
        }
        assert_eq!(resolve_native(W, Mods::CTRL), Outcome::Consume(Shortcut::CloseTab));
        assert_eq!(resolve_native(L, Mods::CTRL), Outcome::Consume(Shortcut::FocusOmnibox));
        assert_eq!(resolve_macos(ESCAPE, false, false, false, false), Outcome::Pass);
    }

    #[test]
    fn alt_navigation_is_suppressed_inside_inputs() {
        assert_eq!(resolve(LEFT, Mods::ALT, true), Outcome::Pass);
        assert_eq!(resolve(A, Mods::ALT, true), Outcome::Pass);
    }

    #[test]
    fn escape_resolves_in_both_contexts() {
        assert_eq!(resolve(ESCAPE, Mods::NONE, false), Outcome::Consume(Shortcut::Escape));
        assert_eq!(resolve(ESCAPE, Mods::NONE, true), Outcome::Consume(Shortcut::Escape));
    }

    #[test]
    fn reserved_bindings_are_not_bound() {
        // Ctrl+\ split view; Ctrl+Shift+1 group switch; Ctrl+Alt+T tear-off.
        assert_eq!(sc(BACKSLASH, Mods::CTRL), None);
        assert_eq!(sc(N0 + 1, Mods::CTRL_SHIFT), None);
        assert_eq!(sc(T, Mods::CTRL_ALT), None);
    }
}