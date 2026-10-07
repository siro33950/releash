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
        let process = lookup_process(client.pid);
        client
            .verify_process(|_| releash_client::discovery::ProcessStartTimeLookup {
                process_list_available: process.process_list_available,
                start_time: process.start_time,
            })
            .map_err(|error| ClientConnectionError(error.to_string()))?;

        Ok(ClientConnectionDto {
            url: format!("http://127.0.0.1:{}", client.port),
            token: client.token,
        })
    }
}
