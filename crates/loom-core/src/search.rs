use crate::{error::LoomError, EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, Result};

pub(crate) struct CompiledQuery {
    pub(crate) match_expression: String,
}

pub(crate) fn compile_query(query: &str) -> Result<CompiledQuery> {
    let query = query.trim();
    if query.is_empty() {
        return Err(LoomError::InvalidQuery("query cannot be empty".into()));
    }
    if query.chars().count() > 512 {
        return Err(LoomError::InvalidQuery(
            "query exceeds the 512-character limit".into(),
        ));
    }

    let mut parts = Vec::new();
    let mut buffer = String::new();
    let mut quoted = false;
    for character in query.chars() {
        match character {
            '"' => {
                if quoted {
                    push_part(&mut parts, &mut buffer);
                    quoted = false;
                } else {
                    push_unquoted(&mut parts, &mut buffer);
                    quoted = true;
                }
            }
            value if value.is_whitespace() && !quoted => push_unquoted(&mut parts, &mut buffer),
            value => buffer.push(value),
        }
    }
    if quoted {
        return Err(LoomError::InvalidQuery(
            "quoted phrase is missing its closing quote".into(),
        ));
    }
    push_unquoted(&mut parts, &mut buffer);

    if parts.is_empty() {
        return Err(LoomError::InvalidQuery(
            "query must contain a letter or number".into(),
        ));
    }
    let match_expression = parts
        .iter()
        .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ");
    Ok(CompiledQuery { match_expression })
}

/// Chooses markers that provably do not occur in this passage.
///
/// SQLite's FTS5 `highlight()` inserts these byte-for-byte around the terms it matched. A
/// passage is finite, so the monotonically increasing suffix guarantees that this loop finds a
/// collision-free pair.
pub(crate) fn collision_free_markers(passage: &str) -> (String, String) {
    for suffix in 0u64.. {
        let start = format!("\u{e000}LOOM-{suffix:016x}-START\u{e001}");
        let end = format!("\u{e000}LOOM-{suffix:016x}-END\u{e001}");
        if !passage.contains(&start) && !passage.contains(&end) {
            return (start, end);
        }
    }
    unreachable!("an unbounded marker sequence must contain a value absent from finite text")
}

/// Converts FTS5's highlighted projection into structured, source-exact evidence.
pub(crate) fn project_fts_evidence(
    passage: &str,
    highlighted: &str,
    passage_anchor: &EvidenceAnchor,
    start_marker: &str,
    end_marker: &str,
) -> Result<(EvidenceExcerpt, EvidenceAnchor)> {
    if start_marker.is_empty()
        || end_marker.is_empty()
        || start_marker == end_marker
        || passage.contains(start_marker)
        || passage.contains(end_marker)
    {
        return Err(LoomError::EvidenceProjection(
            "highlight markers are not collision-free".into(),
        ));
    }

    let mut segments = Vec::new();
    let mut buffer = String::new();
    let mut byte_cursor = 0usize;
    let mut source_char_cursor = 0usize;
    let mut inside_match = false;
    let mut range_start = None;
    let mut ranges = Vec::new();

    while byte_cursor < highlighted.len() {
        let remaining = &highlighted[byte_cursor..];
        if remaining.starts_with(start_marker) {
            if inside_match {
                return Err(LoomError::EvidenceProjection(
                    "FTS5 emitted nested start markers".into(),
                ));
            }
            push_segment(&mut segments, &mut buffer, false);
            inside_match = true;
            range_start = Some(source_char_cursor);
            byte_cursor += start_marker.len();
            continue;
        }
        if remaining.starts_with(end_marker) {
            if !inside_match {
                return Err(LoomError::EvidenceProjection(
                    "FTS5 emitted an unmatched end marker".into(),
                ));
            }
            push_segment(&mut segments, &mut buffer, true);
            let start = range_start.take().ok_or_else(|| {
                LoomError::EvidenceProjection("highlight range has no start".into())
            })?;
            if source_char_cursor <= start {
                return Err(LoomError::EvidenceProjection(
                    "FTS5 emitted an empty highlight range".into(),
                ));
            }
            ranges.push((start, source_char_cursor));
            inside_match = false;
            byte_cursor += end_marker.len();
            continue;
        }

        let character = remaining.chars().next().ok_or_else(|| {
            LoomError::EvidenceProjection("highlight projection ended unexpectedly".into())
        })?;
        buffer.push(character);
        source_char_cursor += 1;
        byte_cursor += character.len_utf8();
    }

    if inside_match {
        return Err(LoomError::EvidenceProjection(
            "FTS5 emitted an unclosed highlight range".into(),
        ));
    }
    push_segment(&mut segments, &mut buffer, false);
    if ranges.is_empty() {
        return Err(LoomError::EvidenceProjection(
            "FTS5 matched a passage without highlighting evidence".into(),
        ));
    }

    let reconstructed = segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<String>();
    if reconstructed != passage {
        return Err(LoomError::EvidenceProjection(
            "highlight projection does not reconstruct the stored passage".into(),
        ));
    }

    let (first_start, _) = ranges[0];
    let (_, last_end) = ranges[ranges.len() - 1];
    let characters = passage.chars().collect::<Vec<_>>();
    if last_end > characters.len() {
        return Err(LoomError::EvidenceProjection(
            "highlight range exceeds the stored passage".into(),
        ));
    }
    let EvidenceAnchor::Text {
        char_start,
        line_start,
        ..
    } = passage_anchor;
    let matched_line_start = line_start
        + characters[..first_start]
            .iter()
            .filter(|character| **character == '\n')
            .count() as u64;
    let matched_line_end = matched_line_start
        + characters[first_start..last_end]
            .iter()
            .filter(|character| **character == '\n')
            .count() as u64;
    let anchor = EvidenceAnchor::Text {
        char_start: char_start + first_start as u64,
        char_end: char_start + last_end as u64,
        line_start: matched_line_start,
        line_end: matched_line_end,
    };
    Ok((EvidenceExcerpt { segments }, anchor))
}

fn push_segment(segments: &mut Vec<EvidenceSegment>, buffer: &mut String, highlighted: bool) {
    if buffer.is_empty() {
        return;
    }
    let text = std::mem::take(buffer);
    if let Some(previous) = segments.last_mut() {
        if previous.highlighted == highlighted {
            previous.text.push_str(&text);
            return;
        }
    }
    segments.push(EvidenceSegment { text, highlighted });
}

fn push_unquoted(parts: &mut Vec<String>, buffer: &mut String) {
    let value = buffer
        .trim_matches(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
        .to_string();
    buffer.clear();
    if !value.is_empty() {
        parts.push(value);
    }
}

fn push_part(parts: &mut Vec<String>, buffer: &mut String) {
    let value = buffer.trim().to_string();
    buffer.clear();
    if !value.is_empty() {
        parts.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::{collision_free_markers, compile_query, project_fts_evidence};
    use crate::EvidenceAnchor;

    #[test]
    fn compiles_terms_and_phrases_without_fts_operators() {
        assert_eq!(
            compile_query("retry \"isolation level\" OR")
                .unwrap()
                .match_expression,
            "\"retry\" AND \"isolation level\" AND \"OR\""
        );
    }

    #[test]
    fn rejects_unclosed_phrase() {
        assert!(compile_query("\"unfinished").is_err());
    }

    #[test]
    fn structured_projection_preserves_literal_marker_like_source_text() {
        let source = "literal ⟦source text⟧ and the Needle remains exact";
        let (start, end) = collision_free_markers(source);
        let highlighted = source.replace("Needle", &format!("{start}Needle{end}"));
        let (excerpt, anchor) = project_fts_evidence(
            source,
            &highlighted,
            &EvidenceAnchor::Text {
                char_start: 20,
                char_end: 20 + source.chars().count() as u64,
                line_start: 4,
                line_end: 4,
            },
            &start,
            &end,
        )
        .unwrap();
        assert_eq!(
            excerpt
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>(),
            source
        );
        assert_eq!(
            excerpt
                .segments
                .iter()
                .filter(|segment| segment.highlighted)
                .map(|segment| segment.text.as_str())
                .collect::<String>(),
            "Needle"
        );
        assert_eq!(
            anchor,
            EvidenceAnchor::Text {
                char_start: 50,
                char_end: 56,
                line_start: 4,
                line_end: 4,
            }
        );
    }

    #[test]
    fn malformed_highlight_projection_fails_closed() {
        let source = "source";
        let (start, end) = collision_free_markers(source);
        let error = project_fts_evidence(
            source,
            &format!("{start}{source}"),
            &EvidenceAnchor::Text {
                char_start: 0,
                char_end: 6,
                line_start: 1,
                line_end: 1,
            },
            &start,
            &end,
        )
        .unwrap_err();
        assert!(matches!(error, crate::LoomError::EvidenceProjection(_)));
    }
}
