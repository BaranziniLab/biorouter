use crate::knowledge::types::{Credibility, CredibilityTier};

pub fn classify_url(url: &str) -> Option<Credibility> {
    let host = host_of(url)?;
    if is_preprint_host(&host) {
        return Some(make(
            CredibilityTier::Preprint,
            0.9,
            "Host is a recognized preprint server.",
            Some(&host),
        ));
    }
    if is_gray_lit_host(&host) {
        return Some(make(
            CredibilityTier::GrayLit,
            0.8,
            "Host is governmental / institutional / standards body.",
            Some(&host),
        ));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(make(
            CredibilityTier::Web,
            0.6,
            "Generic web URL with no recognized academic provenance.",
            Some(&host),
        ));
    }
    None
}

fn make(tier: CredibilityTier, confidence: f32, reason: &str, host: Option<&str>) -> Credibility {
    Credibility {
        tier,
        confidence,
        publisher: host.map(String::from),
        venue: None,
        doi: None,
        retracted: false,
        reasoning: reason.to_string(),
        classifier_version: 1,
    }
}

fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = after_scheme.split('/').next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

fn is_preprint_host(host: &str) -> bool {
    [
        "arxiv.org",
        "biorxiv.org",
        "medrxiv.org",
        "chemrxiv.org",
        "ssrn.com",
        "preprints.org",
        "researchsquare.com",
        "osf.io",
        "psyarxiv.com",
    ]
    .iter()
    .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

fn is_gray_lit_host(host: &str) -> bool {
    if host.ends_with(".gov") || host.ends_with(".edu") {
        return true;
    }
    [
        "who.int",
        "cdc.gov",
        "nih.gov",
        "fda.gov",
        "clinicaltrials.gov",
        "europa.eu",
        "ema.europa.eu",
        "ietf.org",
        "rfc-editor.org",
        "iso.org",
        "oecd.org",
    ]
    .iter()
    .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arxiv_is_preprint() {
        let c = classify_url("https://arxiv.org/abs/2403.12345").unwrap();
        assert_eq!(c.tier, CredibilityTier::Preprint);
    }

    #[test]
    fn biorxiv_subdomain_is_preprint() {
        let c = classify_url("https://www.biorxiv.org/content/x").unwrap();
        assert_eq!(c.tier, CredibilityTier::Preprint);
    }

    #[test]
    fn nih_is_gray_lit() {
        let c = classify_url("https://www.nih.gov/news/x").unwrap();
        assert_eq!(c.tier, CredibilityTier::GrayLit);
    }

    #[test]
    fn random_blog_is_web() {
        let c = classify_url("https://someblog.example/post").unwrap();
        assert_eq!(c.tier, CredibilityTier::Web);
    }

    #[test]
    fn non_http_returns_none() {
        assert!(classify_url("file:///tmp/x.pdf").is_none());
    }
}
