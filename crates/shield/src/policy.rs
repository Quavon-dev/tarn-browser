#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum ShieldLevel {
    #[default]
    Standard,
    Strict,
    Lockdown,
}

impl ShieldLevel {
    pub const ALL: [ShieldLevel; 3] = [ShieldLevel::Standard, ShieldLevel::Strict, ShieldLevel::Lockdown];

    pub fn as_str(self) -> &'static str {
        match self {
            ShieldLevel::Standard => "standard",
            ShieldLevel::Strict => "strict",
            ShieldLevel::Lockdown => "lockdown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == s)
    }

    /// A per-site override replaces the profile level, in either direction.
    pub fn effective(profile: ShieldLevel, site_override: Option<ShieldLevel>) -> ShieldLevel {
        site_override.unwrap_or(profile)
    }

    pub fn policy(self) -> Policy {
        let strict = self >= ShieldLevel::Strict;
        let lockdown = self == ShieldLevel::Lockdown;
        Policy {
            strip_tracking_params: true,
            block_private_network: true,
            https_only: strict,
            trim_referrer: strict,
            javascript: !lockdown,
            save_interaction_state: !lockdown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub strip_tracking_params: bool,
    pub block_private_network: bool,
    pub https_only: bool,
    /// No referrer on cross-origin requests; WebKit's default already trims them to the origin.
    pub trim_referrer: bool,
    pub javascript: bool,
    pub save_interaction_state: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_cumulative() {
        let [s, st, l] = ShieldLevel::ALL.map(ShieldLevel::policy);
        assert!(s.strip_tracking_params && s.block_private_network);
        assert!(!s.https_only && !s.trim_referrer && s.javascript && s.save_interaction_state);
        assert!(st.https_only && st.trim_referrer && st.javascript);
        assert!(l.https_only && !l.javascript && !l.save_interaction_state);
    }

    #[test]
    fn override_wins_and_names_roundtrip() {
        assert_eq!(ShieldLevel::effective(ShieldLevel::Lockdown, Some(ShieldLevel::Standard)), ShieldLevel::Standard);
        assert_eq!(ShieldLevel::effective(ShieldLevel::Strict, None), ShieldLevel::Strict);
        for l in ShieldLevel::ALL {
            assert_eq!(ShieldLevel::parse(l.as_str()), Some(l));
        }
        assert_eq!(ShieldLevel::parse("off"), None);
    }
}
