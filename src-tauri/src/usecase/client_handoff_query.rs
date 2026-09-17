#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientHandoffSummary {
    pub id: String,
    pub command: String,
    pub fingerprint: Vec<u8>,
    pub ordering_target: Vec<u8>,
}
pub(crate) trait ClientHandoffQueryService: Send + Sync {
    fn list(&self) -> Result<Vec<ClientHandoffSummary>, String>;
}
