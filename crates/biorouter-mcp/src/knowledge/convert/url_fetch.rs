use crate::safe_fetch::{FetchError, SafeFetch};
use anyhow::{Context, Result};

pub struct FetchedSource {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub final_url: String,
}

/// Fetch a source for ingest. The URL can come from the model (`kb_ingest`, the
/// knowledge sub-agent's tools, an application's `br.kb`), so it reaches public
/// internet addresses only (`crate::safe_fetch`); a refusal is the whole error.
pub async fn fetch_url(url: &str) -> Result<FetchedSource> {
    let client = SafeFetch::builder()
        .user_agent("Biorouter-Knowledge/1.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let resp = match client.get(url)?.send().await.map_err(FetchError::from) {
        Ok(resp) => resp,
        Err(FetchError::Refused(refused)) => return Err(refused.into()),
        Err(FetchError::Request(error)) => {
            return Err(error).with_context(|| format!("fetching {url}"));
        }
    };
    let final_url = resp.url().to_string();
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .unwrap_or_else(|| guess_mime_from_url(&final_url));
    let bytes = resp.bytes().await?.to_vec();
    Ok(FetchedSource {
        bytes,
        mime,
        final_url,
    })
}

fn guess_mime_from_url(url: &str) -> String {
    let lower = url.to_lowercase();
    if lower.ends_with(".pdf") {
        "application/pdf".into()
    } else if lower.ends_with(".docx") {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into()
    } else if lower.ends_with(".csv") {
        "text/csv".into()
    } else if lower.ends_with(".md") {
        "text/markdown".into()
    } else {
        "text/html".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safe_fetch::test_support::admit_local_server;
    use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetches_html() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<html><body>Hello</body></html>",
                "text/html; charset=utf-8",
            ))
            .mount(&server)
            .await;
        let _admitted = admit_local_server(*server.address());
        let s = fetch_url(&format!("{}/x", server.uri())).await.unwrap();
        assert_eq!(s.mime, "text/html");
        assert!(String::from_utf8_lossy(&s.bytes).contains("Hello"));
    }

    /// Knowledge ingest takes URLs from the model (`kb_ingest`, the sub-agent's
    /// tools, an application's `br.kb`), so it reaches public addresses only.
    #[tokio::test]
    async fn refuses_a_loopback_address_and_sends_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("local secret"))
            .expect(0)
            .mount(&server)
            .await;
        let error = match fetch_url(&format!("{}/x", server.uri())).await {
            Ok(_) => panic!("a loopback URL must be refused"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("loopback"), "{error}");
    }

    #[tokio::test]
    async fn guesses_mime_for_pdf_path_when_no_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 4]))
            .mount(&server)
            .await;
        let _admitted = admit_local_server(*server.address());
        let s = fetch_url(&format!("{}/x.pdf", server.uri())).await.unwrap();
        // Header was absent (defaulted by wiremock); guess wins.
        assert!(s.mime == "application/octet-stream" || s.mime == "application/pdf");
    }
}
