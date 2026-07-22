//! Origins and addresses: what `poid.net` may reach (SPEC §7.2.5).
//!
//! Two separate jobs live here, and conflating them is the classic mistake:
//!
//! 1. **Is this origin allowed?** — a string comparison against the manifest
//!    allowlist intersected with the user's approval.
//! 2. **Is this address allowed?** — a numeric check against the ranges that
//!    reach the user's own machine and network.
//!
//! Doing only (1) and then handing the hostname to an HTTP client is defeated
//! by DNS rebinding: the name resolves to a public address while it is being
//! checked and to `169.254.169.254` when it is connected to. So the caller
//! resolves, checks **every** address with [`classify`], and connects to an
//! address that passed — never to the name it checked. This module cannot
//! enforce that on its own; it can only make the check cheap and total so
//! there is no excuse for skipping it.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};

use crate::error::PolicyError;

/// A web origin: scheme, host, port. Nothing else — no path, no userinfo.
///
/// Stored canonically (lowercased host, default port elided) so that equality
/// is a plain comparison. `https://API.example.com:443` and
/// `https://api.example.com` are the same origin and compare equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Origin {
    scheme: Scheme,
    host: String,
    port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Scheme {
    Http,
    Https,
}

impl Scheme {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }

    const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

impl Origin {
    /// Parses an origin, rejecting anything that is not exactly one.
    ///
    /// Refusals are deliberately broad. An allowlist entry is user-visible
    /// security configuration, and a value that needs interpretation is a
    /// value two implementations will interpret differently.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::MalformedOrigin`] for a value carrying
    /// credentials, a path, a wildcard, whitespace, or CSP-significant
    /// punctuation, and for any scheme other than `http`/`https`.
    pub fn parse(value: &str) -> Result<Self, PolicyError> {
        let bad = |why: &'static str| PolicyError::MalformedOrigin {
            value: value.to_owned(),
            why,
        };

        // Characters that would let an allowlist entry break out of whatever
        // it is later interpolated into — a CSP directive, most importantly.
        // Refused before parsing so no later stage has to think about them.
        if value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ';' | '\'' | '"' | ','))
        {
            return Err(bad(
                "it contains whitespace or punctuation that is not allowed here",
            ));
        }
        if value.contains('*') {
            return Err(bad("wildcards are not supported; list each origin"));
        }

        let (scheme, rest) = value
            .split_once("://")
            .ok_or_else(|| bad("it has no scheme"))?;
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err(bad("only http and https origins can be allowlisted")),
        };

        // An origin ends at the authority. A trailing bare slash is tolerated
        // because people write it; anything after it is a path, which an
        // origin does not have.
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains('/') {
            return Err(bad("an origin has no path"));
        }
        if rest.contains('?') || rest.contains('#') {
            return Err(bad("an origin has no query or fragment"));
        }
        if rest.contains('@') {
            return Err(bad("an origin must not carry credentials"));
        }
        if rest.is_empty() {
            return Err(bad("it has no host"));
        }

        let (host, port) =
            split_authority(rest, scheme).ok_or_else(|| bad("its port is not a number"))?;
        let host = host.to_ascii_lowercase();
        if !valid_host(&host) {
            return Err(bad("its host is not a valid hostname or IP literal"));
        }

        Ok(Self { scheme, host, port })
    }

    /// The origin **of a request URL** — path, query and fragment discarded.
    ///
    /// Distinct from [`Origin::parse`], and the distinction is load-bearing.
    /// `parse` reads an *allowlist entry*, where a path is a mistake worth
    /// refusing: the user wrote something that does not mean what they think.
    /// This reads a *URL an application asked for*, where a path is normal and
    /// simply not part of the origin the allowlist is compared against.
    ///
    /// Everything else `parse` refuses is still refused here — an embedded
    /// credential, a non-HTTP scheme, whitespace — because those are as wrong
    /// in a request as in a rule.
    ///
    /// # Errors
    ///
    /// [`PolicyError::MalformedOrigin`] when no usable origin can be read.
    pub fn of_url(url: &str) -> Result<Self, PolicyError> {
        let bad = |why: &'static str| PolicyError::MalformedOrigin {
            value: url.to_owned(),
            why,
        };
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| bad("it has no scheme"))?;
        // The authority ends at the first `/`, `?` or `#`.
        let authority = rest
            .split(['/', '?', '#'])
            .next()
            .ok_or_else(|| bad("it has no host"))?;
        Self::parse(&format!("{scheme}://{authority}"))
    }

    /// The host, lowercased. An IPv6 literal keeps its brackets.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, with the scheme's default filled in.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The scheme.
    #[must_use]
    pub const fn scheme(&self) -> &'static str {
        self.scheme.as_str()
    }

    /// The host as an IP address, when the host is a literal rather than a
    /// name. A literal needs no resolution — and no chance to rebind.
    #[must_use]
    pub fn literal_address(&self) -> Option<IpAddr> {
        if let Some(inner) = self
            .host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
        {
            return inner.parse::<Ipv6Addr>().ok().map(IpAddr::V6);
        }
        self.host.parse::<Ipv4Addr>().ok().map(IpAddr::V4)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme.as_str(), self.host)?;
        if self.port != self.scheme.default_port() {
            write!(f, ":{}", self.port)?;
        }
        Ok(())
    }
}

impl TryFrom<String> for Origin {
    type Error = PolicyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<Origin> for String {
    fn from(origin: Origin) -> Self {
        origin.to_string()
    }
}

fn split_authority(rest: &str, scheme: Scheme) -> Option<(&str, u16)> {
    // An IPv6 literal is bracketed, so the port colon is the one after `]`.
    if let Some(end) = rest.find(']') {
        let (host, tail) = rest.split_at(end + 1);
        return match tail {
            "" => Some((host, scheme.default_port())),
            _ => tail
                .strip_prefix(':')
                .and_then(|p| p.parse().ok())
                .map(|port| (host, port)),
        };
    }
    match rest.rsplit_once(':') {
        Some((host, port)) => port.parse().ok().map(|port| (host, port)),
        None => Some((rest, scheme.default_port())),
    }
}

fn valid_host(host: &str) -> bool {
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner.parse::<Ipv6Addr>().is_ok();
    }
    if host.is_empty() || host.len() > 253 || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// The set of origins an application may reach: the manifest's request
/// intersected with the user's approval (SPEC §9.1 — the manifest is a
/// request, never a grant).
#[derive(Debug, Clone, Default)]
pub struct NetworkPolicy {
    approved: Vec<Origin>,
    allow_private_addresses: bool,
}

impl NetworkPolicy {
    /// Builds a policy from the origins the manifest declared and the origins
    /// the user approved. The result is the **intersection**: an origin the
    /// user approved but the manifest never asked for is not reachable, and
    /// neither is the reverse.
    #[must_use]
    pub fn new(declared: &[Origin], approved: &[Origin]) -> Self {
        Self {
            approved: declared
                .iter()
                .filter(|o| approved.contains(o))
                .cloned()
                .collect(),
            allow_private_addresses: false,
        }
    }

    /// Permits private and loopback destinations.
    ///
    /// This exists for enterprise policy (SPEC §13), where an organisation
    /// deliberately points POIDs at an intranet host. It is not a development
    /// convenience and has no default-on path: the only way to set it is an
    /// explicit managed-policy decision, made outside the application's reach.
    #[must_use]
    pub const fn with_private_addresses_allowed(mut self, allow: bool) -> Self {
        self.allow_private_addresses = allow;
        self
    }

    /// The origins that survived the intersection.
    #[must_use]
    pub fn approved(&self) -> &[Origin] {
        &self.approved
    }

    /// Whether a request to `origin` may be attempted at all.
    ///
    /// # Errors
    ///
    /// [`PolicyError::OriginNotAllowed`] when the origin is not in the
    /// intersection. This is checked before any name is resolved, so a
    /// disallowed origin does not even produce a DNS query the network can
    /// observe.
    pub fn check_origin(&self, origin: &Origin) -> Result<(), PolicyError> {
        if self.approved.contains(origin) {
            Ok(())
        } else {
            Err(PolicyError::OriginNotAllowed {
                origin: origin.to_string(),
            })
        }
    }

    /// Whether the broker may open a connection to `address`.
    ///
    /// # Errors
    ///
    /// [`PolicyError::AddressBlocked`] when the address is in a range that
    /// reaches the user's own machine or network.
    pub fn check_address(&self, host: &str, address: IpAddr) -> Result<(), PolicyError> {
        match classify(address) {
            None => Ok(()),
            Some(reason) if self.allow_private_addresses => {
                let _ = reason;
                Ok(())
            }
            Some(reason) => Err(PolicyError::AddressBlocked {
                host: host.to_owned(),
                address,
                reason,
            }),
        }
    }

    /// Filters resolved addresses down to those that may be connected to.
    ///
    /// The caller **must** connect to one of the returned addresses rather
    /// than re-resolving the name, which is the whole point of the exercise.
    ///
    /// # Errors
    ///
    /// [`PolicyError::NoUsableAddress`] when every resolved address is
    /// blocked. A partially-blocked result is not an error: a name that
    /// resolves to both a public and a private address is served by the
    /// public one, which is what a browser would do.
    pub fn usable_addresses(
        &self,
        host: &str,
        resolved: &[IpAddr],
    ) -> Result<Vec<IpAddr>, PolicyError> {
        let usable: Vec<IpAddr> = resolved
            .iter()
            .copied()
            .filter(|a| self.check_address(host, *a).is_ok())
            .collect();
        if usable.is_empty() {
            return Err(PolicyError::NoUsableAddress {
                host: host.to_owned(),
            });
        }
        Ok(usable)
    }
}

/// Why an address must not be connected to, or `None` when it is a normal
/// public destination.
///
/// Total by construction: every address falls into exactly one branch, and
/// there is no "unknown" case that could default to allowed.
#[must_use]
pub fn classify(address: IpAddr) -> Option<&'static str> {
    match address {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(address: Ipv4Addr) -> Option<&'static str> {
    let [a, b, _, _] = address.octets();
    let bits = u32::from(address);
    let in_net = |net: [u8; 4], prefix: u32| -> bool {
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        (bits & mask) == (u32::from(Ipv4Addr::new(net[0], net[1], net[2], net[3])) & mask)
    };

    if in_net([0, 0, 0, 0], 8) {
        return Some("an unspecified or this-network address");
    }
    if a == 10 || in_net([172, 16, 0, 0], 12) || (a == 192 && b == 168) {
        return Some("a private address");
    }
    if in_net([100, 64, 0, 0], 10) {
        return Some("a carrier-grade NAT address");
    }
    if a == 127 {
        return Some("a loopback address");
    }
    if in_net([169, 254, 0, 0], 16) {
        // Includes 169.254.169.254, the cloud instance-metadata endpoint —
        // the single most valuable SSRF target there is.
        return Some("a link-local address");
    }
    if in_net([192, 0, 0, 0], 24) {
        return Some("an IETF protocol assignment");
    }
    if in_net([192, 0, 2, 0], 24) || in_net([198, 51, 100, 0], 24) || in_net([203, 0, 113, 0], 24) {
        return Some("a documentation address");
    }
    if in_net([192, 88, 99, 0], 24) {
        return Some("a 6to4 relay anycast address");
    }
    if in_net([198, 18, 0, 0], 15) {
        return Some("a benchmarking address");
    }
    if address.is_multicast() {
        return Some("a multicast address");
    }
    if in_net([240, 0, 0, 0], 4) {
        // 255.255.255.255 lands here too.
        return Some("a reserved address");
    }
    None
}

fn classify_v6(address: Ipv6Addr) -> Option<&'static str> {
    // Embedded IPv4 first, and recursively: an attacker who cannot use
    // 127.0.0.1 will happily try ::ffff:127.0.0.1, ::ffff:7f00:1,
    // 64:ff9b::127.0.0.1 or 2002:7f00:0001::. Every one of these is the same
    // destination wearing a different hat, so each is unwrapped and judged as
    // the IPv4 address it really is.
    if let Some(v4) = embedded_v4(address) {
        return classify_v4(v4);
    }

    let segments = address.segments();
    if address.is_unspecified() {
        return Some("an unspecified address");
    }
    if address.is_loopback() {
        return Some("a loopback address");
    }
    if segments[0] & 0xfe00 == 0xfc00 {
        return Some("a unique-local address");
    }
    if segments[0] & 0xffc0 == 0xfe80 {
        return Some("a link-local address");
    }
    if address.is_multicast() {
        return Some("a multicast address");
    }
    if segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
        return Some("a discard-only address");
    }
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return Some("a documentation address");
    }
    if segments[0] == 0x2001 && segments[1] & 0xfe00 == 0 {
        // 2001::/23, the IETF protocol assignments block (Teredo included).
        return Some("an IETF protocol assignment");
    }
    None
}

/// The IPv4 address embedded in an IPv6 address, for every embedding a host
/// might actually route: IPv4-mapped, IPv4-compatible, NAT64 and 6to4.
fn embedded_v4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = address.segments();
    let from_pair = |hi: u16, lo: u16| Ipv4Addr::from(u32::from(hi) << 16 | u32::from(lo));

    // ::ffff:0:0/96 — IPv4-mapped.
    if s[0..5] == [0, 0, 0, 0, 0] && s[5] == 0xffff {
        return Some(from_pair(s[6], s[7]));
    }
    // ::/96 — deprecated IPv4-compatible. `::` and `::1` are handled by their
    // own checks, so only genuinely embedded addresses reach here.
    if s[0..6] == [0, 0, 0, 0, 0, 0] && !(s[6] == 0 && s[7] <= 1) {
        return Some(from_pair(s[6], s[7]));
    }
    // 64:ff9b::/96 and 64:ff9b:1::/48 — NAT64.
    if s[0] == 0x0064 && s[1] == 0xff9b {
        return Some(from_pair(s[6], s[7]));
    }
    // 2002::/16 — 6to4, address in the next 32 bits.
    if s[0] == 0x2002 {
        return Some(from_pair(s[1], s[2]));
    }
    None
}
