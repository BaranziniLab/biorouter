//! Fetching a URL that a model chose.
//!
//! A built-in tool that fetches a URL the model names (`webdocuments__web_scrape`,
//! knowledge ingest from a URL, the credibility sub-agent's `fetch_url`) runs in
//! the daemon, on the person's computer and network. Unchecked, a page or a
//! prompt injection can point it at `http://127.0.0.1:<port>/` (the daemon, a
//! notebook server, a database console), at the home router, or at a cloud
//! metadata service (`http://169.254.169.254/`), and the reply comes back into
//! the conversation. This module is the one place that decides what such a fetch
//! may reach: public internet addresses only.
//!
//! How the rule holds:
//!
//! 1. [`SafeFetch::get`] parses the URL before anything is sent and refuses a
//!    scheme other than `http` and `https`, a host name that is local by
//!    definition (`localhost`, `*.local`, `*.internal`, `*.home.arpa`, a cloud
//!    metadata name, or a single label such as `metadata`), and an IP literal in
//!    a refused range ([`refused_range`]). reqwest never resolves an IP literal,
//!    so this is the only check an IP literal meets.
//! 2. Every host NAME is resolved by [`CheckedResolver`], the client's DNS
//!    resolver. It refuses the name when ANY address it resolves to is in a
//!    refused range, and otherwise hands those exact addresses to the connector.
//!    The connection goes to an address from the one lookup that was checked,
//!    and there is no second lookup, so a DNS answer that changes between the
//!    check and the connect (rebinding) cannot reach an internal address.
//! 3. Every redirect hop is checked like the first URL (scheme, local names, IP
//!    literals), and a hop to a new host name is resolved by the same
//!    [`CheckedResolver`] when it connects. At most [`MAX_REDIRECTS`] hops are
//!    followed.
//! 4. The client never uses a proxy (`HTTP_PROXY`, `HTTPS_PROXY` or a system
//!    proxy). A proxy resolves the target itself, so nothing here could check or
//!    pin the address it connects to.
//!
//! Not covered: ports are not restricted, user info in a URL is not refused, and
//! tools that reach a URL some other way (the developer shell, the Biorouter
//! Browser, a remote MCP server the person added) are not routed through here.

use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use url::Host;

/// The most redirect hops one fetch follows, the same as reqwest's default.
pub const MAX_REDIRECTS: usize = 10;

/// The sentence every address refusal ends with, so the model learns the rule
/// rather than retrying the next internal address.
const PUBLIC_ONLY: &str = "Built-in fetchers reach public internet addresses only, never this \
     computer, its local network or a cloud metadata service";

/// Names that reach a cloud provider's instance metadata service. Each one is
/// refused by name, before it is resolved, so the refusal holds even where a
/// resolver would answer with something other than the usual link-local
/// address.
const CLOUD_METADATA_NAMES: &[&str] = &[
    "metadata.google.internal",
    "metadata.goog",
    "metadata",
    "instance-data",
    "instance-data.ec2.internal",
];

/// Name suffixes that are local by definition: special-use names (RFC 6761,
/// RFC 6762, RFC 8375) and the ICANN-reserved `.internal`.
const LOCAL_NAME_SUFFIXES: &[&str] = &[".localhost", ".local", ".internal", ".home.arpa"];

/// A fetch that was not made, and why. Its text is written for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRefused {
    message: String,
}

impl FetchRefused {
    fn new(message: String) -> Self {
        Self { message }
    }

    fn address(action: &str, target: &str, reason: &str) -> Self {
        Self::new(format!(
            "Refused to {action} {target}: {reason}. {PUBLIC_ONLY}."
        ))
    }

    /// The whole refusal, as the model sees it.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for FetchRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl StdError for FetchRefused {}

/// Why a safe fetch failed: the address rule refused it, or the request itself
/// failed (a timeout, a TLS error, a server that hung up).
#[derive(Debug)]
pub enum FetchError {
    Refused(FetchRefused),
    Request(reqwest::Error),
}

impl FetchError {
    /// The refusal, when that is what this is.
    pub fn refusal(&self) -> Option<&FetchRefused> {
        match self {
            Self::Refused(refused) => Some(refused),
            Self::Request(_) => None,
        }
    }
}

impl From<FetchRefused> for FetchError {
    fn from(refused: FetchRefused) -> Self {
        Self::Refused(refused)
    }
}

impl From<reqwest::Error> for FetchError {
    /// A refusal from the resolver or the redirect policy reaches the caller
    /// wrapped in a reqwest error (and reqwest calls a resolver refusal a
    /// connect error, which a caller would otherwise retry). This finds it.
    fn from(error: reqwest::Error) -> Self {
        match refusal_in(&error) {
            Some(refused) => Self::Refused(refused.clone()),
            None => Self::Request(error),
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refused) => refused.fmt(f),
            Self::Request(error) => error.fmt(f),
        }
    }
}

impl StdError for FetchError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Refused(_) => None,
            Self::Request(error) => Some(error),
        }
    }
}

/// The refusal somewhere in `error`'s source chain, if there is one.
pub fn refusal_in(error: &reqwest::Error) -> Option<&FetchRefused> {
    let mut current: Option<&(dyn StdError + 'static)> = Some(error);
    while let Some(error) = current {
        if let Some(refused) = error.downcast_ref::<FetchRefused>() {
            return Some(refused);
        }
        current = error.source();
    }
    None
}

/// Why `ip` may not be fetched, or `None` for a public address.
///
/// IPv4 is a deny list of every special-purpose block. IPv6 is an allow list:
/// only global unicast (`2000::/3`) passes, minus its special-purpose blocks,
/// plus NAT64 (`64:ff9b::/96`) when the IPv4 address it carries is public, so
/// an IPv6-only network that reaches IPv4 sites through DNS64 keeps working.
pub fn refused_range(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(ip) => refused_v4(ip),
        IpAddr::V6(ip) => refused_v6(ip),
    }
}

fn in_v4(ip: u32, network: u32, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    ip & mask == network
}

fn in_v6(ip: u128, network: u128, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    ip & mask == network
}

fn refused_v4(ip: Ipv4Addr) -> Option<&'static str> {
    const METADATA: &[(Ipv4Addr, &str)] = &[
        (
            Ipv4Addr::new(169, 254, 169, 254),
            "the cloud instance metadata service",
        ),
        (
            Ipv4Addr::new(169, 254, 170, 2),
            "the container metadata service",
        ),
        (
            Ipv4Addr::new(100, 100, 100, 200),
            "a cloud instance metadata service",
        ),
    ];
    const RANGES: &[(u32, u32, &str)] = &[
        (0x0000_0000, 8, "an unspecified address (0.0.0.0/8)"),
        (0x0A00_0000, 8, "a private address (10.0.0.0/8)"),
        (
            0x6440_0000,
            10,
            "a carrier-grade NAT address (100.64.0.0/10)",
        ),
        (0x7F00_0000, 8, "a loopback address (127.0.0.0/8)"),
        (0xA9FE_0000, 16, "a link-local address (169.254.0.0/16)"),
        (0xAC10_0000, 12, "a private address (172.16.0.0/12)"),
        (0xC000_0000, 24, "an IETF protocol address (192.0.0.0/24)"),
        (0xC000_0200, 24, "a documentation address (192.0.2.0/24)"),
        (0xC058_6300, 24, "a 6to4 relay address (192.88.99.0/24)"),
        (0xC0A8_0000, 16, "a private address (192.168.0.0/16)"),
        (0xC612_0000, 15, "a benchmarking address (198.18.0.0/15)"),
        (0xC633_6400, 24, "a documentation address (198.51.100.0/24)"),
        (0xCB00_7100, 24, "a documentation address (203.0.113.0/24)"),
        (0xE000_0000, 4, "a multicast address (224.0.0.0/4)"),
        (
            0xF000_0000,
            4,
            "a reserved or broadcast address (240.0.0.0/4)",
        ),
    ];
    if let Some((_, name)) = METADATA.iter().find(|(address, _)| *address == ip) {
        return Some(name);
    }
    let bits = u32::from(ip);
    RANGES
        .iter()
        .find(|(network, prefix, _)| in_v4(bits, *network, *prefix))
        .map(|(_, _, name)| *name)
}

fn refused_v6(ip: Ipv6Addr) -> Option<&'static str> {
    const AWS_METADATA: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254);
    // Checked in order, so a narrower block inside a wider one comes first.
    const RANGES: &[(u128, u32, &str)] = &[
        (0, 96, "an IPv4-compatible address (::/96)"),
        (
            0x0000_0000_0000_0000_0000_ffff_0000_0000,
            96,
            "an IPv4-mapped address (::ffff:0:0/96)",
        ),
        (
            0x0064_ff9b_0001_0000_0000_0000_0000_0000,
            48,
            "a local-use NAT64 address (64:ff9b:1::/48)",
        ),
        (
            0x0100_0000_0000_0000_0000_0000_0000_0000,
            64,
            "a discard-only address (100::/64)",
        ),
        (
            0x2001_0db8_0000_0000_0000_0000_0000_0000,
            32,
            "a documentation address (2001:db8::/32)",
        ),
        (
            0x2001_0000_0000_0000_0000_0000_0000_0000,
            23,
            "an IETF protocol address such as Teredo (2001::/23)",
        ),
        (
            0x2002_0000_0000_0000_0000_0000_0000_0000,
            16,
            "a 6to4 address (2002::/16)",
        ),
        (
            0x3fff_0000_0000_0000_0000_0000_0000_0000,
            20,
            "a documentation address (3fff::/20)",
        ),
        (
            0xfc00_0000_0000_0000_0000_0000_0000_0000,
            7,
            "a unique-local address (fc00::/7)",
        ),
        (
            0xfe80_0000_0000_0000_0000_0000_0000_0000,
            10,
            "a link-local address (fe80::/10)",
        ),
        (
            0xfec0_0000_0000_0000_0000_0000_0000_0000,
            10,
            "a site-local address (fec0::/10)",
        ),
        (
            0xff00_0000_0000_0000_0000_0000_0000_0000,
            8,
            "a multicast address (ff00::/8)",
        ),
    ];
    const NAT64: u128 = 0x0064_ff9b_0000_0000_0000_0000_0000_0000;
    const GLOBAL_UNICAST: u128 = 0x2000_0000_0000_0000_0000_0000_0000_0000;

    if ip == AWS_METADATA {
        return Some("the cloud instance metadata service");
    }
    if ip.is_unspecified() {
        return Some("the unspecified address (::)");
    }
    if ip.is_loopback() {
        return Some("a loopback address (::1)");
    }
    let bits = u128::from(ip);
    if in_v6(bits, NAT64, 96) {
        // DNS64 writes every IPv4-only site as one of these, so it is judged by
        // the IPv4 address it carries rather than refused outright.
        let carried = Ipv4Addr::from((bits & 0xffff_ffff) as u32);
        return refused_v4(carried).map(|_| "a NAT64 address of a non-public IPv4 address");
    }
    if let Some((_, _, name)) = RANGES
        .iter()
        .find(|(network, prefix, _)| in_v6(bits, *network, *prefix))
    {
        return Some(name);
    }
    if !in_v6(bits, GLOBAL_UNICAST, 3) {
        return Some("not a global unicast address (outside 2000::/3)");
    }
    None
}

/// Why `host`, a DNS name, is local by definition, or `None`.
fn local_name(host: &str) -> Option<String> {
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() {
        return Some("it names no host".to_string());
    }
    if name == "localhost" || name.ends_with(".localhost") {
        return Some(format!("`{name}` names this computer"));
    }
    if CLOUD_METADATA_NAMES.contains(&name.as_str()) {
        return Some(format!("`{name}` is a cloud metadata service"));
    }
    if let Some(suffix) = LOCAL_NAME_SUFFIXES
        .iter()
        .find(|suffix| name.ends_with(*suffix))
    {
        return Some(format!(
            "names under `{suffix}` are local to a network and never on the public internet"
        ));
    }
    if !name.contains('.') {
        return Some(format!(
            "a single-label name such as `{name}` is resolved on the local network, \
             never on the public internet"
        ));
    }
    None
}

/// Looks a host name up. The system resolver in production; a stub in tests,
/// which is safe to allow anywhere because every answer is still checked.
pub trait HostLookup: Send + Sync {
    fn lookup(&self, host: &str) -> LookupFuture;
}

/// What [`HostLookup::lookup`] returns.
pub type LookupFuture = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;

/// The operating system's resolver (`getaddrinfo`, on a blocking thread).
pub struct SystemLookup;

impl HostLookup for SystemLookup {
    fn lookup(&self, host: &str) -> LookupFuture {
        let host = host.to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(addresses.map(|address| address.ip()).collect())
        })
    }
}

/// The decisions, shared by the pre-flight check, the resolver and the
/// redirect policy so the three can never disagree.
#[derive(Clone, Default)]
struct Policy {
    /// Addresses a unit test admits for its stub resolver's answers. Absent
    /// from every other build.
    #[cfg(test)]
    admitted_for_tests: Arc<Vec<IpAddr>>,
}

enum ResolveError {
    Refused(FetchRefused),
    Lookup(io::Error),
}

impl Policy {
    /// Why a resolved address is refused, or `None`.
    fn address_refusal(&self, ip: IpAddr) -> Option<&'static str> {
        #[cfg(test)]
        if self.admitted_for_tests.contains(&ip) {
            return None;
        }
        refused_range(ip)
    }

    /// The check a URL meets before it is requested and at every redirect hop.
    fn check_url(&self, url: &Url, action: &str) -> Result<(), FetchRefused> {
        let target = url.as_str();
        match url.scheme() {
            "http" | "https" => {}
            other => {
                return Err(FetchRefused::new(format!(
                    "Refused to {action} {target}: only http and https URLs can be fetched, \
                     and this one is `{other}`."
                )));
            }
        }
        match url.host() {
            None => Err(FetchRefused::address(action, target, "it names no host")),
            Some(Host::Domain(name)) => match name.parse::<IpAddr>() {
                // The URL parser turns every IPv4 spelling of an http(s) host
                // into an address, so this arm is for safety, not for use.
                Ok(ip) => self.check_literal(url, ip, action),
                Err(_) => match local_name(name) {
                    Some(reason) => Err(FetchRefused::address(action, target, &reason)),
                    None => Ok(()),
                },
            },
            Some(Host::Ipv4(ip)) => self.check_literal(url, IpAddr::V4(ip), action),
            Some(Host::Ipv6(ip)) => self.check_literal(url, IpAddr::V6(ip), action),
        }
    }

    fn check_literal(&self, url: &Url, ip: IpAddr, action: &str) -> Result<(), FetchRefused> {
        let Some(range) = refused_range(ip) else {
            return Ok(());
        };
        if let Some(port) = url.port_or_known_default() {
            if test_support::is_admitted(SocketAddr::new(ip, port)) {
                return Ok(());
            }
        }
        Err(FetchRefused::address(
            action,
            url.as_str(),
            &format!("{ip} is {range}"),
        ))
    }

    /// Resolve `host` once and return every address, or refuse the name when
    /// any one of them is refused.
    async fn resolve(
        &self,
        lookup: &dyn HostLookup,
        host: &str,
    ) -> Result<Vec<IpAddr>, ResolveError> {
        let refuse = |reason: String| {
            ResolveError::Refused(FetchRefused::address("connect to", host, &reason))
        };
        if let Ok(ip) = host.parse::<IpAddr>() {
            // The connector never asks a resolver for an IP literal; refuse
            // rather than assume.
            return match self.address_refusal(ip) {
                Some(range) => Err(refuse(format!("{ip} is {range}"))),
                None => Ok(vec![ip]),
            };
        }
        if let Some(reason) = local_name(host) {
            return Err(refuse(reason));
        }
        let addresses = lookup.lookup(host).await.map_err(ResolveError::Lookup)?;
        if addresses.is_empty() {
            return Err(ResolveError::Lookup(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{host} resolved to no address"),
            )));
        }
        for ip in &addresses {
            if let Some(range) = self.address_refusal(*ip) {
                return Err(refuse(format!("it resolves to {ip}, {range}")));
            }
        }
        Ok(addresses)
    }
}

/// The client's DNS resolver: one checked lookup per connection, whose exact
/// answer is what the connector dials.
struct CheckedResolver {
    lookup: Arc<dyn HostLookup>,
    policy: Policy,
}

impl reqwest::dns::Resolve for CheckedResolver {
    fn resolve(&self, name: hyper::client::connect::dns::Name) -> reqwest::dns::Resolving {
        let lookup = Arc::clone(&self.lookup);
        let policy = self.policy.clone();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let result: Result<reqwest::dns::Addrs, Box<dyn StdError + Send + Sync>> =
                match policy.resolve(lookup.as_ref(), &host).await {
                    // The connector sets each address's port from the URL.
                    Ok(addresses) => Ok(Box::new(
                        addresses.into_iter().map(|ip| SocketAddr::new(ip, 0)),
                    )),
                    Err(ResolveError::Refused(refused)) => Err(Box::new(refused)),
                    Err(ResolveError::Lookup(error)) => Err(Box::new(error)),
                };
            result
        })
    }
}

#[derive(Debug)]
struct TooManyRedirects;

impl fmt::Display for TooManyRedirects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "more than {MAX_REDIRECTS} redirects")
    }
}

impl StdError for TooManyRedirects {}

fn redirect_policy(policy: Policy) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error(TooManyRedirects);
        }
        match policy.check_url(attempt.url(), "follow a redirect to") {
            Ok(()) => attempt.follow(),
            Err(refused) => attempt.error(refused),
        }
    })
}

/// An HTTP client for URLs a model chose. Build one with [`SafeFetch::builder`]
/// and start every request with [`SafeFetch::get`]; cloning is cheap and shares
/// the connection pool.
#[derive(Clone)]
pub struct SafeFetch {
    client: reqwest::Client,
    policy: Policy,
}

impl SafeFetch {
    pub fn builder() -> SafeFetchBuilder {
        SafeFetchBuilder {
            user_agent: None,
            timeout: None,
            lookup: Arc::new(SystemLookup),
            policy: Policy::default(),
        }
    }

    /// A GET for `url`, or the refusal when `url` itself may not be fetched.
    /// Send it with `.send().await` and turn its error into a [`FetchError`]
    /// (`FetchError::from`), which finds a refusal made later by the resolver or
    /// at a redirect hop.
    pub fn get(&self, url: &str) -> Result<reqwest::RequestBuilder, FetchRefused> {
        let parsed = Url::parse(url).map_err(|error| {
            FetchRefused::new(format!(
                "Cannot fetch {url}: it is not a valid URL ({error})."
            ))
        })?;
        self.policy.check_url(&parsed, "fetch")?;
        Ok(self.client.get(parsed))
    }
}

pub struct SafeFetchBuilder {
    user_agent: Option<String>,
    timeout: Option<Duration>,
    lookup: Arc<dyn HostLookup>,
    policy: Policy,
}

impl SafeFetchBuilder {
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// The whole request's time limit, name lookup included.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Replace the system resolver. Its answers are checked like any other.
    pub fn lookup(mut self, lookup: Arc<dyn HostLookup>) -> Self {
        self.lookup = lookup;
        self
    }

    /// Accept a loopback address from the stub resolver, so a unit test can
    /// fetch its own server through a host name.
    #[cfg(test)]
    pub(crate) fn admit_for_tests(mut self, ip: IpAddr) -> Self {
        assert!(ip.is_loopback(), "only a loopback address can be admitted");
        Arc::make_mut(&mut self.policy.admitted_for_tests).push(ip);
        self
    }

    pub fn build(self) -> reqwest::Result<SafeFetch> {
        let resolver = CheckedResolver {
            lookup: self.lookup,
            policy: self.policy.clone(),
        };
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(resolver))
            .redirect(redirect_policy(self.policy.clone()));
        if let Some(user_agent) = self.user_agent {
            builder = builder.user_agent(user_agent);
        }
        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }
        Ok(SafeFetch {
            client: builder.build()?,
            policy: self.policy,
        })
    }
}

/// Lets a test fetch its own local server through the built-in fetchers.
///
/// Compiled only into this crate's tests and into crates that enable the
/// `test-support` feature, which only a `[dev-dependencies]` entry may do
/// (`test_support_is_enabled_only_by_dev_dependencies` checks every manifest).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use std::net::SocketAddr;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    static ADMITTED: Mutex<Vec<SocketAddr>> = Mutex::new(Vec::new());

    fn admitted() -> MutexGuard<'static, Vec<SocketAddr>> {
        ADMITTED.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Admit one exact loopback address and port as an IP-literal URL target
    /// until the guard drops. A test that checks a loopback refusal against its
    /// own server, on its own port, is unaffected. Declare the guard after the
    /// server, so it drops first and the port is never admitted once freed.
    #[must_use = "the server is admitted only while the guard lives"]
    pub fn admit_local_server(address: SocketAddr) -> AdmittedServer {
        assert!(
            address.ip().is_loopback(),
            "only a loopback test server can be admitted, not {address}"
        );
        admitted().push(address);
        AdmittedServer { address }
    }

    pub struct AdmittedServer {
        address: SocketAddr,
    }

    impl Drop for AdmittedServer {
        fn drop(&mut self) {
            let mut admitted = admitted();
            if let Some(index) = admitted.iter().position(|a| *a == self.address) {
                admitted.swap_remove(index);
            }
        }
    }

    pub(super) fn is_admitted(address: SocketAddr) -> bool {
        admitted().contains(&address)
    }
}

#[cfg(not(any(test, feature = "test-support")))]
mod test_support {
    use std::net::SocketAddr;

    pub(super) fn is_admitted(_address: SocketAddr) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use wiremock::{matchers::method, matchers::path, Mock, MockServer, ResponseTemplate};

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    /// A resolver stub that answers from a script and counts its calls. The
    /// last answer repeats once the script runs out.
    struct ScriptedLookup {
        answers: Mutex<Vec<Vec<IpAddr>>>,
        calls: AtomicUsize,
    }

    impl ScriptedLookup {
        fn new(answers: Vec<Vec<IpAddr>>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                calls: AtomicUsize::new(0),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl HostLookup for ScriptedLookup {
        fn lookup(&self, _host: &str) -> LookupFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut answers = self.answers.lock().unwrap();
            let answer = if answers.len() > 1 {
                answers.remove(0)
            } else {
                answers.first().cloned().unwrap_or_default()
            };
            Box::pin(async move { Ok(answer) })
        }
    }

    fn fetch_with(lookup: Arc<ScriptedLookup>) -> SafeFetch {
        SafeFetch::builder()
            .lookup(lookup)
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    }

    fn refusal(result: Result<reqwest::RequestBuilder, FetchRefused>) -> String {
        match result {
            Ok(_) => panic!("the URL was not refused"),
            Err(refused) => refused.to_string(),
        }
    }

    /// Every special-purpose range the brief names, and its neighbours.
    #[test]
    fn refused_ranges_cover_every_non_public_address() {
        let refused = [
            ("127.0.0.1", "loopback"),
            ("127.255.255.254", "loopback"),
            ("0.0.0.0", "unspecified"),
            ("10.1.2.3", "private"),
            ("172.16.0.1", "private"),
            ("172.31.255.255", "private"),
            ("192.168.1.1", "private"),
            ("169.254.1.1", "link-local"),
            ("169.254.169.254", "metadata"),
            ("169.254.170.2", "metadata"),
            ("100.64.0.1", "carrier-grade NAT"),
            ("100.127.255.254", "carrier-grade NAT"),
            ("100.100.100.200", "metadata"),
            ("192.0.0.8", "IETF"),
            ("192.0.2.10", "documentation"),
            ("198.18.0.1", "benchmarking"),
            ("198.51.100.7", "documentation"),
            ("203.0.113.9", "documentation"),
            ("224.0.0.1", "multicast"),
            ("239.255.255.250", "multicast"),
            ("240.0.0.1", "reserved"),
            ("255.255.255.255", "broadcast"),
            ("::", "unspecified"),
            ("::1", "loopback"),
            ("::ffff:127.0.0.1", "IPv4-mapped"),
            ("::ffff:8.8.8.8", "IPv4-mapped"),
            ("::7f00:1", "IPv4-compatible"),
            ("fe80::1", "link-local"),
            ("fc00::1", "unique-local"),
            ("fd12:3456::1", "unique-local"),
            ("fd00:ec2::254", "metadata"),
            ("fec0::1", "site-local"),
            ("ff02::1", "multicast"),
            ("64:ff9b::a00:1", "NAT64"),
            ("64:ff9b::7f00:1", "NAT64"),
            ("64:ff9b:1::1", "NAT64"),
            ("100::1", "discard"),
            ("2001::1", "Teredo"),
            ("2001:db8::1", "documentation"),
            ("2002:7f00:1::1", "6to4"),
            ("3fff::1", "documentation"),
            ("5f00::1", "global unicast"),
        ];
        for (address, word) in refused {
            let range =
                refused_range(ip(address)).unwrap_or_else(|| panic!("{address} must be refused"));
            assert!(
                range.contains(word),
                "{address} was refused as `{range}`, expected a `{word}` range"
            );
        }
    }

    #[test]
    fn public_addresses_are_admitted() {
        for address in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.215.14",
            "172.32.0.1",
            "172.15.255.255",
            "100.63.255.255",
            "100.128.0.1",
            "169.253.255.255",
            "192.0.1.1",
            "192.169.0.1",
            "223.255.255.255",
            "2606:4700:4700::1111",
            "2a00:1450:4001:80b::200e",
            "64:ff9b::808:808",
        ] {
            assert_eq!(refused_range(ip(address)), None, "{address} is public");
        }
    }

    /// The URL parser reads every IPv4 spelling of an http(s) host as an
    /// address, so octal, hex and integer forms of 127.0.0.1 meet the same check.
    #[test]
    fn ip_literals_are_refused_in_every_spelling() {
        let fetch = SafeFetch::builder().build().unwrap();
        for url in [
            "http://127.0.0.1/",
            "http://2130706433/",
            "http://0x7f.1/",
            "http://0177.0.0.1/",
            "http://127.1/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:7f00:1]:8080/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[fd00:ec2::254]/latest/meta-data/",
            "http://10.0.0.1:8080/",
            "http://100.64.1.1/",
            "https://192.168.0.1/admin",
        ] {
            let message = refusal(fetch.get(url));
            assert!(message.starts_with("Refused to fetch"), "{url}: {message}");
            assert!(message.contains("public internet"), "{url}: {message}");
        }
    }

    /// Local names are refused by name, and the resolver is never asked.
    #[test]
    fn local_names_are_refused_before_any_lookup() {
        let lookup = ScriptedLookup::new(vec![vec![ip("93.184.215.14")]]);
        let fetch = fetch_with(lookup.clone());
        for (url, word) in [
            ("http://localhost/", "this computer"),
            ("http://LOCALHOST.:8080/", "this computer"),
            ("http://app.localhost/", "this computer"),
            (
                "http://metadata.google.internal/computeMetadata/v1/",
                "metadata",
            ),
            ("http://metadata.goog/", "metadata"),
            ("http://metadata/", "metadata"),
            ("http://instance-data/latest/", "metadata"),
            ("http://printer.local/", ".local"),
            ("http://service.corp.internal/", ".internal"),
            ("http://router.home.arpa/", ".home.arpa"),
            ("http://intranet/", "single-label"),
        ] {
            let message = refusal(fetch.get(url));
            assert!(message.contains(word), "{url}: {message}");
        }
        assert_eq!(lookup.calls(), 0, "no local name may reach the resolver");
    }

    #[test]
    fn only_http_and_https_are_fetched() {
        let fetch = SafeFetch::builder().build().unwrap();
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "gopher://example.com/",
        ] {
            let message = refusal(fetch.get(url));
            assert!(message.contains("only http and https"), "{url}: {message}");
        }
        assert!(fetch.get("https://example.com/").is_ok());
        assert!(fetch.get("http://example.com/").is_ok());
    }

    /// One internal address beside public ones refuses the whole name, so a
    /// connector that falls back from a dead public address never reaches it.
    #[tokio::test]
    async fn a_name_with_any_internal_address_is_refused() {
        let lookup = ScriptedLookup::new(vec![vec![ip("93.184.215.14"), ip("10.0.0.7")]]);
        let error = match Policy::default()
            .resolve(lookup.as_ref(), "mixed.example")
            .await
        {
            Err(ResolveError::Refused(refused)) => refused.to_string(),
            Err(ResolveError::Lookup(error)) => panic!("not a refusal: {error}"),
            Ok(addresses) => panic!("resolved to {addresses:?}"),
        };
        assert!(error.contains("10.0.0.7"), "{error}");
        assert!(error.contains("private"), "{error}");
    }

    #[tokio::test]
    async fn each_internal_answer_is_refused_by_the_resolver() {
        for answer in [
            "127.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.168.1.20",
            "::1",
            "fd00:ec2::254",
            "::ffff:10.0.0.1",
            "ff02::1",
        ] {
            let lookup = ScriptedLookup::new(vec![vec![ip(answer)]]);
            match Policy::default()
                .resolve(lookup.as_ref(), "rebound.example")
                .await
            {
                Err(ResolveError::Refused(refused)) => {
                    assert!(refused.to_string().contains(answer), "{refused}")
                }
                Err(ResolveError::Lookup(error)) => panic!("{answer}: not a refusal: {error}"),
                Ok(addresses) => panic!("{answer}: resolved to {addresses:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_public_answer_is_returned_whole() {
        let answer = vec![
            ip("93.184.215.14"),
            ip("2606:2800:21f:cb07:6820:80da:af6b:8b2c"),
        ];
        let lookup = ScriptedLookup::new(vec![answer.clone()]);
        match Policy::default()
            .resolve(lookup.as_ref(), "example.com")
            .await
        {
            Ok(addresses) => assert_eq!(addresses, answer),
            Err(_) => panic!("a public name must resolve"),
        }
    }

    /// The resolver is installed in the client: a name whose answer is
    /// loopback is refused at connect time, and the server gets nothing. The
    /// refusal is found through reqwest's error, which calls it a connect error.
    #[tokio::test]
    async fn the_client_refuses_a_name_that_resolves_to_loopback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("local secret"))
            .expect(0)
            .mount(&server)
            .await;
        let lookup = ScriptedLookup::new(vec![vec![ip("127.0.0.1")]]);
        let fetch = fetch_with(lookup.clone());
        let url = format!("http://looks-public.example:{}/", server.address().port());
        let error = match fetch.get(&url).unwrap().send().await {
            Ok(response) => panic!("fetched {}", response.status()),
            Err(error) => error,
        };
        assert!(error.is_connect(), "reqwest reports it as a connect error");
        let error = FetchError::from(error);
        let refused = error.refusal().expect("the refusal is found in the chain");
        assert!(refused.to_string().contains("loopback"), "{refused}");
        assert_eq!(lookup.calls(), 1);
    }

    /// DNS rebinding: the first answer is the one the connection uses, and the
    /// resolver is asked once. The stub's second answer, an internal address,
    /// is never asked for, so there is no window between check and connect.
    #[tokio::test]
    async fn the_checked_answer_is_the_address_that_is_dialled() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("pinned"))
            .expect(1)
            .mount(&server)
            .await;
        // 127.0.0.1 stands in for a public address here, and 10.0.0.7 for the
        // rebound one.
        let lookup = ScriptedLookup::new(vec![vec![ip("127.0.0.1")], vec![ip("10.0.0.7")]]);
        let fetch = SafeFetch::builder()
            .lookup(lookup.clone())
            .admit_for_tests(ip("127.0.0.1"))
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let url = format!("http://rebind.example:{}/", server.address().port());
        let body = fetch
            .get(&url)
            .unwrap()
            .send()
            .await
            .expect("the checked answer is fetched")
            .text()
            .await
            .unwrap();
        assert_eq!(body, "pinned");
        assert_eq!(lookup.calls(), 1, "one lookup, whose answer was dialled");
    }

    /// A redirect hop to an IP literal is checked like the first URL, before
    /// anything is sent to it. The first server is admitted on its own port; the
    /// second is a loopback server on another port, which is not.
    #[tokio::test]
    async fn a_redirect_to_a_loopback_literal_is_refused() {
        let inner = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("local secret"))
            .expect(0)
            .mount(&inner)
            .await;
        let outer = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/admin", inner.uri()).as_str()),
            )
            .expect(1)
            .mount(&outer)
            .await;
        let _admitted = test_support::admit_local_server(*outer.address());

        let fetch = SafeFetch::builder().build().unwrap();
        let error = match fetch.get(&outer.uri()).unwrap().send().await {
            Ok(response) => panic!("followed the redirect: {}", response.status()),
            Err(error) => FetchError::from(error),
        };
        let refused = error.refusal().expect("the hop is refused");
        assert!(
            refused
                .to_string()
                .starts_with("Refused to follow a redirect to"),
            "{refused}"
        );
        assert!(refused.to_string().contains("loopback"), "{refused}");
    }

    /// A redirect hop to a host name is resolved by the checked resolver when
    /// it connects, so a public page cannot bounce the fetch inward by name.
    #[tokio::test]
    async fn a_redirect_to_a_name_that_resolves_inward_is_refused() {
        let inner = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("local secret"))
            .expect(0)
            .mount(&inner)
            .await;
        let outer = MockServer::start().await;
        let location = format!("http://inward.example:{}/admin", inner.address().port());
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(301).insert_header("location", location.as_str()))
            .expect(1)
            .mount(&outer)
            .await;
        let _admitted = test_support::admit_local_server(*outer.address());

        let lookup = ScriptedLookup::new(vec![vec![ip("127.0.0.1")]]);
        let fetch = fetch_with(lookup.clone());
        let error = match fetch.get(&outer.uri()).unwrap().send().await {
            Ok(response) => panic!("followed the redirect: {}", response.status()),
            Err(error) => FetchError::from(error),
        };
        let refused = error.refusal().expect("the hop's name is refused");
        assert!(refused.to_string().contains("inward.example"), "{refused}");
        assert!(refused.to_string().contains("loopback"), "{refused}");
        assert_eq!(lookup.calls(), 1);
    }

    /// Redirects within an admitted server are followed, and the chain is
    /// capped (a regression guard for the custom policy, which replaced
    /// reqwest's own limit).
    #[tokio::test]
    async fn redirects_are_followed_up_to_the_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/end"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/end"))
            .respond_with(ResponseTemplate::new(200).set_body_string("arrived"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/loop"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/loop"))
            .mount(&server)
            .await;
        let _admitted = test_support::admit_local_server(*server.address());
        let fetch = SafeFetch::builder().build().unwrap();

        let body = fetch
            .get(&format!("{}/start", server.uri()))
            .unwrap()
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "arrived");

        let error = fetch
            .get(&format!("{}/loop", server.uri()))
            .unwrap()
            .send()
            .await
            .expect_err("an endless redirect must stop");
        assert!(error.is_redirect(), "{error}");
        assert!(FetchError::from(error).refusal().is_none());
        let loops = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() == "/loop")
            .count();
        assert_eq!(loops, MAX_REDIRECTS, "{loops} requests to /loop");
    }

    /// The admission seam covers one exact port and ends with its guard.
    #[tokio::test]
    async fn an_admitted_server_is_admitted_on_its_own_port_only() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let fetch = SafeFetch::builder().build().unwrap();
        let other_port = format!("http://127.0.0.1:{}/", server.address().port() ^ 1);
        {
            let _admitted = test_support::admit_local_server(*server.address());
            assert!(fetch.get(&server.uri()).is_ok());
            assert!(fetch.get(&other_port).is_err());
        }
        assert!(fetch.get(&server.uri()).is_err(), "the guard has dropped");
    }

    /// `test-support` turns the admission seam on, so only a
    /// `[dev-dependencies]` entry may enable it: a normal dependency would
    /// compile the seam into a shipped binary.
    #[test]
    fn test_support_is_enabled_only_by_dev_dependencies() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let mut checked = 0;
        for entry in std::fs::read_dir(crates).unwrap() {
            let manifest = entry.unwrap().path().join("Cargo.toml");
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            checked += 1;
            let mut section = String::new();
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('[') {
                    section = trimmed.to_string();
                    continue;
                }
                if trimmed.starts_with('#') || !trimmed.contains("test-support") {
                    continue;
                }
                let is_own_feature_table =
                    manifest.ends_with("biorouter-mcp/Cargo.toml") && section == "[features]";
                assert!(
                    section.contains("dev-dependencies") || is_own_feature_table,
                    "{} enables `test-support` in {section}: {trimmed}",
                    manifest.display()
                );
            }
        }
        assert!(
            checked > 3,
            "found only {checked} manifests under {}",
            crates.display()
        );
    }
}
