/// The fragments a failure carries when the host was never reached. They come from the
/// transports the providers actually use: reqwest and hyper build the first few, the resolver
/// and libc the middle ones, and librespot the last.
const LOST: &[&str] = &[
    "error sending request",
    "error trying to connect",
    "connection refused",
    "connection reset",
    "connection closed",
    "connection aborted",
    "connection timed out",
    "connect timeout",
    "network is unreachable",
    "network is down",
    "host is unreachable",
    "no route to host",
    "dns error",
    "failed to lookup address",
    "name resolution",
    "nodename nor servname",
    "no such host",
    "operation timed out",
    "request timed out",
    "timed out",
    "os error 101",
    "os error 110",
    "os error 113",
    "could not be reached",
    "could not reach",
    "unreachable",
    "offline",
];

/// The fragments a failure carries when the provider is shedding load or rate limiting the
/// account. Apple's own wrapper says its gateway is busy, and the HTTP stacks spell 429 and 503
/// out by their reason phrases.
const BUSY: &[&str] = &[
    "gateway is busy",
    "too many requests",
    "rate limit",
    "rate-limit",
    "ratelimit",
    "service unavailable",
];

/// The fragment a failure carries when the provider refuses the network itself. Both
/// `ytmusic::NetworkBlocked` and `SignInProblem::Blocked` say it.
const BLOCKED: &str = "refuses requests from this network";

/// Whether a failure reads as a lost connection rather than something the provider refused.
/// Every provider flattens its errors into one line before a screen sees them, so this matches
/// the reason's text and never inspects the error chain.
pub fn offline(reason: &str) -> bool {
    let reason = reason.to_lowercase();
    LOST.iter().any(|fragment| reason.contains(fragment))
}

/// Whether a failure reads as the provider asking the app to slow down, which a retry after a
/// short wait usually clears. Like `offline`, it matches the flattened reason's text.
pub fn busy(reason: &str) -> bool {
    let reason = reason.to_lowercase();
    BUSY.iter().any(|fragment| reason.contains(fragment))
}

/// Whether a failure reads as the provider refusing the network the app connects from, as
/// Google does for some VPN servers. Like `offline`, it matches the flattened reason's text.
pub fn blocked(reason: &str) -> bool {
    reason.to_lowercase().contains(BLOCKED)
}
