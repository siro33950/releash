#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    Compatible,
    ServerOlder,
    ClientOlder,
}

impl Compatibility {
    pub fn assess(client_protocol: u32, server_protocol: u32) -> Self {
        match client_protocol.cmp(&server_protocol) {
            std::cmp::Ordering::Equal => Self::Compatible,
            std::cmp::Ordering::Greater => Self::ServerOlder,
            std::cmp::Ordering::Less => Self::ClientOlder,
        }
    }
}

#[cfg(test)]
#[path = "compatibility_test.rs"]
mod compatibility_tests;
