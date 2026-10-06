use crate::infrastructure::process::search_path::{LoginShellPathError, SearchPathSource};
pub struct FailingSearchPathSource(pub LoginShellPathError);
impl SearchPathSource for FailingSearchPathSource {
    fn load(&self) -> Result<std::ffi::OsString, LoginShellPathError> {
        Err(self.0)
    }
}
