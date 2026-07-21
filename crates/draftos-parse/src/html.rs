//! Minimal HTML parsing: strip script/style, break on block-level tags,
//! detect h1–h6 as headings, strip remaining tags, decode common entities.
//! Good enough for saved web pages and exported agreements; a full DOM parser
//! can replace this later without changing the output type.

use draftos_core::error::Result;
use draftos_core::{Paragraph, ParsedDocument};
use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;

pub fn parse(path: &Path, file_name: String) -> Result<ParsedDocument> {
    let raw = std::fs::read(path)?;
    let html = String::from_utf8_lossy(&raw);
    Ok(ParsedDocument {
        file_name,
        paragraphs: paragraphs_from_html(&html),
    })
}

fn paragraphs_from_html(html: &str) -> Vec<Paragraph> {
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    static BLOCK: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    static HEADING: OnceLock<Regex> = OnceLock::new();

    let script = SCRIPT.get_or_init(|| {
        Regex::new(r"(?is)<(script|style|head)[^>]*>.*?</(script|style|head)>").unwrap()
    });
    let heading =
        HEADING.get_or_init(|| Regex::new(r"(?is)<h([1-6])[^>]*>(.*?)</h[1-6]>").unwrap());
    let block = BLOCK.get_or_init(|| {
        Regex::new(r"(?i)</?(p|div|br|li|ul|ol|tr|table|section|article)[^>]*>").unwrap()
    });
    let tag = TAG.get_or_init(|| Regex::new(r"(?s)<[^>]+>").unwrap());

    let cleaned = script.replace_all(html, " ");

    // Pull out headings first, replacing them with sentinel markers so we can
    // reconstruct order after block splitting.
    let mut headings: Vec<(u8, String)> = Vec::new();
    let cleaned = heading.replace_all(&cleaned, |caps: &regex::Captures| {
        let level: u8 = caps[1].parse().unwrap_or(1);
        let text = decode_entities(&tag.replace_all(&caps[2], " "));
        headings.push((level, text));
        format!("\n@@DRAFTOS_HEADING_{}@@\n", headings.len() - 1)
    });

    let with_breaks = block.replace_all(&cleaned, "\n");
    let stripped = tag.replace_all(&with_breaks, " ");

    let mut paragraphs = Vec::new();
    for line in stripped.lines() {
        let text = decode_entities(line);
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        if let Some(idx) = text
            .strip_prefix("@@DRAFTOS_HEADING_")
            .and_then(|r| r.strip_suffix("@@"))
            .and_then(|n| n.parse::<usize>().ok())
        {
            if let Some((level, htext)) = headings.get(idx) {
                let htext = htext.split_whitespace().collect::<Vec<_>>().join(" ");
                if !htext.is_empty() {
                    paragraphs.push(Paragraph {
                        text: htext,
                        heading_level: Some(*level),
                        ooxml: None,
                    });
                }
            }
        } else {
            paragraphs.push(Paragraph {
                text,
                heading_level: None,
                ooxml: None,
            });
        }
    }
    paragraphs
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}
