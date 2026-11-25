//! Frameless window helpers: the title bar is drawn by the app (like Discord),
//! so moving and resizing are handed to the system here.

use slint::winit_030::winit::window::ResizeDirection;
use slint::winit_030::WinitWindowAccessor;
use slint::ComponentHandle;

use crate::AppWindow;

pub fn drag(ui: &AppWindow) {
    ui.window().with_winit_window(|w| {
        let _ = w.drag_window();
    });
}

/// `direction` follows the clock from the top: 0 N, 1 NE, 2 E … 7 NW.
pub fn resize(ui: &AppWindow, direction: i32) {
    let direction = match direction {
        0 => ResizeDirection::North,
        1 => ResizeDirection::NorthEast,
        2 => ResizeDirection::East,
        3 => ResizeDirection::SouthEast,
        4 => ResizeDirection::South,
        5 => ResizeDirection::SouthWest,
        6 => ResizeDirection::West,
        _ => ResizeDirection::NorthWest,
    };
    ui.window().with_winit_window(|w| {
        let _ = w.drag_resize_window(direction);
    });
}
