//! Rate limiter using leaky bucket algorithm

use std::time::{Duration, Instant};

/// Leaky bucket rate limiter for terminal input
///
/// Limits the rate of input to prevent abuse while allowing bursts.
#[derive(Debug)]
pub struct RateLimiter {
    /// Maximum tokens (burst capacity)
    capacity: u32,
    /// Current token count
    tokens: f64,
    /// Tokens added per second
    rate: f64,
    /// Last time tokens were updated
    last_update: Instant,
}

impl RateLimiter {
    /// Create a new rate limiter
    ///
    /// # Arguments
    ///
    /// * `rate` - Tokens added per second (sustained rate)
    /// * `capacity` - Maximum tokens (burst capacity)
    ///
    /// # Example
    ///
    /// ```
    /// use microsandbox_terminal::RateLimiter;
    ///
    /// // Allow 10 inputs/sec with burst of 20
    /// let limiter = RateLimiter::new(10, 20);
    /// ```
    pub fn new(rate: u32, capacity: u32) -> Self {
        Self {
            capacity,
            tokens: capacity as f64,
            rate: rate as f64,
            last_update: Instant::now(),
        }
    }

    /// Default rate limiter for terminal input
    ///
    /// - Rate: 10 inputs/second
    /// - Burst: 20 inputs
    pub fn default_terminal() -> Self {
        Self::new(10, 20)
    }

    /// Check if an action is allowed and consume a token if so
    ///
    /// Returns `true` if the action is allowed, `false` if rate limited.
    pub fn check(&mut self) -> bool {
        self.refill();

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Check if an action is allowed without consuming a token
    pub fn peek(&mut self) -> bool {
        self.refill();
        self.tokens >= 1.0
    }

    /// Get current token count
    pub fn tokens(&mut self) -> f64 {
        self.refill();
        self.tokens
    }

    /// Get the rate (tokens per second)
    pub fn rate(&self) -> u32 {
        self.rate as u32
    }

    /// Get the capacity (max burst)
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Reset the rate limiter to full capacity
    pub fn reset(&mut self) {
        self.tokens = self.capacity as f64;
        self.last_update = Instant::now();
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update);
        let tokens_to_add = elapsed.as_secs_f64() * self.rate;

        self.tokens = (self.tokens + tokens_to_add).min(self.capacity as f64);
        self.last_update = now;
    }

    /// Time until next token is available
    pub fn time_until_available(&mut self) -> Duration {
        self.refill();

        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let tokens_needed = 1.0 - self.tokens;
            let seconds = tokens_needed / self.rate;
            Duration::from_secs_f64(seconds)
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::default_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_initial_burst() {
        let mut limiter = RateLimiter::new(10, 5);

        // Should allow burst of 5
        for _ in 0..5 {
            assert!(limiter.check());
        }

        // 6th should be rate limited
        assert!(!limiter.check());
    }

    #[test]
    fn test_refill() {
        let mut limiter = RateLimiter::new(100, 10);

        // Exhaust all tokens
        for _ in 0..10 {
            assert!(limiter.check());
        }
        assert!(!limiter.check());

        // Wait for refill (100 tokens/sec = 10ms per token)
        sleep(Duration::from_millis(15));

        // Should have at least 1 token now
        assert!(limiter.check());
    }

    #[test]
    fn test_peek_doesnt_consume() {
        let mut limiter = RateLimiter::new(10, 2);

        assert!(limiter.peek());
        assert!(limiter.peek());
        assert!(limiter.peek());

        // Tokens still available
        assert_eq!(limiter.tokens() as u32, 2);
    }

    #[test]
    fn test_reset() {
        let mut limiter = RateLimiter::new(10, 5);

        // Exhaust tokens
        for _ in 0..5 {
            limiter.check();
        }
        assert!(!limiter.check());

        // Reset
        limiter.reset();

        // Should be back to full capacity
        assert!(limiter.check());
        assert_eq!(limiter.tokens() as u32, 4);
    }
}
