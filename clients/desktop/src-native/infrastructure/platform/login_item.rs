#[cfg(target_os = "macos")]
mod native {
    use objc2::{msg_send, rc::Retained, runtime::AnyClass};
    use objc2_foundation::{NSError, NSObject, NSString};

    #[link(name = "ServiceManagement", kind = "framework")]
    extern "C" {}

    fn class() -> Result<&'static AnyClass, String> {
        AnyClass::get(c"SMAppService")
            .ok_or_else(|| "Login items require macOS 13 or later.".into())
    }
    fn service() -> Result<Retained<NSObject>, String> {
        let plist = NSString::from_str("com.releash.app.plist");
        Ok(unsafe { msg_send![class()?, agentServiceWithPlistName: &*plist] })
    }
    pub fn status() -> Result<isize, String> {
        Ok(unsafe { msg_send![&*service()?, status] })
    }
    pub fn set_registered(registered: bool) -> Result<(), String> {
        let service = service()?;
        let mut error: Option<Retained<NSError>> = None;
        let success: bool = unsafe {
            if registered {
                msg_send![&*service, registerAndReturnError: &mut error]
            } else {
                msg_send![&*service, unregisterAndReturnError: &mut error]
            }
        };
        if success {
            Ok(())
        } else {
            Err(error
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_else(|| "Login item registration failed.".into()))
        }
    }
    pub fn open_settings() -> Result<(), String> {
        unsafe {
            let _: () = msg_send![class()?, openSystemSettingsLoginItems];
        }
        Ok(())
    }
}
#[cfg(target_os = "macos")]
pub(crate) use native::{open_settings, set_registered, status};

#[cfg(not(target_os = "macos"))]
pub(crate) fn status() -> Result<isize, String> {
    Ok(0)
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn set_registered(_: bool) -> Result<(), String> {
    Err("Login items require macOS.".into())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn open_settings() -> Result<(), String> {
    Err("Login items require macOS.".into())
}
