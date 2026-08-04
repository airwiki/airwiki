use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use libp2p::PeerId;
use parking_lot::Mutex;

pub const SEARCHES_PER_MINUTE: usize = 30;
const MAX_TRACKED_PEERS: usize = 1_024;
const MAX_EXPIRATIONS_PER_CHECK: usize = 1_024;

#[derive(Debug, Default)]
struct RateLimitState {
    attempts: HashMap<PeerId, VecDeque<Instant>>,
    expirations: VecDeque<(Instant, PeerId)>,
}

#[derive(Debug)]
pub struct PeerRateLimiter {
    limit: usize,
    window: Duration,
    max_tracked_peers: usize,
    state: Mutex<RateLimitState>,
}

impl Default for PeerRateLimiter {
    fn default() -> Self {
        Self::new(SEARCHES_PER_MINUTE, Duration::from_secs(60))
    }
}

impl PeerRateLimiter {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self::with_capacity(limit, window, MAX_TRACKED_PEERS)
    }

    fn with_capacity(limit: usize, window: Duration, max_tracked_peers: usize) -> Self {
        Self {
            limit,
            window,
            max_tracked_peers,
            state: Mutex::new(RateLimitState::default()),
        }
    }

    pub fn check(&self, peer: PeerId) -> bool {
        self.check_at(peer, Instant::now())
    }

    fn check_at(&self, peer: PeerId, now: Instant) -> bool {
        let mut state = self.state.lock();
        for _ in 0..MAX_EXPIRATIONS_PER_CHECK {
            let Some((attempt, expired_peer)) = state.expirations.front().copied() else {
                break;
            };
            if now.duration_since(attempt) < self.window {
                break;
            }
            state.expirations.pop_front();
            if let std::collections::hash_map::Entry::Occupied(mut entry) =
                state.attempts.entry(expired_peer)
            {
                let entries = entry.get_mut();
                if entries.front() == Some(&attempt) {
                    entries.pop_front();
                }
                if entries.is_empty() {
                    entry.remove_entry();
                }
            }
        }
        if !state.attempts.contains_key(&peer) && state.attempts.len() >= self.max_tracked_peers {
            return false;
        }
        let entries = state.attempts.entry(peer).or_default();
        if entries.len() >= self.limit {
            return false;
        }
        entries.push_back(now);
        state.expirations.push_back((now, peer));
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

    #[test]
    fn limiter_bounds_peer_churn_and_reuses_expired_capacity() {
        let limiter = PeerRateLimiter::with_capacity(2, Duration::from_secs(60), 2);
        let first = PeerId::random();
        let second = PeerId::random();
        let next = PeerId::random();
        let start = Instant::now();

        assert!(limiter.check_at(first, start));
        assert!(limiter.check_at(second, start));
        assert!(!limiter.check_at(next, start));
        assert_eq!(limiter.state.lock().attempts.len(), 2);

        assert!(limiter.check_at(next, start + Duration::from_secs(60)));
        assert_eq!(limiter.state.lock().attempts.len(), 1);
    }
}
