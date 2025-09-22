use super::types::{RiskLevel, SecurityConfig, SecurityMode, ToolPermission};

/// Manages security policies and approval workflows
pub struct SecurityManager {
    config: SecurityConfig,
}

impl SecurityManager {
    pub fn new(config: SecurityConfig) -> Self {
        Self { config }
    }

    /// Update security configuration
    pub fn update_config(&mut self, config: SecurityConfig) {
        self.config = config;
    }

    /// Get current security mode
    pub fn get_mode(&self) -> SecurityMode {
        self.config.mode
    }

    /// Set security mode
    pub fn set_mode(&mut self, mode: SecurityMode) {
        self.config.mode = mode;
        tracing::info!("Security mode changed to: {:?}", mode);
    }

    /// Check if a tool operation is permitted
    pub fn check_permission(&self, risk: RiskLevel) -> ToolPermission {
        match risk {
            RiskLevel::ReadOnly => ToolPermission::Allowed,
            RiskLevel::LowRisk => match self.config.mode {
                SecurityMode::ReadOnly => ToolPermission::Denied,
                SecurityMode::Auto | SecurityMode::All => ToolPermission::Allowed,
            },
            RiskLevel::HighRisk => match self.config.mode {
                SecurityMode::ReadOnly | SecurityMode::Auto => ToolPermission::Denied,
                SecurityMode::All => ToolPermission::Allowed,
            },
        }
    }

    /// Get current security configuration
    pub fn get_config(&self) -> &SecurityConfig {
        &self.config
    }
}
