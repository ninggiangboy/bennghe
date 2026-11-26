// SPDX-License-Identifier: AGPL-3.0-only
//! In-memory failure counter used for login, second-factor and re-authentication attempts.
//! One control plane process serves all requests, so process memory is enough; a restart
//! clears the counters, which only ever helps a legitimate user.
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

const MAX_KEYS: usize = 20_000;

pub struct RateLimiter {
    max_failures: usize,
    window: Duration,
    failures: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    pub fn new(max_failures: usize, window: Duration) -> Self {
        Self { max_failures, window, failures: Mutex::new(HashMap::new()) }
    }

    pub fn login() -> Self {
        Self::new(10, Duration::from_secs(15 * 60))
    }

    pub fn is_blocked(&self, key: &str) -> bool {
        let mut map = self.failures.lock().expect("rate limiter lock");
        match map.get_mut(key) {
            Some(list) => {
                list.retain(|t| t.elapsed() < self.window);
                list.len() >= self.max_failures
            }
            None => false,
        }
    }

    pub fn record_failure(&self, key: &str) {
        let mut map = self.failures.lock().expect("rate limiter lock");
        if map.len() >= MAX_KEYS {
            map.retain(|_, list| list.iter().any(|t| t.elapsed() < self.window));
        }
        if map.len() < MAX_KEYS {
            map.entry(key.to_owned()).or_default().push(Instant::now());
        }
    }

    pub fn clear(&self, key: &str) {
        self.failures.lock().expect("rate limiter lock").remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max_failures_and_clears() {
        let rl = RateLimiter::new(3, Duration::from_secs(60));
        for _ in 0..2 {
            rl.record_failure("a");
        }
        assert!(!rl.is_blocked("a"));
        rl.record_failure("a");
        assert!(rl.is_blocked("a"));
        assert!(!rl.is_blocked("b"));
        rl.clear("a");
        assert!(!rl.is_blocked("a"));
    }

    #[test]
    fn failures_expire() {
        let rl = RateLimiter::new(1, Duration::from_millis(20));
        rl.record_failure("a");
        assert!(rl.is_blocked("a"));
        std::thread::sleep(Duration::from_millis(40));
        assert!(!rl.is_blocked("a"));
    }
}
