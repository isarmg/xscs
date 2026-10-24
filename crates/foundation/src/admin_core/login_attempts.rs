//! Fixed-window admission shared by every service using one administrator store.

use std::sync::Mutex;
use tokio::time::Instant;

use super::{LOGIN_ATTEMPTS_MAX, LOGIN_ATTEMPTS_WINDOW};

#[derive(Debug, Default)]
pub struct AuthenticationState {
    window: Mutex<Option<Window>>,
}

#[derive(Debug)]
struct Window {
    started_at: Instant,
    admitted: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthenticationStateError {
    #[error("authentication admission state is unavailable")]
    Unavailable,
    #[error("login attempt limit reached; retry after {retry_after_seconds} seconds")]
    LoginAttemptsExceeded { retry_after_seconds: u64 },
}

impl AuthenticationState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call exactly once before credential lookup or password verification.
    /// Success never clears the window. Rejections do not extend it.
    pub fn admit_login_attempt(&self) -> Result<(), AuthenticationStateError> {
        let now = Instant::now();
        let mut window = self
            .window
            .lock()
            .map_err(|_| AuthenticationStateError::Unavailable)?;
        let current = window.get_or_insert(Window {
            started_at: now,
            admitted: 0,
        });
        if now.duration_since(current.started_at) >= LOGIN_ATTEMPTS_WINDOW {
            *current = Window {
                started_at: now,
                admitted: 0,
            };
        }
        if current.admitted == LOGIN_ATTEMPTS_MAX {
            let remaining = LOGIN_ATTEMPTS_WINDOW - now.duration_since(current.started_at);
            let retry_after_seconds =
                remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0);
            return Err(AuthenticationStateError::LoginAttemptsExceeded {
                retry_after_seconds,
            });
        }
        current.admitted += 1;
        Ok(())
    }

    pub(crate) fn can_retire(&self) -> bool {
        self.window.lock().is_ok_and(|window| {
            window.as_ref().is_none_or(|window| {
                Instant::now().duration_since(window.started_at) >= LOGIN_ATTEMPTS_WINDOW
            })
        })
    }

    #[cfg(test)]
    pub(crate) fn elapse_window_for_test(&self) {
        if let Some(window) = self.window.lock().unwrap().as_mut() {
            window.started_at -= LOGIN_ATTEMPTS_WINDOW;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn admission_is_bounded_and_rejections_do_not_extend_window() {
        let state = AuthenticationState::new();
        for _ in 0..LOGIN_ATTEMPTS_MAX {
            assert_eq!(state.admit_login_attempt(), Ok(()));
        }
        assert_eq!(
            state.admit_login_attempt(),
            Err(AuthenticationStateError::LoginAttemptsExceeded {
                retry_after_seconds: 60
            })
        );
        tokio::time::advance(std::time::Duration::from_millis(59_500)).await;
        assert_eq!(
            state.admit_login_attempt(),
            Err(AuthenticationStateError::LoginAttemptsExceeded {
                retry_after_seconds: 1
            })
        );
        assert!(!state.can_retire());
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        assert!(state.can_retire());
        assert_eq!(state.admit_login_attempt(), Ok(()));
        assert!(!state.can_retire());
    }
}
