use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use url::Url;

use crate::catalog_search::{
    names_only_the_license, rank, CatalogSearch, Weight, EXTENSION_NOISE, SKILL_NOISE,
};
use crate::config::paths::Paths;
use crate::privacy::affiliation::InstitutionId;
use crate::privacy::{ExtensionAffiliation, ProviderTier};

pub const REGISTRY_URL: &str = "https://biorouter.ucsf.edu/registry.json";
const REGISTRY_SOURCE: &str = "https://biorouter.ucsf.edu/baam";
pub const MAX_REGISTRY_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 4_096;
const MAX_INSTITUTIONS: usize = 64;
const MAX_AFFILIATIONS: usize = 32;
const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const EMBEDDED_REGISTRY: &[u8] = include_bytes!("../../../landing/registry.json");
const ASSET_HOSTS: &[&str] = &[
    "biorouter.ucsf.edu",
    "github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    "codeload.github.com",
];

#[derive(Debug, thiserror::Error)]
pub enum MarketplaceError {
    #[error("marketplace registry is invalid: {0}")]
    InvalidRegistry(String),
    #[error("marketplace registry is {actual} bytes; limit is {limit} bytes")]
    RegistryTooLarge { actual: usize, limit: usize },
    #[error("extension `{id}` is not in the trusted marketplace registry")]
    UnknownExtension { id: String },
    #[error("extension `{id}` is unavailable to this caller")]
    ExtensionUnavailableForCaller { id: String },
    #[error("skill `{id}` is not in the trusted marketplace registry")]
    UnknownSkill { id: String },
    #[error("marketplace registry fetch failed: {0}")]
    Fetch(String),
    #[error("marketplace cache failed: {0}")]
    Cache(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketplaceExtensionDescriptor {
    pub registry_id: String,
    pub extension_name: String,
    /// Whether the registry actually stated `extension_name`, as opposed to
    /// `extension_name` being the registry id by fallback. Only a stated name
    /// is advertised to the model.
    pub advertises_name: bool,
    pub name: String,
    pub organization: String,
    pub version: String,
    pub description: String,
    pub tags: Vec<String>,
    pub download_url: Url,
    pub filename: String,
    pub license: String,
    pub privacy: ProviderTier,
    pub affiliation: ExtensionAffiliation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketplaceSkillDescriptor {
    pub registry_id: String,
    pub name: String,
    pub category: String,
    pub skill_type: String,
    pub description: String,
    pub tags: Vec<String>,
    pub keywords: Vec<String>,
    pub download_url: Url,
    pub filename: String,
    pub license: String,
}

#[derive(Clone, Debug)]
pub struct MarketplaceCatalog {
    extensions: BTreeMap<String, MarketplaceExtensionDescriptor>,
    skills: BTreeMap<String, MarketplaceSkillDescriptor>,
}

impl MarketplaceCatalog {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MarketplaceError> {
        if bytes.len() > MAX_REGISTRY_BYTES {
            return Err(MarketplaceError::RegistryTooLarge {
                actual: bytes.len(),
                limit: MAX_REGISTRY_BYTES,
            });
        }
        let raw: RawRegistry = serde_json::from_slice(bytes)
            .map_err(|error| MarketplaceError::InvalidRegistry(error.to_string()))?;
        raw.validate()
    }

    pub fn browse_extensions(&self, caller: ProviderTier) -> Vec<&MarketplaceExtensionDescriptor> {
        self.extensions
            .values()
            .filter(|entry| caller.is_private() || entry.privacy == ProviderTier::Public)
            .collect()
    }

    /// Rank the extensions visible to `caller` against a free-text query. How a
    /// query is matched — and why a phrase is a union of its words rather than
    /// one substring (finding F5) — is documented in `catalog_search.rs`.
    ///
    /// The caller filter runs FIRST, so an extension hidden from `caller` is
    /// never scored and cannot move, or be counted among, what it is shown.
    pub fn search_extensions(
        &self,
        caller: ProviderTier,
        query: &str,
    ) -> CatalogSearch<'_, MarketplaceExtensionDescriptor> {
        rank(
            query,
            EXTENSION_NOISE,
            self.browse_extensions(caller),
            |entry| {
                let mut fields = vec![
                    (entry.registry_id.as_str(), Weight::Name),
                    (entry.extension_name.as_str(), Weight::Name),
                    (entry.name.as_str(), Weight::Name),
                    (entry.organization.as_str(), Weight::Label),
                    (entry.description.as_str(), Weight::Prose),
                ];
                fields.extend(
                    entry
                        .tags
                        .iter()
                        .filter(|tag| !names_only_the_license(tag, &entry.license))
                        .map(|tag| (tag.as_str(), Weight::Label)),
                );
                fields
            },
        )
    }

    pub fn browse_skills(&self) -> Vec<&MarketplaceSkillDescriptor> {
        self.skills.values().collect()
    }

    /// Rank every skill against a free-text query, matched as documented in
    /// `catalog_search.rs`.
    pub fn search_skills(&self, query: &str) -> CatalogSearch<'_, MarketplaceSkillDescriptor> {
        rank(query, SKILL_NOISE, self.skills.values(), |entry| {
            let mut fields = vec![
                (entry.registry_id.as_str(), Weight::Name),
                (entry.name.as_str(), Weight::Name),
                (entry.category.as_str(), Weight::Label),
                (entry.description.as_str(), Weight::Prose),
            ];
            fields.extend(
                entry
                    .tags
                    .iter()
                    .filter(|tag| !names_only_the_license(tag, &entry.license))
                    .map(|tag| (tag.as_str(), Weight::Label)),
            );
            fields.extend(
                entry
                    .keywords
                    .iter()
                    .filter(|keyword| !names_only_the_license(keyword, &entry.license))
                    .map(|keyword| (keyword.as_str(), Weight::Label)),
            );
            fields
        })
    }

    pub fn resolve_extension_for_install(
        &self,
        id: &str,
        caller: ProviderTier,
    ) -> Result<&MarketplaceExtensionDescriptor, MarketplaceError> {
        // Install authorization is absolute: the diagnostic privacy switch may
        // relax ordinary data-flow gates, but it cannot confer marketplace
        // authority on a public model.
        let Some(entry) = self.extensions.get(id) else {
            return Err(if caller.is_private() {
                MarketplaceError::UnknownExtension { id: id.to_owned() }
            } else {
                // A public caller sees the complete public catalog. Collapse an
                // unknown id onto the same answer as a hidden private id so
                // exact-id preflight is not a private catalog membership oracle.
                MarketplaceError::ExtensionUnavailableForCaller { id: id.to_owned() }
            });
        };
        if !caller.is_private() && entry.privacy.is_private() {
            return Err(MarketplaceError::ExtensionUnavailableForCaller { id: id.to_owned() });
        }
        Ok(entry)
    }

    pub fn resolve_skill_for_install(
        &self,
        id: &str,
    ) -> Result<&MarketplaceSkillDescriptor, MarketplaceError> {
        self.skills
            .get(id)
            .ok_or_else(|| MarketplaceError::UnknownSkill { id: id.to_owned() })
    }

    fn raise_daemon_privacy_authority(&self) -> Result<(), MarketplaceError> {
        crate::privacy::registry_live::raise_private_extensions(
            self.extensions
                .values()
                .filter(|entry| entry.privacy.is_private())
                .map(|entry| {
                    let affiliation = match &entry.affiliation {
                        ExtensionAffiliation::Any => None,
                        ExtensionAffiliation::Institutions(ids) => Some(
                            ids.iter()
                                .map(|id| id.as_str().to_owned())
                                .collect::<BTreeSet<_>>(),
                        ),
                    };
                    (
                        entry.registry_id.clone(),
                        entry.extension_name.clone(),
                        affiliation,
                    )
                }),
        )
        .map_err(MarketplaceError::Cache)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRegistry {
    version: u64,
    source: String,
    institutions: BTreeMap<String, String>,
    extensions: Vec<RawExtension>,
    skills: Vec<RawSkill>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExtension {
    id: String,
    name: String,
    organization: String,
    version: String,
    description: String,
    tags: Vec<String>,
    github: String,
    download: String,
    filename: String,
    license: String,
    privacy: ProviderTier,
    extension_name: Option<String>,
    affiliation: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSkill {
    id: String,
    name: String,
    category: String,
    #[serde(rename = "type")]
    skill_type: String,
    description: String,
    tags: Vec<String>,
    keywords: Vec<String>,
    download: String,
    filename: String,
    license: String,
}

impl RawRegistry {
    fn validate(self) -> Result<MarketplaceCatalog, MarketplaceError> {
        self.validate_metadata()?;
        let institutions = validate_institutions(&self.institutions)?;

        let mut extensions = BTreeMap::new();
        let mut extension_names = BTreeSet::new();
        for raw in self.extensions {
            let id = raw.id.clone();
            let entry = validate_extension(raw, &institutions, &mut extension_names)?;
            if extensions.insert(id.clone(), entry).is_some() {
                return invalid(format!("duplicate extension id `{id}`"));
            }
        }

        let mut skills = BTreeMap::new();
        for raw in self.skills {
            let id = raw.id.clone();
            let entry = validate_skill(raw)?;
            if skills.insert(id.clone(), entry).is_some() {
                return invalid(format!("duplicate skill id `{id}`"));
            }
        }

        Ok(MarketplaceCatalog { extensions, skills })
    }

    fn validate_metadata(&self) -> Result<(), MarketplaceError> {
        if self.version != 2 {
            return invalid(format!("unsupported version {}", self.version));
        }
        if self.source != REGISTRY_SOURCE {
            return invalid("unexpected registry source");
        }
        if self.institutions.len() > MAX_INSTITUTIONS {
            return invalid("too many institutions");
        }
        if self.extensions.len() + self.skills.len() > MAX_ENTRIES {
            return invalid("too many registry entries");
        }
        Ok(())
    }
}

fn validate_institutions(
    institutions: &BTreeMap<String, String>,
) -> Result<BTreeSet<String>, MarketplaceError> {
    let mut validated = BTreeSet::new();
    for (id, display_name) in institutions {
        validate_id(id, "institution id")?;
        validate_text(display_name, "institution name")?;
        validated.insert(id.clone());
    }
    Ok(validated)
}

fn validate_extension(
    raw: RawExtension,
    institutions: &BTreeSet<String>,
    extension_names: &mut BTreeSet<String>,
) -> Result<MarketplaceExtensionDescriptor, MarketplaceError> {
    validate_id(&raw.id, "extension id")?;
    validate_text(&raw.name, "extension name")?;
    validate_text(&raw.organization, "extension organization")?;
    validate_text(&raw.version, "extension version")?;
    validate_text(&raw.description, "extension description")?;
    validate_text(&raw.license, "extension license")?;
    validate_text_list(&raw.tags, "extension tags")?;
    validate_https_url(&raw.github, None)?;
    if !raw.filename.ends_with(".brxt") {
        return invalid(format!(
            "extension `{}` asset must be a .brxt bundle",
            raw.id
        ));
    }
    let download_url = validate_https_url(&raw.download, Some(&raw.filename))?;
    // ⚠ **The fallback is a GUESS, and it was advertised as fact.** With no
    // `extension_name` the descriptor took the registry id — so SPOKEAgent, whose
    // id carried its version, was advertised to the model as
    // `extensionName: "spokeagent-0.4.1"` while the extension that actually
    // installs is `spokeagent` (the name comes from the downloaded bundle's
    // manifest). A model that passed the advertised name to `manage_extensions`
    // got "not found". The fallback survives because it is what the uniqueness
    // check and the known-authority raise key on, but `advertises_name` records
    // that it was inferred, and `marketplace_descriptor_json` omits the field
    // rather than stating a name nobody verified.
    let has_extension_name = raw.extension_name.is_some();
    let extension_name = raw.extension_name.unwrap_or_else(|| raw.id.clone());
    validate_id(&extension_name, "extension runtime name")?;
    if !extension_names.insert(extension_name.clone()) {
        return invalid(format!(
            "duplicate extension runtime name `{extension_name}`"
        ));
    }
    if raw.privacy == ProviderTier::Public && raw.affiliation.is_some() {
        return invalid(format!(
            "public extension `{}` cannot declare affiliation",
            raw.id
        ));
    }
    if raw.privacy == ProviderTier::Private && !has_extension_name {
        return invalid(format!(
            "private extension `{}` requires extension_name",
            raw.id
        ));
    }
    let affiliation = validate_affiliation(&raw.id, raw.affiliation, institutions)?;
    let (privacy, affiliation) =
        raise_with_known_authority(raw.privacy, affiliation, [&raw.id, &extension_name]);
    Ok(MarketplaceExtensionDescriptor {
        registry_id: raw.id,
        extension_name,
        advertises_name: has_extension_name,
        name: raw.name,
        organization: raw.organization,
        version: raw.version,
        description: raw.description,
        tags: raw.tags,
        download_url,
        filename: raw.filename,
        license: raw.license,
        privacy,
        affiliation,
    })
}

fn validate_affiliation(
    extension_id: &str,
    affiliation: Option<Vec<String>>,
    institutions: &BTreeSet<String>,
) -> Result<ExtensionAffiliation, MarketplaceError> {
    let Some(ids) = affiliation else {
        return Ok(ExtensionAffiliation::Any);
    };
    if ids.len() > MAX_AFFILIATIONS {
        return invalid(format!("too many affiliations for `{extension_id}`"));
    }
    let mut validated = BTreeSet::new();
    for id in ids {
        validate_id(&id, "affiliation")?;
        if !institutions.contains(&id) {
            return invalid(format!(
                "extension `{extension_id}` names unknown institution `{id}`"
            ));
        }
        validated.insert(InstitutionId::new(&id));
    }
    Ok(ExtensionAffiliation::Institutions(validated))
}

fn raise_with_known_authority(
    mut privacy: ProviderTier,
    mut affiliation: ExtensionAffiliation,
    identities: [&str; 2],
) -> (ProviderTier, ExtensionAffiliation) {
    for identity in identities {
        let known = crate::privacy::resolve_extension(identity, None);
        if known.tier.is_private() {
            privacy = ProviderTier::Private;
            affiliation = crate::privacy::affiliation::restrict_extension_affiliation(
                affiliation,
                known.affiliation,
            );
        }
    }
    (privacy, affiliation)
}

fn validate_skill(raw: RawSkill) -> Result<MarketplaceSkillDescriptor, MarketplaceError> {
    validate_id(&raw.id, "skill id")?;
    validate_text(&raw.name, "skill name")?;
    validate_text(&raw.category, "skill category")?;
    validate_text(&raw.skill_type, "skill type")?;
    validate_text(&raw.description, "skill description")?;
    validate_text(&raw.license, "skill license")?;
    validate_text_list(&raw.tags, "skill tags")?;
    validate_text_list(&raw.keywords, "skill keywords")?;
    if !raw.filename.ends_with(".zip") {
        return invalid(format!("skill `{}` asset must be a .zip archive", raw.id));
    }
    let download_url = validate_https_url(&raw.download, Some(&raw.filename))?;
    Ok(MarketplaceSkillDescriptor {
        registry_id: raw.id,
        name: raw.name,
        category: raw.category,
        skill_type: raw.skill_type,
        description: raw.description,
        tags: raw.tags,
        keywords: raw.keywords,
        download_url,
        filename: raw.filename,
        license: raw.license,
    })
}

fn validate_id(value: &str, field: &str) -> Result<(), MarketplaceError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !value
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || value.contains("..")
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return invalid(format!("invalid {field} `{value}`"));
    }
    Ok(())
}

fn validate_text(value: &str, field: &str) -> Result<(), MarketplaceError> {
    if value.trim().is_empty()
        || value.len() > MAX_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return invalid(format!("invalid {field}"));
    }
    Ok(())
}

fn validate_text_list(values: &[String], field: &str) -> Result<(), MarketplaceError> {
    if values.len() > 256 {
        return invalid(format!("too many {field}"));
    }
    for value in values {
        validate_text(value, field)?;
    }
    Ok(())
}

fn validate_https_url(value: &str, filename: Option<&str>) -> Result<Url, MarketplaceError> {
    let url = Url::parse(value).map_err(|error| {
        MarketplaceError::InvalidRegistry(format!("invalid asset URL: {error}"))
    })?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !ASSET_HOSTS.contains(&host)
    {
        return invalid(format!("untrusted asset URL `{value}`"));
    }
    if let Some(filename) = filename {
        validate_id_filename(filename)?;
        if url.path_segments().and_then(Iterator::last) != Some(filename) {
            return invalid(format!("asset URL does not end in `{filename}`"));
        }
    }
    Ok(url)
}

fn validate_id_filename(filename: &str) -> Result<(), MarketplaceError> {
    if filename.is_empty()
        || filename.len() > 255
        || filename.contains('/')
        || filename.contains('\\')
        || filename.chars().any(char::is_control)
        || !filename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || !(filename.ends_with(".brxt") || filename.ends_with(".zip"))
    {
        return invalid(format!("invalid asset filename `{filename}`"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, MarketplaceError> {
    Err(MarketplaceError::InvalidRegistry(message.into()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketplaceCatalogSource {
    Live,
    LastGood,
    Embedded,
}

#[derive(Debug)]
pub struct MarketplaceCatalogLoad {
    pub catalog: MarketplaceCatalog,
    pub source: MarketplaceCatalogSource,
    pub cache_warning: Option<String>,
}

impl MarketplaceCatalogLoad {
    pub fn is_stale(&self) -> bool {
        self.source != MarketplaceCatalogSource::Live
    }

    /// The registry snapshot shipped in the binary, loaded as the tools see it
    /// offline — for tests of what a tool builds from a catalog, which must not
    /// depend on the network.
    #[cfg(test)]
    pub(crate) fn embedded_for_test() -> Self {
        Self {
            catalog: MarketplaceCatalog::from_bytes(EMBEDDED_REGISTRY)
                .expect("the shipped registry is a valid catalog"),
            source: MarketplaceCatalogSource::Embedded,
            cache_warning: None,
        }
    }
}

pub async fn load_marketplace_catalog() -> Result<MarketplaceCatalogLoad, MarketplaceError> {
    let fetcher = ReqwestRegistryFetcher::new()?;
    load_with(
        &fetcher,
        &Paths::in_state_dir("marketplace/registry-last-good.json"),
        EMBEDDED_REGISTRY,
        true,
    )
    .await
}

#[async_trait]
trait RegistryFetcher: Sync {
    async fn fetch(&self) -> Result<Vec<u8>, MarketplaceError>;
}

struct ReqwestRegistryFetcher {
    client: reqwest::Client,
}

impl ReqwestRegistryFetcher {
    fn new() -> Result<Self, MarketplaceError> {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| MarketplaceError::Fetch(error.to_string()))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl RegistryFetcher for ReqwestRegistryFetcher {
    async fn fetch(&self) -> Result<Vec<u8>, MarketplaceError> {
        let response = self
            .client
            .get(REGISTRY_URL)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(
                reqwest::header::USER_AGENT,
                concat!("Biorouter/", env!("CARGO_PKG_VERSION")),
            )
            .send()
            .await
            .map_err(|error| MarketplaceError::Fetch(error.to_string()))?
            .error_for_status()
            .map_err(|error| MarketplaceError::Fetch(error.to_string()))?;
        if response.content_length().is_some_and(|size| {
            size > u64::try_from(MAX_REGISTRY_BYTES).expect("registry size limit fits in u64")
        }) {
            return Err(MarketplaceError::RegistryTooLarge {
                actual: usize::try_from(response.content_length().unwrap_or(u64::MAX))
                    .unwrap_or(usize::MAX),
                limit: MAX_REGISTRY_BYTES,
            });
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| MarketplaceError::Fetch(error.to_string()))?;
            let actual = bytes.len().saturating_add(chunk.len());
            if actual > MAX_REGISTRY_BYTES {
                return Err(MarketplaceError::RegistryTooLarge {
                    actual,
                    limit: MAX_REGISTRY_BYTES,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

async fn load_with(
    fetcher: &dyn RegistryFetcher,
    cache_path: &Path,
    embedded: &[u8],
    publish_authority: bool,
) -> Result<MarketplaceCatalogLoad, MarketplaceError> {
    let embedded_catalog = MarketplaceCatalog::from_bytes(embedded)?;
    if let Ok(bytes) = fetcher.fetch().await {
        if let Ok(catalog) = MarketplaceCatalog::from_bytes(&bytes) {
            let cache_warning = atomic_write(cache_path, &bytes)
                .err()
                .map(|error| error.to_string());
            if publish_authority {
                catalog.raise_daemon_privacy_authority()?;
            }
            return Ok(MarketplaceCatalogLoad {
                catalog,
                source: MarketplaceCatalogSource::Live,
                cache_warning,
            });
        }
    }
    if let Ok(bytes) = std::fs::read(cache_path) {
        if let Ok(catalog) = MarketplaceCatalog::from_bytes(&bytes) {
            if publish_authority {
                catalog.raise_daemon_privacy_authority()?;
            }
            return Ok(MarketplaceCatalogLoad {
                catalog,
                source: MarketplaceCatalogSource::LastGood,
                cache_warning: None,
            });
        }
    }
    if publish_authority {
        embedded_catalog.raise_daemon_privacy_authority()?;
    }
    Ok(MarketplaceCatalogLoad {
        catalog: embedded_catalog,
        source: MarketplaceCatalogSource::Embedded,
        cache_warning: None,
    })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), MarketplaceError> {
    let parent = path
        .parent()
        .ok_or_else(|| MarketplaceError::Cache("cache path has no parent".to_owned()))?;
    std::fs::create_dir_all(parent).map_err(|error| MarketplaceError::Cache(error.to_string()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| MarketplaceError::Cache(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|()| file.as_file_mut().sync_all())
        .map_err(|error| MarketplaceError::Cache(error.to_string()))?;
    file.persist(path)
        .map_err(|error| MarketplaceError::Cache(error.error.to_string()))?;
    Ok(())
}

#[cfg(test)]
struct FakeFetcher {
    result: Result<Vec<u8>, String>,
}

#[cfg(test)]
impl FakeFetcher {
    fn bytes(bytes: Vec<u8>) -> Self {
        Self { result: Ok(bytes) }
    }

    fn offline() -> Self {
        Self {
            result: Err("offline".to_owned()),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl RegistryFetcher for FakeFetcher {
    async fn fetch(&self) -> Result<Vec<u8>, MarketplaceError> {
        self.result.clone().map_err(MarketplaceError::Fetch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::ProviderTier;
    use serde_json::json;

    fn registry(extension_privacy: &str) -> Vec<u8> {
        serde_json::to_vec_pretty(&json!({
            "version": 2,
            "source": "https://biorouter.ucsf.edu/baam",
            "institutions": { "ucsf": "UCSF" },
            "extensions": [
                {
                    "id": "public-agent",
                    "name": "Public Agent",
                    "organization": "Example",
                    "version": "v1.2.3",
                    "description": "A public fixture",
                    "tags": ["fixture"],
                    "github": "https://github.com/example/public-agent",
                    "download": "https://github.com/example/public-agent/releases/download/v1.2.3/public-agent.brxt",
                    "filename": "public-agent.brxt",
                    "license": "Apache-2.0",
                    "privacy": extension_privacy
                },
                {
                    "id": "private-agent",
                    "name": "Private Agent",
                    "organization": "Example",
                    "version": "v2.0.0",
                    "description": "A private fixture",
                    "tags": ["fixture"],
                    "github": "https://github.com/example/private-agent",
                    "download": "https://github.com/example/private-agent/releases/download/v2.0.0/private-agent.brxt",
                    "filename": "private-agent.brxt",
                    "license": "Apache-2.0",
                    "privacy": "private",
                    "extension_name": "private-agent",
                    "affiliation": ["ucsf"]
                }
            ],
            "skills": [{
                "id": "fixture-skill",
                "name": "Fixture Skill",
                "category": "Testing",
                "type": "User-invocable",
                "description": "A skill fixture",
                "tags": ["fixture"],
                "keywords": ["fixture", "test"],
                "download": "https://github.com/example/skills/releases/download/fixture-skill/fixture-skill.zip",
                "filename": "fixture-skill.zip",
                "license": "Apache-2.0"
            }]
        }))
        .unwrap()
    }

    #[test]
    fn exact_id_resolution_binds_registry_owned_install_fields() {
        let catalog = MarketplaceCatalog::from_bytes(&registry("public")).unwrap();
        let trusted = catalog
            .resolve_extension_for_install("public-agent", ProviderTier::Public)
            .unwrap();
        assert_eq!(trusted.registry_id, "public-agent");
        assert_eq!(trusted.version, "v1.2.3");
        assert_eq!(trusted.download_url.host_str(), Some("github.com"));
        assert!(catalog
            .resolve_extension_for_install("PUBLIC-AGENT", ProviderTier::Private)
            .is_err());

        let skill = catalog.resolve_skill_for_install("fixture-skill").unwrap();
        assert_eq!(skill.registry_id, "fixture-skill");
        assert!(skill.download_url.path().ends_with("fixture-skill.zip"));
    }

    #[test]
    fn shipped_registry_snapshot_is_a_valid_offline_catalog() {
        let catalog = MarketplaceCatalog::from_bytes(EMBEDDED_REGISTRY).unwrap();
        let spoke = catalog
            .resolve_extension_for_install("spokeagent", ProviderTier::Public)
            .unwrap();
        // ⚠ The ID carries no version; the ASSET does. A registry id is a name,
        // and SPOKEAgent's used to be `spokeagent-0.4.1` — which the descriptor
        // then advertised as the extension's name, while the extension that
        // installs is `spokeagent`. The release asset really is versioned on
        // GitHub, so `filename`/`download` keep the version and only the name
        // loses it.
        assert_eq!(spoke.version, "v0.4.1");
        assert_eq!(spoke.filename, "spokeagent-0.4.1.brxt");
        assert_eq!(spoke.extension_name, "spokeagent");
        assert!(
            spoke.advertises_name,
            "the registry must STATE the name, not leave it to the id fallback"
        );
        assert!(!catalog.browse_skills().is_empty());
    }

    /// No shipped registry id may carry a version. An id is a stable name; a
    /// version belongs in `version` and in the asset filename.
    #[test]
    fn no_shipped_registry_id_carries_a_version() {
        let catalog = MarketplaceCatalog::from_bytes(EMBEDDED_REGISTRY).unwrap();
        for entry in catalog.browse_extensions(ProviderTier::Private) {
            let bare = entry.version.trim_start_matches('v');
            assert!(
                !entry.registry_id.ends_with(bare) || bare.is_empty(),
                "registry id `{}` carries its version `{}`",
                entry.registry_id,
                entry.version
            );
        }
    }

    #[test]
    fn public_callers_never_browse_or_preflight_private_extensions() {
        let catalog = MarketplaceCatalog::from_bytes(&registry("public")).unwrap();
        assert_eq!(
            catalog
                .search_extensions(ProviderTier::Public, "agent")
                .len(),
            1
        );
        assert_eq!(
            catalog
                .search_extensions(ProviderTier::Private, "agent")
                .len(),
            2
        );
        assert!(matches!(
            catalog.resolve_extension_for_install("private-agent", ProviderTier::Public),
            Err(MarketplaceError::ExtensionUnavailableForCaller { .. })
        ));
        assert!(catalog
            .resolve_extension_for_install("private-agent", ProviderTier::Private)
            .is_ok());

        let hidden = catalog
            .resolve_extension_for_install("private-agent", ProviderTier::Public)
            .unwrap_err()
            .to_string();
        let absent = catalog
            .resolve_extension_for_install("not-in-the-registry", ProviderTier::Public)
            .unwrap_err()
            .to_string();
        assert_eq!(
            hidden.replace("private-agent", "<id>"),
            absent.replace("not-in-the-registry", "<id>"),
            "exact-id preflight must not reveal private catalog membership"
        );
    }

    /// Seven skill rows copied VERBATIM from `landing/registry.json` at
    /// 7c96d796, the registry the 2026-09-10 composer QA run measured finding
    /// F5 against. Frozen here rather than read from [`EMBEDDED_REGISTRY`] so
    /// the exact rankings below cannot drift when the registry gains a skill;
    /// the shipped-registry test after them pins only the measured shape.
    fn skills_snapshot() -> Vec<u8> {
        fn row(
            id: &str,
            name: &str,
            category: &str,
            kind: &str,
            description: &str,
            tags: &[&str],
            keywords: &[&str],
        ) -> serde_json::Value {
            json!({
                "id": id,
                "name": name,
                "category": category,
                "type": kind,
                "description": description,
                "tags": tags,
                "keywords": keywords,
                "download": format!("https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-{id}/{id}.zip"),
                "filename": format!("{id}.zip"),
                "license": "Apache-2.0"
            })
        }
        serde_json::to_vec(&json!({
            "version": 2,
            "source": "https://biorouter.ucsf.edu/baam",
            "institutions": { "ucsf": "UCSF" },
            "extensions": [],
            "skills": [
                row("data-visualization", "Data Visualization", "Biomedical", "13 skills · auto-applied",
                    "Publication-quality plots: heatmaps, volcano, Manhattan, dimplots.",
                    &["ggplot2", "matplotlib", "ComplexHeatmap"],
                    &["data-visualization", "ggplot2", "matplotlib", "complexheatmap"]),
                row("ggplot-visualization", "ggplot2 Visualization", "Core", "Auto-applied · R plotting",
                    "Applies ggplot2 best-practice style when writing R plotting code.",
                    &["R", "ggplot2"], &[]),
                row("python-scripting", "Python Scripting", "Core", "Auto-applied · Python code",
                    "Applies Python naming, typing, error handling, and project structure conventions when writing Python code.",
                    &["Python"], &[]),
                row("r-scripting", "R Scripting", "Core", "Auto-applied · R code",
                    "Applies tidyverse conventions and documentation standards when writing or reviewing R code.",
                    &["R", "Tidyverse"], &[]),
                row("scientific-visual-communication", "Scientific Visual Communication", "Core",
                    "User-invocable · /scientific-visual-communication",
                    "Plans schematics, posters, slides, figure panels, infographics, visual abstracts, and source-to-visual traceability.",
                    &["Visuals", "Posters", "Apache-2.0"],
                    &["scientific-visual-communication", "schematics", "infographics", "posters", "slides", "visual", "abstracts", "apache"]),
                row("clinical-biostatistics", "Clinical Biostatistics", "Biomedical", "6 skills · auto-applied",
                    "Survival, mixed models, and clinical-trial statistical analysis.",
                    &["survival", "R", "lme4"], &["clinical-biostatistics", "survival", "r", "lme4"]),
                row("single-cell", "Single-cell", "Biomedical", "14 skills · auto-applied",
                    "scRNA-seq clustering, annotation, trajectory, and integration.",
                    &["Scanpy", "Seurat", "scVI"], &["single-cell", "scanpy", "seurat", "scvi"]),
            ]
        }))
        .unwrap()
    }

    fn skill_ids(search: &CatalogSearch<'_, MarketplaceSkillDescriptor>) -> Vec<String> {
        search
            .hits
            .iter()
            .map(|hit| hit.entry.registry_id.clone())
            .collect()
    }

    /// Finding F5, the three queries the QA run measured. The unfixed matcher
    /// asked whether the whole query was a substring of one field, so the
    /// phrase returned `total: 0` while each of its words, asked alone, found
    /// the skills it names. It now returns their union, ranked: the skill
    /// matching three of the four terms, then the two matching two, then the
    /// single-term matches.
    #[test]
    fn a_natural_language_skill_query_returns_the_union_ranked() {
        let catalog = MarketplaceCatalog::from_bytes(&skills_snapshot()).unwrap();

        let phrase = catalog.search_skills("R scripting ggplot visualization");
        assert_eq!(phrase.terms, ["r", "scripting", "ggplot", "visualization"]);
        assert_eq!(
            skill_ids(&phrase),
            [
                "ggplot-visualization",
                "r-scripting",
                "data-visualization",
                "python-scripting",
                "clinical-biostatistics",
            ],
            "measured before this fix: no hits at all"
        );
        assert_eq!(
            phrase.hits[0].matched_terms,
            ["r", "ggplot", "visualization"]
        );
        assert!(
            !skill_ids(&phrase).contains(&"scientific-visual-communication".to_owned()),
            "`visual` is not `visualization`: a long term must be found in the entry, not the \
             other way round"
        );

        // The two single-term controls, measured in the same chat as 2 and 1.
        assert_eq!(
            skill_ids(&catalog.search_skills("ggplot")),
            ["ggplot-visualization", "data-visualization"]
        );
        let id = catalog.search_skills("r-scripting");
        assert_eq!(
            skill_ids(&id)[0],
            "r-scripting",
            "the skill the query names ranks first, ahead of the other skills about R or \
             scripting"
        );
        assert_eq!(id.hits[0].matched_terms, ["r", "scripting"]);

        // The empty query stays the browse case, 129 of 129 in the QA run.
        assert_eq!(
            catalog.search_skills("").len(),
            catalog.browse_skills().len()
        );
    }

    /// The same property against the registry that ships in the binary — the
    /// offline fallback, and the snapshot of the live registry the QA run read.
    /// Only the measured SHAPE is pinned, so a new skill cannot fail it: the
    /// phrase finds every skill that each of its words finds alone.
    #[test]
    fn on_the_shipped_registry_a_phrase_finds_what_each_of_its_words_finds() {
        let catalog = MarketplaceCatalog::from_bytes(EMBEDDED_REGISTRY).unwrap();
        let phrase = skill_ids(&catalog.search_skills("R scripting ggplot visualization"));
        for word in ["ggplot", "r-scripting", "scripting", "visualization"] {
            for id in skill_ids(&catalog.search_skills(word)) {
                assert!(
                    phrase.contains(&id),
                    "`{word}` alone finds `{id}`, the phrase containing it does not: {phrase:?}"
                );
            }
        }
        assert!(phrase.len() >= 2, "{phrase:?}");
    }

    /// A licence is not searchable, and leaving the FIELD out did not make that
    /// true. Measured in the Browse-extensions modal on 2026-09-12 against the
    /// live 37-entry registry, with the field already gone: `PACS` → 31 of 37,
    /// `pac` → 31, `apache` → 31, `Apache-2.0` → 32, empty → 37, `zzzznope` → 0.
    /// The three counts agreeing is the identification — `PACS` reaches `pac`
    /// through the plural fallback, `pac` is inside `apache`, and 31 rows
    /// republish `Apache-2.0` as one of their own TAG chips, which are searched.
    /// None of the 31 was about PACS.
    ///
    /// What this must NOT do is narrow the substring rule: `pac` is inside
    /// "PacBio", "package" and "workspace", and those hits stay.
    #[test]
    fn a_licence_republished_as_a_label_is_not_searchable_through_it() {
        let catalog = MarketplaceCatalog::from_bytes(EMBEDDED_REGISTRY).unwrap();

        // A word of a licence, as a whole label. Spelled out here rather than
        // taken from `catalog_search` so this test cannot be satisfied by the
        // same mistake the fix makes.
        let is_a_licence_word = |license: &str, label: &str| {
            license
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .any(|word| word.eq_ignore_ascii_case(label))
        };

        // Guard. If the registry stops republishing its licence as a label there
        // is nothing here to refuse, and every assertion below would pass while
        // proving nothing — which is exactly how the fix before this one looked
        // covered. Three counts, because there are three label paths.
        let tagged_extensions = catalog
            .browse_extensions(ProviderTier::Private)
            .iter()
            .filter(|entry| {
                entry
                    .tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case(&entry.license))
            })
            .count();
        let tagged_skills = catalog
            .browse_skills()
            .iter()
            .filter(|entry| {
                entry
                    .tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case(&entry.license))
            })
            .count();
        let keyworded_skills = catalog
            .browse_skills()
            .iter()
            .filter(|entry| {
                entry
                    .keywords
                    .iter()
                    .any(|keyword| is_a_licence_word(&entry.license, keyword))
            })
            .count();
        assert!(
            tagged_extensions >= 2 && tagged_skills >= 2 && keyworded_skills >= 2,
            "the shipped registry no longer republishes its licence as a label \
             (extensions tagged {tagged_extensions}, skills tagged {tagged_skills}, skills \
             keyworded {keyworded_skills}) — this test would pass vacuously"
        );

        // `apache` occurs nowhere in the registry except each entry's own
        // licence, so it finds nothing at all. Measured before the fix: 31
        // extensions and 49 skills, none of them about Apache anything.
        let extension_ids = |search: &CatalogSearch<'_, MarketplaceExtensionDescriptor>| {
            search
                .hits
                .iter()
                .map(|hit| hit.entry.registry_id.clone())
                .collect::<Vec<String>>()
        };
        assert_eq!(
            extension_ids(&catalog.search_extensions(ProviderTier::Private, "apache")),
            Vec::<String>::new()
        );
        assert_eq!(
            skill_ids(&catalog.search_skills("apache")),
            Vec::<String>::new()
        );

        // The query as the QA run typed it. The plural fallback and the
        // substring rule both stay: a skill that really says PACS is found, and
        // a skill whose only `pac` was `Apache-2.0` is not.
        let pacs = skill_ids(&catalog.search_skills("PACS"));
        assert!(
            pacs.contains(&"biomedical-imaging-pathology".to_owned()),
            "a skill whose keywords say `pacs` must still be found: {pacs:?}"
        );
        assert!(
            pacs.contains(&"long-read-sequencing".to_owned()),
            "`pac` inside `PacBio` is the substring rule working, not the defect: {pacs:?}"
        );
        for licence_only in ["empirical-research-router", "causal-identification-gates"] {
            assert!(
                !pacs.contains(&licence_only.to_owned()),
                "`{licence_only}`'s only `pac` is its `Apache-2.0` label: {pacs:?}"
            );
        }
        let pacs_extensions =
            extension_ids(&catalog.search_extensions(ProviderTier::Private, "PACS"));
        for licence_only in ["benchlingagent", "dnanexusagent", "omeroagent"] {
            assert!(
                !pacs_extensions.contains(&licence_only.to_owned()),
                "`{licence_only}` is not about PACS; its only `pac` is `Apache-2.0`: \
                 {pacs_extensions:?}"
            );
        }

        // The browse case is untouched — the licence label is dropped from what
        // is SEARCHED, not from the catalog.
        assert_eq!(
            catalog.search_skills("").len(),
            catalog.browse_skills().len()
        );
        assert_eq!(
            catalog.search_extensions(ProviderTier::Private, "").len(),
            catalog.browse_extensions(ProviderTier::Private).len()
        );
    }

    /// The extension catalog shares the matcher, and the caller filter runs
    /// before the ranking: a public caller's multi-word query can match the
    /// private row's words and still never be shown it.
    #[test]
    fn a_multi_word_extension_query_ranks_only_what_the_caller_may_see() {
        let catalog = MarketplaceCatalog::from_bytes(&registry("public")).unwrap();
        let ids = |caller| -> Vec<String> {
            catalog
                .search_extensions(caller, "private fixture")
                .hits
                .iter()
                .map(|hit| hit.entry.registry_id.clone())
                .collect()
        };
        assert_eq!(ids(ProviderTier::Public), ["public-agent"]);
        assert_eq!(
            ids(ProviderTier::Private),
            ["private-agent", "public-agent"],
            "the row matching both terms ranks first"
        );
    }

    #[test]
    fn a_later_public_registry_row_cannot_lower_learned_private_authority() {
        let mut lowered: serde_json::Value = serde_json::from_slice(&registry("public")).unwrap();
        let entry = &mut lowered["extensions"][0];
        entry["id"] = json!("live-ratchet-fixture");
        entry["download"] = json!("https://github.com/example/public-agent/releases/download/v1.2.3/live-ratchet-fixture.brxt");
        entry["filename"] = json!("live-ratchet-fixture.brxt");
        crate::privacy::registry_live::insert_test_authority(
            "live-ratchet-fixture",
            Some(BTreeSet::from(["ucsf".to_owned()])),
        );

        let catalog =
            MarketplaceCatalog::from_bytes(&serde_json::to_vec(&lowered).unwrap()).unwrap();
        assert!(matches!(
            catalog.resolve_extension_for_install("live-ratchet-fixture", ProviderTier::Public),
            Err(MarketplaceError::ExtensionUnavailableForCaller { .. })
        ));
        let private = catalog
            .resolve_extension_for_install("live-ratchet-fixture", ProviderTier::Private)
            .unwrap();
        assert_eq!(private.privacy, ProviderTier::Private);
    }

    #[test]
    fn malformed_hostile_and_oversized_registries_are_rejected() {
        let mut malformed: serde_json::Value = serde_json::from_slice(&registry("public")).unwrap();
        malformed["extensions"][0]["download"] = json!("http://127.0.0.1/private.brxt");
        assert!(MarketplaceCatalog::from_bytes(&serde_json::to_vec(&malformed).unwrap()).is_err());

        malformed = serde_json::from_slice(&registry("public")).unwrap();
        malformed["extensions"][0]["id"] = json!("private-agent");
        assert!(MarketplaceCatalog::from_bytes(&serde_json::to_vec(&malformed).unwrap()).is_err());

        malformed = serde_json::from_slice(&registry("public")).unwrap();
        malformed["extensions"][0]["affiliation"] = json!(["ucsf"]);
        assert!(MarketplaceCatalog::from_bytes(&serde_json::to_vec(&malformed).unwrap()).is_err());

        assert!(matches!(
            MarketplaceCatalog::from_bytes(&vec![b' '; MAX_REGISTRY_BYTES + 1]),
            Err(MarketplaceError::RegistryTooLarge { .. })
        ));
    }

    #[tokio::test]
    async fn live_offline_stale_and_malformed_fetches_preserve_last_good() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("registry-last-good.json");
        let embedded = registry("public");

        let live = load_with(
            &FakeFetcher::bytes(registry("public")),
            &cache,
            &embedded,
            false,
        )
        .await
        .unwrap();
        assert_eq!(live.source, MarketplaceCatalogSource::Live);
        assert!(!live.is_stale());
        assert!(cache.is_file());

        let cached = std::fs::read(&cache).unwrap();
        let offline = load_with(&FakeFetcher::offline(), &cache, &embedded, false)
            .await
            .unwrap();
        assert_eq!(offline.source, MarketplaceCatalogSource::LastGood);
        assert!(offline.is_stale());

        let malformed = load_with(
            &FakeFetcher::bytes(br#"{\"not\":\"a registry\"}"#.to_vec()),
            &cache,
            &embedded,
            false,
        )
        .await
        .unwrap();
        assert_eq!(malformed.source, MarketplaceCatalogSource::LastGood);
        assert_eq!(std::fs::read(&cache).unwrap(), cached);

        let oversized = load_with(
            &FakeFetcher::bytes(vec![b' '; MAX_REGISTRY_BYTES + 1]),
            &cache,
            &embedded,
            false,
        )
        .await
        .unwrap();
        assert_eq!(oversized.source, MarketplaceCatalogSource::LastGood);
        assert_eq!(std::fs::read(&cache).unwrap(), cached);

        std::fs::write(&cache, b"corrupt").unwrap();
        let fallback = load_with(&FakeFetcher::offline(), &cache, &embedded, false)
            .await
            .unwrap();
        assert_eq!(fallback.source, MarketplaceCatalogSource::Embedded);
    }
}
