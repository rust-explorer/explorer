use serde::{Deserialize, Serialize};

use super::conflict::ConflictPolicy;
use super::error::ErrorBody;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub volume: String,
    pub path: String,
}

/// What a background task does; `dest_dir` is the folder the source goes into.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TaskSpec {
    Delete {
        source: Location,
    },
    Copy {
        source: Location,
        dest_dir: Location,
        #[serde(default)]
        conflict: ConflictPolicy,
    },
    Move {
        source: Location,
        dest_dir: Location,
        #[serde(default)]
        conflict: ConflictPolicy,
    },
}

impl TaskSpec {
    #[must_use]
    pub fn source(&self) -> &Location {
        match self {
            Self::Delete { source } | Self::Copy { source, .. } | Self::Move { source, .. } => {
                source
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl TaskState {
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TaskInfo {
    pub id: String,
    pub spec: TaskSpec,
    pub state: TaskState,
    pub files_done: u64,
    pub files_total: Option<u64>,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub skipped: u64,
    /// Copy and move: where the entry ended up.
    pub final_path: Option<String>,
    /// Set when the task failed.
    pub error: Option<ErrorBody>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn location(volume: &str, path: &str) -> Location {
        Location {
            volume: volume.to_owned(),
            path: path.to_owned(),
        }
    }

    #[test]
    fn specs_are_tagged_by_op() {
        let delete = TaskSpec::Delete {
            source: location("media", "Films"),
        };
        assert_eq!(
            serde_json::to_value(&delete).unwrap(),
            json!({"op": "delete", "source": {"volume": "media", "path": "Films"}})
        );
        let copy: TaskSpec = serde_json::from_value(json!({
            "op": "copy",
            "source": {"volume": "media", "path": "a.txt"},
            "dest_dir": {"volume": "documents", "path": ""},
        }))
        .unwrap();
        assert_eq!(
            copy,
            TaskSpec::Copy {
                source: location("media", "a.txt"),
                dest_dir: location("documents", ""),
                conflict: ConflictPolicy::Fail,
            }
        );
        assert_eq!(copy.source(), &location("media", "a.txt"));
        let moved: TaskSpec = serde_json::from_value(json!({
            "op": "move",
            "source": {"volume": "media", "path": "a.txt"},
            "dest_dir": {"volume": "media", "path": "b"},
            "conflict": "keep_both",
        }))
        .unwrap();
        assert!(matches!(
            moved,
            TaskSpec::Move {
                conflict: ConflictPolicy::KeepBoth,
                ..
            }
        ));
    }

    #[test]
    fn states_are_snake_case() {
        assert_eq!(
            serde_json::to_value(TaskState::Cancelled).unwrap(),
            json!("cancelled")
        );
        assert!(TaskState::Queued.is_active() && TaskState::Running.is_active());
        for state in [TaskState::Done, TaskState::Failed, TaskState::Cancelled] {
            assert!(!state.is_active(), "{state:?}");
        }
    }
}
