pub(crate) trait IdentityIssuer: Send + Sync {
    fn issue(&self) -> String;
}
