use std::fs;

use loom_core::{EvidenceAnchor, Library, LibraryLimits, SearchRequest};
use tempfile::tempdir;

#[test]
fn golden_pdf_records_page_anchors_warnings_and_verified_navigation() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("golden.pdf");
    let bytes = build_pdf(&["LOOM first page marker", "", "LOOM third page marker"]);
    fs::write(&source, &bytes).unwrap();

    let library = Library::open_in_memory().unwrap();
    let first = library.index_path(&source).unwrap();
    assert_eq!(first.indexed, 1);
    assert!(first.failures.is_empty());

    let observation = library.inspect_source(&source).unwrap();
    assert_eq!(
        observation.content_hash,
        format!("blake3:{}", blake3::hash(&bytes).to_hex())
    );
    assert_eq!(observation.extractor_id, "loom.pdf");
    assert_eq!(observation.extractor_version, "0.1.0");
    assert_eq!(observation.page_count, Some(3));
    assert_eq!(
        observation.parse_warnings,
        vec!["page 2 contains no extractable text"]
    );
    assert_eq!(observation.passages.len(), 2);
    assert!(matches!(
        observation.passages[0].anchor,
        EvidenceAnchor::PdfPage { page: 1, .. }
    ));
    assert!(matches!(
        observation.passages[1].anchor,
        EvidenceAnchor::PdfPage { page: 3, .. }
    ));

    let hit = library
        .search(&SearchRequest {
            text: "\"third page marker\"".into(),
            limit: 10,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("the page-two marker must be recoverable");
    let EvidenceAnchor::PdfPage {
        page,
        char_start,
        char_end,
        line_start,
        line_end,
    } = hit.anchor
    else {
        panic!("PDF result lost its page anchor")
    };
    assert_eq!(page, 3);
    assert_eq!((line_start, line_end), (1, 1));
    assert_eq!(
        "LOOM third page marker"
            .chars()
            .skip(char_start as usize)
            .take((char_end - char_start) as usize)
            .collect::<String>(),
        "third page marker"
    );

    let opened = library
        .resolve_verified_artifact_path(&hit.artifact_id, &hit.version_id, &hit.content_hash)
        .unwrap();
    assert_eq!(opened, source.canonicalize().unwrap());

    let repeated = library.index_path(&source).unwrap();
    assert_eq!(repeated.unchanged, 1);
    assert_eq!(library.inspect_source(&source).unwrap(), observation);
}

#[test]
fn malformed_encrypted_image_only_and_oversized_pdfs_fail_closed_then_recover() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("bounded.pdf");
    let library = Library::open(directory.path().join("library.sqlite3")).unwrap();

    fs::write(&source, b"%PDF-1.4\nnot a complete document").unwrap();
    let malformed = library.index_path(&source).unwrap();
    assert_eq!(malformed.failed, 1);
    assert!(malformed.failures[0].reason.contains("malformed PDF"));

    fs::write(&source, build_pdf(&[""])).unwrap();
    let image_only = library.index_path(&source).unwrap();
    assert_eq!(image_only.failed, 1);
    assert!(image_only.failures[0]
        .reason
        .contains("no extractable text"));

    let mut encrypted = build_pdf(&["secret page"]);
    encrypted.extend_from_slice(b"\n% /Encrypt 7 0 R\n");
    fs::write(&source, encrypted).unwrap();
    let encrypted = library.index_path(&source).unwrap();
    assert_eq!(encrypted.failed, 1);
    assert!(encrypted.failures[0].reason.contains("encrypted PDF"));

    fs::write(&source, vec![b'x'; 8 * 1024 * 1024 + 1]).unwrap();
    let oversized = library.index_path(&source).unwrap();
    assert_eq!(oversized.failed, 1);
    assert!(oversized.failures[0]
        .reason
        .contains("exceeds the 8388608-byte limit"));

    fs::write(&source, build_pdf(&["recovered page marker"])).unwrap();
    let recovered = library.index_path(&source).unwrap();
    assert_eq!(recovered.indexed, 1);
    assert!(library
        .search(&SearchRequest {
            text: "recovered page marker".into(),
            limit: 10,
        })
        .unwrap()
        .iter()
        .any(|hit| matches!(hit.anchor, EvidenceAnchor::PdfPage { page: 1, .. })));

    let limited = Library::open_with_limits(
        directory.path().join("page-limit.sqlite3"),
        LibraryLimits {
            max_pdf_pages: 1,
            ..LibraryLimits::default()
        },
    )
    .unwrap();
    fs::write(&source, build_pdf(&["page one", "page two"])).unwrap();
    let page_limited = limited.index_path(&source).unwrap();
    assert_eq!(page_limited.failed, 1);
    assert!(page_limited.failures[0].reason.contains("1-page limit"));
}

fn build_pdf(pages: &[&str]) -> Vec<u8> {
    let font_number = 3 + pages.len() * 2;
    let mut objects = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids = pages
        .iter()
        .enumerate()
        .map(|(index, _)| format!("{} 0 R", 3 + index * 2))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()).into_bytes());
    for (index, text) in pages.iter().enumerate() {
        let content_number = 4 + index * 2;
        let stream = format!("BT\n/F1 18 Tf\n72 720 Td\n({text}) Tj\nET\n");
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                 /Resources << /Font << /F1 {font_number} 0 R >> >> \
                 /Contents {content_number} 0 R >>"
            )
            .into_bytes(),
        );
        objects.push(
            format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()).into_bytes(),
        );
    }
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());

    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}
