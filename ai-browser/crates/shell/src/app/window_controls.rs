use serde::Serialize;
use serde_json::{json, Value};

use super::AppState;
use std::cell::RefCell;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Bounds {
    fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }
    fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Display {
    id: String,
    number: String,
    name: String,
    resolution: [u32; 2],
    scale: f64,
    bounds: Bounds,
    work: Bounds,
}

fn resolve_layout(
    displays: &[Display],
    layout: &crate::settings::SpanLayout,
) -> Option<Vec<usize>> {
    if !matches!((layout.cols, layout.rows), (2, 1) | (1, 2) | (2, 2))
        || layout.display_ids.len() != layout.cols * layout.rows
    {
        return None;
    }
    let mut selected = Vec::new();
    for id in &layout.display_ids {
        let index = displays
            .iter()
            .position(|display| !id.is_empty() && display.id == *id)?;
        if selected
            .iter()
            .any(|other: &usize| displays[*other].bounds == displays[index].bounds)
        {
            return None;
        }
        selected.push(index);
    }
    Some(selected)
}

fn span_bounds(displays: &[Display], selected: &[usize]) -> Option<Bounds> {
    let selected: Vec<_> = selected
        .iter()
        .map(|index| displays.get(*index))
        .collect::<Option<_>>()?;
    let left = selected.iter().map(|display| display.bounds.x).min()?;
    let top = selected.iter().map(|display| display.bounds.y).min()?;
    let right = selected
        .iter()
        .map(|display| display.bounds.right())
        .max()?;
    let bottom = selected
        .iter()
        .map(|display| display.bounds.bottom())
        .max()?;
    // Clip only the outside edges; taskbars at internal display boundaries are OS-owned.
    let x = selected
        .iter()
        .filter(|display| display.bounds.x == left)
        .map(|display| display.work.x)
        .max()?;
    let y = selected
        .iter()
        .filter(|display| display.bounds.y == top)
        .map(|display| display.work.y)
        .max()?;
    let right = selected
        .iter()
        .filter(|display| display.bounds.right() == right)
        .map(|display| display.work.right())
        .min()?;
    let bottom = selected
        .iter()
        .filter(|display| display.bounds.bottom() == bottom)
        .map(|display| display.work.bottom())
        .min()?;
    Some(Bounds {
        x,
        y,
        width: u32::try_from(right - i64::from(x)).ok()?,
        height: u32::try_from(bottom - i64::from(y)).ok()?,
    })
}

#[derive(Default)]
pub(super) struct DisplayCache(RefCell<Option<(Instant, Vec<Display>)>>);

#[cfg(windows)]
pub(super) struct SpanState {
    placement: crate::sys::WindowPlacement,
    bounds: Bounds,
}

impl AppState {
    fn span_displays(&self) -> Vec<Display> {
        if let Some((at, displays)) = self.window_displays.0.borrow().as_ref() {
            if at.elapsed() < Duration::from_secs(2) {
                return displays.clone();
            }
        }
        let Some(window) = self.window.as_ref() else {
            return Vec::new();
        };
        let monitors: Vec<_> = window.available_monitors().collect();
        let displays: Vec<_> = monitors
            .iter()
            .map(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                let bounds = Bounds {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                };
                let source = monitor.name().unwrap_or_default();
                #[cfg(windows)]
                let (id, name, width, height) =
                    crate::sys::monitor_details(&source).unwrap_or_default();
                #[cfg(not(windows))]
                let (id, name, width, height) =
                    (source.clone(), source.clone(), size.width, size.height);
                #[cfg(windows)]
                let work = {
                    use tao::platform::windows::MonitorHandleExtWindows;
                    crate::sys::monitor_work_area(monitor.hmonitor())
                        .map(|(x, y, width, height)| Bounds {
                            x,
                            y,
                            width,
                            height,
                        })
                        .unwrap_or(bounds)
                };
                #[cfg(not(windows))]
                let work = bounds;
                Display {
                    id,
                    number: source.trim_start_matches(r"\\.\DISPLAY").to_string(),
                    name,
                    resolution: [
                        if width == 0 { size.width } else { width },
                        if height == 0 { size.height } else { height },
                    ],
                    scale: monitor.scale_factor(),
                    bounds,
                    work,
                }
            })
            .collect();
        *self.window_displays.0.borrow_mut() = Some((Instant::now(), displays.clone()));
        displays
    }

    pub(super) fn refresh_window_displays(&mut self, force: bool) {
        let old = self
            .window_displays
            .0
            .borrow()
            .as_ref()
            .map(|(_, displays)| displays.clone());
        if force {
            *self.window_displays.0.borrow_mut() = None;
        }
        let displays = self.span_displays();
        if force || old.as_ref() != Some(&displays) {
            self.emit_window_state();
        }
    }

    pub(super) fn window_payload(&self) -> Value {
        let displays = self.span_displays();
        let selected = self
            .settings
            .span_layout
            .as_ref()
            .and_then(|layout| resolve_layout(&displays, layout));
        let target = selected
            .as_ref()
            .and_then(|selected| span_bounds(&displays, selected))
            .filter(|_| cfg!(windows));
        let available: Vec<_> = displays
            .iter()
            .filter(|display| !display.id.is_empty())
            .collect();
        let can_span = available
            .iter()
            .enumerate()
            .any(|(i, a)| available.iter().skip(i + 1).any(|b| a.bounds != b.bounds));
        #[cfg(windows)]
        let spanning = self.window_span.is_some();
        #[cfg(not(windows))]
        let spanning = false;
        let window = self.window.as_ref();
        let bounds = window.and_then(|window| {
            window.outer_position().ok().map(|position| {
                let size = window.outer_size();
                Bounds {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                }
            })
        });
        let client_bounds = window.and_then(|window| {
            window.inner_position().ok().map(|position| {
                let size = window.inner_size();
                Bounds {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                }
            })
        });
        json!({
            "span_supported": cfg!(windows), "can_span": cfg!(windows) && can_span, "spanning": spanning,
            "needs_selection": selected.is_none(), "selection_missing": self.settings.span_layout.is_some() && selected.is_none(),
            "selected_layout": self.settings.span_layout,
            "maximized": window.is_some_and(|window| window.is_maximized()),
            "minimized": window.is_some_and(|window| window.is_minimized()),
            "bounds": bounds, "client_bounds": client_bounds, "span_bounds": target, "displays": displays,
        })
    }

    pub(super) fn emit_window_state(&mut self) {
        self.emit("state.window", self.window_payload());
    }

    pub(super) fn restore_spanned_window(&mut self) -> Result<bool, String> {
        #[cfg(windows)]
        if let Some(span) = self.window_span.as_ref() {
            use tao::platform::windows::WindowExtWindows;
            let window = self.window.as_ref().ok_or("no window")?;
            crate::sys::restore_window_placement(window.hwnd() as isize, &span.placement)?;
            self.window_span = None;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn toggle_window_span(&mut self) -> Result<Option<Value>, String> {
        if !self.restore_spanned_window()? {
            *self.window_displays.0.borrow_mut() = None;
            let displays = self.span_displays();
            if !self.window_payload()["can_span"].as_bool().unwrap_or(false) {
                return Err("connect a second display to span monitors".into());
            }
            let Some(selected) = self
                .settings
                .span_layout
                .as_ref()
                .and_then(|layout| resolve_layout(&displays, layout))
            else {
                return Ok(Some(self.window_payload()));
            };
            let target = span_bounds(&displays, &selected).ok_or("choose different displays")?;
            #[cfg(windows)]
            {
                use tao::platform::windows::WindowExtWindows;
                let window = self.window.as_ref().ok_or("no window")?;
                let placement = crate::sys::span_window(
                    window.hwnd() as isize,
                    (target.x, target.y, target.width, target.height),
                )?;
                self.window_span = Some(SpanState {
                    placement,
                    bounds: target,
                });
            }
            #[cfg(not(windows))]
            let _ = target;
        }
        self.apply_layout();
        self.mark_dirty();
        self.emit_window_state();
        Ok(Some(self.window_payload()))
    }

    pub(super) fn select_span_monitors(
        &mut self,
        payload: &Value,
    ) -> Result<Option<Value>, String> {
        let layout: crate::settings::SpanLayout = serde_json::from_value(payload.clone())
            .map_err(|_| "choose a display for each slot")?;
        *self.window_displays.0.borrow_mut() = None;
        let displays = self.span_displays();
        resolve_layout(&displays, &layout)
            .ok_or("choose a different connected display for each slot")?;
        let mut settings = self.settings.clone();
        settings.span_layout = Some(layout);
        crate::settings::save(&self.settings_path(), &settings)
            .map_err(|error| error.to_string())?;
        self.settings = settings;
        self.restore_spanned_window()?;
        self.emit_window_state();
        if payload["span"].as_bool().unwrap_or(true) {
            self.toggle_window_span()
        } else {
            Ok(Some(self.window_payload()))
        }
    }

    pub(super) fn toggle_window_maximize(&mut self) -> Result<(), String> {
        let was_spanning = self.restore_spanned_window()?;
        if let Some(window) = self.window.as_ref() {
            window.set_maximized(was_spanning || !window.is_maximized());
        }
        self.emit_window_state();
        Ok(())
    }

    pub(super) fn window_geometry_changed(&mut self) {
        #[cfg(windows)]
        if let (Some(span), Some(window)) = (&self.window_span, &self.window) {
            if !window.is_minimized() {
                let size = window.inner_size();
                let changed = window.inner_position().is_ok_and(|position| {
                    span.bounds
                        != Bounds {
                            x: position.x,
                            y: position.y,
                            width: size.width,
                            height: size.height,
                        }
                });
                if changed {
                    self.window_span = None;
                }
            }
        }
        self.mark_dirty();
        self.emit_window_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(x: i32, y: i32, width: u32, height: u32, taskbar: u32) -> Display {
        Display {
            id: format!("display-{x}-{y}"),
            number: String::new(),
            name: String::new(),
            resolution: [width, height],
            scale: 1.0,
            bounds: Bounds {
                x,
                y,
                width,
                height,
            },
            work: Bounds {
                x,
                y,
                width,
                height: height - taskbar,
            },
        }
    }

    #[test]
    fn spans_equal_displays_from_either_side_without_covering_the_taskbar() {
        let displays = [
            display(0, 0, 1920, 1080, 40),
            display(1920, 0, 1920, 1080, 0),
        ];
        let expected = Some(Bounds {
            x: 0,
            y: 0,
            width: 3840,
            height: 1040,
        });
        assert_eq!(span_bounds(&displays, &[0, 1]), expected);
        assert_eq!(span_bounds(&displays, &[1, 0]), expected);
    }

    #[test]
    fn handles_left_and_vertical_displays_in_physical_pixels() {
        let displays = [
            display(0, 0, 2560, 1440, 40),
            display(-2560, 0, 2560, 1440, 40),
        ];
        assert_eq!(
            span_bounds(&displays, &[0, 1]),
            Some(Bounds {
                x: -2560,
                y: 0,
                width: 5120,
                height: 1400
            })
        );
        let displays = [
            display(0, 0, 1920, 1080, 40),
            display(0, -1080, 1920, 1080, 0),
        ];
        assert_eq!(
            span_bounds(&displays, &[0, 1]),
            Some(Bounds {
                x: 0,
                y: -1080,
                width: 1920,
                height: 2120
            })
        );
    }

    #[test]
    fn preserves_user_order_and_monitor_identity_without_guessing() {
        let mut displays = vec![
            display(0, 1080, 1280, 720, 40),
            display(0, 0, 1920, 1080, 40),
            display(1920, 0, 1920, 1080, 40),
        ];
        let layout = crate::settings::SpanLayout {
            cols: 2,
            rows: 1,
            display_ids: vec![displays[2].id.clone(), displays[1].id.clone()],
        };
        assert_eq!(resolve_layout(&displays, &layout), Some(vec![2, 1]));
        assert_eq!(
            span_bounds(&displays, &[2, 1]),
            Some(Bounds {
                x: 0,
                y: 0,
                width: 3840,
                height: 1040
            })
        );
        displays.swap(0, 2);
        assert_eq!(resolve_layout(&displays, &layout), Some(vec![0, 1]));
        displays[0].bounds.x = -1920;
        assert_eq!(
            resolve_layout(&displays, &layout),
            Some(vec![0, 1]),
            "position changes never rewrite slot assignments"
        );
        displays.remove(0);
        assert_eq!(
            resolve_layout(&displays, &layout),
            None,
            "do not substitute another connected display"
        );
    }

    #[test]
    fn resolves_two_by_two_layouts_and_rejects_missing_or_duplicate_slots() {
        let displays = vec![
            display(0, 0, 1920, 1080, 40),
            display(1920, 0, 1920, 1080, 40),
            display(0, 1080, 1920, 1080, 40),
            display(1920, 1080, 1920, 1080, 40),
            display(3840, 0, 1280, 720, 0),
        ];
        let mut layout = crate::settings::SpanLayout {
            cols: 2,
            rows: 2,
            display_ids: displays[..4]
                .iter()
                .map(|display| display.id.clone())
                .collect(),
        };
        let selected = resolve_layout(&displays, &layout).unwrap();
        assert_eq!(selected, vec![0, 1, 2, 3]);
        assert_eq!(
            span_bounds(&displays, &selected),
            Some(Bounds {
                x: 0,
                y: 0,
                width: 3840,
                height: 2120
            })
        );
        layout.display_ids[3] = layout.display_ids[0].clone();
        assert_eq!(resolve_layout(&displays, &layout), None);
        layout.display_ids.pop();
        assert_eq!(resolve_layout(&displays, &layout), None);
        layout.cols = 3;
        layout.rows = 1;
        assert_eq!(resolve_layout(&displays, &layout), None);
    }
}
