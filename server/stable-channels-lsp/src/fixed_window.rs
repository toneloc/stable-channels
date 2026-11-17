//! A fixed-window counter: at most `budget` events per `length_secs` window of unix seconds.

/// One window's start and the events counted in it. A clock stepped back starts a new window, so it cannot hold events off.
#[derive(Debug, Default)]
pub struct FixedWindow {
    started_at: u64,
    count: u32,
}

impl FixedWindow {
    /// Whether `now` falls in this window.
    pub fn current(&self, now: u64, length_secs: u64) -> bool {
        now >= self.started_at && now - self.started_at < length_secs
    }

    /// Start a new, empty window at `now` unless `now` falls in this one.
    pub fn roll(&mut self, now: u64, length_secs: u64) {
        if !self.current(now, length_secs) {
            *self = Self {
                started_at: now,
                count: 0,
            };
        }
    }

    /// Whether this window's budget is spent at `now`, without counting an event.
    pub fn spent(&self, now: u64, length_secs: u64, budget: u32) -> bool {
        self.current(now, length_secs) && self.count >= budget
    }

    /// Count an event at `now`; false once this window's budget is spent.
    pub fn allow(&mut self, now: u64, length_secs: u64, budget: u32) -> bool {
        self.roll(now, length_secs);
        if self.count >= budget {
            return false;
        }
        self.count += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_000_000;

    #[test]
    fn a_window_counts_up_to_its_budget_until_it_rolls_over() {
        let mut window = FixedWindow::default();
        assert!(window.allow(T, 60, 2) && window.allow(T + 1, 60, 2));
        assert!(window.spent(T + 59, 60, 2));
        assert!(!window.allow(T + 59, 60, 2));
        assert!(!window.spent(T + 60, 60, 2), "a new window");
        assert!(window.allow(T + 60, 60, 2));
    }

    #[test]
    fn roll_restarts_only_outside_the_window() {
        let mut window = FixedWindow::default();
        assert!(window.allow(T, 60, 1));
        window.roll(T + 59, 60);
        assert!(window.spent(T + 59, 60, 1), "still the same window");
        window.roll(T - 1, 60);
        assert!(
            !window.spent(T - 1, 60, 1),
            "a clock stepped back starts a new window"
        );
        assert!(window.current(T - 1, 60) && !window.current(T + 59, 60));
    }
}
