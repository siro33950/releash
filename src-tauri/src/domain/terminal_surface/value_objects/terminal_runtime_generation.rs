#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TerminalRuntimeGeneration(u64);

impl TerminalRuntimeGeneration {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl From<u64> for TerminalRuntimeGeneration {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}
