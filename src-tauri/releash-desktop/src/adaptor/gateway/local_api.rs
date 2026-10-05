pub(crate) struct ClientConnectionFileQuery(pub(crate) std::path::PathBuf);
impl crate::usecase::client_connection::ClientConnectionQueryService for ClientConnectionFileQuery {
    fn read(
        &self,
    ) -> Result<
        crate::usecase::client_connection::ClientConnectionDto,
        crate::usecase::client_connection::ClientConnectionError,
    > {
        releash_lib::desktop_api::read_client_connection(&self.0)
    }
}
