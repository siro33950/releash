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

pub(crate) fn registration_location(executable: &std::path::Path) -> Result<(bool, bool), String> {
    let translocated = executable
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation");
    #[cfg(target_os = "macos")]
    let read_only = {
        use std::os::unix::ffi::OsStrExt;
        let path =
            std::ffi::CString::new(executable.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        unsafe { info.assume_init() }.f_flags & libc::MNT_RDONLY as u32 != 0
    };
    #[cfg(not(target_os = "macos"))]
    let read_only = false;
    Ok((translocated, read_only))
}

#[cfg(test)]
#[path = "login_item_test.rs"]
mod login_item_tests;
