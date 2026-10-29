//! Strict Valid_Timestamp grammar shared with `desktop/src/lib/instant.ts`.
//!
//! ```text
//! timestamp = date [ sep time [ offset ] ]
//! date      = YYYY "-" MM "-" DD          ; YYYY 0001–9999, valid day for month/year
//! sep       = "T" / "t" / " "
//! time      = HH ":" mm [ ":" ss [ "." 1*9DIGIT ] ]   ; HH 00–23, mm/ss 00–59
//! offset    = "Z" / "z" / ( "+" / "-" ) HH ":" mm     ; offset HH 00–23, mm 00–59
//! ```
//!
//! Date-only values denote 00:00:00 UTC; date-times without an offset are UTC.
//! No leading or trailing whitespace is accepted.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

/// Reads exactly `n` ASCII digits starting at `at`.
fn digits(bytes: &[u8], at: usize, n: usize) -> Option<u32> {
    let slice = bytes.get(at..at + n)?;
    let mut value = 0u32;
    for &b in slice {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(b - b'0');
    }
    Some(value)
}

fn expect(bytes: &[u8], at: usize, byte: u8) -> Option<()> {
    (bytes.get(at) == Some(&byte)).then_some(())
}

/// Parses a Valid_Timestamp per the grammar above. `None` for anything else.
pub fn parse_instant(value: &str) -> Option<DateTime<Utc>> {
    let b = value.as_bytes();

    // date
    let year = digits(b, 0, 4)?;
    expect(b, 4, b'-')?;
    let month = digits(b, 5, 2)?;
    expect(b, 7, b'-')?;
    let day = digits(b, 8, 2)?;
    if year == 0 {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(year as i32, month, day)?;
    if b.len() == 10 {
        return Some(Utc.from_utc_datetime(&date.and_time(NaiveTime::MIN)));
    }

    // sep + HH:mm
    if !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let hour = digits(b, 11, 2)?;
    expect(b, 13, b':')?;
    let minute = digits(b, 14, 2)?;
    let mut pos = 16;

    // [":" ss [ "." 1*9DIGIT ]]
    let mut second = 0u32;
    let mut nanos = 0u32;
    if b.get(pos) == Some(&b':') {
        second = digits(b, pos + 1, 2)?;
        pos += 3;
        if b.get(pos) == Some(&b'.') {
            pos += 1;
            let start = pos;
            while pos < b.len() && b[pos].is_ascii_digit() {
                pos += 1;
            }
            let count = pos - start;
            if !(1..=9).contains(&count) {
                return None;
            }
            nanos = digits(b, start, count)? * 10u32.pow((9 - count) as u32);
        }
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let time = NaiveTime::from_hms_nano_opt(hour, minute, second, nanos)?;
    let local = NaiveDateTime::new(date, time);

    // [offset]
    let offset_secs: i32 = match b.get(pos) {
        None => 0,
        Some(b'Z' | b'z') => {
            pos += 1;
            0
        }
        Some(&sign @ (b'+' | b'-')) => {
            let oh = digits(b, pos + 1, 2)?;
            expect(b, pos + 3, b':')?;
            let om = digits(b, pos + 4, 2)?;
            if oh > 23 || om > 59 {
                return None;
            }
            pos += 6;
            let secs = (oh * 3600 + om * 60) as i32;
            if sign == b'-' {
                -secs
            } else {
                secs
            }
        }
        Some(_) => return None,
    };
    if pos != b.len() {
        return None;
    }
    let offset = FixedOffset::east_opt(offset_secs)?;
    let fixed = offset.from_local_datetime(&local).single()?;
    Some(fixed.with_timezone(&Utc))
}

/// Running "latest Valid_Timestamp" selection for one group (Req 1.3, 1.4).
///
/// The result is the maximum under (instant, then string bytes), so it does not
/// depend on the order candidates are offered.
#[derive(Debug, Default, Clone)]
pub struct LatestTimestamp {
    best: Option<(DateTime<Utc>, String)>,
}

impl LatestTimestamp {
    /// Ignores invalid strings. Replaces the current best when the candidate's
    /// instant is later, or equal with a greater string by byte order.
    pub fn offer(&mut self, raw: &str) {
        let Some(instant) = parse_instant(raw) else {
            return;
        };
        let replace = match &self.best {
            None => true,
            Some((best_instant, best_raw)) => {
                (instant, raw.as_bytes()) > (*best_instant, best_raw.as_bytes())
            }
        };
        if replace {
            self.best = Some((instant, raw.to_string()));
        }
    }

    /// The stored string of the best candidate, verbatim.
    pub fn into_value(self) -> Option<String> {
        self.best.map(|(_, raw)| raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: &str) -> Option<i64> {
        parse_instant(value).map(|d| d.timestamp_millis())
    }

    #[test]
    fn parses_design_examples() {
        assert_eq!(ms("2026-10-01"), Some(1_790_812_800_000));
        assert_eq!(ms("2026-10-01T00:00:01Z"), Some(1_790_812_801_000));
        assert_eq!(ms("2026-10-01T09:00:00+09:00"), Some(1_790_812_800_000));
        assert_eq!(ms("2026-10-01 12:30"), Some(1_790_857_800_000));
        let d = parse_instant("2026-10-01T00:00:00.123456789Z").unwrap();
        assert_eq!(d.timestamp_millis(), 1_790_812_800_123);
        assert_eq!(d.timestamp_subsec_nanos(), 123_456_789);
    }

    #[test]
    fn accepts_lowercase_negative_offset_and_leap_day() {
        assert_eq!(ms("2026-10-01t00:00:00z"), ms("2026-10-01T00:00:00Z"));
        assert_eq!(ms("2026-09-30T19:00:00-05:00"), ms("2026-10-01T00:00:00Z"));
        assert_eq!(ms("2026-10-01T00:00:00.5"), Some(1_790_812_800_500));
        assert!(parse_instant("2024-02-29").is_some());
        assert!(parse_instant("0001-01-01").is_some());
        assert!(parse_instant("9999-12-31T23:59:59.999999999Z").is_some());
    }

    #[test]
    fn rejects_outside_grammar() {
        for bad in [
            "",
            " 2026-10-01",
            "2026-10-01 ",
            "2026-02-30",
            "2025-02-29",
            "2026-13-01",
            "2026-10-01T24:00",
            "2026-10-01T23:60",
            "2026-10-01T23:59:60Z",
            "2026-10-01T00:00:00.Z",
            "2026-10-01T00:00:00.1234567891Z",
            "2026-10-01T00:00+0900",
            "2026-10-01T00:00+24:00",
            "2026-10-01T00:00Z+",
            "2026-10-01T",
            "2026-10-01T1:00",
            "2026-1-01",
            "+2026-10-01",
            "March 4",
            "now",
            "0000-01-01",
            "２026-10-01",
        ] {
            assert!(parse_instant(bad).is_none(), "accepted {bad:?}");
        }
    }

    #[test]
    fn latest_picks_latest_instant_then_greatest_string() {
        let mut latest = LatestTimestamp::default();
        assert_eq!(latest.clone().into_value(), None);
        latest.offer("pending");
        assert_eq!(latest.clone().into_value(), None);
        latest.offer("2026-10-01T01:00:00Z");
        latest.offer("2026-10-01T09:00:00+09:00"); // earlier instant
        assert_eq!(latest.clone().into_value().as_deref(), Some("2026-10-01T01:00:00Z"));

        let mut tie = LatestTimestamp::default();
        tie.offer("2026-10-01T00:00:00Z");
        tie.offer("2026-10-01");
        tie.offer("2026-10-01T09:00:00+09:00");
        tie.offer("zzz");
        assert_eq!(tie.into_value().as_deref(), Some("2026-10-01T09:00:00+09:00"));
    }

    // ---- Property 5 (Rust half) ----------------------------------------------

    use chrono::Duration;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Offset {
        UpperZ,
        LowerZ,
        Signed { negative: bool, hours: u32, minutes: u32 },
    }

    #[derive(Debug, Clone)]
    struct TimeParts {
        sep: char,
        hour: u32,
        minute: u32,
        /// `Some((second, fraction))`; fraction is `(digit_count, value)` with `value < 10^digit_count`.
        seconds: Option<(u32, Option<(u32, u32)>)>,
        offset: Option<Offset>,
    }

    fn days_in_month(year: u32, month: u32) -> u32 {
        let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
        // Year 10000 is fine for chrono, so December 9999 works too.
        let first_next = NaiveDate::from_ymd_opt(ny as i32, nm, 1).unwrap();
        (first_next - Duration::days(1)).day()
    }

    use chrono::Datelike;

    fn date_strategy() -> impl Strategy<Value = (u32, u32, u32)> {
        (1u32..=9999, 1u32..=12).prop_flat_map(|(y, m)| (Just(y), Just(m), 1..=days_in_month(y, m)))
    }

    fn fraction_strategy() -> impl Strategy<Value = (u32, u32)> {
        (1u32..=9).prop_flat_map(|count| (Just(count), 0..10u32.pow(count)))
    }

    fn offset_strategy() -> impl Strategy<Value = Offset> {
        prop_oneof![
            Just(Offset::UpperZ),
            Just(Offset::LowerZ),
            (any::<bool>(), 0u32..=23, 0u32..=59).prop_map(|(negative, hours, minutes)| {
                Offset::Signed { negative, hours, minutes }
            }),
        ]
    }

    fn time_strategy() -> impl Strategy<Value = TimeParts> {
        (
            prop_oneof![Just('T'), Just('t'), Just(' ')],
            0u32..=23,
            0u32..=59,
            proptest::option::of((0u32..=59, proptest::option::of(fraction_strategy()))),
            proptest::option::of(offset_strategy()),
        )
            .prop_map(|(sep, hour, minute, seconds, offset)| TimeParts {
                sep,
                hour,
                minute,
                seconds,
                offset,
            })
    }

    fn format_parts(date: (u32, u32, u32), time: &Option<TimeParts>) -> String {
        let (y, m, d) = date;
        let mut s = format!("{y:04}-{m:02}-{d:02}");
        if let Some(t) = time {
            s.push(t.sep);
            s.push_str(&format!("{:02}:{:02}", t.hour, t.minute));
            if let Some((sec, frac)) = t.seconds {
                s.push_str(&format!(":{sec:02}"));
                if let Some((count, value)) = frac {
                    s.push_str(&format!(".{value:0width$}", width = count as usize));
                }
            }
            match &t.offset {
                None => {}
                Some(Offset::UpperZ) => s.push('Z'),
                Some(Offset::LowerZ) => s.push('z'),
                Some(Offset::Signed { negative, hours, minutes }) => {
                    let sign = if *negative { '-' } else { '+' };
                    s.push_str(&format!("{sign}{hours:02}:{minutes:02}"));
                }
            }
        }
        s
    }

    /// Instant computed from the components without going through the parser.
    fn expected_instant(date: (u32, u32, u32), time: &Option<TimeParts>) -> DateTime<Utc> {
        let naive_date = NaiveDate::from_ymd_opt(date.0 as i32, date.1, date.2).unwrap();
        let Some(t) = time else {
            return naive_date.and_hms_opt(0, 0, 0).unwrap().and_utc();
        };
        let (second, nanos) = match t.seconds {
            None => (0, 0),
            Some((sec, None)) => (sec, 0),
            Some((sec, Some((count, value)))) => (sec, value * 10u32.pow(9 - count)),
        };
        let local = naive_date
            .and_hms_nano_opt(t.hour, t.minute, second, nanos)
            .unwrap()
            .and_utc();
        let offset_minutes: i64 = match &t.offset {
            None | Some(Offset::UpperZ) | Some(Offset::LowerZ) => 0,
            Some(Offset::Signed { negative, hours, minutes }) => {
                let v = i64::from(hours * 60 + minutes);
                if *negative {
                    -v
                } else {
                    v
                }
            }
        };
        local - Duration::minutes(offset_minutes)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Feature: companies-tab-sorting, Property 5: Timestamp parsing round trip and
        /// cross-language agreement. Formatting valid components into a grammar string and
        /// parsing it yields the instant computed from the components (date-only at
        /// 00:00:00 UTC, no offset treated as UTC).
        ///
        /// **Validates: Requirements 1.3, 4.5**
        #[test]
        fn prop_parse_instant_round_trips_components(
            date in date_strategy(),
            time in proptest::option::of(time_strategy()),
        ) {
            let input = format_parts(date, &time);
            let expected = expected_instant(date, &time);
            prop_assert_eq!(parse_instant(&input), Some(expected), "input {:?}", input);
        }
    }

    // ---- Property 3 ----------------------------------------------------------

    /// 2026-10-01T00:00:00Z in epoch seconds.
    const OCT1: i64 = 1_790_812_800;

    /// Hand-written candidates with their expected instant `(epoch secs, nanos)`, or `None`
    /// when invalid. The model below uses these values, never the parser.
    const LATEST_POOL: &[(&str, Option<(i64, u32)>)] = &[
        ("2026-10-01", Some((OCT1, 0))),
        ("2026-10-01T00:00:00Z", Some((OCT1, 0))),
        ("2026-10-01T09:00:00+09:00", Some((OCT1, 0))),
        ("2026-10-01 00:00", Some((OCT1, 0))),
        ("2026-10-01t00:00:00z", Some((OCT1, 0))),
        ("2026-10-01T00:00:00.000000001Z", Some((OCT1, 1))),
        ("2026-10-01T00:00:00.000000001+00:00", Some((OCT1, 1))),
        ("2026-10-01T01:00:00Z", Some((OCT1 + 3600, 0))),
        ("2026-10-01T10:00:00+09:00", Some((OCT1 + 3600, 0))),
        ("2026-09-30T20:00:00-05:00", Some((OCT1 + 3600, 0))),
        ("2025-12-31T23:59:59Z", Some((1_767_225_599, 0))),
        ("2026-10-02", Some((OCT1 + 86_400, 0))),
        ("pending", None),
        ("", None),
        ("now", None),
        ("zzz", None),
        ("2026-02-30", None),
        ("2026-10-01T24:00", None),
        (" 2026-10-01", None),
        ("2026-10-03 ", None),
    ];

    /// Model: maximum valid candidate under (instant, string bytes).
    fn model_latest(indices: &[usize]) -> Option<String> {
        indices
            .iter()
            .filter_map(|&i| {
                let (raw, expected) = LATEST_POOL[i];
                expected.map(|(secs, nanos)| ((secs, nanos), raw.as_bytes(), raw))
            })
            .max_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)))
            .map(|(_, _, raw)| raw.to_string())
    }

    fn fold_latest(indices: &[usize]) -> Option<String> {
        let mut latest = LatestTimestamp::default();
        for &i in indices {
            latest.offer(LATEST_POOL[i].0);
        }
        latest.into_value()
    }

    #[test]
    fn latest_pool_expected_instants_match_parser() {
        for (raw, expected) in LATEST_POOL {
            let parsed = parse_instant(raw).map(|d| (d.timestamp(), d.timestamp_subsec_nanos()));
            assert_eq!(parsed, *expected, "pool entry {raw:?}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Feature: companies-tab-sorting, Property 3: Latest-timestamp selection is
        /// order-independent. Feeding any permutation of a candidate list into
        /// `LatestTimestamp::offer` yields the maximum valid candidate under
        /// (instant, then string bytes), or None when no candidate is valid.
        ///
        /// **Validates: Requirements 1.3, 1.4**
        #[test]
        fn prop_latest_timestamp_is_order_independent(
            (original, shuffled) in proptest::collection::vec(0..LATEST_POOL.len(), 0..16)
                .prop_flat_map(|v| (Just(v.clone()), Just(v).prop_shuffle())),
        ) {
            let expected = model_latest(&original);
            prop_assert_eq!(fold_latest(&original), expected.clone());
            prop_assert_eq!(fold_latest(&shuffled), expected.clone());
            let reversed: Vec<usize> = original.iter().rev().copied().collect();
            prop_assert_eq!(fold_latest(&reversed), expected);
        }
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CorpusValid {
        input: String,
        epoch_ms: i64,
        sub_ms_nanos: u32,
    }

    #[derive(serde::Deserialize)]
    struct Corpus {
        valid: Vec<CorpusValid>,
        invalid: Vec<String>,
    }

    /// Feature: companies-tab-sorting, Property 5: Timestamp parsing round trip and
    /// cross-language agreement (shared corpus half, also read by `instant.test.ts`).
    ///
    /// **Validates: Requirements 1.3, 4.5**
    #[test]
    fn parse_instant_matches_shared_corpus() {
        let corpus: Corpus = serde_json::from_str(include_str!(
            "../../desktop/src/lib/__fixtures__/instant-corpus.json"
        ))
        .expect("instant-corpus.json parses");
        assert!(!corpus.valid.is_empty() && !corpus.invalid.is_empty());

        for entry in &corpus.valid {
            let parsed = parse_instant(&entry.input)
                .unwrap_or_else(|| panic!("rejected valid corpus entry {:?}", entry.input));
            assert_eq!(parsed.timestamp_millis(), entry.epoch_ms, "epochMs for {:?}", entry.input);
            assert_eq!(
                parsed.timestamp_subsec_nanos() % 1_000_000,
                entry.sub_ms_nanos,
                "subMsNanos for {:?}",
                entry.input
            );
        }
        for bad in &corpus.invalid {
            assert!(parse_instant(bad).is_none(), "accepted invalid corpus entry {bad:?}");
        }
    }
}
