mod contracts;
mod routes;

pub use contracts::{EnvironmentResponse, EnvironmentStatus};
pub(crate) use routes::documented_router;
pub use routes::router;
