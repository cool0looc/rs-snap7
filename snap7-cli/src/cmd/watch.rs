use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use bytes::Bytes;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::WatchArgs;

pub async fn run(client: &S7Client<TcpTransport>, args: WatchArgs) -> Result<()> {
    let interval = Duration::from_millis(args.interval_ms);
    let mut prev: Option<Bytes> = None;
    let mut ticker = tokio::time::interval(interval);
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match client.db_read(args.db, args.offset, args.size).await {
                    Ok(data) => {
                        let changed = prev.as_ref().is_none_or(|p| p != &data);
                        if changed || !args.changes_only {
                            let ts = iso8601_now();
                            let hex = data
                                .iter()
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<_>>()
                                .join(" ");
                            println!("{}  {}", ts, hex);
                            prev = Some(data);
                        }
                    }
                    Err(e) => {
                        eprintln!("read error: {}", e);
                    }
                }
            }
            _ = &mut ctrl_c => {
                eprintln!("\nwatch stopped");
                break;
            }
        }
    }
    Ok(())
}

fn iso8601_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, mo, d, h, mi, sec) = seconds_to_ymd_hms(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, sec)
}

fn seconds_to_ymd_hms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let sec = secs % 60;
    let mins = secs / 60;
    let min = mins % 60;
    let hours = mins / 60;
    let hour = hours % 24;
    let days = hours / 24;

    // Hinnant civil-from-days algorithm
    // Shift from Unix epoch (1970-01-01) to proleptic Gregorian epoch (Mar 1, year 0)
    // Days from proleptic year-0 Mar 1 to 1970-01-01 = 719468
    let z = days + 719468;
    let era = z / 146097;
    let doe = z % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hour, min, sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_now_format() {
        let s = iso8601_now();
        // Should be exactly 20 chars: YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(s.len(), 20, "unexpected format: {s}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], "T");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
    }

    #[test]
    fn known_timestamp() {
        // Unix 0 = 1970-01-01T00:00:00Z
        let (y, mo, d, h, mi, s) = seconds_to_ymd_hms(0);
        assert_eq!((y, mo, d, h, mi, s), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn known_timestamp_2() {
        // 2025-05-02T12:00:00Z = 1746187200
        let (y, mo, d, h, mi, s) = seconds_to_ymd_hms(1746187200);
        assert_eq!((y, mo, d, h, mi, s), (2025, 5, 2, 12, 0, 0));
    }
}
