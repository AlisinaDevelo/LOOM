use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use crate::{error::LoomError, EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, Result};

/// A source-type constraint understood by the user-facing query language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceTypeFilter {
    Pdf,
    Image,
    Text,
    Markdown,
    Mime(String),
}

/// The comparison used by a confidence constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceOperator {
    Equal,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

/// A typed OCR/evidence confidence constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfidenceFilter {
    pub operator: ConfidenceOperator,
    pub threshold: f64,
}

/// Typed constraints extracted from a search string.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryFilters {
    /// Inclusive nanosecond lower bound on the source modification time.
    pub after_ns: Option<i64>,
    /// Exclusive nanosecond upper bound on the source modification time.
    pub before_ns: Option<i64>,
    pub source_type: Option<SourceTypeFilter>,
    /// Case-insensitive substring matched against the canonical source locator.
    pub path_contains: Option<String>,
    pub confidence: Option<ConfidenceFilter>,
}

impl QueryFilters {
    pub fn is_empty(&self) -> bool {
        self.after_ns.is_none()
            && self.before_ns.is_none()
            && self.source_type.is_none()
            && self.path_contains.is_none()
            && self.confidence.is_none()
    }

    /// Returns whether one canonical result satisfies every parsed constraint.
    pub fn matches(
        &self,
        media_type: &str,
        source_uri: &str,
        source_modified_ns: Option<i64>,
        anchor: &EvidenceAnchor,
    ) -> bool {
        if let Some(after_ns) = self.after_ns {
            if source_modified_ns.is_none_or(|value| value < after_ns) {
                return false;
            }
        }
        if let Some(before_ns) = self.before_ns {
            if source_modified_ns.is_none_or(|value| value >= before_ns) {
                return false;
            }
        }
        if let Some(source_type) = &self.source_type {
            let matches = match source_type {
                SourceTypeFilter::Pdf => media_type == "application/pdf",
                SourceTypeFilter::Image => media_type.starts_with("image/"),
                SourceTypeFilter::Text => media_type.starts_with("text/"),
                SourceTypeFilter::Markdown => media_type == "text/markdown",
                SourceTypeFilter::Mime(expected) => media_type.eq_ignore_ascii_case(expected),
            };
            if !matches {
                return false;
            }
        }
        if let Some(path_contains) = &self.path_contains {
            if !source_uri
                .to_lowercase()
                .contains(&path_contains.to_lowercase())
            {
                return false;
            }
        }
        if let Some(confidence) = &self.confidence {
            if !confidence.matches(anchor_confidence(anchor)) {
                return false;
            }
        }
        true
    }
}

impl ConfidenceFilter {
    fn matches(&self, value: f64) -> bool {
        match self.operator {
            ConfidenceOperator::Equal => (value - self.threshold).abs() <= f64::EPSILON,
            ConfidenceOperator::GreaterThan => value > self.threshold,
            ConfidenceOperator::GreaterThanOrEqual => value >= self.threshold,
            ConfidenceOperator::LessThan => value < self.threshold,
            ConfidenceOperator::LessThanOrEqual => value <= self.threshold,
        }
    }
}

/// The safe, storage-engine-independent representation of a user query.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedQuery {
    pub text: String,
    pub match_expression: String,
    pub filters: QueryFilters,
}

pub(crate) struct CompiledQuery {
    pub(crate) match_expression: String,
    pub(crate) filters: QueryFilters,
}

pub(crate) fn compile_query(query: &str) -> Result<CompiledQuery> {
    let parsed = parse_query(query)?;
    Ok(CompiledQuery {
        match_expression: parsed.match_expression,
        filters: parsed.filters,
    })
}

/// Parses LOOM's documented query language without exposing SQLite/FTS syntax.
///
/// Examples: `"retry anomalies" after:2026-01-01 type:pdf`,
/// `path:"research notes" confidence:>=0.90`. Dates without a time are UTC midnights;
/// `after` is inclusive and `before` is exclusive. Quoted text is an exact phrase.
pub fn parse_query(query: &str) -> Result<ParsedQuery> {
    let query = query.trim();
    if query.is_empty() {
        return Err(LoomError::InvalidQuery("query cannot be empty".into()));
    }
    if query.chars().count() > 512 {
        return Err(LoomError::InvalidQuery(
            "query exceeds the 512-character limit".into(),
        ));
    }

    let tokens = tokenize(query)?;
    let mut parts = Vec::new();
    let mut filters = QueryFilters::default();
    for token in tokens {
        if !token.starts_with_quote {
            if let Some(filter) = parse_filter(&token.text)? {
                insert_filter(&mut filters, filter)?;
                continue;
            }
        }
        let value = if token.quoted {
            token.text.trim().to_string()
        } else {
            token
                .text
                .trim_matches(|character: char| {
                    !character.is_alphanumeric() && character != '_' && character != '-'
                })
                .to_string()
        };
        if !value.is_empty() && value.chars().any(char::is_alphanumeric) {
            parts.push(value);
        }
    }

    if parts.is_empty() {
        return Err(LoomError::InvalidQuery(
            "query must include search text in addition to any filters".into(),
        ));
    }
    let match_expression = parts
        .iter()
        .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ");
    Ok(ParsedQuery {
        text: parts.join(" "),
        match_expression,
        filters,
    })
}

#[derive(Debug)]
struct QueryToken {
    text: String,
    quoted: bool,
    starts_with_quote: bool,
}

fn tokenize(query: &str) -> Result<Vec<QueryToken>> {
    let mut characters = query.chars().peekable();
    let mut tokens = Vec::new();
    while characters.peek().is_some() {
        while characters.peek().is_some_and(|value| value.is_whitespace()) {
            characters.next();
        }
        if characters.peek().is_none() {
            break;
        }
        let starts_with_quote = characters.peek() == Some(&'"');
        let mut text = String::new();
        let mut quoted = false;
        let mut had_quote = false;
        let mut escaped = false;
        for character in characters.by_ref() {
            if escaped {
                if character != '"' && character != '\\' {
                    return Err(LoomError::InvalidQuery(
                        "only \\\" and \\\\ may be escaped inside a phrase".into(),
                    ));
                }
                text.push(character);
                escaped = false;
                continue;
            }
            if character == '\\' && quoted {
                escaped = true;
                continue;
            }
            if character == '"' {
                had_quote = true;
                quoted = !quoted;
                continue;
            }
            if character.is_whitespace() && !quoted {
                break;
            }
            text.push(character);
        }
        if escaped {
            return Err(LoomError::InvalidQuery(
                "quoted phrase ends with an escape".into(),
            ));
        }
        if quoted {
            return Err(LoomError::InvalidQuery(
                "quoted phrase is missing its closing quote".into(),
            ));
        }
        if text.is_empty() {
            return Err(LoomError::InvalidQuery(
                "query tokens cannot be empty".into(),
            ));
        }
        tokens.push(QueryToken {
            text,
            quoted: had_quote,
            starts_with_quote,
        });
    }
    Ok(tokens)
}

enum ParsedFilter {
    After(i64),
    Before(i64),
    SourceType(SourceTypeFilter),
    Path(String),
    Confidence(ConfidenceFilter),
}

fn parse_filter(token: &str) -> Result<Option<ParsedFilter>> {
    let Some((key, value)) = token.split_once(':') else {
        return Ok(None);
    };
    let key = key.to_ascii_lowercase();
    if !matches!(
        key.as_str(),
        "after" | "before" | "type" | "path" | "confidence"
    ) {
        return Ok(None);
    }
    if value.is_empty() {
        return Err(LoomError::InvalidQuery(format!(
            "{key}: requires a value; try {key}:HELP"
        )));
    }
    let parsed = match key.as_str() {
        "after" => ParsedFilter::After(parse_timestamp(value, "after")?),
        "before" => ParsedFilter::Before(parse_timestamp(value, "before")?),
        "type" => ParsedFilter::SourceType(parse_source_type(value)?),
        "path" => ParsedFilter::Path(value.to_string()),
        "confidence" => ParsedFilter::Confidence(parse_confidence(value)?),
        _ => unreachable!(),
    };
    Ok(Some(parsed))
}

fn insert_filter(filters: &mut QueryFilters, filter: ParsedFilter) -> Result<()> {
    let (slot, name, value) = match filter {
        ParsedFilter::After(value) => (&mut filters.after_ns, "after", value),
        ParsedFilter::Before(value) => (&mut filters.before_ns, "before", value),
        ParsedFilter::SourceType(value) => {
            if filters.source_type.is_some() {
                return Err(LoomError::InvalidQuery(
                    "type filter may appear only once".into(),
                ));
            }
            filters.source_type = Some(value);
            return Ok(());
        }
        ParsedFilter::Path(value) => {
            if filters.path_contains.is_some() {
                return Err(LoomError::InvalidQuery(
                    "path filter may appear only once".into(),
                ));
            }
            filters.path_contains = Some(value);
            return Ok(());
        }
        ParsedFilter::Confidence(value) => {
            if filters.confidence.is_some() {
                return Err(LoomError::InvalidQuery(
                    "confidence filter may appear only once".into(),
                ));
            }
            filters.confidence = Some(value);
            return Ok(());
        }
    };
    if slot.is_some() {
        return Err(LoomError::InvalidQuery(format!(
            "{name} filter may appear only once"
        )));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_timestamp(value: &str, name: &str) -> Result<i64> {
    let timestamp = if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        parsed.with_timezone(&Utc)
    } else if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Utc.from_utc_datetime(
            &date
                .and_hms_opt(0, 0, 0)
                .expect("midnight is always a valid NaiveDate time"),
        )
    } else {
        return Err(LoomError::InvalidQuery(format!(
            "{name}: expected YYYY-MM-DD or RFC3339 timestamp"
        )));
    };
    timestamp.timestamp_nanos_opt().ok_or_else(|| {
        LoomError::InvalidQuery(format!("{name}: timestamp is outside the supported range"))
    })
}

fn parse_source_type(value: &str) -> Result<SourceTypeFilter> {
    let normalized = value.to_ascii_lowercase();
    match normalized.as_str() {
        "pdf" => Ok(SourceTypeFilter::Pdf),
        "image" | "img" => Ok(SourceTypeFilter::Image),
        "text" => Ok(SourceTypeFilter::Text),
        "markdown" | "md" => Ok(SourceTypeFilter::Markdown),
        value if value.contains('/') && !value.chars().any(char::is_whitespace) => {
            Ok(SourceTypeFilter::Mime(normalized))
        }
        _ => Err(LoomError::InvalidQuery(
            "type: expects pdf, image, text, markdown, or a MIME type".into(),
        )),
    }
}

fn parse_confidence(value: &str) -> Result<ConfidenceFilter> {
    let (operator, number) = if let Some(value) = value.strip_prefix(">=") {
        (ConfidenceOperator::GreaterThanOrEqual, value)
    } else if let Some(value) = value.strip_prefix("<=") {
        (ConfidenceOperator::LessThanOrEqual, value)
    } else if let Some(value) = value.strip_prefix('>') {
        (ConfidenceOperator::GreaterThan, value)
    } else if let Some(value) = value.strip_prefix('<') {
        (ConfidenceOperator::LessThan, value)
    } else if let Some(value) = value.strip_prefix('=') {
        (ConfidenceOperator::Equal, value)
    } else {
        (ConfidenceOperator::Equal, value)
    };
    let threshold = number.parse::<f64>().map_err(|_| {
        LoomError::InvalidQuery("confidence: expects a number between 0 and 1".into())
    })?;
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(LoomError::InvalidQuery(
            "confidence: expects a number between 0 and 1".into(),
        ));
    }
    Ok(ConfidenceFilter {
        operator,
        threshold,
    })
}

fn anchor_confidence(anchor: &EvidenceAnchor) -> f64 {
    match anchor {
        EvidenceAnchor::ImageRegion {
            confidence_milli, ..
        } => f64::from(*confidence_milli) / 1_000.0,
        EvidenceAnchor::Text { .. } | EvidenceAnchor::PdfPage { .. } => 1.0,
    }
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
    let (char_start, line_start, page, image) = match passage_anchor {
        EvidenceAnchor::Text {
            char_start,
            line_start,
            ..
        } => (*char_start, *line_start, None, None),
        EvidenceAnchor::PdfPage {
            page,
            char_start,
            line_start,
            ..
        } => (*char_start, *line_start, Some(*page), None),
        EvidenceAnchor::ImageRegion {
            char_start,
            line_start,
            x,
            y,
            width,
            height,
            image_width,
            image_height,
            orientation,
            scale_milli,
            confidence_milli,
            ..
        } => (
            *char_start,
            *line_start,
            None,
            Some((
                *x,
                *y,
                *width,
                *height,
                *image_width,
                *image_height,
                *orientation,
                *scale_milli,
                *confidence_milli,
            )),
        ),
    };
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
    let anchor = match (page, image) {
        (Some(page), None) => EvidenceAnchor::PdfPage {
            page,
            char_start: char_start + first_start as u64,
            char_end: char_start + last_end as u64,
            line_start: matched_line_start,
            line_end: matched_line_end,
        },
        (
            None,
            Some((
                x,
                y,
                width,
                height,
                image_width,
                image_height,
                orientation,
                scale_milli,
                confidence_milli,
            )),
        ) => EvidenceAnchor::ImageRegion {
            char_start: char_start + first_start as u64,
            char_end: char_start + last_end as u64,
            line_start: matched_line_start,
            line_end: matched_line_end,
            x,
            y,
            width,
            height,
            image_width,
            image_height,
            orientation,
            scale_milli,
            confidence_milli,
        },
        (None, None) => EvidenceAnchor::Text {
            char_start: char_start + first_start as u64,
            char_end: char_start + last_end as u64,
            line_start: matched_line_start,
            line_end: matched_line_end,
        },
        (Some(_), Some(_)) => {
            return Err(LoomError::EvidenceProjection(
                "an evidence anchor cannot be both a PDF page and image region".into(),
            ))
        }
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

#[cfg(test)]
mod tests {
    use super::{
        collision_free_markers, compile_query, parse_query, project_fts_evidence,
        ConfidenceOperator, SourceTypeFilter,
    };
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
    fn parses_typed_filters_without_leaking_fts_syntax() {
        let parsed = parse_query(
            r#""retry anomalies" after:2026-01-01 before:2026-02-01 type:pdf path:"Research Notes" confidence:>=0.90"#,
        )
        .unwrap();
        assert_eq!(parsed.text, "retry anomalies");
        assert_eq!(parsed.match_expression, "\"retry anomalies\"");
        assert!(parsed.filters.after_ns.is_some());
        assert!(parsed.filters.before_ns.is_some());
        assert_eq!(parsed.filters.source_type, Some(SourceTypeFilter::Pdf));
        assert_eq!(
            parsed.filters.path_contains.as_deref(),
            Some("Research Notes")
        );
        assert_eq!(
            parsed.filters.confidence,
            Some(super::ConfidenceFilter {
                operator: ConfidenceOperator::GreaterThanOrEqual,
                threshold: 0.90,
            })
        );
    }

    #[test]
    fn parser_handles_unicode_escaping_and_adversarial_values() {
        for index in 0..128 {
            let query = format!(r#"日本語 term-{index} path:"notes {index}""#);
            let parsed = parse_query(&query).unwrap();
            assert!(parsed.match_expression.starts_with('"'));
            assert!(!parsed.match_expression.contains("MATCH"));
        }
        let escaped = parse_query(r#"needle "quoted \"value\"""#).unwrap();
        assert!(escaped.match_expression.contains("quoted \"\"value\"\""));
        let injection_like = parse_query(r#"needle OR 1=1 --"#).unwrap();
        assert_eq!(
            injection_like.match_expression,
            "\"needle\" AND \"OR\" AND \"1=1\""
        );
        assert!(parse_query("needle confidence:wat").is_err());
        assert!(parse_query("needle after:not-a-date").is_err());
        assert!(parse_query("needle type:sqlite").is_err());
        assert!(parse_query("type:pdf").is_err());
    }

    #[test]
    fn typed_filters_apply_to_time_type_path_and_confidence() {
        let filters = parse_query(
            "needle after:2020-01-01 before:2030-01-01 type:image path:shots confidence:>=0.80",
        )
        .unwrap()
        .filters;
        let anchor = EvidenceAnchor::ImageRegion {
            char_start: 0,
            char_end: 6,
            line_start: 1,
            line_end: 1,
            x: 1,
            y: 2,
            width: 3,
            height: 4,
            image_width: 100,
            image_height: 100,
            orientation: 1,
            scale_milli: 1_000,
            confidence_milli: 900,
        };
        assert!(filters.matches(
            "image/png",
            "/tmp/shots/capture.png",
            Some(1_650_000_000_000_000_000),
            &anchor
        ));
        assert!(!filters.matches(
            "text/plain",
            "/tmp/shots/capture.txt",
            Some(1_650_000_000_000_000_000),
            &anchor
        ));
        assert!(!filters.matches(
            "image/png",
            "/tmp/other/capture.png",
            Some(1_650_000_000_000_000_000),
            &anchor
        ));
        assert!(!filters.matches(
            "image/png",
            "/tmp/shots/capture.png",
            Some(1_650_000_000_000_000_000),
            &EvidenceAnchor::ImageRegion {
                char_start: 0,
                char_end: 6,
                line_start: 1,
                line_end: 1,
                x: 1,
                y: 2,
                width: 3,
                height: 4,
                image_width: 100,
                image_height: 100,
                orientation: 1,
                scale_milli: 1_000,
                confidence_milli: 700,
            }
        ));
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
