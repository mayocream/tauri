// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use cef::{
  CefString, ImplBrowserHost as _, KeyEvent as CefKeyEvent, KeyEventType, MouseButtonType,
  MouseEvent,
};
use winit::{
  event::{ElementState, Ime, MouseButton, MouseScrollDelta, PointerSource, WindowEvent},
  keyboard::{Key, ModifiersState, NamedKey},
};

use crate::{offscreen::OffscreenBounds, window::AppWindow};

#[derive(Default)]
pub(crate) struct OffscreenInputState {
  cursor_x: f64,
  cursor_y: f64,
  modifiers: ModifiersState,
  pressed_buttons: u32,
}

pub(crate) fn handle(appwindow: &mut AppWindow, event: &WindowEvent) {
  match event {
    WindowEvent::ModifiersChanged(modifiers) => {
      appwindow.offscreen_input.modifiers = modifiers.state();
    }
    WindowEvent::Focused(focused) => {
      for child in visible_children(appwindow) {
        child.host.set_focus(i32::from(*focused));
        if !focused {
          child.host.send_capture_lost_event();
        }
      }
    }
    WindowEvent::PointerMoved {
      position,
      source: PointerSource::Mouse,
      ..
    }
    | WindowEvent::PointerEntered { position, .. } => {
      appwindow.offscreen_input.cursor_x = position.x;
      appwindow.offscreen_input.cursor_y = position.y;
      if let Some((child, event)) = mouse_target(appwindow, position.x, position.y) {
        child.host.send_mouse_move_event(Some(&event), 0);
      }
    }
    WindowEvent::PointerLeft { .. } => {
      let position = mouse_position(appwindow);
      for child in visible_children(appwindow) {
        let event = mouse_event(
          appwindow,
          child.offscreen_surface.as_ref().unwrap(),
          position,
        );
        child.host.send_mouse_move_event(Some(&event), 1);
      }
    }
    WindowEvent::PointerButton {
      state,
      position,
      button,
      ..
    } => {
      let Some(button) = button.clone().mouse_button().and_then(cef_mouse_button) else {
        return;
      };
      appwindow.offscreen_input.cursor_x = position.x;
      appwindow.offscreen_input.cursor_y = position.y;
      update_pressed_buttons(&mut appwindow.offscreen_input, button, *state);
      if let Some((child, event)) = mouse_target(appwindow, position.x, position.y) {
        if state.is_pressed() {
          child.host.set_focus(1);
        }
        child.host.send_mouse_click_event(
          Some(&event),
          button,
          i32::from(*state == ElementState::Released),
          1,
        );
      }
    }
    WindowEvent::MouseWheel { delta, .. } => {
      let (delta_x, delta_y) = match delta {
        MouseScrollDelta::LineDelta(x, y) => ((*x * 120.0).round(), (*y * 120.0).round()),
        MouseScrollDelta::PixelDelta(position) => {
          (position.x.round() as f32, position.y.round() as f32)
        }
      };
      let position = mouse_position(appwindow);
      if let Some((child, event)) = mouse_target(appwindow, position.0, position.1) {
        child
          .host
          .send_mouse_wheel_event(Some(&event), delta_x as i32, delta_y as i32);
      }
    }
    WindowEvent::KeyboardInput { event, .. } => {
      let Some(child) = visible_children(appwindow).next_back() else {
        return;
      };
      let key_code = windows_key_code(&event.logical_key);
      let modifiers = cef_modifiers(&appwindow.offscreen_input);
      let is_system_key = i32::from(appwindow.offscreen_input.modifiers.alt_key());
      let type_ = if event.state.is_pressed() {
        KeyEventType::RAWKEYDOWN
      } else {
        KeyEventType::KEYUP
      };
      child.host.send_key_event(Some(&CefKeyEvent {
        type_,
        modifiers,
        windows_key_code: key_code,
        native_key_code: key_code,
        is_system_key,
        ..Default::default()
      }));

      if event.state.is_pressed()
        && !appwindow.offscreen_input.modifiers.control_key()
        && !appwindow.offscreen_input.modifiers.alt_key()
        && !appwindow.offscreen_input.modifiers.meta_key()
        && let Some(text) = &event.text
      {
        for character in text.encode_utf16() {
          child.host.send_key_event(Some(&CefKeyEvent {
            type_: KeyEventType::CHAR,
            modifiers,
            windows_key_code: i32::from(character),
            native_key_code: key_code,
            is_system_key,
            character,
            unmodified_character: character,
            ..Default::default()
          }));
        }
      }
    }
    WindowEvent::Ime(ime) => {
      let Some(child) = visible_children(appwindow).next_back() else {
        return;
      };
      match ime {
        Ime::Preedit(text, cursor) => {
          let selection = cursor.map(|(start, end)| cef::Range {
            from: text[..start].encode_utf16().count() as u32,
            to: text[..end].encode_utf16().count() as u32,
          });
          child.host.ime_set_composition(
            Some(&CefString::from(text.as_str())),
            None,
            None,
            selection.as_ref(),
          );
        }
        Ime::Commit(text) => {
          child
            .host
            .ime_commit_text(Some(&CefString::from(text.as_str())), None, 0)
        }
        Ime::Disabled => child.host.ime_cancel_composition(),
        Ime::Enabled | Ime::DeleteSurrounding { .. } => {}
      }
    }
    _ => {}
  }
}

fn visible_children(
  appwindow: &AppWindow,
) -> impl DoubleEndedIterator<Item = &crate::webview::AppWebview> {
  appwindow.children.iter().filter(|child| {
    child
      .offscreen_surface
      .as_ref()
      .is_some_and(|surface| surface.is_visible())
  })
}

fn mouse_target(
  appwindow: &AppWindow,
  x: f64,
  y: f64,
) -> Option<(&crate::webview::AppWebview, MouseEvent)> {
  visible_children(appwindow).rev().find_map(|child| {
    let surface = child.offscreen_surface.as_ref().unwrap();
    let bounds = surface.bounds();
    contains(bounds, x, y).then(|| (child, mouse_event(appwindow, surface, (x, y))))
  })
}

fn contains(bounds: OffscreenBounds, x: f64, y: f64) -> bool {
  x >= f64::from(bounds.x)
    && y >= f64::from(bounds.y)
    && x < f64::from(bounds.x) + f64::from(bounds.width)
    && y < f64::from(bounds.y) + f64::from(bounds.height)
}

fn mouse_position(appwindow: &AppWindow) -> (f64, f64) {
  (
    appwindow.offscreen_input.cursor_x,
    appwindow.offscreen_input.cursor_y,
  )
}

fn mouse_event(
  appwindow: &AppWindow,
  surface: &crate::OffscreenSurface,
  position: (f64, f64),
) -> MouseEvent {
  let bounds = surface.bounds();
  MouseEvent {
    x: ((position.0 - f64::from(bounds.x)) / bounds.scale_factor).round() as i32,
    y: ((position.1 - f64::from(bounds.y)) / bounds.scale_factor).round() as i32,
    modifiers: cef_modifiers(&appwindow.offscreen_input),
  }
}

fn update_pressed_buttons(
  input: &mut OffscreenInputState,
  button: MouseButtonType,
  state: ElementState,
) {
  use cef::sys::cef_event_flags_t;
  let flag = if button == MouseButtonType::LEFT {
    cef_event_flags_t::EVENTFLAG_LEFT_MOUSE_BUTTON.0 as u32
  } else if button == MouseButtonType::MIDDLE {
    cef_event_flags_t::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0 as u32
  } else {
    cef_event_flags_t::EVENTFLAG_RIGHT_MOUSE_BUTTON.0 as u32
  };
  if state.is_pressed() {
    input.pressed_buttons |= flag;
  } else {
    input.pressed_buttons &= !flag;
  }
}

fn cef_modifiers(input: &OffscreenInputState) -> u32 {
  use cef::sys::cef_event_flags_t;
  let mut flags = input.pressed_buttons;
  if input.modifiers.shift_key() {
    flags |= cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32;
  }
  if input.modifiers.control_key() {
    flags |= cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32;
  }
  if input.modifiers.alt_key() {
    flags |= cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32;
  }
  if input.modifiers.meta_key() {
    flags |= cef_event_flags_t::EVENTFLAG_COMMAND_DOWN.0 as u32;
  }
  flags
}

fn cef_mouse_button(button: MouseButton) -> Option<MouseButtonType> {
  match button {
    MouseButton::Left => Some(MouseButtonType::LEFT),
    MouseButton::Middle => Some(MouseButtonType::MIDDLE),
    MouseButton::Right => Some(MouseButtonType::RIGHT),
    _ => None,
  }
}

fn windows_key_code(key: &Key) -> i32 {
  match key {
    Key::Character(value) => value
      .chars()
      .next()
      .map(|character| character.to_ascii_uppercase() as i32)
      .unwrap_or_default(),
    Key::Named(key) => match key {
      NamedKey::Backspace => 0x08,
      NamedKey::Tab => 0x09,
      NamedKey::Enter => 0x0d,
      NamedKey::Shift => 0x10,
      NamedKey::Control => 0x11,
      NamedKey::Alt => 0x12,
      NamedKey::Escape => 0x1b,
      NamedKey::PageUp => 0x21,
      NamedKey::PageDown => 0x22,
      NamedKey::End => 0x23,
      NamedKey::Home => 0x24,
      NamedKey::ArrowLeft => 0x25,
      NamedKey::ArrowUp => 0x26,
      NamedKey::ArrowRight => 0x27,
      NamedKey::ArrowDown => 0x28,
      NamedKey::Delete => 0x2e,
      NamedKey::F1 => 0x70,
      NamedKey::F2 => 0x71,
      NamedKey::F3 => 0x72,
      NamedKey::F4 => 0x73,
      NamedKey::F5 => 0x74,
      NamedKey::F6 => 0x75,
      NamedKey::F7 => 0x76,
      NamedKey::F8 => 0x77,
      NamedKey::F9 => 0x78,
      NamedKey::F10 => 0x79,
      NamedKey::F11 => 0x7a,
      NamedKey::F12 => 0x7b,
      _ => 0,
    },
    _ => 0,
  }
}
