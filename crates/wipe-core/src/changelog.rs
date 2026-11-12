//! wipe's own release notes, built into the binary: `wipe changelog` and the
//! board UI's "What's new" read them from here.
//!
//! The text is `crates/wipe-core/CHANGELOG.md`, a copy of the repo's root
//! `CHANGELOG.md` kept inside the crate so it ships with the published crate
//! too (`scripts/set-version.mjs` refreshes it; a test fails when it drifts).

use serde::Serialize;

/// The full changelog, Markdown.
pub const TEXT: &str = include_str!("../CHANGELOG.md");

/// One version's section of the changelog.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Release {
    /// `1.1.2`
    pub version: String,
    /// `2026-11-12`, or `None` while unreleased.
    pub date: Option<String>,
    /// The section's Markdown, without its `## ` heading.
    pub body: String,
}

/// Every version in the changelog, newest first.
pub fn releases() -> Vec<Release> {
    parse(TEXT)
}

/// Split changelog Markdown into its `## <version> - <date>` sections.
pub fn parse(text: &str) -> Vec<Release> {
    let mut out: Vec<Release> = Vec::new();
    let mut fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence {
            if let Some(head) = line.strip_prefix("## ") {
                let (version, date) = match head.split_once(" - ") {
                    Some((v, d)) => (v.trim(), Some(d.trim())),
                    None => (head.trim(), None),
                };
                let date = date
                    .filter(|d| d.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    .map(str::to_string);
                out.push(Release {
                    version: version.trim_start_matches('v').to_string(),
                    date,
                    body: String::new(),
                });
                continue;
            }
        }
        if let Some(r) = out.last_mut() {
            r.body.push_str(line);
            r.body.push('\n');
        }
    }
    for r in &mut out {
        r.body = r.body.trim().to_string();
    }
    out
}

/// The section for `version` (a leading `v` is ignored).
pub fn release(version: &str) -> Option<Release> {
    let v = version.trim().trim_start_matches('v');
    releases().into_iter().find(|r| r.version == v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_newest_first() {
        let rs = parse(
            "# Changelog\n\n## 1.1.0 - unreleased\n\n- **New.** thing\n\n## 1.0.0 - 2026-11-01\n\n### Sub\n\n- a\n```\n## not a heading\n```\n",
        );
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].version, "1.1.0");
        assert_eq!(rs[0].date, None);
        assert_eq!(rs[0].body, "- **New.** thing");
        assert_eq!(rs[1].date.as_deref(), Some("2026-11-01"));
        assert!(rs[1].body.contains("## not a heading"));
    }

    #[test]
    fn embedded_changelog_covers_this_version() {
        let rs = releases();
        assert!(
            rs.iter().any(|r| r.version == env!("CARGO_PKG_VERSION")),
            "CHANGELOG.md has no section for {}",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(rs.last().map(|r| r.version.as_str()), Some("0.1.0"));
    }

    /// Every crate's LICENSE copy must match the repo's root LICENSE.
    #[test]
    fn crate_licenses_match_root() {
        let base = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let Ok(root) = std::fs::read_to_string(format!("{base}/LICENSE")) else {
            return;
        };
        for c in ["wipe-cli", "wipe-core", "wipe-daemon"] {
            let copy =
                std::fs::read_to_string(format!("{base}/crates/{c}/LICENSE")).unwrap_or_default();
            assert!(
                copy.replace("\r\n", "\n") == root.replace("\r\n", "\n"),
                "crates/{c}/LICENSE is out of date - copy the root LICENSE over it"
            );
        }
    }

    /// The crate's copy must match the repo's root CHANGELOG.md
    /// (`node scripts/set-version.mjs` or a plain copy refreshes it).
    #[test]
    fn embedded_copy_matches_root() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md");
        if let Ok(text) = std::fs::read_to_string(root) {
            assert!(
                text.replace("\r\n", "\n") == TEXT.replace("\r\n", "\n"),
                "crates/wipe-core/CHANGELOG.md is out of date - copy the root CHANGELOG.md over it"
            );
        }
    }
}
