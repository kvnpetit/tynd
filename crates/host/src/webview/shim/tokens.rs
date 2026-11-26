//! The two IPC tokens of a webview and the app-token hand-off.
//!
//! WebKit exposes the raw script-message handler to every frame and
//! WebKitGTK cannot tell the host which frame posted, so the host trusts a
//! message only by the token it starts with:
//!
//! - the *remote* token is baked into the init script, which every document
//!   of the webview receives (remote pages included). A message carrying it
//!   never gets more than remote-origin privileges;
//! - the *app* token is never part of any script. An app document asks for it
//!   with a remote-token message `?<request id>`; the host answers only while
//!   the main frame shows app content, and the answer is evaluated in the main
//!   frame behind a check of its own `location`. A message carrying it is
//!   accepted only while the main frame is app content.
//!
//! wry 0.57 cannot replace an init script after the webview is built, which
//! is why the app token travels out of band instead of being baked in. The
//! app token is re-minted whenever a remote document commits, so one obtained
//! before a remote page was shown is worthless afterwards.

use std::{collections::VecDeque, sync::Arc};

use parking_lot::Mutex;

use super::new_ipc_token;

/// Prefix of the request id an app document sends to get the app token. The
/// answer to such an id is location-guarded by [`super::eval_os_result`].
pub(crate) const APP_TOKEN_ID_PREFIX: &str = "tynd-app-token:";

/// First character of an app-token request body (after the remote token).
const APP_TOKEN_REQUEST: char = '?';

/// Longest accepted request id suffix (a UUID is 36 characters).
const MAX_REQUEST_SUFFIX: usize = 64;

/// Unanswered requests kept while the main frame is not app content: a
/// request can arrive just before the commit of the app document that sent
/// it. The oldest are dropped first; the newest one is the live document's.
const MAX_PENDING_REQUESTS: usize = 8;

/// Which token a message carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    /// App token: app privileges, only while the main frame is app content.
    App,
    /// Remote token: remote-origin privileges at most.
    Remote,
}

/// Token state of one webview, shared by its IPC and page-load handlers.
pub(crate) struct IpcTokens {
    remote: String,
    state: Mutex<AppTokenState>,
}

struct AppTokenState {
    app: String,
    pending: VecDeque<String>,
}

impl IpcTokens {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            remote: new_ipc_token(),
            state: Mutex::new(AppTokenState {
                app: new_ipc_token(),
                pending: VecDeque::new(),
            }),
        })
    }

    /// The token baked into the webview's init script.
    pub(crate) fn remote(&self) -> &str {
        &self.remote
    }

    /// The current app token.
    pub(crate) fn app(&self) -> String {
        self.state.lock().app.clone()
    }

    /// Strip the token `body` starts with and report which one it was.
    /// `None` (body untouched) when it carries neither.
    pub(crate) fn take(&self, body: &mut String) -> Option<TokenKind> {
        let app = self.app();
        if super::take_ipc_token(body, &app) {
            Some(TokenKind::App)
        } else if super::take_ipc_token(body, &self.remote) {
            Some(TokenKind::Remote)
        } else {
            None
        }
    }

    /// Re-mint the app token (a remote document committed). Pending
    /// requests stay: they are answered on the next app-content commit.
    pub(crate) fn rotate_app(&self) {
        self.state.lock().app = new_ipc_token();
    }

    /// Keep `request_id` until the main frame shows app content.
    pub(crate) fn defer_request(&self, request_id: &str) {
        let mut state = self.state.lock();
        if state.pending.len() >= MAX_PENDING_REQUESTS {
            state.pending.pop_front();
        }
        state.pending.push_back(request_id.to_owned());
    }

    /// Drain the deferred requests together with the token that answers them.
    pub(crate) fn take_pending(&self) -> (Vec<String>, String) {
        let mut state = self.state.lock();
        let pending = state.pending.drain(..).collect();
        (pending, state.app.clone())
    }
}

/// The request id of an app-token request body (remote token already
/// stripped), or `None` when `body` is an ordinary message.
pub(crate) fn app_token_request(body: &str) -> Option<&str> {
    let id = body.strip_prefix(APP_TOKEN_REQUEST)?;
    let suffix = id.strip_prefix(APP_TOKEN_ID_PREFIX)?;
    let valid = !suffix.is_empty()
        && suffix.len() <= MAX_REQUEST_SUFFIX
        && suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'));
    valid.then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_distinct_and_classified() {
        let tokens = IpcTokens::new();
        let app = tokens.app();
        assert_ne!(app, tokens.remote());

        let mut body = format!("{app}{{}}");
        assert_eq!(tokens.take(&mut body), Some(TokenKind::App));
        assert_eq!(body, "{}");

        let mut body = format!("{}{{}}", tokens.remote());
        assert_eq!(tokens.take(&mut body), Some(TokenKind::Remote));
        assert_eq!(body, "{}");

        let mut body = "{}".to_owned();
        assert_eq!(tokens.take(&mut body), None);
        assert_eq!(body, "{}");
    }

    #[test]
    fn rotation_invalidates_the_old_app_token() {
        let tokens = IpcTokens::new();
        let old = tokens.app();
        tokens.rotate_app();
        assert_ne!(tokens.app(), old);
        let mut body = format!("{old}{{}}");
        assert_eq!(tokens.take(&mut body), None);
    }

    #[test]
    fn deferred_requests_are_bounded_and_drained() {
        let tokens = IpcTokens::new();
        for i in 0..MAX_PENDING_REQUESTS + 2 {
            tokens.defer_request(&format!("{APP_TOKEN_ID_PREFIX}{i}"));
        }
        let (pending, token) = tokens.take_pending();
        assert_eq!(token, tokens.app());
        assert_eq!(pending.len(), MAX_PENDING_REQUESTS);
        assert_eq!(pending[0], format!("{APP_TOKEN_ID_PREFIX}2"));
        assert_eq!(tokens.take_pending().0, Vec::<String>::new());
    }

    #[test]
    fn app_token_requests_are_recognised_strictly() {
        let id = format!("{APP_TOKEN_ID_PREFIX}0f8e-AB.12");
        assert_eq!(app_token_request(&format!("?{id}")), Some(id.as_str()));
        for body in [
            "{\"type\":\"call\"}".to_owned(),
            format!("?{APP_TOKEN_ID_PREFIX}"),
            "?os_123".to_owned(),
            format!("?{APP_TOKEN_ID_PREFIX}a\"b"),
            format!(
                "?{APP_TOKEN_ID_PREFIX}{}",
                "a".repeat(MAX_REQUEST_SUFFIX + 1)
            ),
            id.clone(),
        ] {
            assert_eq!(app_token_request(&body), None, "{body}");
        }
    }
}
