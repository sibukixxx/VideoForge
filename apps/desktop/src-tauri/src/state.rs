//! Mutable state shared by commands: the cancellation token of the generate
//! that is currently running, if any.

use std::sync::Mutex;

use videoforge_core::CancellationToken;

#[derive(Default)]
pub struct AppState {
    pub generate: Mutex<Option<CancellationToken>>,
}

impl AppState {
    /// Register a new run. Returns `None` while another generate is running:
    /// the core would reject a second run of the same slug anyway
    /// (`AppError::Busy`), but the UI has a single progress panel, so one at
    /// a time is the rule for the whole window.
    pub fn begin_generate(&self) -> Option<CancellationToken> {
        let mut slot = self.generate.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return None;
        }
        let token = CancellationToken::new();
        *slot = Some(token.clone());
        Some(token)
    }

    pub fn end_generate(&self) {
        let mut slot = self.generate.lock().unwrap_or_else(|e| e.into_inner());
        *slot = None;
    }

    pub fn cancel_generate(&self) -> bool {
        let slot = self.generate.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_generate_at_a_time() {
        let state = AppState::default();
        assert!(!state.cancel_generate(), "nothing to cancel yet");
        let token = state.begin_generate().unwrap();
        assert!(state.begin_generate().is_none());
        assert!(state.cancel_generate());
        assert!(token.is_cancelled());
        state.end_generate();
        assert!(state.begin_generate().is_some());
    }
}
