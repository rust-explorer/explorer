use crate::auth::CurrentUser;
use crate::http::error::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read,
    Write,
}

/// Per-volume authorization. Membership in the allowed group is checked at sign-in, so v1 lets
/// every signed-in user do everything.
#[derive(Debug, Default, Clone, Copy)]
pub struct Policy;

impl Policy {
    pub fn check(
        &self,
        _user: &CurrentUser,
        _volume: &str,
        _action: Action,
    ) -> Result<(), ApiError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_allows_every_action() {
        let user = CurrentUser {
            sub: "sub".to_owned(),
            name: "User".to_owned(),
            email: None,
            groups: vec!["explorer-users".to_owned()],
        };
        for action in [Action::Read, Action::Write] {
            assert!(Policy.check(&user, "media", action).is_ok());
        }
    }
}
