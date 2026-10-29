use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};

use super::client_addr::{Cidr, client_addr, untrusted_forwarder};
use super::error::ApiError;
use super::request_id::request_id;

/// 20 sign-in requests a minute per client, after a burst of 10.
const SIGN_IN_INTERVAL: Duration = Duration::from_secs(3);
const SIGN_IN_BURST: u32 = 10;
const MAX_TRACKED: usize = 10_000;

/// A browser navigating to a limited sign-in route lands on the sign-in error page.
const RETRY_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
    <meta http-equiv=\"refresh\" content=\"0; url=/signin-error?reason=too_many_requests\">\
    <title>Explorer</title></head><body>\
    <a href=\"/signin-error?reason=too_many_requests\">Too many sign-in attempts</a>\
    </body></html>";

/// A token bucket per key, stored as the instant its bucket is full again (GCRA): an entry
/// whose instant has passed is idle and can be dropped.
pub struct RateLimiter {
    interval: Duration,
    tolerance: Duration,
    max_tracked: usize,
    full_at: Mutex<HashMap<IpAddr, Instant>>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(interval: Duration, burst: u32, max_tracked: usize) -> Self {
        Self {
            interval,
            tolerance: interval * burst.saturating_sub(1),
            max_tracked,
            full_at: Mutex::new(HashMap::new()),
        }
    }

    /// Takes a token for `key` at `now`; `Err` is how long until one is available.
    pub fn check(&self, key: IpAddr, now: Instant) -> Result<(), Duration> {
        let mut full_at = self.full_at.lock().unwrap_or_else(PoisonError::into_inner);
        if !full_at.contains_key(&key) && full_at.len() >= self.max_tracked {
            full_at.retain(|_, at| *at > now);
            // Still full of active clients: forget the one closest to a full bucket.
            if full_at.len() >= self.max_tracked
                && let Some(oldest) = full_at.iter().min_by_key(|(_, at)| **at).map(|(k, _)| *k)
            {
                full_at.remove(&oldest);
            }
        }
        let at = full_at
            .get(&key)
            .copied()
            .filter(|at| *at > now)
            .unwrap_or(now);
        let ahead = at.saturating_duration_since(now);
        if let Some(wait) = ahead.checked_sub(self.tolerance).filter(|w| !w.is_zero()) {
            return Err(wait);
        }
        full_at.insert(key, at + self.interval);
        Ok(())
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.full_at.lock().unwrap().len()
    }
}

#[derive(Clone)]
pub struct SignInLimit {
    limiter: Arc<RateLimiter>,
    trusted_proxies: Arc<[Cidr]>,
    warned_untrusted_forwarder: Arc<AtomicBool>,
}

impl SignInLimit {
    #[must_use]
    pub fn new(trusted_proxies: &[Cidr]) -> Self {
        Self {
            limiter: Arc::new(RateLimiter::new(
                SIGN_IN_INTERVAL,
                SIGN_IN_BURST,
                MAX_TRACKED,
            )),
            trusted_proxies: trusted_proxies.into(),
            warned_untrusted_forwarder: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Logs once per process the first peer whose `X-Forwarded-For` is ignored: the usual
    /// sign that the reverse proxy is missing from `EXPLORER_TRUSTED_PROXIES`.
    fn warn_untrusted_forwarder(&self, peer: IpAddr, headers: &HeaderMap) -> bool {
        let Some(peer) = untrusted_forwarder(peer, headers, &self.trusted_proxies) else {
            return false;
        };
        if self
            .warned_untrusted_forwarder
            .swap(true, Ordering::Relaxed)
        {
            return false;
        }
        tracing::warn!(
            peer = %peer,
            "a sign-in request from an untrusted peer carries X-Forwarded-For, which is ignored; \
             if this peer is the reverse proxy, add it to EXPLORER_TRUSTED_PROXIES"
        );
        true
    }
}

/// One IPv6 client usually holds a whole /64, so it shares one bucket.
fn bucket_key(addr: IpAddr) -> IpAddr {
    match addr {
        IpAddr::V4(_) => addr,
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & !u128::from(u64::MAX))),
    }
}

/// Limits a sign-in route per client address. A request without a peer address (served
/// in-process, as in tests) is not limited; the server always records one.
pub async fn limit_sign_in(
    State(limit): State<SignInLimit>,
    request: Request,
    next: Next,
) -> Response {
    let Some(ConnectInfo(peer)) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied()
    else {
        return next.run(request).await;
    };
    limit.warn_untrusted_forwarder(peer.ip(), request.headers());
    let client = client_addr(peer.ip(), request.headers(), &limit.trusted_proxies);
    match limit.limiter.check(bucket_key(client), Instant::now()) {
        Ok(()) => next.run(request).await,
        Err(wait) => {
            tracing::warn!(
                request_id = request_id(request.headers()),
                client = %client,
                path = request.uri().path(),
                "too many sign-in requests"
            );
            too_many_requests(&request, wait)
        }
    }
}

fn too_many_requests(request: &Request, wait: Duration) -> Response {
    let wants_html = request
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"));
    let mut response = if wants_html {
        (StatusCode::TOO_MANY_REQUESTS, Html(RETRY_PAGE)).into_response()
    } else {
        ApiError::too_many_requests().into_response()
    };
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(seconds.max(1)));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn sign_in_limiter(max_tracked: usize) -> RateLimiter {
        RateLimiter::new(SIGN_IN_INTERVAL, SIGN_IN_BURST, max_tracked)
    }

    #[test]
    fn allows_the_burst_then_one_request_every_three_seconds() {
        let limiter = sign_in_limiter(MAX_TRACKED);
        let start = Instant::now();
        let a = ip("198.51.100.1");
        for _ in 0..10 {
            assert_eq!(limiter.check(a, start), Ok(()));
        }
        assert_eq!(limiter.check(a, start), Err(Duration::from_secs(3)));
        let later = start + Duration::from_secs(1);
        assert_eq!(limiter.check(a, later), Err(Duration::from_secs(2)));
        let next = start + Duration::from_secs(3);
        assert_eq!(limiter.check(a, next), Ok(()));
        assert!(limiter.check(a, next).is_err());
    }

    #[test]
    fn sustains_twenty_requests_a_minute() {
        let limiter = sign_in_limiter(MAX_TRACKED);
        let start = Instant::now();
        let a = ip("198.51.100.1");
        let allowed = (0..600)
            .map(|i| start + Duration::from_millis(i * 1000))
            .filter(|now| limiter.check(a, *now).is_ok())
            .count();
        // The burst, then 20 a minute over the next ten minutes.
        assert_eq!(allowed, 10 + 200 - 1);
    }

    #[test]
    fn limits_each_address_on_its_own_and_resets_when_idle() {
        let limiter = sign_in_limiter(MAX_TRACKED);
        let start = Instant::now();
        let (a, b) = (ip("198.51.100.1"), ip("198.51.100.2"));
        for _ in 0..10 {
            limiter.check(a, start).unwrap();
        }
        assert!(limiter.check(a, start).is_err());
        assert_eq!(limiter.check(b, start), Ok(()));
        let idle = start + Duration::from_secs(30);
        for _ in 0..10 {
            assert_eq!(limiter.check(a, idle), Ok(()));
        }
        assert!(limiter.check(a, idle).is_err());
    }

    #[test]
    fn tracked_addresses_are_bounded() {
        let limiter = sign_in_limiter(3);
        let start = Instant::now();
        for i in 1..=3 {
            limiter
                .check(ip(&format!("198.51.100.{i}")), start)
                .unwrap();
        }
        let idle = start + Duration::from_secs(3);
        limiter.check(ip("198.51.100.4"), idle).unwrap();
        assert_eq!(limiter.tracked(), 1, "idle entries are dropped first");

        for i in 5..=6 {
            limiter.check(ip(&format!("198.51.100.{i}")), idle).unwrap();
        }
        let later = idle + Duration::from_millis(1);
        limiter.check(ip("198.51.100.7"), later).unwrap();
        assert_eq!(limiter.tracked(), 3);
    }

    #[test]
    fn an_ipv6_client_shares_one_bucket_per_64() {
        assert_eq!(bucket_key(ip("2001:db8:1:2:a::1")), ip("2001:db8:1:2::"));
        assert_eq!(bucket_key(ip("198.51.100.1")), ip("198.51.100.1"));
    }

    #[test]
    fn warns_once_about_a_forwarding_peer_that_is_not_trusted() {
        let limit = SignInLimit::new(&["172.18.0.0/16".parse().unwrap()]);
        let mut headers = HeaderMap::new();
        assert!(!limit.warn_untrusted_forwarder(ip("172.19.0.2"), &headers));
        headers.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.7"));
        assert!(!limit.warn_untrusted_forwarder(ip("172.18.0.2"), &headers));
        assert!(limit.warn_untrusted_forwarder(ip("172.19.0.2"), &headers));
        assert!(!limit.warn_untrusted_forwarder(ip("172.19.0.3"), &headers));
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        let request = Request::new(axum::body::Body::empty());
        let res = too_many_requests(&request, Duration::from_millis(2001));
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(res.headers()[header::RETRY_AFTER], "3");
        let res = too_many_requests(&request, Duration::from_millis(1));
        assert_eq!(res.headers()[header::RETRY_AFTER], "1");
    }
}
