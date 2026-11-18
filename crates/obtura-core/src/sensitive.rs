//! Smart redact: finds what should not be shared in recognised text (emails, phone numbers,
//! card numbers, IBANs, IP addresses, keys and passwords) and where it is. The user always
//! sees and confirms the proposals; finding a little too much is better than too little.

use crate::ocr::{lines, Word};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Email,
    Phone,
    Card,
    Iban,
    Ip,
    /// API keys, tokens, passwords.
    Secret,
    /// A QR code (it may hold a Wi-Fi password, a login secret, a ticket...).
    QrCode,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Email, Kind::Phone, Kind::Card, Kind::Iban, Kind::Ip, Kind::Secret, Kind::QrCode];

    /// What the user reads, for one or several.
    pub fn name(self, n: usize) -> &'static str {
        match (self, n == 1) {
            (Kind::Email, true) => "email address",
            (Kind::Email, false) => "email addresses",
            (Kind::Phone, true) => "phone number",
            (Kind::Phone, false) => "phone numbers",
            (Kind::Card, true) => "card number",
            (Kind::Card, false) => "card numbers",
            (Kind::Iban, true) => "bank account (IBAN)",
            (Kind::Iban, false) => "bank accounts (IBAN)",
            (Kind::Ip, true) => "IP address",
            (Kind::Ip, false) => "IP addresses",
            (Kind::Secret, true) => "key or password",
            (Kind::Secret, false) => "keys and passwords",
            (Kind::QrCode, true) => "QR code",
            (Kind::QrCode, false) => "QR codes",
        }
    }
}

/// Something to hide: what it is, the text read, and the box of its words (x, y, w, h).
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub kind: Kind,
    pub text: String,
    pub rect: (u32, u32, u32, u32),
}

/// The QR codes of a picture as things to hide (their text shown, the whole code covered).
pub fn qr_findings(codes: &[obtura_image::QrCode]) -> Vec<Finding> {
    codes
        .iter()
        .map(|c| {
            let (x, y, w, h) = c.bounds();
            let (x0, y0) = (x.floor().max(0.0), y.floor().max(0.0));
            Finding { kind: Kind::QrCode, text: c.text.clone(), rect: (x0 as u32, y0 as u32, (x + w - x0).ceil() as u32, (y + h - y0).ceil() as u32) }
        })
        .collect()
}

/// Everything worth hiding in `words`, in reading order. A word is part of one finding at most.
pub fn find(words: &[Word]) -> Vec<Finding> {
    let mut out = Vec::new();
    for line in lines(words) {
        // The line as text, and where each word sits in it (in chars).
        let mut chars: Vec<char> = Vec::new();
        let mut spans = Vec::new();
        for (i, w) in line.iter().enumerate() {
            if i > 0 {
                chars.push(' ');
            }
            let start = chars.len();
            chars.extend(w.text.chars());
            spans.push((start, chars.len()));
        }
        let mut taken = vec![false; line.len()];
        let detectors: [(Kind, fn(&[char]) -> Vec<(usize, usize)>); 6] =
            [(Kind::Email, emails), (Kind::Iban, ibans), (Kind::Card, cards), (Kind::Ip, ips), (Kind::Phone, phones), (Kind::Secret, secrets)];
        for (kind, detect) in detectors {
            for (s, e) in detect(&chars) {
                let hit: Vec<usize> = spans.iter().enumerate().filter(|(_, (ws, we))| *ws < e && s < *we).map(|(i, _)| i).collect();
                if hit.is_empty() || hit.iter().any(|&i| taken[i]) {
                    continue;
                }
                let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
                for &i in &hit {
                    taken[i] = true;
                    let w = line[i];
                    x0 = x0.min(w.x);
                    y0 = y0.min(w.y);
                    x1 = x1.max(w.x + w.w);
                    y1 = y1.max(w.y + w.h);
                }
                out.push(Finding { kind, text: chars[s..e].iter().collect(), rect: (x0, y0, x1 - x0, y1 - y0) });
            }
        }
    }
    out
}

fn is_email_local(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._%+-".contains(c)
}

fn is_domain(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '-'
}

fn emails(t: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (at, _) in t.iter().enumerate().filter(|(_, c)| **c == '@') {
        let mut s = at;
        while s > 0 && is_email_local(t[s - 1]) {
            s -= 1;
        }
        let mut e = at + 1;
        while e < t.len() && is_domain(t[e]) {
            e += 1;
        }
        while e > at + 1 && (t[e - 1] == '.' || t[e - 1] == '-') {
            e -= 1;
        }
        let domain: String = t[at + 1..e].iter().collect();
        let tld_ok = domain.rsplit_once('.').is_some_and(|(name, tld)| !name.is_empty() && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
        if s < at && tld_ok {
            out.push((s, e));
        }
    }
    out
}

/// Starts of runs: a position whose previous char is not a letter or digit.
fn starts_clean(t: &[char], i: usize) -> bool {
    i == 0 || !t[i - 1].is_alphanumeric()
}

fn ips(t: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t[i].is_ascii_digit() && (i == 0 || !(t[i - 1].is_ascii_alphanumeric() || t[i - 1] == '.')) {
            if let Some(e) = ipv4_at(t, i) {
                out.push((i, e));
                i = e;
                continue;
            }
        }
        i += 1;
    }
    // IPv6: a word of hex groups and colons.
    let mut s = 0;
    while s < t.len() {
        let e = (s..t.len()).find(|&k| t[k].is_whitespace()).unwrap_or(t.len());
        let (mut a, mut b) = (s, e);
        while a < b && "([<\"'".contains(t[a]) {
            a += 1;
        }
        while b > a && ",;)]>\"'.".contains(t[b - 1]) {
            b -= 1;
        }
        let word: String = t[a..b].iter().collect();
        let colons = word.matches(':').count();
        let hex_digits = word.chars().filter(|c| c.is_ascii_hexdigit()).count();
        let groups_ok = word.split(':').all(|g| g.len() <= 4 || g.contains('.') || g.contains('/') || g.contains('%'));
        if colons >= 2 && hex_digits >= 3 && groups_ok && (word.contains("::") || colons == 7) && word.chars().all(|c| c.is_ascii_hexdigit() || ":./%".contains(c)) {
            out.push((a, b));
        }
        s = e + 1;
    }
    out
}

/// An IPv4 address (and its port or prefix) starting at `i`: where it ends.
fn ipv4_at(t: &[char], i: usize) -> Option<usize> {
    let mut k = i;
    for g in 0..4 {
        let start = k;
        while k < t.len() && t[k].is_ascii_digit() && k - start < 3 {
            k += 1;
        }
        let n: u32 = t[start..k].iter().collect::<String>().parse().ok()?;
        if k == start || n > 255 {
            return None;
        }
        if g < 3 {
            if k >= t.len() || t[k] != '.' {
                return None;
            }
            k += 1;
        }
    }
    if k < t.len() && (t[k].is_ascii_alphanumeric() || (t[k] == '.' && t.get(k + 1).is_some_and(|c| c.is_ascii_digit()))) {
        return None;
    }
    // ":8080" or "/24"
    if k + 1 < t.len() && (t[k] == ':' || t[k] == '/') && t[k + 1].is_ascii_digit() {
        k += 1;
        while k < t.len() && t[k].is_ascii_digit() {
            k += 1;
        }
    }
    Some(k)
}

/// A run of digits with single separators from `seps` between them, starting at `i`: its end
/// and its digits.
fn digit_run(t: &[char], i: usize, seps: &str) -> (usize, String) {
    let mut digits = String::new();
    let mut k = i;
    while k < t.len() {
        if t[k].is_ascii_digit() {
            digits.push(t[k]);
            k += 1;
        } else if seps.contains(t[k]) && k + 1 < t.len() && t[k + 1].is_ascii_digit() && k > i {
            k += 1;
        } else {
            break;
        }
    }
    (k, digits)
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0;
    for (i, c) in digits.chars().rev().enumerate() {
        let mut d = c.to_digit(10).unwrap_or(0);
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0
}

fn cards(t: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t[i].is_ascii_digit() && starts_clean(t, i) {
            let (e, digits) = digit_run(t, i, " -");
            let grouped_16 = digits.len() == 16 && t[i..e].iter().filter(|c| **c == ' ' || **c == '-').count() == 3;
            let ends_clean = e >= t.len() || !t[e].is_alphanumeric();
            if ends_clean && (13..=19).contains(&digits.len()) && (luhn(&digits) || grouped_16) {
                out.push((i, e));
                i = e;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Whether `s` (letters and digits, no spaces) is a valid IBAN: mod 97 of the rearranged
/// number is 1.
fn iban_ok(s: &str) -> bool {
    if !(15..=34).contains(&s.len()) {
        return false;
    }
    let (head, rest) = s.split_at(4);
    let mut r: u32 = 0;
    for c in rest.chars().chain(head.chars()) {
        let v = match c.to_digit(36) {
            Some(v) => v,
            None => return false,
        };
        r = if v < 10 { (r * 10 + v) % 97 } else { (r * 100 + v) % 97 };
    }
    r == 1
}

fn ibans(t: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= t.len() {
        let looks = t[i].is_ascii_uppercase() && t[i + 1].is_ascii_uppercase() && t[i + 2].is_ascii_digit() && t[i + 3].is_ascii_digit();
        if looks && starts_clean(t, i) {
            // Letters and digits, single spaces allowed between groups; the longest valid one.
            let mut k = i;
            let mut code = String::new();
            let mut ends = Vec::new();
            while k < t.len() && code.len() < 34 {
                let c = t[k];
                if c.is_ascii_uppercase() || c.is_ascii_digit() {
                    code.push(c);
                    k += 1;
                    if k >= t.len() || !(t[k].is_ascii_uppercase() || t[k].is_ascii_digit()) {
                        ends.push((k, code.clone()));
                    }
                } else if c == ' ' && k + 1 < t.len() && (t[k + 1].is_ascii_uppercase() || t[k + 1].is_ascii_digit()) {
                    k += 1;
                } else {
                    break;
                }
            }
            if let Some((e, _)) = ends.iter().rev().find(|(_, c)| iban_ok(c)) {
                out.push((i, *e));
                i = *e;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn phones(t: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let c = t[i];
        if (c == '+' || c == '(' || c.is_ascii_digit()) && starts_clean(t, i) {
            let mut k = i + 1;
            let mut digits = usize::from(c.is_ascii_digit());
            let mut seps = 0;
            let mut last_digit = if c.is_ascii_digit() { k } else { i };
            while k < t.len() {
                let d = t[k];
                if d.is_ascii_digit() {
                    digits += 1;
                    last_digit = k + 1;
                } else if " -.()/".contains(d) {
                    // Two separators in a row end the number (except ") " and " (").
                    if " -./".contains(d) && k > 0 && " -./".contains(t[k - 1]) {
                        break;
                    }
                    seps += 1;
                } else {
                    break;
                }
                k += 1;
            }
            let mut e = last_digit;
            if e < t.len() && t[e] == ')' {
                e += 1;
            }
            let ends_clean = e >= t.len() || !t[e].is_alphanumeric();
            let shaped = c == '+' || c == '(' || c == '0' || seps >= 2;
            if ends_clean && (9..=15).contains(&digits) && shaped {
                out.push((i, e));
                i = e;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Starts of well-known keys and tokens.
const KEY_PREFIXES: [&str; 22] = [
    "sk-", "sk_live_", "sk_test_", "rk_live_", "pk_live_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "glpat-", "xoxb-", "xoxp-", "xoxa-", "xoxs-", "AKIA", "ASIA", "AIza", "ya29.", "hf_", "npm_",
];

/// Words that announce a secret: the next word (or what follows `=` or `:`) is hidden.
const KEY_WORDS: [&str; 11] = ["password", "passwd", "pwd", "pass", "secret", "token", "apikey", "api_key", "api-key", "pin", "mot de passe"];

/// Bits of information per character.
fn entropy(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let n = s.chars().count() as f64;
    counts.values().map(|&k| {
        let p = k as f64 / n;
        -p * p.log2()
    }).sum()
}

fn looks_like_key(w: &str) -> bool {
    let n = w.chars().count();
    if KEY_PREFIXES.iter().any(|p| w.starts_with(p)) && n >= 16 {
        return true;
    }
    // A JSON web token: three parts.
    if w.starts_with("eyJ") && w.matches('.').count() == 2 && n >= 30 {
        return true;
    }
    let charset = w.chars().all(|c| c.is_ascii_alphanumeric() || "_-+/=.".contains(c));
    let digits = w.chars().any(|c| c.is_ascii_digit());
    let letters = w.chars().any(|c| c.is_ascii_alphabetic());
    n >= 24 && charset && digits && letters && !w.contains("://") && entropy(w) >= 3.5
}

fn secrets(t: &[char]) -> Vec<(usize, usize)> {
    // The words of the line, without the punctuation around them.
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut s = 0;
    while s < t.len() {
        let e = (s..t.len()).find(|&k| t[k] == ' ').unwrap_or(t.len());
        let (mut a, mut b) = (s, e);
        while a < b && "([<\"'`".contains(t[a]) {
            a += 1;
        }
        while b > a && ",;)]>\"'`".contains(t[b - 1]) {
            b -= 1;
        }
        if a < b {
            words.push((a, b));
        }
        s = e + 1;
    }
    let text = |(a, b): (usize, usize)| t[a..b].iter().collect::<String>();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = text(words[i]);
        let lower = w.to_lowercase();
        // "password: hunter2", "password = hunter2", "token=abc".
        let key = lower.trim_end_matches([':', '=']);
        if lower.split_once(['=', ':']).is_some_and(|(k, v)| KEY_WORDS.contains(&k) && !v.is_empty()) {
            out.push(words[i]);
        } else if KEY_WORDS.contains(&key) || (lower == "de" && i >= 2 && text(words[i - 1]).to_lowercase() == "passe") {
            let mut j = i + 1;
            if j < words.len() && matches!(text(words[j]).as_str(), ":" | "=") {
                j += 1;
            }
            if j < words.len() && (w.ends_with([':', '=']) || j > i + 1) {
                out.push(words[j]);
                i = j + 1;
                continue;
            }
        } else if looks_like_key(&w) {
            out.push(words[i]);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One line of words, each 10 px wide per letter, 20 px tall, 10 px apart.
    fn line(text: &str) -> Vec<Word> {
        let mut x = 0;
        text.split(' ')
            .map(|t| {
                let w = Word { text: t.into(), x, y: 100, w: 10 * t.chars().count() as u32, h: 20, conf: 90.0, block: 1, par: 1, line: 1 };
                x += w.w + 10;
                w
            })
            .collect()
    }

    fn kinds(text: &str) -> Vec<(Kind, String)> {
        find(&line(text)).into_iter().map(|f| (f.kind, f.text)).collect()
    }

    #[test]
    fn qr_codes_are_found_whole() {
        let png = include_bytes!("../../../tests/fixtures/qr-two.png");
        let codes = obtura_image::decode_qr(&obtura_image::decode_png(png).unwrap());
        let found = qr_findings(&codes);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|f| f.kind == Kind::QrCode));
        assert_eq!(found[1].text, "WIFI:S:Demo;T:WPA;P:not-a-real-secret;;");
        // Covering each finding's area leaves no dark module of the codes.
        let mut pic = obtura_image::decode_png(png).unwrap();
        for f in &found {
            let (x, y, w, h) = f.rect;
            obtura_image::redact(&mut pic, obtura_image::Rect::new(x as i32, y as i32, w, h), obtura_image::Redaction::Box { rgb: [255, 255, 255] });
        }
        assert!(pic.rgba.chunks(4).all(|p| p[0] > 128), "a part of a code is left");
        assert!(obtura_image::decode_qr(&pic).is_empty());
    }

    #[test]
    fn finds_email_addresses() {
        assert_eq!(kinds("Write to jane.doe+news@example.co.uk today."), vec![(Kind::Email, "jane.doe+news@example.co.uk".into())]);
        assert_eq!(kinds("@handle and a@b are not addresses"), vec![]);
    }

    #[test]
    fn finds_ip_addresses_but_not_versions_or_times() {
        assert_eq!(kinds("server 192.168.1.20:8080 is up"), vec![(Kind::Ip, "192.168.1.20:8080".into())]);
        assert_eq!(kinds("ip fe80::1ff:fe23:4567:890a and 2001:db8::1"), vec![(Kind::Ip, "fe80::1ff:fe23:4567:890a".into()), (Kind::Ip, "2001:db8::1".into())]);
        assert_eq!(kinds("version 1.2.3 at 12:30:45 or 999.1.1.1"), vec![]);
    }

    #[test]
    fn finds_card_numbers() {
        assert_eq!(kinds("Card 4111 1111 1111 1111 exp 12/29"), vec![(Kind::Card, "4111 1111 1111 1111".into())]);
        assert_eq!(kinds("amex 378282246310005"), vec![(Kind::Card, "378282246310005".into())]);
        // Misread by one digit but shaped like a card: still proposed.
        assert_eq!(kinds("4111 1111 1111 1112"), vec![(Kind::Card, "4111 1111 1111 1112".into())]);
    }

    #[test]
    fn finds_ibans_only_when_their_check_digits_agree() {
        assert_eq!(kinds("IBAN FR76 3000 6000 0112 3456 7890 189 BIC AGRIFRPP"), vec![(Kind::Iban, "FR76 3000 6000 0112 3456 7890 189".into())]);
        assert_eq!(kinds("GB82WEST12345698765432"), vec![(Kind::Iban, "GB82WEST12345698765432".into())]);
        assert!(kinds("FR77 3000 6000 0112 3456 7890 189").iter().all(|(k, _)| *k != Kind::Iban));
    }

    #[test]
    fn finds_phone_numbers_but_not_dates_or_sizes() {
        assert_eq!(kinds("Call +33 6 12 34 56 78 now"), vec![(Kind::Phone, "+33 6 12 34 56 78".into())]);
        assert_eq!(kinds("or (555) 123-4567"), vec![(Kind::Phone, "(555) 123-4567".into())]);
        assert_eq!(kinds("tel 06.12.34.56.78"), vec![(Kind::Phone, "06.12.34.56.78".into())]);
        assert_eq!(kinds("on 2026-11-14 at 800x500, 1920 x 1080"), vec![]);
    }

    #[test]
    fn finds_keys_tokens_and_passwords() {
        assert_eq!(kinds("key sk-proj-4f9Qa8ZpL2mX7vT1"), vec![(Kind::Secret, "sk-proj-4f9Qa8ZpL2mX7vT1".into())]);
        assert_eq!(kinds("export KEY=sk-proj-4f9Qa8ZpL2mX7vT1"), vec![(Kind::Secret, "KEY=sk-proj-4f9Qa8ZpL2mX7vT1".into())]);
        assert_eq!(kinds("token ghp_16C7e42F292c6912E7710c838347Ae178B4a"), vec![(Kind::Secret, "ghp_16C7e42F292c6912E7710c838347Ae178B4a".into())]);
        assert_eq!(kinds("aws AKIAIOSFODNN7EXAMPLE"), vec![(Kind::Secret, "AKIAIOSFODNN7EXAMPLE".into())]);
        assert_eq!(kinds("Password: hunter2"), vec![(Kind::Secret, "hunter2".into())]);
        assert_eq!(kinds("password=hunter2"), vec![(Kind::Secret, "password=hunter2".into())]);
        assert_eq!(kinds("a generic f3a9C0d1E8b74A6c2D9e0F1a7B3c5D8e9f0A"), vec![(Kind::Secret, "f3a9C0d1E8b74A6c2D9e0F1a7B3c5D8e9f0A".into())]);
        assert_eq!(kinds("Obtura fixture 800x500 documentation_internationalization"), vec![]);
    }

    #[test]
    fn a_finding_covers_the_boxes_of_its_words() {
        let f = find(&line("Card 4111 1111 1111 1111 exp"));
        assert_eq!(f.len(), 1);
        // "Card" is 40 px + 10 gap: the number starts at 50; four words of 40 px, 10 px apart.
        assert_eq!(f[0].rect, (50, 100, 190, 20));
    }

    #[test]
    fn findings_follow_the_lines() {
        let mut words = line("mail bob@example.org");
        let mut second = line("call +1 202 555 0143");
        second.iter_mut().for_each(|w| {
            w.line = 2;
            w.y = 140;
        });
        words.extend(second);
        let f = find(&words);
        assert_eq!(f.iter().map(|f| f.kind).collect::<Vec<_>>(), vec![Kind::Email, Kind::Phone]);
        assert_eq!(f[1].rect.1, 140);
    }
}
