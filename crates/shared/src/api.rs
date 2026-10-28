use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub request_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Health {
    pub status: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Me {
    pub sub: String,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LogoutResponse {
    pub redirect_to: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Dir,
    File,
    Other,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    /// Milliseconds since the Unix epoch; negative before it.
    pub modified_ms: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Listing {
    pub volume: String,
    pub path: String,
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Space {
    pub total: u64,
    pub available: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VolumeInfo {
    pub name: String,
    pub read_only: bool,
    pub space: Option<Space>,
}

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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CreateFolder {
    pub path: String,
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
