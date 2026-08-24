//! Input event routing — dispatches kernel input events to windows.
//!
//! The kernel reports raw keyboard/mouse events via `poll_input`.
//! This module converts them to window-level actions:
//!
//! - Mouse move → cursor update + hit test
//! - Mouse button down on title bar → start dragging
//! - Mouse button up → stop dragging
//! - Mouse button down on window → focus + raise to top
//! - Keyboard → route to focused window
//!
//! **No multi-mouse support** — single cursor only.

use crate::cursor::Cursor;
use crate::syscalls::{InputEvent, EVENT_KEYBOARD, EVENT_MOUSE};
use crate::window::WindowList;

/// Drag state: which window is being dragged and the offset.
pub struct DragState {
    /// Window being dragged (0 = none).
    pub window_id: u32,
    /// Mouse X offset from window origin when drag started.
    pub offset_x: i32,
    /// Mouse Y offset from window origin when drag started.
    pub offset_y: i32,
}

impl DragState {
    pub const fn new() -> Self {
        Self {
            window_id: 0,
            offset_x: 0,
            offset_y: 0,
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.window_id != 0
    }

    pub fn start(&mut self, window_id: u32, mouse_x: i32, mouse_y: i32, win_x: i32, win_y: i32) {
        self.window_id = window_id;
        self.offset_x = mouse_x - win_x;
        self.offset_y = mouse_y - win_y;
    }

    pub fn stop(&mut self) {
        self.window_id = 0;
    }
}

/// Input router — processes one event at a time, updating cursor
/// and window state.
pub struct InputRouter {
    /// Current drag state.
    pub drag: DragState,
}

impl InputRouter {
    pub const fn new() -> Self {
        Self {
            drag: DragState::new(),
        }
    }

    /// Process a single input event.
    ///
    /// Updates `cursor` and `windows` as needed.
    /// Returns the window ID that should receive this event (0 = none).
    pub fn process_event(
        &mut self,
        event: &InputEvent,
        cursor: &mut Cursor,
        windows: &mut WindowList,
        fb_width: u32,
        fb_height: u32,
    ) -> u32 {
        match event.event_type {
            EVENT_MOUSE => self.process_mouse(event, cursor, windows, fb_width, fb_height),
            EVENT_KEYBOARD => self.process_keyboard(event, windows),
            _ => 0,
        }
    }

    /// Process a mouse event.
    fn process_mouse(
        &mut self,
        event: &InputEvent,
        cursor: &mut Cursor,
        windows: &mut WindowList,
        fb_width: u32,
        fb_height: u32,
    ) -> u32 {
        // 1. Update cursor position.
        let prev_left = cursor.left_button();
        let prev_right = cursor.right_button();
        cursor.apply_delta(event.mouse_dx, event.mouse_dy, event.mouse_buttons, fb_width, fb_height);

        let left_down = cursor.left_button() && !prev_left;
        let left_up = !cursor.left_button() && prev_left;

        // 2. Handle dragging: if dragging, move the window.
        if self.drag.is_dragging() {
            if !cursor.left_button() {
                // Button released — stop dragging.
                self.drag.stop();
                return 0;
            }
            // Move the dragged window to follow the cursor.
            let new_x = cursor.x - self.drag.offset_x;
            let new_y = cursor.y - self.drag.offset_y;
            windows.move_window(self.drag.window_id, new_x, new_y);
            return self.drag.window_id;
        }

        // 3. Mouse button down — start potential drag or focus.
        if left_down {
            // Hit test: which window is under the cursor?
            let hit_id = windows.hit_test(cursor.x, cursor.y);
            if hit_id == 0 {
                // Clicked on desktop — clear focus.
                windows.set_focus(0);
                return 0;
            }

            // Focus and raise the window.
            windows.set_focus(hit_id);
            windows.raise_to_top(hit_id);

            // Check if the click is on the title bar → start dragging.
            if let Some(win) = windows.get(hit_id) {
                if win.draggable && win.is_in_title_bar(cursor.x, cursor.y) {
                    self.drag.start(hit_id, cursor.x, cursor.y, win.x, win.y);
                }
            }

            return hit_id;
        }

        // 4. Mouse move without button → just hit test (no action).
        if !cursor.left_button() && !cursor.right_button() {
            return windows.hit_test(cursor.x, cursor.y);
        }

        // 5. Button held (not a new press) → if dragging handled above.
        // If not dragging and button held, route to the window under cursor.
        windows.hit_test(cursor.x, cursor.y)
    }

    /// Process a keyboard event.
    fn process_keyboard(
        &mut self,
        event: &InputEvent,
        windows: &WindowList,
    ) -> u32 {
        // Route keyboard events to the focused window.
        windows.focused_id()
    }
}
