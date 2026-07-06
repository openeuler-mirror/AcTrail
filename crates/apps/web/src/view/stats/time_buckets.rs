use std::collections::BTreeMap;

use super::model::{BucketTotal, Rollup};

const MS_PER_MINUTE: u64 = 60_000;
const MS_PER_HOUR: u64 = 60 * MS_PER_MINUTE;
const MS_PER_DAY: u64 = 24 * MS_PER_HOUR;

pub(super) fn bucket_start_ms(value: u64, rollup: Rollup) -> u64 {
    match rollup {
        Rollup::Minute => value / MS_PER_MINUTE * MS_PER_MINUTE,
        Rollup::Hour => value / MS_PER_HOUR * MS_PER_HOUR,
        Rollup::Day => value / MS_PER_DAY * MS_PER_DAY,
        Rollup::Week => {
            let days = i64::try_from(value / MS_PER_DAY).unwrap_or(i64::MAX);
            let monday_days = days - (days + 3).rem_euclid(7);
            u64::try_from(monday_days.max(0)).unwrap_or(0) * MS_PER_DAY
        }
        Rollup::Month => month_start_ms(value),
    }
}

pub(super) fn bucket_points(rollup: Rollup, values: BTreeMap<u64, u64>) -> Vec<BucketTotal> {
    values
        .into_iter()
        .map(|(bucket, value)| BucketTotal {
            bucket_key: format!("{}:{bucket}", rollup.as_str()),
            bucket_label: bucket_label(bucket, rollup),
            bucket_start_ms: bucket,
            value,
        })
        .collect()
}

fn bucket_label(bucket: u64, rollup: Rollup) -> String {
    let (year, month, day, hour, minute) = timestamp_parts(bucket);
    match rollup {
        Rollup::Minute => format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}"),
        Rollup::Hour => format!("{year:04}-{month:02}-{day:02} {hour:02}:00"),
        Rollup::Day | Rollup::Week => format!("{year:04}-{month:02}-{day:02}"),
        Rollup::Month => format!("{year:04}-{month:02}"),
    }
}

fn month_start_ms(value: u64) -> u64 {
    let days = i64::try_from(value / MS_PER_DAY).unwrap_or(i64::MAX);
    let (year, month, _) = civil_from_days(days);
    u64::try_from(days_from_civil(year, month, 1).max(0)).unwrap_or(0) * MS_PER_DAY
}

fn timestamp_parts(value: u64) -> (i32, u32, u32, u32, u32) {
    let days = i64::try_from(value / MS_PER_DAY).unwrap_or(i64::MAX);
    let millis_of_day = value % MS_PER_DAY;
    let (year, month, day) = civil_from_days(days);
    let hour = u32::try_from(millis_of_day / MS_PER_HOUR).unwrap_or(0);
    let minute = u32::try_from((millis_of_day % MS_PER_HOUR) / MS_PER_MINUTE).unwrap_or(0);
    (year, month, day, hour, minute)
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
