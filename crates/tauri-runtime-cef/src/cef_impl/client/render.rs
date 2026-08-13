// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::sync::Arc;

use cef::*;

use crate::OffscreenSurface;

wrap_render_handler! {
  pub(crate) struct TauriCefRenderHandler {
    surface: OffscreenSurface,
  }

  impl RenderHandler {
    fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
      let Some(rect) = rect else {
        return;
      };
      let bounds = self.surface.bounds();
      rect.x = 0;
      rect.y = 0;
      rect.width = (f64::from(bounds.width) / bounds.scale_factor)
        .round()
        .max(1.0) as i32;
      rect.height = (f64::from(bounds.height) / bounds.scale_factor)
        .round()
        .max(1.0) as i32;
    }

    fn screen_info(
      &self,
      _browser: Option<&mut Browser>,
      screen_info: Option<&mut ScreenInfo>,
    ) -> std::os::raw::c_int {
      let Some(screen_info) = screen_info else {
        return 0;
      };
      let bounds = self.surface.bounds();
      let width = (f64::from(bounds.width) / bounds.scale_factor)
        .round()
        .max(1.0) as i32;
      let height = (f64::from(bounds.height) / bounds.scale_factor)
        .round()
        .max(1.0) as i32;
      screen_info.device_scale_factor = bounds.scale_factor as f32;
      screen_info.depth = 32;
      screen_info.depth_per_component = 8;
      screen_info.is_monochrome = 0;
      screen_info.rect = Rect {
        x: 0,
        y: 0,
        width,
        height,
      };
      screen_info.available_rect = screen_info.rect.clone();
      1
    }

    fn on_popup_show(&self, _browser: Option<&mut Browser>, show: std::os::raw::c_int) {
      self.surface.set_popup_visible(show != 0);
    }

    fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
      if let Some(rect) = rect {
        self.surface.set_popup_rect(
          rect.x,
          rect.y,
          rect.width.max(0) as u32,
          rect.height.max(0) as u32,
        );
      }
    }

    fn on_paint(
      &self,
      _browser: Option<&mut Browser>,
      type_: PaintElementType,
      _dirty_rects: Option<&[Rect]>,
      buffer: *const u8,
      width: std::os::raw::c_int,
      height: std::os::raw::c_int,
    ) {
      let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return;
      };
      let Some(length) = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
      else {
        return;
      };
      if buffer.is_null() || length == 0 {
        return;
      }

      // CEF guarantees a tightly packed BGRA8 buffer containing `width * height`
      // pixels for the duration of this callback. Copy it before returning so no
      // CEF-owned pointer escapes the unsafe boundary.
      let pixels = unsafe { std::slice::from_raw_parts(buffer, length) };
      self.surface.publish(
        type_ == PaintElementType::POPUP,
        width,
        height,
        Arc::from(pixels),
      );
    }
  }
}
