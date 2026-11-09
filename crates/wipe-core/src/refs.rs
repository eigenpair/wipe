//! `@` references in text: one syntax for everything a board holds.
//!
//! - `@T-02A` - a ticket
//! - `@F-01C` - a forum post (`@F-3.1` on boards with the old thread forum)
//! - `@claude`, `@ada@example.com` - an identity (a mention)
//!
//! A tag starts at the beginning of the text or after a character that can't be
//! part of a word or an address (so `ada@example.com` alone is not a tag). The
//! post tags a forum post *opens* with - before any other text - make it a reply
//! to those posts ([`reply_parents`]); every other tag is a plain reference.

/// One `@` tag found in a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tag {
    /// `@T-02A`
    Ticket(String),
    /// `@F-01C` (or a legacy dotted `@F-3.1`)
    Post(String),
    /// `@claude` - an identity id, as written.
    Identity(String),
}

impl Tag {
    /// The referenced id without the `@`.
    pub fn id(&self) -> &str {
        match self {
            Tag::Ticket(s) | Tag::Post(s) | Tag::Identity(s) => s,
        }
    }
}

fn word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-')
}

/// Parse one tag at `s` (just after the `@`): the tag and its byte length.
fn tag_at(s: &str) -> Option<(Tag, usize)> {
    let b = s.as_bytes();
    // Ticket / post ids: an upper-case `T-` / `F-` and hex digits (posts on old
    // boards: decimal with `.n` reply segments).
    if b.len() > 2 && matches!(b[0], b'T' | b'F') && b[1] == b'-' {
        let mut j = 2;
        while j < b.len() && matches!(b[j], b'0'..=b'9' | b'A'..=b'F') {
            j += 1;
        }
        if j > 2 && (j == b.len() || !b[j].is_ascii_alphanumeric() && b[j] != b'_') {
            if b[0] == b'F' {
                // `F-3.1.2`: dotted reply ids from the thread forum.
                while j + 1 < b.len() && b[j] == b'.' && b[j + 1].is_ascii_digit() {
                    j += 1;
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                return Some((Tag::Post(s[..j].to_string()), j));
            }
            return Some((Tag::Ticket(s[..j].to_string()), j));
        }
    }
    // Identities: a slug or an email address.
    if !b.first().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let mut j = 0;
    while j < b.len() && word(b[j]) {
        j += 1;
    }
    // An email: `ada@example.com`.
    if j < b.len() && b[j] == b'@' && b.get(j + 1).is_some_and(|c| c.is_ascii_alphanumeric()) {
        let mut k = j + 1;
        while k < b.len() && word(b[k]) {
            k += 1;
        }
        if s[j + 1..k].contains('.') {
            j = k;
        }
    }
    // Trailing punctuation ends a sentence, not the name: `@claude.`
    while j > 0 && matches!(b[j - 1], b'.' | b'-' | b'+') {
        j -= 1;
    }
    (j > 0).then(|| (Tag::Identity(s[..j].to_string()), j))
}

/// Every tag in `text`, with its byte range (including the `@`), in order.
/// Tags inside inline code or fenced code blocks are skipped.
pub fn scan(text: &str) -> Vec<(std::ops::Range<usize>, Tag)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut fenced = false;
    let mut line_start = true;
    while i < b.len() {
        if line_start {
            let rest = text[i..].trim_start_matches([' ', '\t']);
            if rest.starts_with("```") || rest.starts_with("~~~") {
                fenced = !fenced;
            }
        }
        line_start = b[i] == b'\n';
        if fenced {
            i += 1;
            continue;
        }
        if b[i] == b'`' {
            // Inline code: skip to the matching run of backticks on this text.
            let mut n = 0;
            while i + n < b.len() && b[i + n] == b'`' {
                n += 1;
            }
            let fence = &text[i..i + n];
            match text[i + n..].find(fence) {
                Some(end) => i += n + end + n,
                None => i += n,
            }
            continue;
        }
        if b[i] == b'@' && (i == 0 || !(word(b[i - 1]) || b[i - 1] == b'@' || b[i - 1] == b'/')) {
            if let Some((tag, len)) = tag_at(&text[i + 1..]) {
                out.push((i..i + 1 + len, tag));
                i += 1 + len;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Every tag in `text`, in order (see [`scan`]).
pub fn tags(text: &str) -> Vec<Tag> {
    scan(text).into_iter().map(|(_, t)| t).collect()
}

/// The identities `text` mentions (`@claude`), deduplicated, in order.
pub fn mentions(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags(text) {
        if let Tag::Identity(id) = t {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// The posts a forum post replies to: the post tags its body opens with, before
/// any other text (`@F-01A @F-01B thanks` -> `F-01A`, `F-01B`). Ticket and
/// identity tags may be mixed into that opening run; they don't end it.
pub fn reply_parents(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body.trim_start();
    while let Some(after) = rest.strip_prefix('@') {
        let Some((tag, len)) = tag_at(after) else {
            break;
        };
        let tail = &after[len..];
        // A tag must stand alone: followed by whitespace, a separator, or the end.
        if !(tail.is_empty() || tail.starts_with([' ', '\t', '\n', '\r', ',', ':', ';'])) {
            break;
        }
        if let Tag::Post(id) = tag {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        rest = tail.trim_start_matches([',', ':', ';']).trim_start();
    }
    out
}

/// `body` opening with a tag for each of `parents` (those it doesn't open with
/// already go in front, in order), so the post replies to them.
pub fn with_reply_prefix(body: &str, parents: &[String]) -> String {
    let have = reply_parents(body);
    let missing: Vec<String> = parents
        .iter()
        .filter(|p| !have.contains(p))
        .map(|p| format!("@{p}"))
        .collect();
    if missing.is_empty() {
        return body.to_string();
    }
    let body = body.trim_start();
    if body.is_empty() {
        missing.join(" ")
    } else {
        format!("{} {body}", missing.join(" "))
    }
}

/// Replace the ticket/post ids `map` names in `@` tags and in whole-word plain
/// mentions (`T-23`, `see F-3.1`). Used when ids are translated.
pub fn rewrite(text: &str, map: &std::collections::BTreeMap<String, String>) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let boundary = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if boundary && i + 2 < b.len() && matches!(b[i], b'T' | b'F') && b[i + 1] == b'-' {
            if let Some((tag, len)) = tag_at(&text[i..]) {
                if let Some(new) = map.get(tag.id()) {
                    out.push_str(new);
                    i += len;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// A one-sentence summary derived from a body, for posts that have none (posts
/// written before summaries existed): the first sentence of the first line of
/// prose, without the opening reply tags, at most ~120 characters.
pub fn derive_summary(body: &str) -> String {
    let mut text = body.trim_start();
    // Drop the opening tags.
    while let Some(after) = text.strip_prefix('@') {
        match tag_at(after) {
            Some((_, len)) => {
                text = after[len..]
                    .trim_start_matches([',', ':', ';'])
                    .trim_start()
            }
            None => break,
        }
    }
    let line = text
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['#', '>', '-', '*', ' '])
                .trim()
        })
        .find(|l| !l.is_empty() && !l.starts_with("```"))
        .unwrap_or("");
    let mut end = line.len();
    for (i, c) in line.char_indices() {
        if matches!(c, '.' | '!' | '?') && line[i + c.len_utf8()..].starts_with(' ') && i > 12 {
            end = i + c.len_utf8();
            break;
        }
    }
    let s = &line[..end];
    if s.chars().count() > 120 {
        let cut: String = s.chars().take(117).collect();
        format!("{}…", cut.trim_end())
    } else if s.is_empty() {
        "(no summary)".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_tags_and_skips_emails_and_code() {
        let t = tags("see @T-02A and @F-01C, ask @claude. mail ada@x.com or @ada@x.com `@T-001`\n```\n@F-002\n```\n@F-3.1!");
        assert_eq!(
            t,
            vec![
                Tag::Ticket("T-02A".into()),
                Tag::Post("F-01C".into()),
                Tag::Identity("claude".into()),
                Tag::Identity("ada@x.com".into()),
                Tag::Post("F-3.1".into()),
            ]
        );
        assert_eq!(mentions("@a @b @a"), vec!["a", "b"]);
        // `@T-` followed by letters beyond hex is a name, not a ticket.
        assert_eq!(tags("@T-xyz"), vec![Tag::Identity("T-xyz".into())]);
    }

    #[test]
    fn replies_are_the_opening_post_tags() {
        assert_eq!(
            reply_parents("@F-01A @F-01B thanks"),
            vec!["F-01A", "F-01B"]
        );
        assert_eq!(
            reply_parents("  @F-01A, @claude: yes @F-002"),
            vec!["F-01A"]
        );
        assert!(reply_parents("thanks @F-01A").is_empty());
        assert!(reply_parents("@F-01Ax").is_empty());
        assert_eq!(with_reply_prefix("hi", &["F-001".into()]), "@F-001 hi");
        assert_eq!(
            with_reply_prefix("@F-001 hi", &["F-001".into()]),
            "@F-001 hi"
        );
    }

    #[test]
    fn rewrites_ids_and_derives_summaries() {
        let map = [
            ("F-3.1".to_string(), "F-00A".to_string()),
            ("T-23".to_string(), "T-017".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            rewrite("@F-3.1 per T-23 (not T-230, F-3)", &map),
            "@F-00A per T-017 (not T-230, F-3)"
        );
        assert_eq!(
            derive_summary("@F-001 Agreed, ship it today. More later."),
            "Agreed, ship it today."
        );
        assert_eq!(derive_summary(""), "(no summary)");
    }
}
