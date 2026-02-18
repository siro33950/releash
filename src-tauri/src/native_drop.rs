use tauri::WebviewWindow;

#[derive(Clone, serde::Serialize)]
pub struct NativeFileDrop {
    pub paths: Vec<String>,
    pub position: (f64, f64),
}

pub fn install(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    macos::install(window);

    #[cfg(target_os = "windows")]
    let _ = window; // TODO: Windows実装

    #[cfg(target_os = "linux")]
    let _ = window; // TODO: Linux実装
}

#[cfg(target_os = "macos")]
mod macos {
    use super::NativeFileDrop;
    use objc2::runtime::{AnyObject, Imp, Sel};
    use objc2::sel;
    use std::mem;
    use std::sync::OnceLock;
    use tauri::{Emitter, Manager, WebviewWindow};

    static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

    type PerformDragFn =
        unsafe extern "C-unwind" fn(*const AnyObject, Sel, *const AnyObject) -> bool;

    static ORIGINAL_PERFORM_DRAG: OnceLock<PerformDragFn> = OnceLock::new();

    pub fn install(window: &WebviewWindow) {
        let _ = APP_HANDLE.set(window.app_handle().clone());

        window
            .with_webview(|webview| unsafe {
                let wk_webview: *const AnyObject = webview.inner().cast();
                let cls = (*wk_webview).class();

                let sel = sel!(performDragOperation:);
                if let Some(method) = cls.instance_method(sel) {
                    let original_imp: Imp = method.implementation();
                    let _ = ORIGINAL_PERFORM_DRAG
                        .set(mem::transmute::<Imp, PerformDragFn>(original_imp));

                    let new_imp: Imp = mem::transmute::<PerformDragFn, Imp>(swizzled_perform_drag);
                    method.set_implementation(new_imp);
                }
            })
            .unwrap_or_else(|e| {
                log::error!("Failed to install native drop handler: {e}");
            });
    }

    unsafe extern "C-unwind" fn swizzled_perform_drag(
        this: *const AnyObject,
        cmd: Sel,
        sender: *const AnyObject,
    ) -> bool {
        if let Some(app_handle) = APP_HANDLE.get() {
            if let Some(drop) = extract_drop_info(this, sender) {
                let _ = app_handle.emit("native-file-drop", drop);
                // 外部ファイルドロップ: 元のIMPを呼ばない
                // （呼ぶとWKWebViewがファイルURLにナビゲートしてしまう）
                return true;
            }
        }

        // 外部ファイルでない場合（HTML5 D&D: タブ並び替え等）は元のIMPに委譲
        if let Some(original) = ORIGINAL_PERFORM_DRAG.get() {
            original(this, cmd, sender)
        } else {
            false
        }
    }

    unsafe fn extract_drop_info(
        view: *const AnyObject,
        sender: *const AnyObject,
    ) -> Option<NativeFileDrop> {
        use objc2::msg_send;
        use objc2_app_kit::NSPasteboardTypeFileURL;
        use objc2_foundation::{NSArray, NSPoint, NSString, NSURL};

        // draggingPasteboard を取得
        let pasteboard: *const AnyObject = msg_send![sender, draggingPasteboard];
        if pasteboard.is_null() {
            return None;
        }

        // pasteboardItems を取得
        let items: Option<&NSArray<AnyObject>> = msg_send![pasteboard, pasteboardItems];
        let items = items?;

        let mut paths = Vec::new();
        for item in items.iter() {
            let item_ref: &AnyObject = &item;
            let url_string: Option<&NSString> =
                msg_send![item_ref, stringForType: NSPasteboardTypeFileURL];
            if let Some(url_str) = url_string {
                let nsurl = NSURL::URLWithString(url_str);
                if let Some(nsurl) = nsurl {
                    if let Some(path) = nsurl.path() {
                        let p = path.to_string();
                        if !p.is_empty() {
                            paths.push(p);
                        }
                    }
                }
            }
        }

        if paths.is_empty() {
            return None;
        }

        // draggingLocation を取得（ウィンドウ座標系）
        let location: NSPoint = msg_send![sender, draggingLocation];

        // NSView の frame を取得して座標変換
        // draggingLocation はポイント単位（= CSSピクセル）なので scale_factor 不要
        let frame: objc2_foundation::NSRect = msg_send![view, frame];

        // Cocoa座標（左下原点）→ CSS座標（左上原点）
        let css_x = location.x;
        let css_y = frame.size.height - location.y;

        Some(NativeFileDrop {
            paths,
            position: (css_x, css_y),
        })
    }
}

/// Cocoa座標（ポイント単位、左下原点）→ CSS座標（左上原点）変換のテスト用ヘルパー
/// draggingLocation はポイント単位でCSSピクセルと同一なので scale_factor 不要
#[cfg(test)]
fn convert_cocoa_to_css(location_x: f64, location_y: f64, frame_height: f64) -> (f64, f64) {
    let css_x = location_x;
    let css_y = frame_height - location_y;
    (css_x, css_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_file_drop_serializes_correctly() {
        let drop = NativeFileDrop {
            paths: vec!["/Users/test/file.txt".to_string()],
            position: (100.0, 200.0),
        };
        let json = serde_json::to_value(&drop).unwrap();
        assert_eq!(json["paths"][0], "/Users/test/file.txt");
        assert_eq!(json["position"][0], 100.0);
        assert_eq!(json["position"][1], 200.0);
    }

    #[test]
    fn native_file_drop_multiple_paths() {
        let drop = NativeFileDrop {
            paths: vec![
                "/Users/test/a.txt".to_string(),
                "/Users/test/b.txt".to_string(),
            ],
            position: (50.0, 75.0),
        };
        let json = serde_json::to_value(&drop).unwrap();
        assert_eq!(json["paths"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn cocoa_to_css_y_flip() {
        // Cocoa (200, 600) → CSS (200, 200) for frame_height=800
        let (x, y) = convert_cocoa_to_css(200.0, 600.0, 800.0);
        assert_eq!(x, 200.0);
        assert_eq!(y, 200.0);
    }

    #[test]
    fn cocoa_to_css_center() {
        let (x, y) = convert_cocoa_to_css(400.0, 300.0, 600.0);
        assert_eq!(x, 400.0);
        assert_eq!(y, 300.0);
    }

    #[test]
    fn cocoa_to_css_origin() {
        // 左下原点 (0, 0) → CSS左上原点 (0, frame_height)
        let (x, y) = convert_cocoa_to_css(0.0, 0.0, 1000.0);
        assert_eq!(x, 0.0);
        assert_eq!(y, 1000.0);
    }
}
