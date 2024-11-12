#[cfg(target_os = "linux")]
mod linux_specific {
    pub use gtk::cairo::Context;
    pub use gtk::gdk::Display;
    pub use gtk::prelude::GtkWindowExt;
    pub use gtk::prelude::*;
    pub use gtk::{Application, ApplicationWindow};
    pub use std::io::Read;
    pub use std::os::unix::net::UnixListener;
    pub use std::time::{Duration, Instant};
    pub use std::{fs, thread};
}

#[cfg(target_os = "linux")]
use linux_specific::*;

#[cfg(target_os = "linux")]
static SHOW_CURSOR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("Cursor overlay is not supported on this platform");
}

#[cfg(target_os = "linux")]
fn main() {
    // Create a Unix socket to listen for visibility commands
    let socket_path = "/tmp/cursor_overlay.sock";
    if std::path::Path::new(socket_path).exists() {
        fs::remove_file(socket_path).expect("Failed to remove existing socket file");
    }
    let listener: UnixListener = UnixListener::bind(socket_path).unwrap();

    // Spawn a thread to listen for commands and update the visibility flag
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut buffer = [0; 4]; // Buffer for incoming message (e.g., "show" or "hide")
            if let Ok(mut stream) = stream {
                stream.read_exact(&mut buffer).unwrap();
                let command = std::str::from_utf8(&buffer).unwrap();
                if command == "show" {
                    SHOW_CURSOR.store(true, std::sync::atomic::Ordering::SeqCst);
                } else if command == "hide" {
                    SHOW_CURSOR.store(false, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    });

    let app = Application::builder()
        .application_id("overbind.cursor-overlay")
        .build();

    println!("Starting cursor overlay");
    app.connect_activate(build_ui);
    let return_code = app.run();
    println!("Cursor overlay stopped with code: {:?}", return_code);
}

#[cfg(target_os = "linux")]
fn build_ui(app: &Application) {
    // Create a new top-level window
    let window = ApplicationWindow::new(app);
    window.set_title("Cursor Overlay");
    window.set_default_size(200, 200);
    window.set_type_hint(gtk::gdk::WindowTypeHint::Utility); // Mark as utility window
    window.set_decorated(false); // Remove window decorations
    window.set_resizable(false);
    window.set_app_paintable(true);

    // Make the window transparent
    let screen = window.display().default_screen();
    window.set_visual(screen.rgba_visual().as_ref());

    // Make the window always on top
    window.set_keep_above(true);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_accept_focus(false);

    window.realize();
    if let Some(gdk_window) = window.window() {
        println!("Setting pass through");
        gdk_window.set_pass_through(true);
    }

    window.input_shape_combine_region(Some(&gtk::gdk::cairo::Region::create_rectangle(
        &gtk::gdk::cairo::RectangleInt::new(0, 0, 100, 1),
    )));

    window.connect_draw(|_, cr| {
        // Set transparent background
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        cr.set_operator(gtk::cairo::Operator::Source);
        let _ = cr.paint();
        gtk::glib::Propagation::Proceed
    });

    // Create an event box to catch motion events
    let event_box = gtk::EventBox::new();
    event_box.set_events(gtk::gdk::EventMask::POINTER_MOTION_MASK);
    window.add(&event_box);

    // Set up the custom drawing area for the cursor
    let drawing_area = gtk::DrawingArea::new();
    event_box.add(&drawing_area);
    drawing_area.set_size_request(50, 50); // Initial cursor size

    drawing_area.connect_draw(move |_, cr| {
        // Draw the custom cursor (a red circle)
        draw_cursor(cr);
        gtk::glib::Propagation::Proceed
        //Inhibit(false)
    });

    // Move the window with the mouse cursor
    let window_clone = window.clone();
    // event_box.connect_motion_notify_event(move |_, event| {
    //     let (x, y) = event.position();
    //     window_clone.move_((x as i32) - 25, (y as i32) - 25); // Center window over cursor
    //                                                           //Inhibit(false)
    //     gtk::glib::Propagation::Proceed
    // });

    glib::MainContext::default().spawn_local(async move {
        // Get the default display to track the global mouse position
        let display = Display::default().expect("Could not get display");
        let device_manager = display
            .default_seat()
            .expect("Could not get default seat")
            .pointer()
            .expect("Could not get pointer");

        let mut current_x = 0;
        let mut current_y = 0;
        let mut last_move_time = Instant::now();

        loop {
            // Get the current mouse position globally
            let (_, x, y) = device_manager.position();
            if x != current_x || y != current_y {
                current_x = x;
                current_y = y;
                last_move_time = Instant::now();
            }

            if SHOW_CURSOR.load(std::sync::atomic::Ordering::SeqCst)
                && last_move_time.elapsed().as_millis() < 3000
            {
                window_clone.move_(x as i32 - 10, y as i32 - 10); // Adjust to center the window on cursor
                window_clone.show();
            } else {
                window_clone.hide();
            }

            // Sleep to avoid excessive CPU usage
            thread::sleep(Duration::from_millis(10));
            glib::timeout_future(Duration::from_millis(10)).await;
        }
    });

    window.show_all();
}

#[cfg(target_os = "linux")]
fn draw_cursor(cr: &Context) {
    let cursor_size = 20.0; // Size of the entire cursor
    let rect_width = 2.0; // Width of the rectangles (thin)
    let rect_length = 20.0; // Length of each rectangle

    // Set the color for the plus sign (red for visibility)
    cr.set_source_rgb(1.0, 0.0, 0.0); // Red color

    // Draw the horizontal rectangle (centered)
    cr.rectangle(
        (cursor_size - rect_length) / 2.0, // x position (centered horizontally)
        (cursor_size - rect_width) / 2.0,  // y position (centered vertically)
        rect_length,                       // width of the rectangle
        rect_width,                        // height of the rectangle
    );
    let _ = cr.fill(); // Fill the rectangle with the current color

    // Draw the vertical rectangle (centered)
    cr.rectangle(
        (cursor_size - rect_width) / 2.0, // x position (centered horizontally)
        (cursor_size - rect_length) / 2.0, // y position (centered vertically)
        rect_width,                       // width of the rectangle
        rect_length,                      // height of the rectangle
    );
    let _ = cr.fill(); // Fill the rectangle with the current color
}
