use super::*;

/// HTTP 429 is the server asking us to slow down, not a failure. It used not
/// to be retryable, which turned "slow down" into a per-game error — and, for
/// the daemon, into a six-hour backoff on a target that was perfectly fine.
#[test]
fn rate_limiting_is_retryable() {
    assert!(is_retryable(&ScrapeError::RateLimit { retry_after: None }));
    assert!(is_retryable(&ScrapeError::ServerError {
        status: 503,
        message: "busy".to_owned(),
    }));

    // A missing game or bad credentials will not improve on retry.
    assert!(!is_retryable(&ScrapeError::NotFound {
        warnings: Vec::new()
    }));
    assert!(!is_retryable(&ScrapeError::InvalidCredentials(
        "bad".to_owned()
    )));
    assert!(!is_retryable(&ScrapeError::QuotaExceeded {
        used: 20_000,
        max: 20_000,
    }));
}

/// `Retry-After` is the server stating exactly when it will serve us again;
/// guessing anything shorter earns another 429.
#[test]
fn a_server_supplied_retry_after_wins_over_the_backoff_curve() {
    let error = ScrapeError::RateLimit {
        retry_after: Some(17),
    };

    assert_eq!(
        rate_limit_backoff(&error, 0),
        Some(Duration::from_secs(17)),
        "first attempt should honor the header, not INITIAL_BACKOFF"
    );
    assert_eq!(
        rate_limit_backoff(&error, 3),
        Some(Duration::from_secs(17)),
        "later attempts should not inflate a stated wait"
    );
}

/// Without a header the wait grows, but stays bounded: a server asking for
/// longer than the ceiling is really telling us to come back later, which is
/// the caller's quota reserve to handle, not a sleeping request.
#[test]
fn rate_limit_backoff_grows_but_is_bounded() {
    let error = ScrapeError::RateLimit { retry_after: None };

    assert_eq!(rate_limit_backoff(&error, 0), Some(INITIAL_BACKOFF));
    assert_eq!(rate_limit_backoff(&error, 1), Some(INITIAL_BACKOFF * 2));
    assert_eq!(
        rate_limit_backoff(&error, 20),
        Some(MAX_RATE_LIMIT_BACKOFF),
        "an unbounded doubling would park a worker for hours"
    );
}

#[test]
fn non_rate_limit_errors_have_no_rate_limit_wait() {
    assert_eq!(
        rate_limit_backoff(
            &ScrapeError::ServerError {
                status: 500,
                message: String::new(),
            },
            2
        ),
        None
    );
}

/// The global gate is what makes a 429 slow every worker down. Moving only the
/// worker that saw it just feeds the next one into the same limit.
#[tokio::test(start_paused = true)]
async fn a_rate_limit_hold_delays_every_worker() {
    let client = ScreenScraperClient::offline_for_tests();
    client.set_request_rate(6_000); // 10ms spacing — not what we are measuring

    client.hold_request_slot(Duration::from_secs(30)).await;
    let start = tokio::time::Instant::now();
    client.wait_for_request_slot().await;

    assert!(
        start.elapsed() >= Duration::from_secs(30),
        "the gate should hold for the full stated wait, got {:?}",
        start.elapsed()
    );
}

/// A hold must never pull the gate earlier than it already is.
#[tokio::test(start_paused = true)]
async fn a_shorter_hold_does_not_shorten_a_longer_one() {
    let client = ScreenScraperClient::offline_for_tests();

    client.hold_request_slot(Duration::from_mins(1)).await;
    client.hold_request_slot(Duration::from_secs(1)).await;
    let start = tokio::time::Instant::now();
    client.wait_for_request_slot().await;

    assert!(start.elapsed() >= Duration::from_mins(1));
}

/// Headroom is unknown until a response has carried quota. Reporting "0 left"
/// would make an unattended run refuse to start.
#[tokio::test]
async fn headroom_is_unknown_before_any_response() {
    let client = ScreenScraperClient::offline_for_tests();

    assert_eq!(client.quota_headroom().await, None);
}

#[tokio::test]
async fn headroom_is_the_budget_minus_what_today_spent() {
    let client = ScreenScraperClient::offline_for_tests();
    *client.quota.lock().await = Some(UserQuota {
        requeststoday: Some("19950".to_owned()),
        maxrequestsperday: Some("20000".to_owned()),
    });

    assert_eq!(client.quota_headroom().await, Some((50, 20_000)));
}
