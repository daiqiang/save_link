//! 本机持久化时间的兼容解析与规范化。

use chrono::{DateTime, Local, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use std::cmp::Ordering;

/// 兼容云协议 RFC 3339 和 v0.1.0 本机数据库中的旧时间格式。
pub fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Some(timestamp.with_timezone(&Utc));
    }

    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    Local
        .from_local_datetime(&parsed)
        .earliest()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

/// SQLite 统一保存固定秒精度的 UTC RFC 3339，保证字符串序等于时间序。
pub fn normalize_timestamp(value: &str) -> Option<String> {
    parse_timestamp(value).map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Secs, true))
}

pub fn same_instant(left: &str, right: &str) -> bool {
    match (parse_timestamp(left), parse_timestamp(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

pub fn compare_timestamps(left: &str, right: &str) -> Ordering {
    match (parse_timestamp(left), parse_timestamp(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_timestamp_normalizes_to_fixed_utc() {
        assert_eq!(
            normalize_timestamp("2026-08-05T21:25:00+08:00").as_deref(),
            Some("2026-08-05T13:25:00Z")
        );
        assert!(same_instant(
            "2026-08-05T21:25:00+08:00",
            "2026-08-05T13:25:00Z"
        ));
    }
}
