pub mod config;
pub mod manager;

#[cfg(test)]
mod tests;

pub use config::{ProviderConfig, Settings};
pub use manager::SettingsManager;
