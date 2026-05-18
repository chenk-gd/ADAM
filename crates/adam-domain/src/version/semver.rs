//! Semantic versioning support

use serde::{Deserialize, Serialize};
use std::fmt;

/// Semantic version
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl SemVer {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            prerelease: None,
        }
    }

    pub fn parse(version: &str) -> Result<Self, String> {
        let version = version.trim_start_matches('v');
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid semver format".to_string());
        }
        Ok(Self::new(
            parts[0].parse::<u64>().map_err(|e| e.to_string())?,
            parts[1].parse::<u64>().map_err(|e| e.to_string())?,
            parts[2].parse::<u64>().map_err(|e| e.to_string())?,
        ))
    }

    pub fn is_compatible_with(&self, other: &SemVer) -> bool {
        self.major == other.major
    }

    pub fn next_major(&self) -> Self {
        Self::new(self.major + 1, 0, 0)
    }

    pub fn next_minor(&self) -> Self {
        Self::new(self.major, self.minor + 1, 0)
    }

    pub fn next_patch(&self) -> Self {
        Self::new(self.major, self.minor, self.patch + 1)
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_parse_and_display() {
        let v = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn test_semver_compatibility() {
        let v1 = SemVer::new(1, 0, 0);
        let v2 = SemVer::new(1, 5, 0);
        let v3 = SemVer::new(2, 0, 0);
        assert!(v1.is_compatible_with(&v2));
        assert!(!v1.is_compatible_with(&v3));
    }

    #[test]
    fn test_semver_parse_with_v_prefix() {
        let v = SemVer::parse("v1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_semver_next_versions() {
        let v = SemVer::new(1, 2, 3);
        assert_eq!(v.next_major(), SemVer::new(2, 0, 0));
        assert_eq!(v.next_minor(), SemVer::new(1, 3, 0));
        assert_eq!(v.next_patch(), SemVer::new(1, 2, 4));
    }

    #[test]
    fn test_semver_comparison() {
        let v1 = SemVer::new(1, 0, 0);
        let v2 = SemVer::new(1, 2, 0);
        let v3 = SemVer::new(2, 0, 0);
        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v1 < v3);
    }

    #[test]
    fn test_semver_parse_invalid() {
        assert!(SemVer::parse("1.2").is_err());
        assert!(SemVer::parse("1").is_err());
        assert!(SemVer::parse("abc").is_err());
    }
}
