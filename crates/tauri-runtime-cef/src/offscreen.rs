// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex};

/// A rectangle in physical pixels within an off-screen browser surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OffscreenRect {
  pub x: i32,
  pub y: i32,
  pub width: u32,
  pub height: u32,
}

/// One premultiplied BGRA8 frame rendered by CEF.
#[derive(Clone, Debug)]
pub struct OffscreenFrame {
  pub width: u32,
  pub height: u32,
  pub pixels: Arc<[u8]>,
  pub serial: u64,
}

/// The latest view and popup frames produced by an off-screen CEF browser.
#[derive(Clone, Debug, Default)]
pub struct OffscreenSnapshot {
  pub view: Option<OffscreenFrame>,
  pub popup: Option<OffscreenFrame>,
  pub popup_rect: OffscreenRect,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OffscreenBounds {
  pub(crate) x: i32,
  pub(crate) y: i32,
  pub(crate) width: u32,
  pub(crate) height: u32,
  pub(crate) scale_factor: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OffscreenImeCursorArea {
  pub(crate) x: i32,
  pub(crate) y: i32,
  pub(crate) width: u32,
  pub(crate) height: u32,
}

impl Default for OffscreenBounds {
  fn default() -> Self {
    Self {
      x: 0,
      y: 0,
      width: 1,
      height: 1,
      scale_factor: 1.0,
    }
  }
}

struct OffscreenState {
  bounds: OffscreenBounds,
  visible: bool,
  popup_visible: bool,
  popup_rect: OffscreenRect,
  view: Option<OffscreenFrame>,
  popup: Option<OffscreenFrame>,
  ime_character_bounds: Vec<OffscreenRect>,
  next_serial: u64,
}

impl Default for OffscreenState {
  fn default() -> Self {
    Self {
      bounds: OffscreenBounds::default(),
      visible: true,
      popup_visible: false,
      popup_rect: OffscreenRect::default(),
      view: None,
      popup: None,
      ime_character_bounds: Vec::new(),
      next_serial: 1,
    }
  }
}

struct OffscreenSurfaceInner {
  state: Mutex<OffscreenState>,
  request_redraw: Arc<dyn Fn() + Send + Sync>,
}

/// Shared output surface for a windowless CEF webview.
///
/// CEF owns the paint buffers only for the duration of its render callback, so
/// this surface copies each completed frame into application-owned memory. The
/// consumer can then upload it to its own GPU texture and compose it with other
/// native content without retaining CEF's transient pointers or handles.
#[derive(Clone)]
pub struct OffscreenSurface {
  inner: Arc<OffscreenSurfaceInner>,
}

impl std::fmt::Debug for OffscreenSurface {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("OffscreenSurface").finish_non_exhaustive()
  }
}

impl OffscreenSurface {
  pub fn new(request_redraw: impl Fn() + Send + Sync + 'static) -> Self {
    Self {
      inner: Arc::new(OffscreenSurfaceInner {
        state: Mutex::new(OffscreenState::default()),
        request_redraw: Arc::new(request_redraw),
      }),
    }
  }

  pub fn snapshot(&self) -> OffscreenSnapshot {
    let state = self.inner.state.lock().unwrap();
    OffscreenSnapshot {
      view: state.view.clone(),
      popup: state.popup_visible.then(|| state.popup.clone()).flatten(),
      popup_rect: state.popup_rect,
    }
  }

  pub(crate) fn bounds(&self) -> OffscreenBounds {
    self.inner.state.lock().unwrap().bounds
  }

  pub(crate) fn set_bounds(&self, bounds: OffscreenBounds) {
    self.inner.state.lock().unwrap().bounds = bounds;
  }

  pub(crate) fn set_visible(&self, visible: bool) {
    self.inner.state.lock().unwrap().visible = visible;
  }

  pub(crate) fn is_visible(&self) -> bool {
    self.inner.state.lock().unwrap().visible
  }

  pub(crate) fn set_popup_visible(&self, visible: bool) {
    let mut state = self.inner.state.lock().unwrap();
    state.popup_visible = visible;
    if !visible {
      state.popup = None;
    }
    drop(state);
    (self.inner.request_redraw)();
  }

  pub(crate) fn set_popup_rect(&self, x: i32, y: i32, width: u32, height: u32) {
    let mut state = self.inner.state.lock().unwrap();
    let scale = state.bounds.scale_factor;
    state.popup_rect = OffscreenRect {
      x: (f64::from(x) * scale).round() as i32,
      y: (f64::from(y) * scale).round() as i32,
      width: (f64::from(width) * scale).round() as u32,
      height: (f64::from(height) * scale).round() as u32,
    };
  }

  pub(crate) fn set_ime_character_bounds(&self, bounds: &[cef::Rect]) {
    self.inner.state.lock().unwrap().ime_character_bounds = bounds
      .iter()
      .map(|rect| OffscreenRect {
        x: rect.x,
        y: rect.y,
        width: rect.width.max(0) as u32,
        height: rect.height.max(0) as u32,
      })
      .collect();
    (self.inner.request_redraw)();
  }

  pub(crate) fn ime_cursor_area(&self, cursor: usize) -> Option<OffscreenImeCursorArea> {
    let state = self.inner.state.lock().unwrap();
    let index = cursor
      .saturating_sub(1)
      .min(state.ime_character_bounds.len().checked_sub(1)?);
    let rect = state.ime_character_bounds[index];
    let scale = state.bounds.scale_factor;
    Some(OffscreenImeCursorArea {
      x: state.bounds.x + (f64::from(rect.x) * scale).round() as i32,
      y: state.bounds.y + (f64::from(rect.y) * scale).round() as i32,
      width: (f64::from(rect.width) * scale).round().max(1.0) as u32,
      height: (f64::from(rect.height) * scale).round().max(1.0) as u32,
    })
  }

  pub(crate) fn publish(&self, popup: bool, width: u32, height: u32, pixels: Arc<[u8]>) {
    let mut state = self.inner.state.lock().unwrap();
    let serial = state.next_serial;
    state.next_serial = state.next_serial.wrapping_add(1).max(1);
    let frame = OffscreenFrame {
      width,
      height,
      pixels,
      serial,
    };
    if popup {
      state.popup = Some(frame);
    } else {
      state.view = Some(frame);
    }
    drop(state);
    (self.inner.request_redraw)();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ime_cursor_area_maps_cef_logical_bounds_into_parent_pixels() {
    let surface = OffscreenSurface::new(|| {});
    surface.set_bounds(OffscreenBounds {
      x: 40,
      y: 60,
      width: 800,
      height: 600,
      scale_factor: 1.5,
    });
    surface.set_ime_character_bounds(&[
      cef::Rect {
        x: 10,
        y: 20,
        width: 8,
        height: 12,
      },
      cef::Rect {
        x: 18,
        y: 20,
        width: 9,
        height: 12,
      },
    ]);

    assert_eq!(
      surface.ime_cursor_area(2),
      Some(OffscreenImeCursorArea {
        x: 67,
        y: 90,
        width: 14,
        height: 18,
      })
    );
  }
}
