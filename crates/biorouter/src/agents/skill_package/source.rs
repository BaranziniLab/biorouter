//! Resolving what the user gave us into archive bytes.
//!
//! A repository URL, a direct archive URL, or a `.zip` already on this machine.
//! The point of doing it here rather than at each call site is that the
//! **repository URL** case is the one the interface, the CLI, the marketplace
//! and the agent all lacked, and giving four surfaces four ways to resolve one
//! would reproduce the divergence #115 is about.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::archive::{self, Entry, WrapperHint, MAX_ARCHIVE_BYTES};
use super::SourceProvenance;

/// Hosts an archive may be fetched from.
///
/// ⚠ **An allowlist, not a scheme check.** This runs inside the daemon, on the
/// user's machine, with the user's filesystem — a URL is an instruction to
/// fetch and then to *write what comes back*. The same list the marketplace
/// download path uses (`biorouter-server`'s `REGISTRY_DOWNLOAD_HOSTS`), so a
/// package source and a marketplace asset are held to one rule.
pub const ALLOWED_HOSTS: &[&str] = &[
    "biorouter.ucsf.edu",
    "github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    "codeload.github.com",
];

const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// What the caller wants imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSource {
    /// A repository page, a `/tree/<ref>` URL, or a direct archive URL.
    Url {
        url: String,
        /// Overrides any ref in the URL. `None` means the default branch.
        reference: Option<String>,
    },
    /// A `.zip` already on this machine.
    Archive { path: PathBuf },
}

/// A resolved archive: the files a plan can use (unwrapped; see
/// [`archive::read_package`]), the candidate names, and where it came from.
#[derive(Debug, Clone)]
pub struct FetchedArchive {
    pub entries: Vec<Entry>,
    /// Candidate package names in preference order.
    pub id_hints: Vec<String>,
    pub source: SourceProvenance,
}

/// A GitHub repository URL, decomposed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubRepo {
    owner: String,
    repo: String,
    reference: Option<String>,
}

/// Recognise the forms a user actually pastes.
///
/// `https://github.com/owner/repo`, with or without `.git`, with or without a
/// trailing slash, and `…/tree/<ref>` (including a ref containing slashes,
/// which `tree/release/1.2` is).
fn parse_github(url: &reqwest::Url) -> Option<GithubRepo> {
    if url.host_str()? != "github.com" {
        return None;
    }
    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    let owner = segments.next()?.to_string();
    let repo = segments.next()?.trim_end_matches(".git").to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    // ⚠ **An ALLOWLIST, because the default here decides what gets downloaded.**
    // Only `tree` and `commit` name a repository at a ref; a bare
    // `github.com/<owner>/<repo>` is the repository itself. Everything else on
    // github.com is a *direct artifact* — `/archive/…zip`, `/releases/download/…`,
    // `/raw/…`, `/blob/…` — and must be fetched verbatim.
    //
    // This used to fall through as `Some(_) => None`, i.e. "the repo at HEAD",
    // and only `archive` short-circuited. Every BAAM skill's download URL is
    // `github.com/<owner>/<repo>/releases/download/<tag>/<file>.zip`, so every
    // `installMarketplaceSkill` silently fetched
    // `codeload.github.com/<owner>/<repo>/zip/HEAD` — the WHOLE repository (65
    // skills) instead of the one named asset. It also defeated the marketplace's
    // own integrity contract: `validate_https_url` checks the URL ends in the
    // registry's declared `filename`, and then a different URL was fetched.
    let reference = match segments.next() {
        Some("tree") | Some("commit") => {
            let rest: Vec<&str> = segments.collect();
            (!rest.is_empty()).then(|| rest.join("/"))
        }
        None => None,
        Some(_) => return None,
    };
    Some(GithubRepo {
        owner,
        repo,
        reference,
    })
}

/// GitHub's source archive for a ref.
///
/// The wrapper directory it carries is deliberately *not* predicted here — see
/// [`WrapperHint::SourceArchive`] for why naming it is the wrong instinct.
fn github_archive(repo: &GithubRepo, reference: &str) -> String {
    format!(
        "https://codeload.github.com/{}/{}/zip/{}",
        repo.owner, repo.repo, reference
    )
}

fn check_host(url: &reqwest::Url) -> Result<()> {
    if url.scheme() != "https" {
        bail!(
            "only https sources are allowed, and this one is `{}`",
            url.scheme()
        );
    }
    let host = url.host_str().unwrap_or_default();
    if !ALLOWED_HOSTS.contains(&host) {
        bail!(
            "`{host}` is not one of the hosts Biorouter will install a skill package from ({})",
            ALLOWED_HOSTS.join(", ")
        );
    }
    Ok(())
}

async fn download(url: &str) -> Result<(Vec<u8>, Option<String>)> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .context("building the download client")?;
    let mut response = client
        .get(url)
        .header("User-Agent", "biorouter-skill-import")
        .send()
        .await
        .with_context(|| format!("fetching {url}"))?;
    if !response.status().is_success() {
        bail!("{url} returned {}", response.status());
    }
    // GitHub's ETag on a codeload archive is the commit, which is the only
    // immutable identity a branch download has.
    let commit = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_matches('"').trim_start_matches("W/").to_string())
        .filter(|value| value.len() >= 7 && value.chars().all(|c| c.is_ascii_hexdigit()));
    if let Some(length) = response.content_length() {
        if length > MAX_ARCHIVE_BYTES {
            bail!(archive::too_large_message(MAX_ARCHIVE_BYTES));
        }
    }
    // ⚠ Counted while it arrives. codeload sends no `Content-Length` for a
    // repository archive, so the check above never fires there, and reading
    // the whole body before counting it held a repository of any size in
    // memory first.
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("reading the archive")? {
        append_within(&mut bytes, &chunk, MAX_ARCHIVE_BYTES)?;
    }
    Ok((bytes, commit))
}

/// Append `chunk`, or refuse once the body would pass `limit`.
fn append_within(body: &mut Vec<u8>, chunk: &[u8], limit: u64) -> Result<()> {
    if (body.len() as u64).saturating_add(chunk.len() as u64) > limit {
        bail!(archive::too_large_message(limit));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

/// Resolve a source into normalised archive entries.
pub async fn fetch(source: &ImportSource) -> Result<FetchedArchive> {
    match source {
        ImportSource::Archive { path } => {
            let bytes =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            // No hint about the wrapper: a local zip carries no record of how
            // it was made, so the reader falls back to its reveal test.
            let (entries, wrapper) = archive::read_package(&bytes, WrapperHint::Infer)?;
            let mut id_hints = Vec::new();
            if let Some(stem) = archive_stem(path) {
                id_hints.push(stem);
            }
            if let Some(wrapper) = wrapper {
                id_hints.push(wrapper);
            }
            Ok(FetchedArchive {
                entries,
                id_hints,
                source: SourceProvenance {
                    installer: Some("archive".to_string()),
                    ..Default::default()
                },
            })
        }
        ImportSource::Url { url, reference } => {
            let parsed =
                reqwest::Url::parse(url).with_context(|| format!("`{url}` is not a URL"))?;
            check_host(&parsed)?;

            let repo = parse_github(&parsed);
            let (fetch_url, hint, resolved_ref, id_hint) = match &repo {
                Some(repo) => {
                    let reference = reference
                        .clone()
                        .or_else(|| repo.reference.clone())
                        .unwrap_or_else(|| "HEAD".to_string());
                    let path = if reference == "HEAD" {
                        "HEAD".to_string()
                    } else {
                        format!("refs/heads/{reference}")
                    };
                    (
                        github_archive(repo, &path),
                        WrapperHint::SourceArchive,
                        Some(reference),
                        Some(repo.repo.clone()),
                    )
                }
                None => (
                    url.clone(),
                    WrapperHint::Infer,
                    reference.clone(),
                    archive_stem(Path::new(parsed.path())),
                ),
            };

            let (bytes, commit) = match download(&fetch_url).await {
                Ok(result) => result,
                Err(error) if repo.is_some() && resolved_ref.as_deref() != Some("HEAD") => {
                    // A ref that is a tag rather than a branch, which
                    // `refs/heads/<ref>` does not reach. Tried second rather
                    // than probed first so the common case is one request.
                    let repo = repo.as_ref().expect("checked above");
                    let reference = resolved_ref.clone().unwrap_or_default();
                    let tag_url = github_archive(repo, &format!("refs/tags/{reference}"));
                    download(&tag_url).await.map_err(|_| error)?
                }
                Err(error) => return Err(error),
            };

            let (entries, wrapper) = archive::read_package(&bytes, hint)?;
            let mut id_hints = Vec::new();
            if let Some(hint) = id_hint {
                id_hints.push(hint);
            }
            if let Some(wrapper) = wrapper {
                id_hints.push(wrapper);
            }
            Ok(FetchedArchive {
                entries,
                id_hints,
                source: SourceProvenance {
                    url: Some(url.clone()),
                    reference: resolved_ref.filter(|r| r != "HEAD"),
                    resolved_commit: commit,
                    installer: Some(
                        if repo.is_some() {
                            "repository"
                        } else {
                            "archive"
                        }
                        .to_string(),
                    ),
                },
            })
        }
    }
}

fn archive_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.trim_end_matches(".tar").to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> reqwest::Url {
        reqwest::Url::parse(raw).unwrap()
    }

    #[test]
    fn a_pasted_repository_url_is_recognised_in_the_forms_people_paste() {
        for raw in [
            "https://github.com/heygen-com/hyperframes",
            "https://github.com/heygen-com/hyperframes/",
            "https://github.com/heygen-com/hyperframes.git",
        ] {
            let repo = parse_github(&url(raw)).unwrap_or_else(|| panic!("{raw}"));
            assert_eq!(repo.owner, "heygen-com");
            assert_eq!(repo.repo, "hyperframes");
            assert_eq!(repo.reference, None);
        }
    }

    #[test]
    fn a_tree_url_carries_its_ref_including_one_with_slashes() {
        let repo = parse_github(&url("https://github.com/o/r/tree/release/1.2")).unwrap();
        assert_eq!(repo.reference.as_deref(), Some("release/1.2"));
    }

    #[test]
    fn a_repository_is_fetched_from_codeload_at_the_requested_ref() {
        let repo = GithubRepo {
            owner: "heygen-com".into(),
            repo: "hyperframes".into(),
            reference: None,
        };
        assert_eq!(
            github_archive(&repo, "refs/heads/main"),
            "https://codeload.github.com/heygen-com/hyperframes/zip/refs/heads/main"
        );
        assert_eq!(
            github_archive(&repo, "refs/tags/v1.0"),
            "https://codeload.github.com/heygen-com/hyperframes/zip/refs/tags/v1.0"
        );
    }

    #[test]
    fn a_host_outside_the_allowlist_is_refused_before_anything_is_fetched() {
        let err = check_host(&url("https://example.com/x.zip")).unwrap_err();
        assert!(err.to_string().contains("not one of the hosts"));
        let err = check_host(&url("http://github.com/o/r")).unwrap_err();
        assert!(err.to_string().contains("only https"));
        assert!(check_host(&url("https://github.com/o/r")).is_ok());
    }

    #[test]
    fn a_direct_archive_url_is_not_mistaken_for_a_repository_page() {
        assert_eq!(
            parse_github(&url("https://github.com/o/r/archive/refs/heads/main.zip")),
            None
        );
    }

    /// Every BAAM skill asset is a GitHub **release asset**, and reading one as
    /// a repository reference substituted `codeload …/zip/HEAD` — the whole
    /// repository — for the one named `.zip`. That is not a near miss: the
    /// `biorouter-skills` repo holds 65 skills, so asking to install
    /// `anti-ai-writing` returned a 65-way ambiguity prompt instead of a
    /// 2,683-byte skill.
    ///
    /// Asserted as a family, not a single case: `releases` was only the first
    /// direct-artifact path to reach the fall-through, and `raw` and `blob`
    /// reach it the same way.
    #[test]
    fn a_direct_artifact_url_is_never_resolved_to_its_repository() {
        for direct in [
            "https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-anti-ai-writing/anti-ai-writing.zip",
            "https://github.com/o/r/releases/latest",
            "https://github.com/o/r/raw/main/skill.zip",
            "https://github.com/o/r/blob/main/SKILL.md",
        ] {
            assert_eq!(parse_github(&url(direct)), None, "{direct}");
        }
    }

    /// The other half of the same rule: a real repository reference still
    /// resolves, so tightening the match did not cost the repo-import path.
    #[test]
    fn a_repository_reference_still_resolves_to_its_source_archive() {
        let bare = parse_github(&url("https://github.com/heygen-com/hyperframes")).unwrap();
        assert_eq!(bare.repo, "hyperframes");
        assert_eq!(bare.reference, None);

        let at_ref = parse_github(&url("https://github.com/o/r/tree/some-branch")).unwrap();
        assert_eq!(at_ref.reference.as_deref(), Some("some-branch"));

        let at_commit = parse_github(&url("https://github.com/o/r/commit/abc123")).unwrap();
        assert_eq!(at_commit.reference.as_deref(), Some("abc123"));
    }

    /// A download over the limit is refused while it arrives, with the same
    /// sentence the reader gives, not after the whole body is held.
    #[test]
    fn a_download_is_refused_as_it_passes_the_limit_with_the_one_sentence() {
        let mut body = Vec::new();
        append_within(&mut body, &[0; 600], 1024).unwrap();
        append_within(&mut body, &[0; 424], 1024).expect("exactly the limit fits");
        let error = append_within(&mut body, &[0; 1], 1024).unwrap_err();
        assert_eq!(error.to_string(), archive::too_large_message(1024));
        assert_eq!(body.len(), 1024, "the refused chunk was not kept");
    }
}
