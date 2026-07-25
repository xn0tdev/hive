//! Loop detection: catches when the agent repeats the same tool call
//! (same name + same arguments) too many times in a row, signalling it is
//! stuck. The turn loop uses this to inject a redirect, compact, or give up.

/// Number of identical consecutive tool calls that triggers a loop.
pub const LOOP_THRESHOLD: usize = 10;

/// Tracks recent tool calls to detect repetition.
#[derive(Default)]
pub struct LoopDetector {
    /// (name, arguments) of the last tool call.
    last: Option<(String, String)>,
    /// How many times that exact call has repeated consecutively.
    streak: usize,
}

impl LoopDetector {
    /// Record a tool call. Returns `true` if this completes a loop (the streak
    /// just reached [`LOOP_THRESHOLD`]).
    pub fn record(&mut self, name: &str, arguments: &str) -> bool {
        let key = (name.to_string(), arguments.to_string());
        match &self.last {
            Some(prev) if *prev == key => {
                self.streak += 1;
            }
            _ => {
                self.last = Some(key);
                self.streak = 1;
            }
        }
        self.streak >= LOOP_THRESHOLD
    }

    /// Reset the streak (call this after a redirect so the detector starts fresh).
    pub fn reset_streak(&mut self) {
        self.last = None;
        self.streak = 0;
    }
}

/// The redirect message injected into the conversation when a loop is detected.
/// Short and direct — tells the agent what happened and to try a different approach.
pub fn redirect_message(tool: &str, args: &str) -> String {
    format!(
        "You are stuck: `{tool}` called {n} times with the same arguments ({args}). \
Stop repeating it. Re-read the situation and try a different approach. \
If you cannot proceed, explain the blocker.",
        n = LOOP_THRESHOLD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_repeated_calls() {
        let mut d = LoopDetector::default();
        // 9 repeats don't trigger.
        for _ in 0..9 {
            assert!(!d.record("read_file", r#"{"path":"a.rs"}"#));
        }
        // 10th identical call triggers.
        assert!(d.record("read_file", r#"{"path":"a.rs"}"#));
    }

    #[test]
    fn different_args_reset_streak() {
        let mut d = LoopDetector::default();
        d.record("read_file", r#"{"path":"a.rs"}"#);
        d.record("read_file", r#"{"path":"a.rs"}"#);
        // Different args → streak resets.
        assert!(!d.record("read_file", r#"{"path":"b.rs"}"#));
        assert_eq!(d.streak, 1);
    }

    #[test]
    fn different_tool_resets_streak() {
        let mut d = LoopDetector::default();
        d.record("read_file", r#"{"path":"a.rs"}"#);
        d.record("read_file", r#"{"path":"a.rs"}"#);
        assert!(!d.record("run_shell", r#"{"command":"ls"}"#));
        assert_eq!(d.streak, 1);
    }

    #[test]
    fn redirect_message_mentions_tool_and_approach() {
        let msg = redirect_message("read_file", r#"{"path":"a.rs"}"#);
        assert!(msg.contains("read_file"), "{msg}");
        assert!(msg.contains("different approach"), "{msg}");
        assert!(msg.contains("10"), "{msg}");
    }
}
