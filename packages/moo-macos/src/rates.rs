//! Currency rates for the calculator: the European Central Bank's daily reference rates, cached
//! in `~/Library/Caches/Moo/rates.txt` and refetched in the background when over 12 hours old.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::calc::{self, Rates};
use crate::http;

const URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
const MAX_AGE: f64 = 12.0 * 3600.0;

struct Cached {
    rates: Rates,
    date: String,
    fetched: f64,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
static LOADED: AtomicBool = AtomicBool::new(false);
static FETCHING: AtomicBool = AtomicBool::new(false);

fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MOO_RATES") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/Moo/rates.txt"))
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// `date\nfetched\nCODE rate\n...`
fn parse_file(s: &str) -> Option<Cached> {
    let mut lines = s.lines();
    let date = lines.next()?.to_string();
    let fetched = lines.next()?.parse().ok()?;
    let rates: Rates = lines.filter_map(|l| l.split_once(' ')).filter_map(|(c, r)| Some((c.to_string(), r.parse().ok()?))).collect();
    (!rates.is_empty()).then_some(Cached { rates, date, fetched })
}

fn write_file(c: &Cached) {
    let Some(p) = path() else { return };
    let mut out = format!("{}\n{}\n", c.date, c.fetched);
    let mut codes: Vec<_> = c.rates.iter().collect();
    codes.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in codes {
        out.push_str(&format!("{k} {v}\n"));
    }
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, out);
}

fn load_once() {
    if LOADED.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Some(c) = path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| parse_file(&s)) {
        *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(c);
    }
}

/// Fetch in the background if the cache is missing or old.
pub fn refresh_if_stale() {
    load_once();
    let fresh = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|c| now() - c.fetched < MAX_AGE);
    if fresh || FETCHING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let mut req = http::Request::get(URL);
        req.timeout = 15.0;
        match http::fetch(&req) {
            Ok((200, xml)) => {
                let (rates, date) = calc::parse_ecb(&xml);
                if !rates.is_empty() {
                    let c = Cached { rates, date, fetched: now() };
                    write_file(&c);
                    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(c);
                }
            }
            Ok((st, _)) => eprintln!("moo: currency rates: HTTP {st}"),
            Err(e) => eprintln!("moo: currency rates: {e}"),
        }
        FETCHING.store(false, Ordering::Release);
    });
}

/// Current rates (per 1 EUR) and their date, if any have been loaded.
pub fn get() -> Option<(Rates, String)> {
    load_once();
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|c| (c.rates.clone(), c.date.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_file_round_trip() {
        let mut rates = Rates::new();
        rates.insert("USD".into(), 1.08);
        rates.insert("EUR".into(), 1.0);
        let c = Cached { rates, date: "2026-10-02".into(), fetched: 1000.0 };
        let p = std::env::temp_dir().join(format!("moo-rates-{}", std::process::id()));
        std::env::set_var("MOO_RATES", &p);
        write_file(&c);
        let back = parse_file(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!((back.date.as_str(), back.fetched, back.rates.get("USD").copied()), ("2026-10-02", 1000.0, Some(1.08)));
        assert!(parse_file("x\n1\n").is_none());
        let _ = std::fs::remove_file(p);
    }
}
