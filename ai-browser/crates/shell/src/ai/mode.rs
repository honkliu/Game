//! Measurement-only shell behaviour, off by default and reachable only under
//! `--proof`.
//!
//! Two of R0a-1's budgets cannot be measured while the shell behaves normally,
//! because its normal behaviour removes the thing the budget counts:
//!
//! * `AppState::sleep_background_tabs` destroys every non-active tab's webview, so
//!   five tabs hold one live page and "+4 tabs adds +4 renderers" has no referent.
//!   `--proof=…+no_tab_sleep` turns that destruction off for the run.
//! * The overlay webview is built at startup from the shared environment, so it is
//!   inside every sample and its cost cannot be differenced out.
//!   `--proof=…+no_overlay` starts the run without it, and the scenario builds it
//!   one step later so C5 measures a delta.
//!
//! Both default to the shipping behaviour, and `install` is called from `main` only
//! when the driver name is a proof scenario, so `active()` is false in ordinary
//! browsing: `sleep_background_tabs` sleeps tabs and the overlay is built at
//! startup. That is the whole reason these read as functions here instead of
//! `AppState` fields — the state a measurement changes must not be reachable from a
//! page, and the dispatch gate for the overlay step checks `active()` for exactly
//! that reason.
//!
//! The tab-sleep toggle is not a feature probe. It prices a product decision — what
//! keeping background tabs alive costs in working set — and `ai::proof` reports that
//! number every time the mode runs, so the mode is never mistaken for a proposal.
//!
//! The third flag, `+self_refresh`, changes what the *scenario* opens rather than how
//! the shell behaves: it holds a page that reloads itself in JavaScript in a
//! background tab, which is the condition that makes the tab-sleep price real. With
//! tab sleeping on there is nothing to measure — the destroyed webview takes the
//! page's refresh timer with it — so the pair `+no_tab_sleep+self_refresh` is the
//! only combination that produces the number.

use std::sync::OnceLock;

/// Tabs opened when the scenario does not say. 4 is the number R0a-1's assertion is
/// written against.
pub const DEFAULT_TABS: usize = 4;

/// Flag: do not destroy background tabs' webviews during the run.
pub const NO_TAB_SLEEP: &str = "no_tab_sleep";

/// Flag: do not build the overlay at startup; the scenario builds it as a measured
/// step so its cost is a delta.
pub const NO_OVERLAY: &str = "no_overlay";

/// Flag: open one tab on a page that reloads itself in JavaScript, leave it in the
/// background, and watch it. Prices what tab sleeping accidentally prevents.
pub const SELF_REFRESH: &str = "self_refresh";

/// What the run was asked to measure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Tabs the scenario opens, clamped to the caller's page list.
    pub tabs: usize,
    /// True while tab sleeping is off, i.e. every tab keeps its webview.
    pub keep_background_tabs_alive: bool,
    /// False when the overlay was deliberately not built at startup.
    pub overlay_at_startup: bool,
    /// True when the scenario should hold a self-refreshing page in a background
    /// tab. Only meaningful with `keep_background_tabs_alive`; a slept tab has no
    /// webview left to refresh, which is the point of the measurement.
    pub self_refresh_tab: bool,
    /// Unrecognised flags, so a typo cannot silently run the default mode.
    pub unknown: Vec<String>,
}

impl Default for Config {
    /// Shipping behaviour: sleep background tabs, build the overlay at startup.
    fn default() -> Self {
        Config {
            tabs: DEFAULT_TABS,
            keep_background_tabs_alive: false,
            overlay_at_startup: true,
            self_refresh_tab: false,
            unknown: Vec::new(),
        }
    }
}

impl Config {
    /// Parse a driver name: `proof`, `proof:6`, `proof=6`, and any of the three flags
    /// after `+` in any order and with or without a count — `proof:6+no_tab_sleep`,
    /// `proof+no_overlay+no_tab_sleep`, `proof=no_overlay`,
    /// `proof=4+no_tab_sleep+self_refresh`.
    ///
    /// `max_tabs` comes from the caller because the cap is the length of its list of
    /// distinct origins; clamping here without it would let a caller ask for tabs
    /// that share an origin, which fails the renderer budget for a reason that has
    /// nothing to do with the browser.
    pub fn parse(driver_name: &str, max_tabs: usize) -> Config {
        let rest = driver_name.strip_prefix("proof").unwrap_or("");
        let mut out = Config::default();
        for token in rest.trim_start_matches([':', '=']).split('+') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            match token {
                NO_TAB_SLEEP => out.keep_background_tabs_alive = true,
                NO_OVERLAY => out.overlay_at_startup = false,
                SELF_REFRESH => out.self_refresh_tab = true,
                other => match other.parse::<usize>() {
                    Ok(n) => out.tabs = n.clamp(1, max_tabs),
                    // A typo must not degrade into "the default run", because the
                    // reader of the report would be told the wrong thing about
                    // which mode produced the numbers.
                    Err(_) => out.unknown.push(other.to_string()),
                },
            }
        }
        out
    }
}

static INSTALLED: OnceLock<Config> = OnceLock::new();

/// Record the mode for the lifetime of the process. Called from `main` before the
/// event loop exists, when the driver name is a proof scenario.
pub fn install(config: Config) {
    // A second call is a bug, not an error to report: the mode is fixed before the
    // first webview is built and must not change under a running measurement.
    let _ = INSTALLED.set(config);
}

/// The mode in force. Ordinary browsing reads the default: sleeping tabs, overlay
/// built at startup, nothing reachable from here.
pub fn config() -> Config {
    INSTALLED.get().cloned().unwrap_or_default()
}

/// True only inside a proof run. Gates every toggle consumer, so a message that
/// arrives over IPC can turn measurement behaviour on only when the binary was
/// started with `--proof`.
pub fn active() -> bool {
    INSTALLED.get().is_some()
}

/// False when the run asked for background tabs to be kept alive. Read by
/// `AppState::sleep_background_tabs`.
pub fn tabs_may_sleep() -> bool {
    !(active() && config().keep_background_tabs_alive)
}

/// False when the run asked to start without the overlay. Read once, at startup.
pub fn overlay_at_startup() -> bool {
    !active() || config().overlay_at_startup
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spelling the harness accepts, and the default when the scenario name
    /// says nothing.
    #[test]
    fn every_documented_spelling_parses() {
        let max = 8;
        assert_eq!(Config::parse("proof", max).tabs, DEFAULT_TABS);
        assert_eq!(Config::parse("proof:6", max).tabs, 6);
        assert_eq!(Config::parse("proof=6", max).tabs, 6);
        assert_eq!(Config::parse("proof:0", max).tabs, 1, "never zero tabs");
        assert_eq!(Config::parse("proof:999", max).tabs, max, "capped at the page list");

        let both = Config::parse("proof:3+no_tab_sleep+no_overlay", max);
        assert_eq!(both.tabs, 3);
        assert!(both.keep_background_tabs_alive);
        assert!(!both.overlay_at_startup);
        assert!(both.unknown.is_empty());

        // Flags before the count, and flags in either order, are the same run.
        let flipped = Config::parse("proof+no_overlay+no_tab_sleep+3", max);
        assert_eq!(flipped, both);
        assert!(Config::parse("proof=no_tab_sleep", max).keep_background_tabs_alive);
        assert!(!Config::parse("proof+no_overlay", max).overlay_at_startup);

        // The self-refresh flag on its own and next to the others; it is a scenario
        // shape, so it must not disturb the two shell toggles.
        let busy = Config::parse("proof:4+no_tab_sleep+self_refresh", max);
        assert!(busy.self_refresh_tab);
        assert!(busy.keep_background_tabs_alive);
        assert!(busy.overlay_at_startup, "no overlay flag asked, so the overlay ships");
        assert_eq!(
            Config::parse("proof+self_refresh+no_tab_sleep+4", max),
            busy,
            "flag order does not change the run"
        );
        assert!(Config::parse("proof+self_refresh", max).self_refresh_tab);
    }

    /// The defaults are the shipping behaviour, not "whatever the last run wanted":
    /// a run that asks for nothing must not measure a shell that behaves
    /// differently from the one users get.
    #[test]
    fn the_default_is_shipping_behaviour() {
        let none = Config::parse("proof", 8);
        assert!(
            none.keep_background_tabs_alive == false,
            "tab sleeping is on unless asked off"
        );
        assert!(none.overlay_at_startup, "the overlay is built unless asked not to be");
        assert!(
            !none.self_refresh_tab,
            "no scenario page beyond the tabs the caller asked for"
        );
        assert_eq!(Config::default(), none);
    }

    /// A mistyped flag has to surface. Silently running the default mode would put
    /// a wrong description of the run in the report header.
    #[test]
    fn an_unknown_flag_is_reported_rather_than_ignored() {
        let parsed = Config::parse("proof:4+nosleep", 8);
        assert_eq!(parsed.unknown, vec!["nosleep".to_string()]);
        assert!(!parsed.keep_background_tabs_alive, "a typo must not change behaviour");

        let clean = Config::parse("proof:4", 8);
        assert!(clean.unknown.is_empty());
    }

    /// Outside a proof run nothing is armed: the guards `app.rs` reads have to say
    /// "ship behaviour" when no mode was installed, which is the property that keeps
    /// a page from reaching either toggle.
    #[test]
    fn without_install_everything_is_shipped_behaviour() {
        // This test can only observe the *uninstalled* case: `install` is
        // OnceLock-backed and process-global, so the installed case is exercised by
        // the harness run itself and asserted in `proof`'s report header.
        if INSTALLED.get().is_some() {
            return; // another test in this binary installed a mode; nothing to assert
        }
        assert!(!active());
        assert!(tabs_may_sleep());
        assert!(overlay_at_startup());
    }
}
