use serde::{Deserialize, Serialize};

/// What an upload does when its destination already exists.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    #[default]
    Fail,
    Replace,
    KeepBoth,
}

impl ConflictPolicy {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Replace => "replace",
            Self::KeepBoth => "keep_both",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fail" => Some(Self::Fail),
            "replace" => Some(Self::Replace),
            "keep_both" => Some(Self::KeepBoth),
            _ => None,
        }
    }
}

/// An upload of the current user that is not finished yet.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PendingUpload {
    pub id: String,
    pub volume: String,
    pub path: String,
    pub length: u64,
    pub offset: u64,
    pub modified_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_policy_round_trips() {
        for policy in [
            ConflictPolicy::Fail,
            ConflictPolicy::Replace,
            ConflictPolicy::KeepBoth,
        ] {
            assert_eq!(ConflictPolicy::parse(policy.as_str()), Some(policy));
            assert_eq!(
                serde_json::to_string(&policy).unwrap(),
                format!("\"{}\"", policy.as_str())
            );
        }
        assert_eq!(ConflictPolicy::default(), ConflictPolicy::Fail);
        assert_eq!(ConflictPolicy::parse("Fail"), None);
    }
}
