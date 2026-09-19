#[cfg(target_os = "macos")]
#[path = "../src/infrastructure/platform/tray.rs"]
mod tray;

#[cfg(target_os = "macos")]
fn main() {
    test_メニューバーアイコン_犬の模様を透明な抜きで表す();
    test_メニューバーアイコン_外観と色付けと選択状態に追従する();
}

#[cfg(target_os = "macos")]
fn test_メニューバーアイコン_犬の模様を透明な抜きで表す() {
    // Given: the image used by the production tray.
    let icon = tauri::image::Image::from_bytes(tray::ICON).unwrap();
    assert_eq!((icon.width(), icon.height()), (36, 36));

    // When: inspect the template's color and alpha channels.
    let alpha = |x, y| icon.rgba()[((y * icon.width() + x) * 4 + 3) as usize];

    // Then: background, eyes, forehead and chest are cut out, while the nose remains visible.
    assert!(icon
        .rgba()
        .chunks_exact(4)
        .all(|pixel| pixel[..3] == [0, 0, 0]));
    for (x, y) in [(0, 0), (13, 13), (22, 13), (17, 9), (17, 33)] {
        assert!(alpha(x, y) < 32, "({x}, {y}) must be transparent");
    }
    assert_eq!(alpha(18, 17), 255);
}

#[cfg(target_os = "macos")]
fn test_メニューバーアイコン_外観と色付けと選択状態に追従する() {
    use objc2::{
        msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::{NSRect, NSString};

    unsafe fn status_button(view: *mut AnyObject) -> Option<*mut AnyObject> {
        if view.is_null() {
            return None;
        }
        let matches: bool =
            msg_send![view, isKindOfClass: AnyClass::get(c"NSStatusBarButton").unwrap()];
        if matches {
            return Some(view);
        }
        let children: *mut AnyObject = msg_send![view, subviews];
        let count: usize = msg_send![children, count];
        (0..count).find_map(|index| status_button(msg_send![children, objectAtIndex: index]))
    }

    unsafe fn rendered_icon(button: *mut AnyObject) -> Vec<u8> {
        let bounds: NSRect = msg_send![button, bounds];
        let cell: *mut AnyObject = msg_send![button, cell];
        let rect: NSRect = msg_send![cell, imageRectForBounds: bounds];
        let bitmap: *mut AnyObject = msg_send![button, bitmapImageRepForCachingDisplayInRect: rect];
        assert!(!bitmap.is_null(), "tray image must have a drawable area");
        let planar: bool = msg_send![bitmap, isPlanar];
        assert!(!planar);
        let stride: isize = msg_send![bitmap, bytesPerRow];
        let height: isize = msg_send![bitmap, pixelsHigh];
        assert!(stride > 0 && height > 0);
        let data: *mut u8 = msg_send![bitmap, bitmapData];
        assert!(!data.is_null());
        let length = usize::try_from(stride * height).unwrap();
        data.write_bytes(0, length);
        let _: () = msg_send![button, cacheDisplayInRect: rect, toBitmapImageRep: bitmap];
        let pixels = std::slice::from_raw_parts(data, length).to_vec();
        assert!(
            pixels.iter().any(|&byte| byte != 0),
            "tray image must be visible"
        );
        pixels
    }

    tauri::Builder::default()
        .setup(|app| {
            // Given: build the production tray, including its actual image and template flag.
            let _guard = tray::QUIT_REQUESTED_TEST_LOCK.lock().unwrap();
            tray::setup_tray(app, |_| panic!("unexpected Quit"), |_| {})?;
            unsafe {
                let application: *mut AnyObject = msg_send![AnyClass::get(c"NSApplication").unwrap(), sharedApplication];
                let windows: *mut AnyObject = msg_send![application, windows];
                let count: usize = msg_send![windows, count];
                let button = (0..count).find_map(|index| {
                    let window: *mut AnyObject = msg_send![windows, objectAtIndex: index];
                    status_button(msg_send![window, contentView])
                }).expect("production tray must expose an NSStatusBarButton");
                let image: *mut AnyObject = msg_send![button, image];
                assert!(!image.is_null());
                let reference_image: Retained<AnyObject> = msg_send![image, copy];
                let _: () = msg_send![&*reference_image, setTemplate: true];
                let status_bar: *mut AnyObject = msg_send![AnyClass::get(c"NSStatusBar").unwrap(), systemStatusBar];
                let reference_item: *mut AnyObject = msg_send![status_bar, statusItemWithLength: objc2_app_kit::NSVariableStatusItemLength];
                let reference: *mut AnyObject = msg_send![reference_item, button];
                let _: () = msg_send![reference, setImage: &*reference_image];
                let position: isize = msg_send![button, imagePosition];
                let _: () = msg_send![reference, setImagePosition: position];
                let mut rendered = Vec::new();
                for appearance in ["NSAppearanceNameAqua", "NSAppearanceNameDarkAqua"] {
                    for tint in [None, Some(c"systemBlueColor")] {
                        for selected in [false, true] {
                            // When: AppKit applies light/dark, tint and highlighted states.
                            let appearance: *mut AnyObject = msg_send![AnyClass::get(c"NSAppearance").unwrap(), appearanceNamed: &*NSString::from_str(appearance)];
                            assert!(!appearance.is_null());
                            let _: () = msg_send![button, setAppearance: appearance];
                            let _: () = msg_send![reference, setAppearance: appearance];
                            let color: *mut AnyObject = tint.map_or(std::ptr::null_mut(), |tint| msg_send![AnyClass::get(c"NSColor").unwrap(), performSelector: objc2::runtime::Sel::register(tint)]);
                            let _: () = msg_send![button, setContentTintColor: color];
                            let _: () = msg_send![reference, setContentTintColor: color];
                            let _: () = msg_send![button, highlight: selected];
                            let _: () = msg_send![reference, highlight: selected];
                            let cell: *mut AnyObject = msg_send![button, cell];
                            let highlighted: bool = msg_send![cell, isHighlighted];
                            assert_eq!(highlighted, selected);
                            let _: () = msg_send![button, displayIfNeeded];
                            // Then: compare with AppKit's template rendering, including states that preserve the icon color.
                            let image: *mut AnyObject = msg_send![button, image];
                            assert!(!image.is_null());
                            let template: bool = msg_send![image, isTemplate];
                            assert!(template, "tray image must follow AppKit appearance");
                            let pixels = rendered_icon(button);
                            assert!(pixels == rendered_icon(reference), "tray rendering must match a native template in every appearance, tint and selection state");
                            rendered.push(pixels);
                        }
                    }
                }
                assert!(rendered[0] != rendered[4], "light/dark must change the rendered icon");
                for offset in [0, 4] {
                    assert!(rendered[offset] != rendered[offset + 2], "tint must change the rendered icon");
                }
                let _: () = msg_send![status_bar, removeStatusItem: reference_item];
            }
            tray::mark_quit_requested();
            app.handle().exit(0);
            Ok(())
        })
        .run(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("native tray acceptance");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("desktop_tray acceptance requires macOS");
}
