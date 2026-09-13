pub(crate) mod code;
pub(crate) mod comment;
mod dependencies;
pub(crate) mod dispatch;
pub(crate) mod repository;
pub(crate) use dependencies::ClientDependencies;
pub(crate) use dispatch::{
    command_admitted, convert, invalid_request, optional, outcome, required, value,
    ClientCommandDispatch,
};
