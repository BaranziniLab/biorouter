use anyhow::{Context, Result};
use lopdf::{content::Content, Document, Object};
use std::panic::{self, AssertUnwindSafe};
use std::{io::Write, process::Command};
use tempfile::NamedTempFile;

pub struct PdfConversion {
    pub markdown: String,
    pub title: Option<String>,
    pub needs_llm_fallback: bool,
}

pub fn pdf_to_markdown(bytes: &[u8]) -> Result<PdfConversion> {
    // Primary: pdf-inspector — structured markdown (headings, tables,
    // multi-column reading order) plus a scanned-vs-text classification that
    // replaces the old "fewer than 32 chars" heuristic.
    if let Some(conversion) = pdf_inspector_convert(bytes) {
        return Ok(conversion);
    }

    // Fallback: legacy text-layer chain (pdf-extract → lopdf → pdfminer →
    // lossy content-op scan).
    let text = extract_pdf_text(bytes)?;
    let cleaned = normalize_text(&text);
    let needs_llm_fallback = cleaned.trim().len() < 32;
    Ok(PdfConversion {
        markdown: cleaned,
        title: None,
        needs_llm_fallback,
    })
}

fn pdf_inspector_convert(bytes: &[u8]) -> Option<PdfConversion> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| pdf_inspector::process_pdf_mem(bytes)))
        .ok()?
        .ok()?;

    let scanned = matches!(
        result.pdf_type,
        pdf_inspector::PdfType::Scanned | pdf_inspector::PdfType::ImageBased
    );
    let markdown = result.markdown.unwrap_or_default().trim().to_string();

    // For text-based PDFs with no usable output, let the legacy extractor
    // chain have a go instead of giving up here.
    if markdown.len() < 32 && !scanned {
        return None;
    }

    Some(PdfConversion {
        title: result.title.filter(|t| !t.trim().is_empty()),
        needs_llm_fallback: scanned || markdown.len() < 32,
        markdown,
    })
}

fn extract_pdf_text(bytes: &[u8]) -> Result<String> {
    extract_pdf_text_with(
        bytes,
        |bytes| pdf_extract::extract_text_from_mem(bytes).context("pdf-extract failed"),
        lopdf_extract_text,
    )
}

fn extract_pdf_text_with<P, F>(bytes: &[u8], primary: P, fallback: F) -> Result<String>
where
    P: FnOnce(&[u8]) -> Result<String>,
    F: FnOnce(&[u8]) -> Result<String>,
{
    let primary_result = panic::catch_unwind(AssertUnwindSafe(|| primary(bytes)));
    let primary_issue = match primary_result {
        Ok(Ok(text)) if !text.trim().is_empty() => return Ok(text),
        Ok(Ok(_)) => Some("primary extractor returned no readable text".into()),
        Ok(Err(err)) => Some(err.to_string()),
        Err(payload) => Some(format!(
            "primary extractor panicked: {}",
            panic_message(payload)
        )),
    };

    match fallback(bytes) {
        Ok(text) if !text.trim().is_empty() => Ok(text),
        Ok(text) => Ok(text),
        Err(fallback_err) => match primary_issue {
            Some(primary_issue) => Err(anyhow::anyhow!(
                "PDF text extraction failed: {primary_issue}; fallback extractor failed: {fallback_err}"
            )),
            None => Err(fallback_err),
        },
    }
}

fn lopdf_extract_text(bytes: &[u8]) -> Result<String> {
    let doc = Document::load_mem(bytes).context("load pdf into lopdf")?;
    let pages = doc.get_pages();
    if pages.is_empty() {
        return Ok(String::new());
    }
    let page_numbers = pages.keys().copied().collect::<Vec<_>>();

    match doc.extract_text(&page_numbers) {
        Ok(text) => Ok(text),
        Err(err) => {
            let partial = doc
                .extract_text_chunks(&page_numbers)
                .into_iter()
                .filter_map(|chunk| chunk.ok())
                .collect::<Vec<_>>()
                .join("\n");
            if partial.trim().is_empty() {
                if let Ok(python_text) = python_pdfminer_extract_text(bytes) {
                    if !python_text.trim().is_empty() {
                        return Ok(python_text);
                    }
                }

                let lossy = extract_lossy_text_from_operations(&doc, &pages)?;
                if !lossy.trim().is_empty() {
                    Ok(lossy)
                } else {
                    Err(err).context("extract text with lopdf")
                }
            } else {
                Ok(partial)
            }
        }
    }
}

fn python_pdfminer_extract_text(bytes: &[u8]) -> Result<String> {
    let python = which::which("python3").context("python3 not found")?;
    let mut file = NamedTempFile::new().context("create temp pdf for pdfminer")?;
    file.write_all(bytes)
        .context("write temp pdf for pdfminer")?;

    let mut command = Command::new(python);
    command
        .arg("-c")
        .arg(PYTHON_PDFMINER_SCRIPT)
        .arg(file.path());
    crate::developer::shell::strip_daemon_private_env_std(&mut command);
    crate::developer::shell::no_console_window_std(&mut command);
    let output = command.output().context("run python pdfminer extractor")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("python pdfminer failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

const PYTHON_PDFMINER_SCRIPT: &str = r#"
from pdfminer.high_level import extract_text
import sys

text = extract_text(sys.argv[1])
sys.stdout.write(text or "")
"#;

fn extract_lossy_text_from_operations(
    doc: &Document,
    pages: &std::collections::BTreeMap<u32, lopdf::ObjectId>,
) -> Result<String> {
    let mut out = String::new();

    for page_id in pages.values().copied() {
        let content = Content::decode(&doc.get_page_content(page_id)?)
            .context("decode lopdf page content")?;
        let mut page_text = String::new();

        for operation in content.operations {
            match operation.operator.as_str() {
                "Tj" | "TJ" => {
                    collect_lossy_operands(&mut page_text, &operation.operands);
                    page_text.push('\n');
                }
                "'" | "\"" => {
                    page_text.push('\n');
                    collect_lossy_operands(&mut page_text, &operation.operands);
                    page_text.push('\n');
                }
                "ET" => {
                    if !page_text.ends_with("\n\n") {
                        page_text.push('\n');
                    }
                }
                _ => {}
            }
        }

        let trimmed = page_text.trim();
        if !trimmed.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(trimmed);
        }
    }

    Ok(out)
}

fn collect_lossy_operands(text: &mut String, operands: &[Object]) {
    for operand in operands {
        match operand {
            Object::String(bytes, _) => text.push_str(&decode_lossy_pdf_text(bytes)),
            Object::Array(items) => collect_lossy_operands(text, items),
            Object::Integer(value) if *value < -100 => text.push(' '),
            Object::Real(value) if *value < -100.0 => text.push(' '),
            _ => {}
        }
    }
}

fn decode_lossy_pdf_text(bytes: &[u8]) -> String {
    if let Some(decoded) = decode_utf16_pdf_text(bytes) {
        return decoded;
    }

    String::from_utf8_lossy(bytes).replace('\0', "")
}

fn decode_utf16_pdf_text(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 || !bytes.len().is_multiple_of(2) {
        return None;
    }

    let utf16 = if bytes.starts_with(&[0xFE, 0xFF]) {
        bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>()
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>()
    } else if looks_like_utf16_be(bytes) {
        bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>()
    } else {
        return None;
    };

    String::from_utf16(&utf16).ok()
}

fn looks_like_utf16_be(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }

    let zero_high_bytes = bytes.iter().step_by(2).filter(|&&byte| byte == 0).count();
    zero_high_bytes * 2 >= bytes.len() / 2
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "unknown panic".to_string(),
        },
    }
}

fn normalize_text(s: &str) -> String {
    // Collapse runs of whitespace, keep paragraph boundaries (double newlines).
    let mut out = String::new();
    for para in s.split("\n\n") {
        let joined: String = para.split_whitespace().collect::<Vec<_>>().join(" ");
        if !joined.is_empty() {
            out.push_str(&joined);
            out.push_str("\n\n");
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};

    fn make_pdf(text: &str) -> Vec<u8> {
        let mut pdf = Pdf::new();
        let catalog_id = Ref::new(1);
        let page_tree_id = Ref::new(2);
        let page_id = Ref::new(3);
        let font_id = Ref::new(4);
        let content_id = Ref::new(5);

        pdf.catalog(catalog_id).pages(page_tree_id);
        pdf.pages(page_tree_id).kids([page_id]).count(1);

        let mut page = pdf.page(page_id);
        page.parent(page_tree_id)
            .media_box(Rect::new(0.0, 0.0, 595.0, 842.0))
            .resources()
            .fonts()
            .pair(Name(b"F1"), font_id);
        page.contents(content_id);
        page.finish();

        pdf.type1_font(font_id).base_font(Name(b"Helvetica"));

        let mut content = Content::new();
        content
            .begin_text()
            .set_font(Name(b"F1"), 12.0)
            .next_line(72.0, 770.0)
            .show(Str(text.as_bytes()))
            .end_text();
        pdf.stream(content_id, &content.finish());
        pdf.finish()
    }

    #[test]
    fn extracts_text_from_simple_pdf() {
        // Use a string longer than 32 chars so needs_llm_fallback is false.
        let bytes = make_pdf("Hello, knowledge base! This is a test PDF document.");
        let c = pdf_to_markdown(&bytes).unwrap();
        assert!(c.markdown.contains("Hello"), "got {:?}", c.markdown);
        assert!(!c.needs_llm_fallback);
    }

    #[test]
    fn flags_empty_pdf_for_llm_fallback() {
        let bytes = make_pdf("");
        let c = pdf_to_markdown(&bytes).unwrap();
        assert!(c.needs_llm_fallback);
    }

    #[test]
    fn falls_back_when_primary_panics() {
        let text = extract_pdf_text_with(
            b"ignored",
            |_| panic!("boom"),
            |_| Ok("fallback text".to_string()),
        )
        .unwrap();
        assert_eq!(text, "fallback text");
    }

    #[test]
    fn reports_both_extractors_when_everything_fails() {
        let err = extract_pdf_text_with(
            b"ignored",
            |_| Err(anyhow::anyhow!("primary failed")),
            |_| Err(anyhow::anyhow!("fallback failed")),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("primary failed"));
        assert!(msg.contains("fallback failed"));
    }

    #[test]
    fn decodes_utf16_pdf_text_with_bom() {
        let text = decode_lossy_pdf_text(&[0xFE, 0xFF, 0x00, b'H', 0x00, b'i']);
        assert_eq!(text, "Hi");
    }
}
