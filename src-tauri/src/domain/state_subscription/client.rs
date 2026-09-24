use super::Version;

#[derive(Debug)]
pub(crate) struct InvalidVersion;
impl std::fmt::Display for InvalidVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid state version")
    }
}
impl std::error::Error for InvalidVersion {}

#[derive(Default)]
pub(crate) struct Cursor {
    version: Option<Version>,
}
impl Cursor {
    pub fn version(&self) -> Option<&Version> {
        self.version.as_ref()
    }
    pub fn accept(&mut self, version: Version, snapshot: bool) -> Result<(), InvalidVersion> {
        if self.version.as_ref().is_some_and(|previous| {
            previous.epoch == version.epoch && previous.sequence > version.sequence
        }) || (!snapshot
            && self
                .version
                .as_ref()
                .is_none_or(|previous| previous.epoch != version.epoch))
        {
            return Err(InvalidVersion);
        }
        self.version = Some(version);
        Ok(())
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_tests;
