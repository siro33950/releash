use crate::usecase::client_connection::{
    ClientConnectionDto, ClientConnectionError, ClientConnectionQueryService,
};
use releash_client::discovery::{lookup_process_start_time, ProcessStartTimeLookup};
use std::path::PathBuf;

pub struct ClientConnectionFileQuery(pub PathBuf);

impl ClientConnectionQueryService for ClientConnectionFileQuery {
    fn read(&self) -> Result<ClientConnectionDto, ClientConnectionError> {
        self.read_with_process_lookup(lookup_process_start_time)
    }
}

impl ClientConnectionFileQuery {
    pub fn read_with_process_lookup(
        &self,
        lookup_process: impl FnOnce(u32) -> ProcessStartTimeLookup,
    ) -> Result<ClientConnectionDto, ClientConnectionError> {
        let client = releash_client::discovery::read(&self.0)
            .map_err(|error| ClientConnectionError(error.to_string()))?;
        client
            .verify_process(lookup_process)
            .map_err(|error| ClientConnectionError(error.to_string()))?;

        Ok(ClientConnectionDto {
            url: format!("http://127.0.0.1:{}", client.port),
            token: client.token,
        })
    }
}
