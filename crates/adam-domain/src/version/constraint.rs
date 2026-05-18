//! Version constraint expressions

use serde::{Deserialize, Serialize};
use super::semver::SemVer;

/// Version constraint expression
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionConstraint {
    Exact(SemVer),           // =1.0.0
    Caret(SemVer),          // ^1.0.0 -> >=1.0.0, <2.0.0
    Tilde(SemVer),          // ~1.0.0 -> >=1.0.0, <1.1.0
    Range { min: Bound, max: Bound },
    Wildcard,               // *
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bound {
    Inclusive(SemVer),
    Exclusive(SemVer),
}

impl VersionConstraint {
    /// Parse constraint from string
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();

        if s == "*" {
            return Ok(Self::Wildcard);
        }

        if let Some(version_str) = s.strip_prefix('^') {
            let version = SemVer::parse(version_str)?;
            return Ok(Self::Caret(version));
        }

        if let Some(version_str) = s.strip_prefix('~') {
            let version = SemVer::parse(version_str)?;
            return Ok(Self::Tilde(version));
        }

        if let Some(version_str) = s.strip_prefix('=') {
            let version = SemVer::parse(version_str)?;
            return Ok(Self::Exact(version));
        }

        // Try parsing as exact version
        let version = SemVer::parse(s)?;
        Ok(Self::Exact(version))
    }

    /// Check if version satisfies constraint
    pub fn matches(&self, version: &SemVer) -> bool {
        match self {
            Self::Exact(v) => version == v,
            Self::Caret(v) => {
                version >= v && version.major == v.major
            }
            Self::Tilde(v) => {
                version >= v && version.major == v.major && version.minor == v.minor
            }
            Self::Range { min, max } => {
                let min_satisfied = match min {
                    Bound::Inclusive(v) => version >= v,
                    Bound::Exclusive(v) => version > v,
                };
                let max_satisfied = match max {
                    Bound::Inclusive(v) => version <= v,
                    Bound::Exclusive(v) => version < v,
                };
                min_satisfied && max_satisfied
            }
            Self::Wildcard => true,
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            Self::Exact(v) => format!("={}", v),
            Self::Caret(v) => format!("^{}", v),
            Self::Tilde(v) => format!("~{}", v),
            Self::Range { min, max } => {
                let min_str = match min {
                    Bound::Inclusive(v) => format!(">={}", v),
                    Bound::Exclusive(v) => format!(">{}", v),
                };
                let max_str = match max {
                    Bound::Inclusive(v) => format!(">={}", v),
                    Bound::Exclusive(v) => format!(">{}", v),
                };
                format!("{}, {}", min_str, max_str)
            }
            Self::Wildcard => "*".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_caret_constraint() {
        let c = VersionConstraint::parse("^1.0.0").unwrap();
        assert!(c.matches(&SemVer::new(1, 0, 0)));
        assert!(c.matches(&SemVer::new(1, 5, 0)));
        assert!(c.matches(&SemVer::new(1, 0, 5)));
        assert!(!c.matches(&SemVer::new(2, 0, 0)));
        assert!(!c.matches(&SemVer::new(0, 9, 0)));
    }

    #[test]
    fn test_tilde_constraint() {
        let c = VersionConstraint::parse("~1.0.0").unwrap();
        assert!(c.matches(&SemVer::new(1, 0, 0)));
        assert!(c.matches(&SemVer::new(1, 0, 5)));
        assert!(!c.matches(&SemVer::new(1, 1, 0)));
        assert!(!c.matches(&SemVer::new(2, 0, 0)));
    }

    #[test]
    fn test_exact_constraint() {
        let c = VersionConstraint::parse("=1.0.0").unwrap();
        assert!(c.matches(&SemVer::new(1, 0, 0)));
        assert!(!c.matches(&SemVer::new(1, 0, 1)));
        assert!(!c.matches(&SemVer::new(2, 0, 0)));
    }

    #[test]
    fn test_wildcard_constraint() {
        let c = VersionConstraint::parse("*").unwrap();
        assert!(c.matches(&SemVer::new(1, 0, 0)));
        assert!(c.matches(&SemVer::new(2, 5, 3)));
        assert!(c.matches(&SemVer::new(0, 0, 1)));
    }

    #[test]
    fn test_range_constraint() {
        let c = VersionConstraint::Range {
            min: Bound::Inclusive(SemVer::new(1, 0, 0)),
            max: Bound::Exclusive(SemVer::new(2, 0, 0)),
        };
        assert!(c.matches(&SemVer::new(1, 0, 0)));
        assert!(c.matches(&SemVer::new(1, 5, 0)));
        assert!(!c.matches(&SemVer::new(2, 0, 0)));
        assert!(!c.matches(&SemVer::new(0, 9, 0)));
    }

    #[test]
    fn test_constraint_display() {
        assert_eq!(VersionConstraint::parse("^1.0.0").unwrap().to_string(), "^1.0.0");
        assert_eq!(VersionConstraint::parse("~1.0.0").unwrap().to_string(), "~1.0.0");
        assert_eq!(VersionConstraint::parse("=1.0.0").unwrap().to_string(), "=1.0.0");
        assert_eq!(VersionConstraint::parse("*").unwrap().to_string(), "*");
    }

    #[test]
    fn test_constraint_parse_implicit_exact() {
        let c = VersionConstraint::parse("1.0.0").unwrap();
        assert!(matches!(c, VersionConstraint::Exact(_)));
        assert!(c.matches(&SemVer::new(1, 0, 0)));
        assert!(!c.matches(&SemVer::new(1, 0, 1)));
    }
}
