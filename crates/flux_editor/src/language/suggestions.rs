//! "Did you mean …?" suggestions via Levenshtein distance. Used to turn a typo
//! like `GetServi` into a hint pointing at `GetService`.

/// Levenshtein edit distance between two strings (byte-insensitive to case).
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca.eq_ignore_ascii_case(&cb) { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Best candidate for `name` among `options`, if one is close enough to be a
/// plausible typo. Threshold scales with the word length.
pub fn closest<'a>(name: &str, options: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let max = match name.len() {
        0..=2 => 1,
        3..=5 => 2,
        _ => 3,
    };
    let mut best: Option<(usize, &str)> = None;
    for opt in options {
        let d = levenshtein(name, opt);
        if d <= max && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, opt));
        }
    }
    best.map(|(_, o)| o)
}

/// Format a "did you mean" suffix, e.g. ` Did you mean \`GetService\`?`.
pub fn did_you_mean(candidate: Option<&str>) -> String {
    candidate
        .map(|c| format!(" Did you mean `{c}`?"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_basics() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("GetServi", "GetService"), 2);
        assert_eq!(levenshtein("abc", "abc"), 0);
    }

    #[test]
    fn suggests_close_match() {
        let opts = ["GetService", "GetChildren", "FindFirstChild"];
        assert_eq!(closest("GetServi", opts), Some("GetService"));
        assert_eq!(closest("xyzzy", opts), None);
    }
}
