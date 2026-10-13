//! The pseudo-languages, made from English at run time: `qps` swaps every
//! Latin letter for an accented one, `qps-rtl` for a Hebrew letter (right
//! to left, drawn by the OS text stack); both bracket the string and pad
//! it to 140 % of its length, so text that overflows, is cut off, is
//! glued together from pieces, or was never extracted shows at a glance.
//! `{placeholders}` and line breaks are kept.

const ACCENTED_LOWER: [char; 26] = [
    'á', 'ƀ', 'ç', 'ð', 'é', 'ƒ', 'ĝ', 'ĥ', 'í', 'ĵ', 'ķ', 'ļ', 'ɱ', 'ñ', 'ö', 'þ', 'ǫ', 'ŕ', 'š', 'ţ', 'ü', 'ṽ', 'ŵ', 'ẋ', 'ý', 'ž',
];
const ACCENTED_UPPER: [char; 26] = [
    'Å', 'Ɓ', 'Ç', 'Ð', 'É', 'Ƒ', 'Ĝ', 'Ĥ', 'Î', 'Ĵ', 'Ķ', 'Ļ', 'Ṁ', 'Ñ', 'Ö', 'Þ', 'Ǫ', 'Ŕ', 'Š', 'Ţ', 'Û', 'Ṽ', 'Ŵ', 'Ẋ', 'Ý', 'Ž',
];
/// The 22 Hebrew letters and four final forms, for a..z.
const HEBREW: [char; 26] = [
    'א', 'ב', 'ג', 'ד', 'ה', 'ו', 'ז', 'ח', 'ט', 'י', 'כ', 'ל', 'מ', 'נ', 'ס', 'ע', 'פ', 'צ', 'ק', 'ר', 'ש', 'ת', 'ך', 'ם', 'ן', 'ף',
];

fn map(c: char, rtl: bool) -> char {
    let (lower, upper) = (c.is_ascii_lowercase(), c.is_ascii_uppercase());
    if !lower && !upper {
        return c;
    }
    let i = (c.to_ascii_lowercase() as u8 - b'a') as usize;
    if rtl {
        HEBREW[i]
    } else if lower {
        ACCENTED_LOWER[i]
    } else {
        ACCENTED_UPPER[i]
    }
}

/// `s` in the pseudo-language (`rtl`: qps-rtl, else qps).
pub fn pseudo(s: &str, rtl: bool) -> String {
    let mut out = String::with_capacity(s.len() * 2 + 4);
    let mut letters: Vec<char> = Vec::new();
    let mut n = 0usize;
    let mut rest = s;
    out.push('[');
    while let Some(c) = rest.chars().next() {
        // A placeholder goes through untouched.
        if c == '{'
            && let Some(j) = rest.find('}')
            && rest[1..j].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && j > 1
        {
            out.push_str(&rest[..=j]);
            rest = &rest[j + 1..];
            n += j + 1;
            continue;
        }
        let m = map(c, rtl);
        if m != c {
            letters.push(m);
        }
        out.push(m);
        n += 1;
        rest = &rest[c.len_utf8()..];
    }
    // 40 % longer in all, the brackets included.
    let pad = (n * 2).div_ceil(5).saturating_sub(2);
    if pad > 0 {
        out.push(' ');
        let fill: Vec<char> = if letters.is_empty() { vec![map('x', rtl)] } else { letters };
        out.extend(fill.iter().cycle().take(pad.saturating_sub(1)));
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_placeholders_and_grows_by_forty_percent() {
        let p = pseudo("Saved {name} to {dir}", false);
        assert!(p.starts_with("[Šáṽéð {name} ţö {dir}"), "{p}");
        assert!(p.ends_with(']'));
        let (a, b) = ("Saved {name} to {dir}".chars().count(), p.chars().count());
        assert!(b * 10 >= a * 14 - 10 && b * 10 <= a * 14 + 20, "{a} -> {b}: {p}");
        assert_eq!(pseudo("OK", false), "[ÖĶ]");
        assert_eq!(pseudo("a\nb", false).chars().nth(2), Some('\n'));
    }

    #[test]
    fn rtl_is_hebrew() {
        let p = pseudo("Save {n}", true);
        assert!(p.contains("{n}"));
        assert!(p.chars().any(|c| ('\u{5d0}'..='\u{5ea}').contains(&c)));
        assert!(!p.chars().any(|c| c.is_ascii_lowercase() && !"n".contains(c)), "{p}");
    }
}
