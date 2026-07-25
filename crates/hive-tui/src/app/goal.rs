//! `/goal` overlay state.

use std::time::{Duration, Instant};

/// Which field of the goal overlay is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalField {
    Objective,
    TimeLimit,
}

/// Active goal state (after the overlay is submitted).
#[derive(Debug, Clone)]
pub struct GoalOverlayState {
    pub objective: String,
    pub time_limit: String,
    pub focus: GoalField,
}

impl GoalOverlayState {
    pub fn new() -> Self {
        Self {
            objective: String::new(),
            time_limit: String::new(),
            focus: GoalField::Objective,
        }
    }

    /// Push a char into the focused field.
    pub fn type_char(&mut self, c: char) {
        match self.focus {
            GoalField::Objective => self.objective.push(c),
            GoalField::TimeLimit => self.time_limit.push(c),
        }
    }

    /// Backspace on the focused field.
    pub fn backspace(&mut self) {
        match self.focus {
            GoalField::Objective => {
                self.objective.pop();
            }
            GoalField::TimeLimit => {
                self.time_limit.pop();
            }
        }
    }

    /// Cycle focus between fields.
    #[allow(dead_code)]
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            GoalField::Objective => GoalField::TimeLimit,
            GoalField::TimeLimit => GoalField::Objective,
        };
    }
}

/// Active goal state (after the overlay is submitted).
#[derive(Debug, Clone)]
pub struct GoalStatus {
    pub objective: String,
    pub deadline: Option<Instant>,
    pub paused: bool,
    /// How many goal turns have completed (for "Circle N" labels).
    pub circle: u64,
}

impl GoalStatus {
    pub fn remaining_secs(&self) -> u64 {
        self.deadline
            .map(|d| {
                let now = Instant::now();
                if d > now {
                    d.duration_since(now).as_secs()
                } else {
                    0
                }
            })
            .unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn expired(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }

    /// Compact display: "23m left", "no limit", "paused".
    pub fn timer_label(&self) -> String {
        if self.paused {
            return "paused".to_string();
        }
        match self.deadline {
            None => "no limit".to_string(),
            Some(_) => {
                let secs = self.remaining_secs();
                if secs >= 3600 {
                    format!("{}h {}m left", secs / 3600, (secs % 3600) / 60)
                } else if secs >= 60 {
                    format!("{}m left", secs / 60)
                } else {
                    format!("{}s left", secs)
                }
            }
        }
    }
}

/// Parse a duration string like "30m", "1h", "2h30m", "45s".
/// Empty string → None. Returns None on unparseable input.
#[allow(dead_code)]
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total_secs: u64 = 0;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let n: u64 = num.parse().ok()?;
            num.clear();
            match c {
                'h' => total_secs += n * 3600,
                'm' => total_secs += n * 60,
                's' => total_secs += n,
                _ => return None,
            }
        }
    }
    // Trailing digits without a unit → treat as minutes.
    if !num.is_empty() {
        let n: u64 = num.parse().ok()?;
        total_secs += n * 60;
    }
    if total_secs == 0 {
        return None;
    }
    Some(Duration::from_secs(total_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_units() {
        assert_eq!(parse_duration("30m"), Some(Duration::from_secs(1800)));
        assert_eq!(parse_duration("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_duration("45s"), Some(Duration::from_secs(45)));
    }

    #[test]
    fn parse_combo() {
        assert_eq!(parse_duration("1h30m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("2h15m30s"), Some(Duration::from_secs(8130)));
    }

    #[test]
    fn parse_bare_number_is_minutes() {
        assert_eq!(parse_duration("30"), Some(Duration::from_secs(1800)));
    }

    #[test]
    fn parse_empty_is_none() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("   "), None);
    }

    #[test]
    fn parse_zero_is_none() {
        assert_eq!(parse_duration("0m"), None);
        assert_eq!(parse_duration("0s"), None);
    }

    #[test]
    fn parse_invalid_is_none() {
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("1x"), None);
    }

    #[test]
    fn overlay_type_char() {
        let mut st = GoalOverlayState::new();
        st.type_char('h');
        st.type_char('i');
        assert_eq!(st.objective, "hi");
    }

    #[test]
    fn overlay_backspace() {
        let mut st = GoalOverlayState::new();
        st.type_char('a');
        st.type_char('b');
        st.backspace();
        assert_eq!(st.objective, "a");
    }

    #[test]
    fn overlay_time_limit_field() {
        let mut st = GoalOverlayState::new();
        st.focus = GoalField::TimeLimit;
        st.type_char('3');
        st.type_char('0');
        st.type_char('m');
        assert_eq!(st.time_limit, "30m");
        st.backspace();
        assert_eq!(st.time_limit, "30");
    }

    #[test]
    fn overlay_toggle_focus() {
        let mut st = GoalOverlayState::new();
        assert_eq!(st.focus, GoalField::Objective);
        st.toggle_focus();
        assert_eq!(st.focus, GoalField::TimeLimit);
        st.toggle_focus();
        assert_eq!(st.focus, GoalField::Objective);
    }
}
