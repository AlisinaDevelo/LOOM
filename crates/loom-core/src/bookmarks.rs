//! Local-only parsing and canonicalization of Netscape bookmark HTML exports.
//!
//! The format is shared by Chrome and Firefox. Parsing deliberately stops at metadata: LOOM never
//! follows an imported URL or asks a browser to resolve it.

use std::collections::BTreeMap;

use crate::{
    domain::{BookmarkEntry, BookmarkExport},
    error::{LoomError, Result},
};

pub(crate) const BOOKMARK_EXTRACTOR_ID: &str = "loom.bookmark";
pub(crate) const BOOKMARK_EXTRACTOR_VERSION: &str = "0.1.0";

type BookmarkTag = (usize, usize, bool, String, BTreeMap<String, String>);

pub fn parse_bookmark_export(input: &str) -> Result<BookmarkExport> {
    if !input
        .to_ascii_lowercase()
        .contains("netscape-bookmark-file-1")
    {
        return Err(LoomError::UnsupportedSource(
            "bookmark export is not Netscape HTML format".into(),
        ));
    }
    let mut cursor = 0usize;
    let mut folders: Vec<String> = Vec::new();
    let mut pending_folder: Option<String> = None;
    let mut bookmarks = Vec::new();
    while let Some((start, end, closing, name, attributes)) = next_tag(input, cursor)? {
        cursor = end;
        if closing {
            if name == "dl" && !folders.is_empty() {
                folders.pop();
            }
            continue;
        }
        if name == "h3" {
            let Some((text, after)) = inner_element(input, cursor, "h3")? else {
                return Err(LoomError::InvalidPath(
                    "bookmark folder is not closed".into(),
                ));
            };
            pending_folder = Some(clean_text(&text));
            cursor = after;
            continue;
        }
        if name == "dl" {
            if let Some(folder) = pending_folder.take() {
                if !folder.is_empty() {
                    folders.push(folder);
                }
            }
            continue;
        }
        if name != "a" {
            continue;
        }
        if pending_folder.is_some() {
            if let Some(folder) = pending_folder.take() {
                if !folder.is_empty() {
                    folders.push(folder);
                }
            }
        }
        let Some(url) = attributes.get("href") else {
            return Err(LoomError::InvalidPath(format!(
                "bookmark anchor at byte {start} has no HREF"
            )));
        };
        let url = clean_text(url);
        validate_bookmark_url(&url)?;
        let Some((text, after)) = inner_element(input, cursor, "a")? else {
            return Err(LoomError::InvalidPath(
                "bookmark anchor is not closed".into(),
            ));
        };
        let title = clean_text(&text);
        if title.is_empty() {
            return Err(LoomError::InvalidPath(format!(
                "bookmark at byte {start} has an empty title"
            )));
        }
        bookmarks.push(BookmarkEntry {
            folder_path: folders.join(" / "),
            title,
            url,
            added_at: attributes.get("add_date").map(|value| clean_text(value)),
            modified_at: attributes
                .get("last_modified")
                .map(|value| clean_text(value)),
        });
        cursor = after;
    }
    if bookmarks.is_empty() {
        return Err(LoomError::InvalidPath(
            "bookmark export contains no usable bookmarks".into(),
        ));
    }
    Ok(BookmarkExport {
        format: "netscape_html".into(),
        bookmarks,
    })
}

fn next_tag(input: &str, mut cursor: usize) -> Result<Option<BookmarkTag>> {
    while let Some(relative) = input[cursor..].find('<') {
        let start = cursor + relative;
        if input[start..].starts_with("<!--") {
            let Some(end_relative) = input[start + 4..].find("-->") else {
                return Err(LoomError::InvalidPath(
                    "unterminated bookmark comment".into(),
                ));
            };
            cursor = start + 4 + end_relative + 3;
            continue;
        }
        let end = find_tag_end(input, start + 1)?;
        let mut body = input[start + 1..end].trim();
        let closing = body.starts_with('/');
        if closing {
            body = body[1..].trim_start();
        }
        let name_end = body
            .find(|value: char| value.is_ascii_whitespace() || value == '/')
            .unwrap_or(body.len());
        let name = body[..name_end].to_ascii_lowercase();
        if name.is_empty() || name.starts_with('!') || name.starts_with('?') {
            cursor = end + 1;
            continue;
        }
        let attributes = if closing {
            BTreeMap::new()
        } else {
            parse_attributes(&body[name_end..])?
        };
        return Ok(Some((start, end + 1, closing, name, attributes)));
    }
    Ok(None)
}

fn find_tag_end(input: &str, mut cursor: usize) -> Result<usize> {
    let mut quote = None;
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if let Some(expected) = quote {
            if byte == expected {
                quote = None;
            }
        } else if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'>' {
            return Ok(cursor);
        }
        cursor += 1;
    }
    Err(LoomError::InvalidPath("unterminated bookmark tag".into()))
}

fn parse_attributes(input: &str) -> Result<BTreeMap<String, String>> {
    let mut attributes = BTreeMap::new();
    let bytes = input.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            break;
        }
        let key_start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && bytes[cursor] != b'='
            && bytes[cursor] != b'/'
        {
            cursor += 1;
        }
        let key = input[key_start..cursor].to_ascii_lowercase();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            attributes.insert(key, String::new());
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let value = if cursor < bytes.len() && (bytes[cursor] == b'\'' || bytes[cursor] == b'"') {
            let quote = bytes[cursor];
            cursor += 1;
            let value_start = cursor;
            while cursor < bytes.len() && bytes[cursor] != quote {
                cursor += 1;
            }
            if cursor >= bytes.len() {
                return Err(LoomError::InvalidPath(
                    "unterminated bookmark attribute".into(),
                ));
            }
            let value = input[value_start..cursor].to_string();
            cursor += 1;
            value
        } else {
            let value_start = cursor;
            while cursor < bytes.len()
                && !bytes[cursor].is_ascii_whitespace()
                && bytes[cursor] != b'/'
            {
                cursor += 1;
            }
            input[value_start..cursor].to_string()
        };
        attributes.insert(key, value);
    }
    Ok(attributes)
}

fn inner_element(input: &str, cursor: usize, name: &str) -> Result<Option<(String, usize)>> {
    let close = format!("</{name}");
    let Some(relative) = input[cursor..].to_ascii_lowercase().find(&close) else {
        return Ok(None);
    };
    let close_start = cursor + relative;
    let close_end = find_tag_end(input, close_start + 2 + name.len())?;
    Ok(Some((
        strip_tags(&input[cursor..close_start]),
        close_end + 1,
    )))
}

fn strip_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut inside = false;
    for character in input.chars() {
        match character {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => output.push(character),
            _ => {}
        }
    }
    output
}

fn clean_text(input: &str) -> String {
    decode_entities(&strip_tags(input)).trim().to_string()
}

fn decode_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while let Some(relative) = input[cursor..].find('&') {
        let start = cursor + relative;
        output.push_str(&input[cursor..start]);
        let Some(end_relative) = input[start..].find(';') else {
            output.push('&');
            cursor = start + 1;
            continue;
        };
        let end = start + end_relative;
        let entity = &input[start + 1..end];
        let replacement = match entity.to_ascii_lowercase().as_str() {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "nbsp" => Some(' '),
            value if value.starts_with("#x") => u32::from_str_radix(&value[2..], 16)
                .ok()
                .and_then(char::from_u32),
            value if value.starts_with('#') => {
                value[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            output.push(replacement);
            cursor = end + 1;
        } else {
            output.push_str(&input[start..=end]);
            cursor = end + 1;
        }
    }
    output.push_str(&input[cursor..]);
    output
}

fn validate_bookmark_url(url: &str) -> Result<()> {
    if url.is_empty() || url.len() > 8 * 1024 || url.chars().any(char::is_control) {
        return Err(LoomError::InvalidPath(
            "bookmark URL is empty, too long, or contains control characters".into(),
        ));
    }
    let Some(colon) = url.find(':') else {
        return Err(LoomError::InvalidPath("bookmark URL has no scheme".into()));
    };
    if colon == 0
        || !url[..colon]
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '+' | '-' | '.'))
    {
        return Err(LoomError::InvalidPath(
            "bookmark URL has an invalid scheme".into(),
        ));
    }
    if matches!(
        url[..colon].to_ascii_lowercase().as_str(),
        "javascript" | "data" | "vbscript"
    ) {
        return Err(LoomError::InvalidPath(
            "executable bookmark URLs are not imported".into(),
        ));
    }
    Ok(())
}
