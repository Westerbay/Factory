mod credentials;
mod error;
mod routes;
mod state;

pub use error::AuthError;
pub use routes::router;
pub use state::{AuthState, Authenticated};

#[cfg(test)]
mod tests;
