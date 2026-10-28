use shared::api::conflict::ConflictPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Cancel,
    Replace,
    KeepBoth,
}

impl Decision {
    pub fn policy(self) -> Option<ConflictPolicy> {
        match self {
            Self::Cancel => None,
            Self::Replace => Some(ConflictPolicy::Replace),
            Self::KeepBoth => Some(ConflictPolicy::KeepBoth),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_map_to_policies() {
        assert_eq!(Decision::Cancel.policy(), None);
        assert_eq!(Decision::Replace.policy(), Some(ConflictPolicy::Replace));
        assert_eq!(Decision::KeepBoth.policy(), Some(ConflictPolicy::KeepBoth));
    }
}
