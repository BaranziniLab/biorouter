pub mod csv;
pub mod docx;
pub mod html;
pub mod note;
pub mod pdf;
pub mod pptx;
pub mod spreadsheet;
pub mod url_fetch;

use anyhow::Result;

#[derive(Debug, Clone)]
pub enum SourceInput {
    File {
        bytes: Vec<u8>,
        filename: String,
        mime: Option<String>,
    },
    Path(std::path::PathBuf),
    Url(String),
    Text {
        text: String,
        title: Option<String>,
    },
}

/// A source no converter could turn into text: a corrupt or encrypted PDF, a
/// file of a type nothing here reads, a workbook with no cells.
///
/// It is the person's file that is wrong, not the daemon, so
/// `POST /knowledge/bases/{id}/raw` answers 422 for it rather than 500, and its
/// message is written for that person: it says the source could not be read,
/// then gives the converter's reason in full (rq2-029). Fetching a URL and
/// reading a local path are not conversion and are not marked.
#[derive(Debug)]
pub struct UnreadableSource {
    reason: String,
}

impl UnreadableSource {
    /// Does `error`, or anything it wraps, say the source itself was unreadable?
    pub fn is(error: &anyhow::Error) -> bool {
        error.downcast_ref::<Self>().is_some() || error.chain().any(|cause| cause.is::<Self>())
    }
}

impl std::fmt::Display for UnreadableSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Could not read this source: {}", self.reason)
    }
}

impl std::error::Error for UnreadableSource {}

#[derive(Debug, Clone)]
pub struct Converted {
    pub markdown: String,
    pub title: Option<String>,
    pub mime: String,
    pub needs_llm_fallback: bool,
}

fn sniff_html(bytes: &[u8]) -> bool {
    let prefix = String::from_utf8_lossy(bytes);
    let trimmed = prefix.trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<head")
        || trimmed.starts_with("<body")
        || trimmed.starts_with("<main")
        || trimmed.starts_with("<article")
}

fn normalize_mime(filename: &str, mime: Option<&str>, bytes: &[u8]) -> String {
    let guessed = guess_mime(filename);
    let base = mime
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| guessed.clone());

    match base.as_str() {
        "application/octet-stream" | "binary/octet-stream" | "application/binary" => {
            if bytes.starts_with(b"%PDF-") {
                "application/pdf".into()
            } else if sniff_html(bytes) {
                "text/html".into()
            } else {
                guessed
            }
        }
        "text/x-markdown" | "application/markdown" => "text/markdown".into(),
        "text/x-csv" | "application/csv" => "text/csv".into(),
        // Legacy quirk: many servers label CSV downloads with the old Excel
        // MIME. Only treat it as a real .xls workbook when the filename says
        // so; otherwise fall back to CSV like before.
        "application/vnd.ms-excel" => {
            if std::path::Path::new(filename)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xls"))
            {
                "application/vnd.ms-excel".into()
            } else {
                "text/csv".into()
            }
        }
        "application/xhtml+xml" => "application/xhtml+xml".into(),
        "text/plain" if sniff_html(bytes) => "text/html".into(),
        other => other.to_string(),
    }
}

pub async fn convert(input: &SourceInput) -> Result<Converted> {
    match input {
        SourceInput::Text { text, title } => Ok(Converted {
            markdown: text.clone(),
            title: title.clone(),
            mime: "text/plain".into(),
            needs_llm_fallback: false,
        }),
        SourceInput::Url(url) => {
            let fetched = url_fetch::fetch_url(url).await?;
            convert_file_async(
                fetched.bytes,
                filename_from_url(&fetched.final_url),
                Some(fetched.mime),
            )
            .await
        }
        SourceInput::Path(path) => {
            let bytes = tokio::fs::read(path).await?;
            let filename = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("source")
                .to_string();
            convert_file_async(bytes, filename, None).await
        }
        SourceInput::File {
            bytes,
            filename,
            mime,
        } => convert_file_async(bytes.clone(), filename.clone(), mime.clone()).await,
    }
}

async fn convert_file_async(
    bytes: Vec<u8>,
    filename: String,
    mime: Option<String>,
) -> Result<Converted> {
    tokio::task::spawn_blocking(move || convert_file(&bytes, &filename, mime.as_deref()))
        .await
        .map_err(|error| anyhow::anyhow!("source conversion task failed: {error}"))?
}

/// Every failure past this point is about the bytes themselves, so each one is
/// marked [`UnreadableSource`], carrying the converter's whole cause chain.
fn convert_file(bytes: &[u8], filename: &str, mime: Option<&str>) -> Result<Converted> {
    convert_bytes(bytes, filename, mime).map_err(|error| {
        UnreadableSource {
            reason: format!("{error:#}"),
        }
        .into()
    })
}

fn convert_bytes(bytes: &[u8], filename: &str, mime: Option<&str>) -> Result<Converted> {
    let effective_mime = normalize_mime(filename, mime, bytes);
    match effective_mime.as_str() {
        "text/html" | "application/xhtml+xml" => {
            let s = String::from_utf8_lossy(bytes);
            let c = html::html_to_markdown(&s)?;
            Ok(Converted {
                markdown: c.markdown,
                title: c.title,
                mime: effective_mime,
                needs_llm_fallback: false,
            })
        }
        "application/pdf" => {
            let c = pdf::pdf_to_markdown(bytes)?;
            Ok(Converted {
                markdown: c.markdown,
                title: c.title,
                mime: effective_mime,
                needs_llm_fallback: c.needs_llm_fallback,
            })
        }
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            let md = docx::docx_to_markdown(bytes)?;
            Ok(Converted {
                markdown: md,
                title: None,
                mime: effective_mime,
                needs_llm_fallback: false,
            })
        }
        "text/csv" => {
            let md = csv::csv_to_markdown(bytes)?;
            Ok(Converted {
                markdown: md,
                title: None,
                mime: effective_mime,
                needs_llm_fallback: false,
            })
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.ms-excel.sheet.macroenabled.12"
        | "application/vnd.ms-excel"
        | "application/vnd.oasis.opendocument.spreadsheet" => {
            let md = spreadsheet::spreadsheet_to_markdown(bytes)?;
            Ok(Converted {
                markdown: md,
                title: None,
                mime: effective_mime,
                needs_llm_fallback: false,
            })
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            let c = pptx::pptx_to_markdown(bytes)?;
            Ok(Converted {
                markdown: c.markdown,
                title: c.title,
                mime: effective_mime,
                needs_llm_fallback: false,
            })
        }
        "application/vnd.ms-powerpoint" => anyhow::bail!(
            "legacy .ppt files are not supported; please re-save the deck as .pptx and ingest that"
        ),
        "text/markdown" | "text/plain" => Ok(Converted {
            markdown: String::from_utf8_lossy(bytes).into_owned(),
            title: None,
            mime: effective_mime,
            needs_llm_fallback: false,
        }),
        other => anyhow::bail!("unsupported mime: {other}"),
    }
}

fn filename_from_url(url: &str) -> String {
    url.split('/').next_back().unwrap_or("source").to_string()
}

fn guess_mime(filename: &str) -> String {
    let lower = filename.to_lowercase();
    if lower.ends_with(".pdf") {
        "application/pdf".into()
    } else if lower.ends_with(".docx") {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into()
    } else if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into()
    } else if lower.ends_with(".xls") {
        "application/vnd.ms-excel".into()
    } else if lower.ends_with(".ods") {
        "application/vnd.oasis.opendocument.spreadsheet".into()
    } else if lower.ends_with(".pptx") {
        "application/vnd.openxmlformats-officedocument.presentationml.presentation".into()
    } else if lower.ends_with(".ppt") {
        "application/vnd.ms-powerpoint".into()
    } else if lower.ends_with(".csv") {
        "text/csv".into()
    } else if lower.ends_with(".md") {
        "text/markdown".into()
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html".into()
    } else {
        "text/plain".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatches_html() {
        let html = "<html><head><title>T</title></head><body><h1>H</h1></body></html>";
        let c = convert(&SourceInput::File {
            bytes: html.as_bytes().to_vec(),
            filename: "x.html".into(),
            mime: Some("text/html".into()),
        })
        .await
        .unwrap();
        assert!(c.markdown.contains("# H"));
        assert_eq!(c.title.as_deref(), Some("T"));
    }

    #[tokio::test]
    async fn dispatches_text_passthrough() {
        let c = convert(&SourceInput::Text {
            text: "hello".into(),
            title: Some("Note".into()),
        })
        .await
        .unwrap();
        assert_eq!(c.markdown, "hello");
        assert_eq!(c.title.as_deref(), Some("Note"));
    }

    #[tokio::test]
    async fn convert_html_with_invalid_utf8_bytes_succeeds_via_lossy_decode() {
        // 0xFF is not a valid UTF-8 byte. With std::str::from_utf8 this would error;
        // with String::from_utf8_lossy the bad byte gets replaced with U+FFFD and
        // conversion continues normally.
        let mut bytes = b"<html><body><p>Hello".to_vec();
        bytes.push(0xFF);
        bytes.extend_from_slice(b" world</p></body></html>");
        let c = convert(&SourceInput::File {
            bytes,
            filename: "x.html".into(),
            mime: Some("text/html".into()),
        })
        .await
        .unwrap();
        assert!(
            c.markdown.contains("Hello"),
            "markdown missing 'Hello': {}",
            c.markdown
        );
        assert!(
            c.markdown.contains("world"),
            "markdown missing 'world': {}",
            c.markdown
        );
    }

    #[tokio::test]
    async fn dispatches_html_with_charset_parameter() {
        let html = "<html><body><h1>Hello</h1></body></html>";
        let c = convert(&SourceInput::File {
            bytes: html.as_bytes().to_vec(),
            filename: "page".into(),
            mime: Some("text/html; charset=utf-8".into()),
        })
        .await
        .unwrap();
        assert!(c.markdown.contains("# Hello"));
        assert_eq!(c.mime, "text/html");
    }

    /// rq2-029. A corrupt file is marked as the person's input, and the mark
    /// survives the context a caller adds, which is how a route tells a 422
    /// from a 500.
    #[tokio::test]
    async fn a_corrupt_pdf_is_an_unreadable_source_that_names_its_cause() {
        let error = convert(&SourceInput::File {
            bytes: b"%PDF-1.7\nthis is not a pdf body at all\n%%EOF\n".to_vec(),
            filename: "broken.pdf".into(),
            mime: Some("application/pdf".into()),
        })
        .await
        .expect_err("a corrupt PDF does not convert");
        assert!(UnreadableSource::is(&error), "{error:#}");
        let message = error.to_string();
        assert!(
            message.starts_with("Could not read this source: ")
                && message.contains("PDF text extraction failed"),
            "{message}"
        );
        let wrapped = error.context("staging");
        assert!(UnreadableSource::is(&wrapped), "{wrapped:#}");
    }

    /// A failure that is not about the bytes is not marked, so it stays a 500.
    #[tokio::test]
    async fn a_missing_local_path_is_not_an_unreadable_source() {
        let error = convert(&SourceInput::Path(std::path::PathBuf::from(
            "/nonexistent/biorouter-l3/source.pdf",
        )))
        .await
        .expect_err("a missing path does not convert");
        assert!(!UnreadableSource::is(&error), "{error:#}");
    }

    #[tokio::test]
    async fn dispatches_html_from_octet_stream_when_sniffed() {
        let html = "<html><body><h1>Hello</h1></body></html>";
        let c = convert(&SourceInput::File {
            bytes: html.as_bytes().to_vec(),
            filename: "upload.bin".into(),
            mime: Some("application/octet-stream".into()),
        })
        .await
        .unwrap();
        assert!(c.markdown.contains("# Hello"));
        assert_eq!(c.mime, "text/html");
    }
}
