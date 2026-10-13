#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    Compatible,
    ServerOlder,
    ClientOlder,
}

impl Compatibility {
    pub fn assess_release(client: &str, server: &str) -> Result<Self, semver::Error> {
        Ok(
            match semver::Version::parse(client)?.cmp(&semver::Version::parse(server)?) {
                std::cmp::Ordering::Equal => Self::Compatible,
                std::cmp::Ordering::Greater => Self::ServerOlder,
                std::cmp::Ordering::Less => Self::ClientOlder,
            },
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::ServerOlder => "server_older",
            Self::ClientOlder => "client_older",
        }
    }

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
