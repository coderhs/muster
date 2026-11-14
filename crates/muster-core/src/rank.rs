//! Fractional-index ranks.
//!
//! A rank is a non-empty string of base-62 digits (`0-9A-Za-z`, which sort in
//! ASCII order) read as a fraction `0.d1d2d3...`. Ranks never end in `0`, so
//! there is always room both before and after any rank, and between any two
//! distinct ranks. Moving a card therefore rewrites only that card's row.

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

fn val(c: u8) -> usize {
    match c {
        b'0'..=b'9' => (c - b'0') as usize,
        b'A'..=b'Z' => (c - b'A') as usize + 10,
        b'a'..=b'z' => (c - b'a') as usize + 36,
        _ => panic!("invalid rank digit {c}"),
    }
}

pub fn is_valid(rank: &str) -> bool {
    !rank.is_empty() && rank.bytes().all(|c| c.is_ascii_alphanumeric()) && !rank.ends_with('0')
}

/// Returns a rank strictly between `a` and `b`. `None` stands for the start or
/// end of the list. Panics if `a >= b` or either rank is malformed.
pub fn between(a: Option<&str>, b: Option<&str>) -> String {
    let a = a.unwrap_or("");
    if !a.is_empty() {
        assert!(is_valid(a), "bad rank {a:?}");
    }
    if let Some(b) = b {
        assert!(is_valid(b), "bad rank {b:?}");
        assert!(a < b, "ranks out of order: {a:?} >= {b:?}");
    }
    let out = match (a.is_empty(), b) {
        (false, None) => after(a.as_bytes()),
        (true, Some(b)) => before(b.as_bytes()),
        _ => midpoint(a.as_bytes(), b.map(str::as_bytes)),
    };
    String::from_utf8(out).expect("ascii")
}

/// Smallest-growth rank greater than `a` (appending stays short).
fn after(a: &[u8]) -> Vec<u8> {
    match a.first() {
        None => vec![b'1'],
        Some(&c) if val(c) < BASE - 1 => vec![DIGITS[val(c) + 1]],
        Some(&c) => {
            let mut out = vec![c];
            out.extend(after(&a[1..]));
            out
        }
    }
}

/// Smallest-growth rank less than `b` (prepending stays short). `b` is valid.
fn before(b: &[u8]) -> Vec<u8> {
    let d = val(b[0]);
    if d == 0 {
        let mut out = vec![b'0'];
        out.extend(before(&b[1..]));
        out
    } else if d > 1 {
        vec![DIGITS[d - 1]]
    } else if b.len() > 1 {
        vec![b[0]]
    } else {
        vec![b'0', DIGITS[BASE - 1]]
    }
}

/// `a` may be empty (zero); `b == None` means one. Requires `a < b`.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        // Skip the common prefix, padding `a` with zeros.
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(b'0') == b[n] {
            n += 1;
        }
        if n > 0 {
            let mut out = b[..n].to_vec();
            let rest_a = if n < a.len() { &a[n..] } else { &[][..] };
            out.extend(midpoint(rest_a, Some(&b[n..])));
            return out;
        }
    }
    let da = a.first().map(|&c| val(c)).unwrap_or(0);
    let db = b.and_then(|b| b.first()).map(|&c| val(c)).unwrap_or(BASE);
    if db - da > 1 {
        return vec![DIGITS[(da + db) / 2]];
    }
    // The first digits are adjacent.
    if let Some(b) = b {
        if b.len() > 1 {
            return vec![b[0]];
        }
    }
    let mut out = vec![DIGITS[da]];
    let rest_a = if a.len() > 1 { &a[1..] } else { &[][..] };
    out.extend(midpoint(rest_a, None));
    out
}

/// `n` evenly spread ranks, for bulk renumbering.
pub fn spread(n: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(n);
    let mut prev: Option<String> = None;
    for _ in 0..n {
        let r = between(prev.as_deref(), None);
        out.push(r.clone());
        prev = Some(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(between(None, None), "V");
        let a = between(None, None);
        let b = between(Some(&a), None);
        assert!(a < b);
        let c = between(None, Some(&a));
        assert!(c < a);
        let m = between(Some(&a), Some(&b));
        assert!(a < m && m < b);
    }

    #[test]
    fn adjacent_and_prefix() {
        for (a, b) in [("1", "2"), ("1", "11"), ("A", "A1"), ("z", "zz"), ("01", "1"), ("Vz", "W")] {
            let m = between(Some(a), Some(b));
            assert!(a < m.as_str() && m.as_str() < b, "{a} < {m} < {b}");
            assert!(is_valid(&m));
        }
        let m = between(None, Some("01"));
        assert!(m.as_str() < "01" && is_valid(&m));
        let m = between(Some("zzz"), None);
        assert!(m.as_str() > "zzz" && is_valid(&m));
    }

    /// Deterministic pseudo-random insertions keep order and validity, and
    /// ranks stay short under repeated front/back/middle inserts.
    #[test]
    fn random_inserts_keep_order() {
        let mut list: Vec<String> = Vec::new();
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        for _ in 0..3000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let pos = if list.is_empty() { 0 } else { (seed as usize) % (list.len() + 1) };
            let a = if pos == 0 { None } else { Some(list[pos - 1].as_str()) };
            let b = list.get(pos).map(String::as_str);
            let r = between(a, b);
            assert!(is_valid(&r));
            list.insert(pos, r);
        }
        for w in list.windows(2) {
            assert!(w[0] < w[1]);
        }
    }

    #[test]
    fn repeated_append_and_prepend_grow_slowly() {
        let mut last = between(None, None);
        for _ in 0..1000 {
            last = between(Some(&last), None);
        }
        assert!(last.len() < 40, "{}", last.len());
        let mut first = between(None, None);
        for _ in 0..1000 {
            let n = between(None, Some(&first));
            assert!(n < first);
            first = n;
        }
        assert!(first.len() < 40, "{}", first.len());
    }

    #[test]
    fn spread_is_sorted() {
        let s = spread(100);
        for w in s.windows(2) {
            assert!(w[0] < w[1]);
        }
    }
}
