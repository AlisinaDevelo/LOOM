//! Fixed-length, versioned one-shot framing. Source identity is not part of this protocol.

use std::io::{Read, Write};

use crate::{
    ExtractionError, HelperResponse, RequestMetadata, Result, MAX_INPUT_BYTES, MAX_RESPONSE_BYTES,
};

const MAGIC: &[u8; 8] = b"LOOMEXT\0";
const VERSION: u16 = 1;
const METADATA: u16 = 1;
const BYTES: u16 = 2;
const RESPONSE: u16 = 3;
const MAX_METADATA: usize = 4096;

pub fn write_request(
    mut writer: impl Write,
    metadata: &RequestMetadata,
    bytes: &[u8],
) -> Result<()> {
    metadata.budget.validate()?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ExtractionError::Protocol("input exceeds 8 MiB".into()));
    }
    let encoded = serde_json::to_vec(metadata)
        .map_err(|_| ExtractionError::Protocol("invalid request metadata".into()))?;
    write_frame(&mut writer, METADATA, &encoded, MAX_METADATA)?;
    write_frame(&mut writer, BYTES, bytes, MAX_INPUT_BYTES)?;
    writer.flush().map_err(|_| protocol_io())
}

pub fn read_metadata(reader: impl Read) -> Result<RequestMetadata> {
    let bytes = read_frame(reader, METADATA, MAX_METADATA)?;
    let metadata: RequestMetadata = serde_json::from_slice(&bytes)
        .map_err(|_| ExtractionError::Protocol("invalid request metadata".into()))?;
    metadata.budget.validate()?;
    Ok(metadata)
}

pub fn read_input(mut reader: impl Read) -> Result<Vec<u8>> {
    let bytes = read_frame(&mut reader, BYTES, MAX_INPUT_BYTES)?;
    require_eof(reader)?;
    Ok(bytes)
}

pub fn write_response(mut writer: impl Write, response: &HelperResponse) -> Result<()> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_RESPONSE_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("response limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut encoded = Bounded(Vec::new());
    serde_json::to_writer(&mut encoded, response).map_err(|_| ExtractionError::OutputLimit)?;
    write_frame(&mut writer, RESPONSE, &encoded.0, MAX_RESPONSE_BYTES)?;
    writer.flush().map_err(|_| protocol_io())
}

pub fn read_response(mut reader: impl Read) -> Result<HelperResponse> {
    let mut first = [0];
    loop {
        match reader.read(&mut first) {
            Ok(0) => return Err(ExtractionError::ChildCrashed),
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(protocol_io()),
        }
    }
    let bytes = read_frame(
        std::io::Cursor::new(first).chain(&mut reader),
        RESPONSE,
        MAX_RESPONSE_BYTES,
    )?;
    let response = serde_json::from_slice(&bytes)
        .map_err(|_| ExtractionError::Protocol("invalid response JSON".into()))?;
    require_eof(reader)?;
    Ok(response)
}

fn protocol_io() -> ExtractionError {
    ExtractionError::Protocol("truncated or unavailable transport".into())
}

fn write_frame(mut writer: impl Write, kind: u16, bytes: &[u8], max: usize) -> Result<()> {
    if bytes.len() > max {
        return Err(ExtractionError::OutputLimit);
    }
    let length = u32::try_from(bytes.len()).map_err(|_| ExtractionError::OutputLimit)?;
    writer.write_all(MAGIC).map_err(|_| protocol_io())?;
    writer
        .write_all(&VERSION.to_le_bytes())
        .map_err(|_| protocol_io())?;
    writer
        .write_all(&kind.to_le_bytes())
        .map_err(|_| protocol_io())?;
    writer
        .write_all(&length.to_le_bytes())
        .map_err(|_| protocol_io())?;
    writer.write_all(bytes).map_err(|_| protocol_io())
}

fn read_frame(mut reader: impl Read, kind: u16, max: usize) -> Result<Vec<u8>> {
    let mut header = [0u8; 16];
    reader.read_exact(&mut header).map_err(|_| protocol_io())?;
    let version = u16::from_le_bytes([header[8], header[9]]);
    let actual_kind = u16::from_le_bytes([header[10], header[11]]);
    let length = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    if &header[..8] != MAGIC || version != VERSION || actual_kind != kind {
        return Err(ExtractionError::Protocol(
            "wrong frame magic/version/kind".into(),
        ));
    }
    // Check before allocating or reading body bytes. The wire length is never trusted.
    if length > max {
        return Err(ExtractionError::OutputLimit);
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).map_err(|_| protocol_io())?;
    Ok(bytes)
}

fn require_eof(mut reader: impl Read) -> Result<()> {
    let mut byte = [0];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                return Err(ExtractionError::Protocol(
                    "extra frame or trailing bytes".into(),
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(protocol_io()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExtractionBudget, MediaKind};
    use std::io::Cursor;

    fn metadata() -> RequestMetadata {
        RequestMetadata {
            media: MediaKind::Markdown,
            budget: ExtractionBudget::for_media(MediaKind::Markdown),
        }
    }

    #[test]
    fn byte_only_request_round_trips_and_contains_no_authority_fields() {
        let mut frame = Vec::new();
        write_request(&mut frame, &metadata(), b"rights-clean source bytes").unwrap();
        let mut input = Cursor::new(frame);
        assert_eq!(read_metadata(&mut input).unwrap(), metadata());
        assert_eq!(
            read_input(&mut input).unwrap(),
            b"rights-clean source bytes"
        );
        assert_eq!(input.position(), input.get_ref().len() as u64);
        let value = serde_json::to_value(metadata()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert!(value.get("locator").is_none());
        assert!(value.get("authorization").is_none());
        assert!(value.get("database").is_none());
    }

    #[test]
    fn unknown_media_and_authority_fields_are_rejected() {
        let mut value = serde_json::to_value(metadata()).unwrap();
        value["media"] = serde_json::json!("video/mp4");
        assert!(serde_json::from_value::<RequestMetadata>(value).is_err());
        let mut value = serde_json::to_value(metadata()).unwrap();
        value["locator"] = serde_json::json!("/untrusted/source");
        assert!(serde_json::from_value::<RequestMetadata>(value).is_err());
    }

    #[test]
    fn malformed_version_truncation_and_extra_input_are_refused() {
        let mut frame = Vec::new();
        write_request(&mut frame, &metadata(), b"source").unwrap();
        for length in 0..16 {
            assert!(read_metadata(Cursor::new(&frame[..length])).is_err());
        }
        let mut wrong = frame.clone();
        wrong[8] = 2;
        assert!(read_metadata(Cursor::new(wrong)).is_err());
        let mut wrong = frame.clone();
        wrong[10] = BYTES as u8;
        assert!(read_metadata(Cursor::new(wrong)).is_err());
        frame.push(0);
        let mut input = Cursor::new(frame);
        read_metadata(&mut input).unwrap();
        assert!(read_input(input).is_err());
    }

    #[test]
    fn declared_oversize_is_rejected_before_a_body_read() {
        struct HeaderOnly {
            header: Cursor<Vec<u8>>,
        }
        impl Read for HeaderOnly {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                assert!(self.header.position() < 16, "oversized body was read");
                self.header.read(output)
            }
        }
        let mut header = MAGIC.to_vec();
        header.extend(VERSION.to_le_bytes());
        header.extend(METADATA.to_le_bytes());
        header.extend(u32::MAX.to_le_bytes());
        assert!(matches!(
            read_metadata(HeaderOnly {
                header: Cursor::new(header)
            }),
            Err(ExtractionError::OutputLimit)
        ));
    }

    #[test]
    fn one_response_round_trips_without_identity_or_duplicate_text() {
        let response = HelperResponse::Success {
            output: crate::SourceOutput::Pdf {
                page_count: 1,
                pages: vec![(1, "page evidence".into())],
                warnings: vec![],
            },
            metrics: crate::ExtractionMetrics {
                address_space_limit_installed: true,
                sample_interval_ms: crate::WATCH_INTERVAL_MS,
                ..Default::default()
            },
        };
        let mut wire = Vec::new();
        write_response(&mut wire, &response).unwrap();
        assert_eq!(read_response(Cursor::new(wire)).unwrap(), response);
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("normalized_text"));
        assert!(!json.contains("artifact_id"));
        assert!(!json.contains("content_hash"));
    }

    #[test]
    fn response_rejects_trailing_bytes_unknown_fields_and_excessive_encoding() {
        let response = HelperResponse::Failure {
            code: crate::FailureCode::OcrUnavailable,
        };
        let mut wire = Vec::new();
        write_response(&mut wire, &response).unwrap();
        wire.push(0);
        assert!(read_response(Cursor::new(wire)).is_err());
        let mut wire = Vec::new();
        write_frame(
            &mut wire,
            RESPONSE,
            br#"{"kind":"failure","code":"ocr_unavailable","artifact_id":"forged"}"#,
            MAX_RESPONSE_BYTES,
        )
        .unwrap();
        assert!(read_response(Cursor::new(wire)).is_err());
        let response = HelperResponse::Success {
            output: crate::SourceOutput::Text {
                text: "x".repeat(MAX_RESPONSE_BYTES),
            },
            metrics: Default::default(),
        };
        assert_eq!(
            write_response(Vec::new(), &response),
            Err(ExtractionError::OutputLimit)
        );
    }

    #[test]
    fn truncated_metadata_input_and_response_bodies_never_form_a_valid_request() {
        let encoded = serde_json::to_vec(&metadata()).unwrap();
        let mut metadata_frame = Vec::new();
        write_frame(&mut metadata_frame, METADATA, &encoded, MAX_METADATA).unwrap();
        for end in 16..metadata_frame.len() {
            assert!(read_metadata(Cursor::new(&metadata_frame[..end])).is_err());
        }
        let mut input = Vec::new();
        write_frame(&mut input, BYTES, b"rights-clean evidence", MAX_INPUT_BYTES).unwrap();
        for end in 16..input.len() {
            assert!(read_input(Cursor::new(&input[..end])).is_err());
        }
        let mut response = Vec::new();
        write_response(
            &mut response,
            &HelperResponse::Failure {
                code: crate::FailureCode::Protocol,
            },
        )
        .unwrap();
        for end in 0..response.len() {
            assert!(read_response(Cursor::new(&response[..end])).is_err());
        }
    }
}
