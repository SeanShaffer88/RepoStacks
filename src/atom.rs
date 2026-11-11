//! Minimal parser for GitHub's public Atom feeds (`releases.atom`, `tags.atom`).
//!
//! The feed carries the tag/title and the publish time for recent releases and
//! is served from github.com, not api.github.com, so polling it does not spend
//! REST API quota. It does not carry asset lists — those still cost one REST
//! call per actual new release (see `cache_policy`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseNote {
    pub tag: String,
    pub title: String,
    pub updated: String,
    pub link: String,
    /// The release notes as plain text (the feed's HTML content with
    /// tags stripped), empty when the release has no notes.
    pub body: String,
}

/// Parse a releases/tags Atom document. Entries come back in feed order
/// (newest first on GitHub). Malformed entries are skipped, never fatal:
/// a partial feed must not lose the releases that did parse.
pub fn parse_releases_atom(xml: &str) -> Vec<ReleaseNote> {
    let mut out = Vec::new();
    for entry in xml.split("<entry>").skip(1) {
        let entry = match entry.split("</entry>").next() {
            Some(e) => e,
            None => entry,
        };
        let title = match tag_text(entry, "title") {
            Some(t) if !t.trim().is_empty() => unescape(t.trim()),
            _ => continue,
        };
        let updated = tag_text(entry, "updated")
            .map(|u| u.trim().to_string())
            .unwrap_or_default();
        let link = atom_link(entry).unwrap_or_default();
        // In releases.atom the <title> is the release name; the tag lives in
        // the entry <id> (…/Release/<id>) only loosely, so the canonical tag
        // is taken from the release link's /tag/<tag> segment when present.
        let tag = link
            .split("/releases/tag/")
            .nth(1)
            .map(|t| t.trim_matches('/').to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| title.clone());
        out.push(ReleaseNote {
            tag,
            title,
            updated,
            link,
            body: content_text(entry),
        });
    }
    out
}

/// The entry's <content> (GitHub puts the release notes there as
/// escaped HTML) reduced to readable plain text.
fn content_text(entry: &str) -> String {
    let Some(start) = entry.find("<content") else {
        return String::new();
    };
    let Some(gt) = entry[start..].find('>') else {
        return String::new();
    };
    let body_start = start + gt + 1;
    let Some(end) = entry[body_start..].find("</content>") else {
        return String::new();
    };
    let raw = unescape(&entry[body_start..body_start + end]);
    // Strip tags but keep block structure: </p>, <br>, </li> become
    // newlines before the tags go away.
    let spaced = raw
        .replace("</p>", "\n\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</li>", "\n")
        .replace("</h2>", "\n\n")
        .replace("</h3>", "\n\n");
    let mut text = String::with_capacity(spaced.len());
    let mut in_tag = false;
    for ch in spaced.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    // The feed escapes the HTML once, and the HTML escapes its own
    // text again (& in a code sample arrives as &amp;amp;) — resolve
    // the second level now that the tags are gone.
    let text = unescape(&text);
    // Tidy: trim each line, drop runs of blank lines, cap the length.
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines().map(|l| l.trim()) {
        if line.is_empty() && (lines.is_empty() || lines.last() == Some(&"")) {
            continue;
        }
        lines.push(line);
    }
    let joined = lines.join("\n").trim().to_string();
    joined.chars().take(4000).collect()
}

fn tag_text(entry: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = entry.find(&open)? + open.len();
    let end = entry[start..].find(&close)? + start;
    Some(entry[start..end].to_string())
}

fn atom_link(entry: &str) -> Option<String> {
    // <link rel="alternate" type="text/html" href="https://github.com/…"/>
    for part in entry.split("<link").skip(1) {
        let tag_end = part.find('>')?;
        let tag = &part[..tag_end];
        if tag.contains("rel=\"alternate\"") || !tag.contains("rel=") {
            if let Some(h) = tag.split("href=\"").nth(1) {
                if let Some(url) = h.split('"').next() {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}

pub fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Release notes from Demo</title>
  <entry>
    <id>tag:github.com,2008:Repository/1/2026</id>
    <updated>2026-11-01T00:00:00Z</updated>
    <link rel="alternate" type="text/html" href="https://github.com/o/r/releases/tag/v1.2.0"/>
    <title>v1.2.0 &amp; fixes</title>
    <content type="html">&lt;p&gt;What is new&lt;/p&gt;&lt;ul&gt;&lt;li&gt;First thing&lt;/li&gt;&lt;li&gt;Second &amp;amp; final&lt;/li&gt;&lt;/ul&gt;</content>
  </entry>
  <entry>
    <updated>2026-10-01T00:00:00Z</updated>
    <link rel="alternate" type="text/html" href="https://github.com/o/r/releases/tag/v1.1.0"/>
    <title>v1.1.0</title>
  </entry>
</feed>"#;

    #[test]
    fn parses_entries_newest_first_with_tags() {
        let notes = parse_releases_atom(FEED);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].tag, "v1.2.0");
        assert_eq!(notes[0].title, "v1.2.0 & fixes");
        assert_eq!(notes[0].updated, "2026-11-01T00:00:00Z");
        assert_eq!(notes[1].tag, "v1.1.0");
        assert_eq!(notes[0].body, "What is new\n\nFirst thing\nSecond & final");
        assert_eq!(notes[1].body, "", "no content element, no notes");
    }

    #[test]
    fn empty_and_garbage_feeds_yield_nothing() {
        assert!(parse_releases_atom("").is_empty());
        assert!(parse_releases_atom("<feed></feed>").is_empty());
        assert!(parse_releases_atom("not xml at all").is_empty());
    }
}
