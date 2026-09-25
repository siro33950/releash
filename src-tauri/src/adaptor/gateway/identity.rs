pub(crate) struct RandomIdentityIssuer;
impl crate::domain::identity::IdentityIssuer for RandomIdentityIssuer {
    fn issue(&self) -> String {
        crate::infrastructure::id::unique_simple_id()
    }
}
