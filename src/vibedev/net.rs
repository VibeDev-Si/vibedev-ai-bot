//! Keeps the bot's requests to API addresses supplied by users on the public internet.
//!
//! Users choose the API's base URL, so the bot must not become a way into the private network it runs in
//! (other containers, the cloud provider's metadata service, the homeserver's admin API, etc.):
//!
//! - [`parse_base_url`] accepts only `https://` URLs without credentials, query or fragment, whose host is a public
//!   IP address or a domain name (not `localhost` or a single-label name like `synapse`).
//! - [`check_resolves_publicly`] resolves the domain name and rejects it if any address is not public
//!   (see [`is_public_ip`]).
//! - [`harden`] makes HTTP clients resolve through [`PublicOnlyResolver`], which does the same check on every
//!   connection and only hands the checked addresses to the connector (so a DNS answer changing after the check -
//!   DNS rebinding - can't redirect requests), and turns off redirects and proxies.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use url::{Host, Url};

/// The longest base URL accepted.
pub const MAX_BASE_URL_LENGTH: usize = 2048;

const DNS_TIMEOUT: Duration = Duration::from_secs(5);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    TooLong,
    Invalid,
    NotHttps,
    HasCredentials,
    HasQueryOrFragment,
    LocalHostName,
    NonPublicAddress,
    ResolutionFailed,
}

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "base_url is empty",
            Self::TooLong => "base_url is too long",
            Self::Invalid => "base_url is not a valid URL",
            Self::NotHttps => "base_url must start with https://",
            Self::HasCredentials => "base_url must not contain a user name or password",
            Self::HasQueryOrFragment => "base_url must not contain a query (?) or fragment (#)",
            Self::LocalHostName => {
                "base_url must use a public domain name (not localhost or a name without dots)"
            }
            Self::NonPublicAddress => {
                "base_url must point to a public address (not a loopback, private, link-local, shared, multicast or reserved one)"
            }
            Self::ResolutionFailed => "the host name of base_url could not be resolved",
        })
    }
}

impl std::error::Error for UrlError {}

/// A base URL which passed the checks that need no DNS (see [`parse_base_url`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseUrl {
    url: Url,
    normalized: String,
}

impl BaseUrl {
    /// The URL without a trailing slash (e.g. `https://api.example.com/v1`).
    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    /// The URL of an endpoint below this one (e.g. `models`).
    pub fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.normalized, path.trim_start_matches('/'))
    }

    fn host(&self) -> Option<Host<&str>> {
        self.url.host()
    }

    fn port(&self) -> u16 {
        self.url.port_or_known_default().unwrap_or(443)
    }
}

/// Checks a base URL as far as possible without DNS: `https://`, no credentials, query or fragment,
/// and a public IP address or a domain name with dots (other than `localhost`).
pub fn parse_base_url(input: &str) -> Result<BaseUrl, UrlError> {
    let input = input.trim();

    if input.is_empty() {
        return Err(UrlError::Empty);
    }

    if input.len() > MAX_BASE_URL_LENGTH {
        return Err(UrlError::TooLong);
    }

    let url = Url::parse(input).map_err(|_| UrlError::Invalid)?;

    if url.scheme() != "https" {
        return Err(UrlError::NotHttps);
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlError::HasCredentials);
    }

    if url.query().is_some() || url.fragment().is_some() {
        return Err(UrlError::HasQueryOrFragment);
    }

    match url.host() {
        None => return Err(UrlError::Invalid),
        Some(Host::Ipv4(ip)) => check_public(IpAddr::V4(ip))?,
        Some(Host::Ipv6(ip)) => check_public(IpAddr::V6(ip))?,
        Some(Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();

            if domain.is_empty()
                || !domain.contains('.')
                || domain == "localhost"
                || domain.ends_with(".localhost")
            {
                return Err(UrlError::LocalHostName);
            }
        }
    }

    let normalized = url.as_str().trim_end_matches('/').to_owned();

    Ok(BaseUrl { url, normalized })
}

fn check_public(ip: IpAddr) -> Result<(), UrlError> {
    if is_public_ip(ip) {
        Ok(())
    } else {
        Err(UrlError::NonPublicAddress)
    }
}

/// Resolves the base URL's host (if it's a domain name) and checks that all its addresses are public.
pub async fn check_resolves_publicly(base_url: &BaseUrl) -> Result<(), UrlError> {
    match base_url.host() {
        Some(Host::Domain(domain)) => resolve_public(domain, base_url.port()).await.map(|_| ()),
        // Checked when parsing.
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => Ok(()),
        None => Err(UrlError::Invalid),
    }
}

/// Resolves a host name, returning its addresses only if every one of them is public.
async fn resolve_public(host: &str, port: u16) -> Result<Vec<SocketAddr>, UrlError> {
    let addresses: Vec<SocketAddr> =
        match tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((host, port))).await {
            Ok(Ok(addresses)) => addresses.collect(),
            Ok(Err(_)) | Err(_) => return Err(UrlError::ResolutionFailed),
        };

    if addresses.is_empty() {
        return Err(UrlError::ResolutionFailed);
    }

    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(UrlError::NonPublicAddress);
    }

    Ok(addresses)
}

/// Tells whether an address is publicly routable, i.e. not loopback, private, link-local, shared (CGNAT),
/// multicast, documentation, benchmarking or otherwise reserved.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();

    let special = a == 0 // "this network" (0.0.0.0/8)
        || a == 10 // private (10.0.0.0/8)
        || (a == 100 && (b & 0xc0) == 64) // shared address space / CGNAT (100.64.0.0/10)
        || a == 127 // loopback (127.0.0.0/8)
        || (a == 169 && b == 254) // link-local, including cloud metadata services (169.254.0.0/16)
        || (a == 172 && (b & 0xf0) == 16) // private (172.16.0.0/12)
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments (192.0.0.0/24)
        || (a == 192 && b == 0 && c == 2) // documentation (192.0.2.0/24)
        || (a == 192 && b == 88 && c == 99) // 6to4 relay anycast (192.88.99.0/24)
        || (a == 192 && b == 168) // private (192.168.0.0/16)
        || (a == 198 && (b & 0xfe) == 18) // benchmarking (198.18.0.0/15)
        || (a == 198 && b == 51 && c == 100) // documentation (198.51.100.0/24)
        || (a == 203 && b == 0 && c == 113) // documentation (203.0.113.0/24)
        || a >= 224; // multicast (224.0.0.0/4), reserved (240.0.0.0/4) and broadcast

    !special
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    // IPv4-mapped addresses (::ffff:a.b.c.d) are IPv4 addresses.
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        return is_public_ipv4(ipv4);
    }

    let segments = ip.segments();

    // NAT64 (64:ff9b::/96) reaches the IPv4 address in the last 32 bits.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let [high, low] = [segments[6], segments[7]];
        return is_public_ipv4(Ipv4Addr::new(
            (high >> 8) as u8,
            high as u8,
            (low >> 8) as u8,
            low as u8,
        ));
    }

    // Only global unicast addresses (2000::/3) are public. This leaves out the unspecified and loopback
    // addresses, IPv4-compatible ones, unique local (fc00::/7), link-local (fe80::/10), site-local and multicast ones.
    if (segments[0] & 0xe000) != 0x2000 {
        return false;
    }

    let special = (segments[0] == 0x2001 && segments[1] < 0x0200) // IETF protocol assignments, incl. Teredo (2001::/23)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation (2001:db8::/32)
        || segments[0] == 0x2002 // 6to4, which embeds IPv4 addresses (2002::/16)
        || (segments[0] == 0x3fff && (segments[1] & 0xf000) == 0); // documentation (3fff::/20)

    !special
}

/// A DNS resolver which only returns addresses if all of them are public (see [`is_public_ip`]).
///
/// HTTP clients only connect to the addresses a resolver returns, so the checked addresses are the ones used.
/// IP addresses in URLs don't go through resolvers: [`parse_base_url`] checks them.
#[derive(Clone, Copy, Debug, Default)]
pub struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();

        Box::pin(async move {
            // Boxed as the error itself (not as a `Box<UrlError>`), so that callers can find it among the causes.
            let addresses = resolve_public(&host, 0)
                .await
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)?;
            let addresses: reqwest::dns::Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

/// Restricts an HTTP client to public https addresses: every resolved address is checked
/// (see [`PublicOnlyResolver`]), redirects aren't followed and proxies aren't used
/// (a proxy would resolve names itself).
pub fn harden(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder
        .dns_resolver(PublicOnlyResolver)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .https_only(true)
        .connect_timeout(CONNECT_TIMEOUT)
}

/// Builds a client restricted by [`harden`].
///
/// Building only fails when TLS can't be initialized, in which case `reqwest::Client::new()` (what other
/// clients fall back to) panics too. There is no fallback to an unrestricted client.
pub fn hardened_client(builder: reqwest::ClientBuilder) -> reqwest::Client {
    harden(builder)
        .build()
        .expect("Failed building the HTTP client for public addresses (TLS initialization failed)")
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{UrlError, check_resolves_publicly, is_public_ip, parse_base_url};

    fn ip(value: &str) -> IpAddr {
        value.parse().expect(value)
    }

    #[test]
    fn non_public_addresses_are_recognized() {
        for value in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "10.255.255.255",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "127.1.2.3",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:1",
            "64:ff9b:1::1",
            "100::1",
            "2001::1",
            "2001:2::1",
            "2001:db8::1",
            "2002:7f00:1::1",
            "3fff::1",
            "fc00::1",
            "fd12:3456:789a::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
        ] {
            assert!(!is_public_ip(ip(value)), "{value} should not be public");
        }
    }

    #[test]
    fn public_addresses_are_recognized() {
        for value in [
            "1.1.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "192.0.1.1",
            "192.169.0.1",
            "198.17.255.255",
            "198.20.0.1",
            "223.255.255.254",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "2400:cb00::1",
        ] {
            assert!(is_public_ip(ip(value)), "{value} should be public");
        }
    }

    #[test]
    fn base_urls_must_be_https_without_extras() {
        let url = parse_base_url(" https://api.vibedev.studio/v1/ ").unwrap();
        assert_eq!(url.as_str(), "https://api.vibedev.studio/v1");
        assert_eq!(
            url.endpoint("models"),
            "https://api.vibedev.studio/v1/models"
        );

        // Host names are normalized, the default port is dropped and other ports are kept.
        assert_eq!(
            parse_base_url("https://API.Example.COM:443")
                .unwrap()
                .as_str(),
            "https://api.example.com"
        );
        assert_eq!(
            parse_base_url("https://api.example.com:8443/openai/v1")
                .unwrap()
                .as_str(),
            "https://api.example.com:8443/openai/v1"
        );

        for (input, expected) in [
            ("", UrlError::Empty),
            ("   ", UrlError::Empty),
            ("not a url", UrlError::Invalid),
            ("https://", UrlError::Invalid),
            ("http://api.example.com/v1", UrlError::NotHttps),
            ("ftp://api.example.com/v1", UrlError::NotHttps),
            ("file:///etc/passwd", UrlError::NotHttps),
            (
                "https://user:pass@api.example.com/v1",
                UrlError::HasCredentials,
            ),
            ("https://user@api.example.com/v1", UrlError::HasCredentials),
            (
                "https://api.example.com/v1?key=x",
                UrlError::HasQueryOrFragment,
            ),
            ("https://api.example.com/v1#x", UrlError::HasQueryOrFragment),
            ("https://localhost/v1", UrlError::LocalHostName),
            ("https://LOCALHOST./v1", UrlError::LocalHostName),
            ("https://api.localhost/v1", UrlError::LocalHostName),
            ("https://synapse:8008/", UrlError::LocalHostName),
            ("https://127.0.0.1/v1", UrlError::NonPublicAddress),
            ("https://10.1.2.3/v1", UrlError::NonPublicAddress),
            ("https://169.254.169.254/latest", UrlError::NonPublicAddress),
            ("https://[::1]/v1", UrlError::NonPublicAddress),
            ("https://[fd00::1]/v1", UrlError::NonPublicAddress),
            (
                "https://[::ffff:192.168.0.1]/v1",
                UrlError::NonPublicAddress,
            ),
            // Other ways of writing IPv4 addresses are normalized by the URL parser.
            ("https://2130706433/", UrlError::NonPublicAddress),
            ("https://0x7f.0.0.1/", UrlError::NonPublicAddress),
            ("https://127.1/", UrlError::NonPublicAddress),
        ] {
            assert_eq!(parse_base_url(input), Err(expected), "{input}");
        }

        let too_long = format!("https://api.example.com/{}", "a".repeat(2048));
        assert_eq!(parse_base_url(&too_long), Err(UrlError::TooLong));

        // Public IP addresses are fine.
        assert!(parse_base_url("https://8.8.8.8/v1").is_ok());
        assert!(parse_base_url("https://[2606:4700:4700::1111]/v1").is_ok());
    }

    #[tokio::test]
    async fn host_names_resolving_to_non_public_addresses_are_rejected() {
        // Not rejected by name: `localtest.me`-style names resolve to loopback addresses on the public DNS, but
        // tests can't rely on the network. A literal public address needs no resolution.
        let url = parse_base_url("https://8.8.8.8/v1").unwrap();
        assert_eq!(check_resolves_publicly(&url).await, Ok(()));

        // `resolve_public` refuses loopback answers (`localhost` always resolves locally).
        assert_eq!(
            super::resolve_public("localhost", 443).await,
            Err(UrlError::NonPublicAddress)
        );
    }

    #[tokio::test]
    async fn the_resolver_refuses_non_public_answers() {
        use reqwest::dns::Resolve;

        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        let result = super::PublicOnlyResolver.resolve(name).await;
        let err = result.err().expect("loopback addresses must be refused");
        assert!(err.to_string().contains("public address"), "{err}");
    }

    #[tokio::test]
    async fn hardened_clients_refuse_plain_http_and_private_addresses() {
        let client = super::hardened_client(reqwest::Client::builder());

        // https_only: plain http is refused before connecting.
        assert!(client.get("http://example.com/").send().await.is_err());

        // A name resolving to loopback is refused by the resolver.
        let err = client
            .get("https://localhost:9/")
            .send()
            .await
            .expect_err("loopback must be refused");
        assert!(format!("{err:?}").contains("NonPublicAddress"), "{err:?}");
    }
}
