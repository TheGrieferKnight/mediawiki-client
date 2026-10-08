mod client;
mod error;

pub use client::{Stats, WikiClient, WikiClientBuilder};

pub use error::{Error, Result};
