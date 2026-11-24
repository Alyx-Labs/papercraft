//! Semantic Version 2.0.0 parsing and precedence comparison for release updates.

use std::cmp::Ordering;
use std::fmt;

use crate::UpdateError;

const MAX_VERSION_LEN: usize = 128;

/// Parsed Semantic Version (`major.minor.patch[-prerelease]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl SemVer {
    /// Parses a SemVer string such as `"0.1.2"`, `"v0.1.2"`, or `"0.2.0-rc.1"`.
    pub fn parse(raw: &str) -> Result<Self, UpdateError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_VERSION_LEN {
            return Err(UpdateError::InvalidVersion(
                "version string is empty or exceeds maximum length".to_string(),
            ));
        }

        let no_prefix = trimmed
            .strip_prefix('v')
            .or_else(|| trimmed.strip_prefix('V'))
            .unwrap_or(trimmed);
        let core_and_pre = no_prefix
            .split_once('+')
            .map_or(no_prefix, |(before, _)| before);

        let (core, prerelease) = match core_and_pre.split_once('-') {
            Some((c, pre)) => {
                if !is_valid_prerelease(pre) {
                    return Err(UpdateError::InvalidVersion(format!(
                        "invalid prerelease identifier in `{raw}`"
                    )));
                }
                (c, Some(pre.to_string()))
            }
            None => (core_and_pre, None),
        };

        let mut parts = core.split('.');
        let major = parse_numeric_part(parts.next(), raw)?;
        let minor = parse_numeric_part(parts.next(), raw)?;
        let patch = parse_numeric_part(parts.next(), raw)?;
        if parts.next().is_some() {
            return Err(UpdateError::InvalidVersion(format!(
                "version `{raw}` has too many numeric components"
            )));
        }

        Ok(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }

    /// Returns `true` when this version carries a `-prerelease` suffix.
    #[must_use]
    pub fn is_prerelease(&self) -> bool {
        self.prerelease.is_some()
    }
}

fn parse_numeric_part(part: Option<&str>, full: &str) -> Result<u64, UpdateError> {
    let raw = part.ok_or_else(|| {
        UpdateError::InvalidVersion(format!("missing version component in `{full}`"))
    })?;
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(UpdateError::InvalidVersion(format!(
            "invalid numeric component `{raw}` in `{full}`"
        )));
    }
    if raw.len() > 1 && raw.starts_with('0') {
        return Err(UpdateError::InvalidVersion(format!(
            "leading zero in version component `{raw}` in `{full}`"
        )));
    }
    raw.parse::<u64>()
        .map_err(|_| UpdateError::InvalidVersion(format!("version component overflow in `{full}`")))
}

fn is_valid_prerelease(pre: &str) -> bool {
    if pre.is_empty() {
        return false;
    }
    pre.split('.')
        .all(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

fn compare_prerelease_segment(a: &str, b: &str) -> Ordering {
    let a_num = a.bytes().all(|ch| ch.is_ascii_digit());
    let b_num = b.bytes().all(|ch| ch.is_ascii_digit());
    match (a_num, b_num) {
        (true, true) => {
            let a_trim = a.trim_start_matches('0');
            let b_trim = b.trim_start_matches('0');
            a_trim
                .len()
                .cmp(&b_trim.len())
                .then_with(|| a_trim.cmp(b_trim))
        }
        // SemVer 2.0.0: numeric identifiers always have lower precedence than non-numeric.
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.cmp(b),
    }
}

fn compare_prereleases(a: &str, b: &str) -> Ordering {
    let mut a_segs = a.split('.');
    let mut b_segs = b.split('.');
    loop {
        match (a_segs.next(), b_segs.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(sa), Some(sb)) => {
                let ord = compare_prerelease_segment(sa, sb);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                // A normal version has higher precedence than a pre-release version.
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => compare_prereleases(a, b),
            })
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.prerelease {
            Some(pre) => write!(f, "{}.{}.{}-{pre}", self.major, self.minor, self.patch),
            None => write!(f, "{}.{}.{}", self.major, self.minor, self.patch),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_versions_and_rejects_malformed_inputs() {
        let v = SemVer::parse("v0.1.2").unwrap();
        assert_eq!(v.to_string(), "0.1.2");
        assert!(!v.is_prerelease());

        let rc = SemVer::parse("V1.2.3-rc.1+build.42").unwrap();
        assert_eq!(rc.to_string(), "1.2.3-rc.1");
        assert!(rc.is_prerelease());

        for bad in [
            "",
            "   ",
            "0.1",
            "0.1.2.3",
            "01.1.2",
            "0.1.2-",
            "0.1.2-rc..1",
            "0.1.2-rc$1",
            "9999999999999999999999999999999.0.0",
        ] {
            assert!(SemVer::parse(bad).is_err(), "expected error for `{bad}`");
        }
        let oversized = format!("1.0.0-{}", "a".repeat(MAX_VERSION_LEN));
        assert!(SemVer::parse(&oversized).is_err());
    }

    #[test]
    fn semver_ordering_matches_spec() {
        let chain = [
            "0.1.1",
            "0.1.2-alpha",
            "0.1.2-alpha.1",
            "0.1.2-alpha.beta",
            "0.1.2-beta",
            "0.1.2-rc.1",
            "0.1.2-rc.2",
            "0.1.2-rc.10",
            "0.1.2",
            "0.2.0",
            "1.0.0",
        ];
        let parsed: Vec<SemVer> = chain.iter().map(|s| SemVer::parse(s).unwrap()).collect();
        for pair in parsed.windows(2) {
            assert!(pair[0] < pair[1], "expected {} < {}", pair[0], pair[1]);
        }
    }
}
