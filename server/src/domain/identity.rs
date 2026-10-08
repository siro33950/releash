pub trait IdentityIssuer: Send + Sync {
    fn issue(&self) -> String;
}
