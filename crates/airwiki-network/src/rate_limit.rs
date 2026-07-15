use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use libp2p::PeerId;
use parking_lot::Mutex;

pub const SEARCHES_PER_MINUTE: usize = 30;

#[derive(Debug)]
pub struct PeerRateLimiter {
    limit: usize,
    window: Duration,
    attempts: Mutex<HashMap<PeerId, VecDeque<Instant>>>,
}

impl Default for PeerRateLimiter {
    fn default() -> Self {
        Self::new(SEARCHES_PER_MINUTE, Duration::from_secs(60))
    }
}

impl PeerRateLimiter {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    pub fn check(&self, peer: PeerId) -> bool {
        self.check_at(peer, Instant::now())
    }

    fn check_at(&self, peer: PeerId, now: Instant) -> bool {
        let mut attempts = self.attempts.lock();
        let entries = attempts.entry(peer).or_default();
        while entries
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) >= self.window)
        {
            entries.pop_front();
        }
        if entries.len() >= self.limit {
            return false;
        }
        entries.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_is_per_peer_and_recovers_after_window() {
        let limiter = PeerRateLimiter::new(2, Duration::from_secs(60));
        let peer = PeerId::random();
        let other = PeerId::random();
        let start = Instant::now();
        assert!(limiter.check_at(peer, start));
        assert!(limiter.check_at(peer, start + Duration::from_secs(1)));
        assert!(!limiter.check_at(peer, start + Duration::from_secs(2)));
        assert!(limiter.check_at(other, start + Duration::from_secs(2)));
        assert!(limiter.check_at(peer, start + Duration::from_secs(61)));
    }
}
