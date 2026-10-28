mod credentials;
mod error;
mod routes;
mod state;

pub(crate) use credentials::SESSION_COOKIE;
pub use error::AuthError;
pub(crate) use routes::documented_router;
pub use routes::router;
pub(crate) use state::RequiredIdentity;
pub use state::{AuthState, Authenticated};

#[cfg(test)]
mod tests;
