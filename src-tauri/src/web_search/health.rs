use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ProviderHealthStatus {
    pub consecutive_failures: u32,
    pub cooldown_until: Option<Instant>,
    pub total_successes: u64,
    pub total_failures: u64,
}

impl Default for ProviderHealthStatus {
    fn default() -> Self {
        Self {
            consecutive_failures: 0,
            cooldown_until: None,
            total_successes: 0,
            total_failures: 0,
        }
    }
}

impl ProviderHealthStatus {
    pub fn is_available(&self) -> bool {
        if self.consecutive_failures < 3 {
            return true;
        }
        if let Some(cooldown) = self.cooldown_until {
            Instant::now() >= cooldown
        } else {
            true
        }
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.cooldown_until = None;
        self.total_successes += 1;
    }

    pub fn record_failure(&mut self, cooldown_duration: Duration) {
        self.consecutive_failures += 1;
        self.total_failures += 1;
        if self.consecutive_failures >= 3 {
            self.cooldown_until = Some(Instant::now() + cooldown_duration);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderHealthRegistry(Arc<Mutex<HashMap<String, ProviderHealthStatus>>>);

impl ProviderHealthRegistry {
    pub fn is_available(&self, provider: &str) -> bool {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(provider)
            .map(|status| status.is_available())
            .unwrap_or(true)
    }

    pub fn record_success(&self, provider: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(provider.to_string())
            .or_default()
            .record_success();
    }

    pub fn record_failure(&self, provider: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(provider.to_string())
            .or_default()
            .record_failure(Duration::from_secs(300));
    }
}
