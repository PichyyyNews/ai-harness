use super::ChatMessage;
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, Local, TimeZone, Utc};
use serde::Deserialize;
use std::time::{Duration, Instant};

const GAP_THRESHOLD: ChronoDuration = ChronoDuration::hours(1);
const NETWORK_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const NETWORK_FAILURE_BACKOFF: Duration = Duration::from_secs(5 * 60);
const MAX_CACHED_NETWORK_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(3);
const IP_TIMEZONE_ENDPOINT: &str = "https://ipwho.is/";
const TIME_API_CURRENT_ZONE_ENDPOINT: &str = "https://timeapi.io/api/time/current/zone";

/// A live temporal reference for one inference pass. The network source is
/// optional; the OS clock remains a fully offline fallback.
#[derive(Debug, Clone)]
pub struct TimeContext {
    pub iso_8601_local: String,
    pub utc_timestamp: String,
    pub date_human: String,
    pub time_human: String,
    pub day_of_week: String,
    pub timezone_offset: String,
    pub timezone_id: Option<String>,
    pub source: TimeSource,
    pub clock_delta_seconds: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSource {
    NetworkLocation,
    CachedNetworkLocation,
    SystemClock,
}

impl TimeSource {
    fn description(self) -> &'static str {
        match self {
            Self::NetworkLocation => "network time calibrated for the network-estimated location",
            Self::CachedNetworkLocation => "cached network time calibration for the network-estimated location",
            Self::SystemClock => "local operating system clock (offline fallback)",
        }
    }
}

/// Keeps only the minimum metadata needed to project a verified network time
/// forward. It never stores the public IP address, city, coordinates, or the
/// provider's complete response.
#[derive(Debug)]
pub struct TimeAuthority {
    cached_network: Option<NetworkClock>,
    last_attempt: Option<Instant>,
}

#[derive(Debug)]
struct NetworkClock {
    local_at_sync: DateTime<FixedOffset>,
    synced_at: Instant,
    timezone_id: Option<String>,
    clock_delta_seconds: i64,
}

impl Default for TimeAuthority {
    fn default() -> Self {
        Self { cached_network: None, last_attempt: None }
    }
}

impl TimeContext {
    pub fn system_now() -> Self {
        Self::from_local(Local::now(), TimeSource::SystemClock, None, None)
    }

    fn from_local(
        local_now: DateTime<Local>,
        source: TimeSource,
        timezone_id: Option<String>,
        clock_delta_seconds: Option<i64>,
    ) -> Self {
        Self {
            iso_8601_local: local_now.to_rfc3339(),
            utc_timestamp: local_now.with_timezone(&Utc).to_rfc3339(),
            date_human: local_now.format("%Y-%m-%d").to_string(),
            time_human: local_now.format("%H:%M:%S").to_string(),
            day_of_week: local_now.format("%A").to_string(),
            timezone_offset: local_now.format("%:z").to_string(),
            timezone_id,
            source,
            clock_delta_seconds,
        }
    }

    fn from_fixed_offset(
        local_now: DateTime<FixedOffset>,
        source: TimeSource,
        timezone_id: Option<String>,
        clock_delta_seconds: Option<i64>,
    ) -> Self {
        Self {
            iso_8601_local: local_now.to_rfc3339(),
            utc_timestamp: local_now.with_timezone(&Utc).to_rfc3339(),
            date_human: local_now.format("%Y-%m-%d").to_string(),
            time_human: local_now.format("%H:%M:%S").to_string(),
            day_of_week: local_now.format("%A").to_string(),
            timezone_offset: local_now.format("%:z").to_string(),
            timezone_id,
            source,
            clock_delta_seconds,
        }
    }

    pub fn to_system_prompt_header(&self) -> String {
        let timezone = self.timezone_id.as_deref().unwrap_or("system-local timezone");
        let calibration = self.clock_delta_seconds.map(|delta| {
            format!("\n- Network-to-system clock difference at calibration: {delta:+} seconds")
        }).unwrap_or_default();
        format!(
            "[System Temporal Context - authoritative]\n- Current local date: {}\n- Current local time: {} ({})\n- Time zone: {} (UTC{})\n- Local ISO-8601 reference: {}\n- Current UTC reference: {}\n- Time source: {}{}\nUse this live system context for questions about dates, times, and relative words such as today or tomorrow. Do not expose IP-derived location details and do not claim a different current time unless the user provides one.",
            self.date_human,
            self.time_human,
            self.day_of_week,
            timezone,
            self.timezone_offset,
            self.iso_8601_local,
            self.utc_timestamp,
            self.source.description(),
            calibration,
        )
    }
}

/// Resolves time dynamically from two sources: the OS clock and a network
/// response whose timezone is inferred by the provider from the public IP.
/// The API is consulted at most every 15 minutes; network failure never blocks
/// chat beyond a short timeout and always falls back to local time.
pub fn resolve(authority: &mut TimeAuthority) -> TimeContext {
    let now = Instant::now();
    if let Some(network) = authority.cached_network.as_ref() {
        if now.duration_since(network.synced_at) < NETWORK_REFRESH_INTERVAL {
            return context_from_network(network, TimeSource::CachedNetworkLocation);
        }
    }

    let may_retry = authority.last_attempt.map(|last| now.duration_since(last) >= NETWORK_FAILURE_BACKOFF).unwrap_or(true);
    if may_retry {
        authority.last_attempt = Some(now);
        if let Some(network) = fetch_network_clock() {
            authority.cached_network = Some(network);
            return context_from_network(authority.cached_network.as_ref().expect("network clock was stored"), TimeSource::NetworkLocation);
        }
    }

    if let Some(network) = authority.cached_network.as_ref() {
        if now.duration_since(network.synced_at) < MAX_CACHED_NETWORK_AGE {
            return context_from_network(network, TimeSource::CachedNetworkLocation);
        }
    }
    TimeContext::system_now()
}

pub fn system_message(context: &TimeContext) -> ChatMessage {
    ChatMessage {
        role: "system".to_string(),
        content: context.to_system_prompt_header(),
        created_at: None,
    }
}

fn context_from_network(network: &NetworkClock, source: TimeSource) -> TimeContext {
    let elapsed = ChronoDuration::from_std(network.synced_at.elapsed()).unwrap_or_default();
    let current = network.local_at_sync.checked_add_signed(elapsed).unwrap_or(network.local_at_sync);
    TimeContext::from_fixed_offset(current, source, network.timezone_id.clone(), Some(network.clock_delta_seconds))
}

#[derive(Debug, Deserialize)]
struct IpWhoResponse {
    #[serde(default)]
    success: bool,
    timezone: Option<IpWhoTimezone>,
}

#[derive(Debug, Deserialize)]
struct IpWhoTimezone {
    id: Option<String>,
    utc: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TimeApiResponse {
    #[serde(rename = "dateTime")]
    datetime: String,
}

fn fetch_network_clock() -> Option<NetworkClock> {
    let client = reqwest::blocking::Client::builder()
        .timeout(NETWORK_TIMEOUT)
        .user_agent("AI Harness time authority")
        .build().ok()?;
    let response = client
        .get(IP_TIMEZONE_ENDPOINT)
        .send().ok()?
        .error_for_status().ok()?
        .json::<IpWhoResponse>().ok()?;
    let timezone = response.timezone?;
    if !response.success { return None; }
    let timezone_id = timezone.id?;
    if !is_safe_timezone_id(&timezone_id) { return None; }
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("timeZone", &timezone_id);
    let response = client
        .get(format!("{TIME_API_CURRENT_ZONE_ENDPOINT}?{}", query.finish()))
        .send().ok()?
        .error_for_status().ok()?
        .json::<TimeApiResponse>().ok()?;
    let local_at_sync = parse_network_datetime(&response.datetime, timezone.utc.as_deref()?)?;
    let clock_delta_seconds = local_at_sync.with_timezone(&Utc).signed_duration_since(Utc::now()).num_seconds();
    Some(NetworkClock {
        local_at_sync,
        synced_at: Instant::now(),
        timezone_id: Some(timezone_id),
        clock_delta_seconds,
    })
}

fn is_safe_timezone_id(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '+' | '/'))
}

fn parse_network_datetime(value: &str, timezone_offset: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok().or_else(|| {
        DateTime::parse_from_str(
            &format!("{value}{timezone_offset}"),
            "%Y-%m-%dT%H:%M:%S%.f%:z",
        ).ok()
    })
}

/// Adds invisible-to-the-UI system notes between saved turns when a user
/// returns after a meaningful pause. A negative delta is ignored because it
/// can occur after a manual clock adjustment or DST correction.
pub fn inject_gap_markers(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut processed = Vec::with_capacity(messages.len());
    let mut previous_timestamp = None;

    for message in messages {
        let current_timestamp = message.created_at.as_deref().and_then(parse_timestamp);
        if let (Some(previous), Some(current)) = (previous_timestamp, current_timestamp) {
            let elapsed = current.signed_duration_since(previous);
            if elapsed >= GAP_THRESHOLD {
                processed.push(ChatMessage {
                    role: "system".to_string(),
                    content: format_gap_marker(elapsed, current),
                    created_at: Some(current.to_rfc3339()),
                });
            }
        }
        processed.push(message.clone());
        if current_timestamp.is_some() {
            previous_timestamp = current_timestamp;
        }
    }

    processed
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value).ok().map(|value| value.with_timezone(&Utc)).or_else(|| {
        value.parse::<i64>().ok().and_then(|milliseconds| Utc.timestamp_millis_opt(milliseconds).single())
    })
}

fn format_gap_marker(elapsed: ChronoDuration, current: DateTime<Utc>) -> String {
    if elapsed.num_hours() < 24 {
        format!("[System Note: {} hours have elapsed since the previous message.]", elapsed.num_hours())
    } else {
        format!(
            "[System Note: {} days have elapsed since the previous message. Current UTC date: {}.]",
            elapsed.num_days(),
            current.format("%Y-%m-%d"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{inject_gap_markers, is_safe_timezone_id, parse_network_datetime, TimeContext, TimeSource};
    use crate::engine::ChatMessage;
    use chrono::{FixedOffset, TimeZone};

    fn message(role: &str, content: &str, timestamp: &str) -> ChatMessage {
        ChatMessage { role: role.to_string(), content: content.to_string(), created_at: Some(timestamp.to_string()) }
    }

    #[test]
    fn adds_a_marker_after_an_hour() {
        let history = vec![
            message("user", "first", "2026-07-21T01:00:00Z"),
            message("assistant", "second", "2026-07-21T02:30:00Z"),
        ];
        let result = inject_gap_markers(&history);
        assert_eq!(result.len(), 3);
        assert!(result[1].content.contains("1 hours have elapsed"));
    }

    #[test]
    fn ignores_negative_clock_changes() {
        let history = vec![
            message("user", "first", "2026-07-21T02:00:00Z"),
            message("assistant", "second", "2026-07-21T01:00:00Z"),
        ];
        assert_eq!(inject_gap_markers(&history).len(), 2);
    }

    #[test]
    fn temporal_header_identifies_the_network_source_without_location_details() {
        let offset = FixedOffset::east_opt(7 * 3_600).expect("valid offset");
        let value = offset.with_ymd_and_hms(2026, 7, 21, 20, 0, 0).single().expect("valid date");
        let context = TimeContext::from_fixed_offset(value, TimeSource::NetworkLocation, Some("Asia/Bangkok".to_string()), Some(2));
        let header = context.to_system_prompt_header();
        assert!(header.contains("network time calibrated"));
        assert!(header.contains("Asia/Bangkok"));
        assert!(!header.contains("latitude"));
    }

    #[test]
    fn only_allows_iana_style_timezone_identifiers_for_network_requests() {
        assert!(is_safe_timezone_id("Asia/Bangkok"));
        assert!(!is_safe_timezone_id("../private-network"));
        assert!(!is_safe_timezone_id("https://example.com"));
    }

    #[test]
    fn parses_time_api_timestamp_using_the_ip_timezone_offset() {
        let parsed = parse_network_datetime("2026-07-21T20:00:00.123", "+07:00").expect("parse network timestamp");
        assert_eq!(parsed.format("%:z").to_string(), "+07:00");
    }
}
