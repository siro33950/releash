#[cfg(target_os = "macos")]
mod native {
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject, ClassBuilder, Sel},
        sel,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    };
    static REQUEST: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
    static PENDING: AtomicBool = AtomicBool::new(false);

    unsafe extern "C-unwind" fn should_terminate(
        _: &AnyObject,
        _: Sel,
        _: *mut AnyObject,
    ) -> usize {
        super::dispatch_termination(
            super::super::window_lifecycle::should_prevent_exit(),
            || {
                PENDING.store(true, Ordering::SeqCst);
                if let Some(request) = REQUEST.get() {
                    request();
                }
            },
        )
    }
    pub fn install(request: impl Fn() + Send + Sync + 'static) -> Result<(), String> {
        REQUEST
            .set(Box::new(request))
            .map_err(|_| "Native termination handler already installed")?;
        unsafe {
            let app: *mut AnyObject = msg_send![
                AnyClass::get(c"NSApplication").ok_or("NSApplication unavailable")?,
                sharedApplication
            ];
            let delegate: *mut AnyObject = msg_send![app, delegate];
            let delegate = delegate
                .as_mut()
                .ok_or("NSApplication delegate unavailable")?;
            let mut class = ClassBuilder::new(c"ReleashTerminationDelegate", delegate.class())
                .ok_or("Termination delegate class unavailable")?;
            class.add_method(
                sel!(applicationShouldTerminate:),
                should_terminate as unsafe extern "C-unwind" fn(_, _, _) -> _,
            );
            AnyObject::set_class(delegate, class.register());
        }
        Ok(())
    }
    pub fn reply() -> bool {
        if !PENDING.swap(false, Ordering::SeqCst) {
            return false;
        }
        log::logger().flush();
        unsafe {
            let app: *mut AnyObject =
                msg_send![AnyClass::get(c"NSApplication").unwrap(), sharedApplication];
            let _: () = msg_send![app, replyToApplicationShouldTerminate: true];
        }
        true
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn dispatch_termination(prevent_exit: bool, request: impl FnOnce()) -> usize {
    if !prevent_exit {
        return 1;
    }
    request();
    2
}
#[cfg(target_os = "macos")]
pub(crate) use native::install;
#[cfg(not(target_os = "macos"))]
pub(crate) fn install(_: impl Fn() + Send + Sync + 'static) -> Result<(), String> {
    Ok(())
}

pub(crate) fn exit(app: &tauri::AppHandle, code: i32) {
    super::tray::mark_quit_requested();
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        if native::reply() {
            return;
        }
        handle.exit(code);
    });
    if let Err(error) = result {
        log::error!("Could not finish application termination: {error}");
    }
}
