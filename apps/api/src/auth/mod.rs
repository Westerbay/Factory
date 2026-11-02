mod credentials;
mod error;
mod routes;
mod state;

pub(crate) use credentials::SESSION_COOKIE;
pub(crate) use credentials::cookie;
pub(crate) use credentials::{hash_token, random_token};
pub use error::AuthError;
pub(crate) use routes::documented_router;
pub use routes::router;
pub(crate) use state::RequiredIdentity;
pub use state::{AuthState, Authenticated};

#[cfg(test)]
mod tests;
