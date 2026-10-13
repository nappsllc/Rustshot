//! tesseract's `tsv` output (Linux word boxes): one row per page, block,
//! paragraph, line and word; the words (level 5) with their boxes, grouped
//! by (page, block, paragraph, line) in the order tesseract read them.
#![cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]

use super::{Line, Word};

/// The lines of tesseract's TSV `s`, boxes divided by `k` (the image was
/// upscaled `k` times); rows that do not parse and empty words are skipped.
pub fn parse_tsv(s: &str, k: u32) -> Vec<Line> {
    let k = k.max(1) as f32;
    let mut out: Vec<Line> = Vec::new();
    let mut key: Option<[u32; 4]> = None;
    for row in s.lines() {
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let n = |i: usize| cols[i].trim().parse::<f32>().ok();
        let id = |i: usize| cols[i].trim().parse::<u32>().ok();
        let (Some(page), Some(block), Some(par), Some(line)) = (id(1), id(2), id(3), id(4)) else { continue };
        let (Some(x), Some(y), Some(w), Some(h)) = (n(6), n(7), n(8), n(9)) else { continue };
        // The text is the rest of the row (a tab inside it is kept).
        let text = cols[11..].join("\t");
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let this = [page, block, par, line];
        if key != Some(this) || out.is_empty() {
            out.push(Line::default());
            key = Some(this);
        }
        if let Some(l) = out.last_mut() {
            l.words.push(Word { text: text.to_string(), x: x / k, y: y / k, w: w / k, h: h / k });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext
1\t1\t0\t0\t0\t0\t0\t0\t400\t100\t-1\t
2\t1\t1\t0\t0\t0\t10\t10\t300\t60\t-1\t
4\t1\t1\t1\t1\t0\t10\t10\t300\t20\t-1\t
5\t1\t1\t1\t1\t1\t10\t10\t80\t20\t96.1\tHello
5\t1\t1\t1\t1\t2\t100\t12\t120\t18\t95.0\tRustshot
5\t1\t1\t1\t2\t1\t10\t50\t60\t20\t91.0\tsecond
5\t1\t1\t1\t2\t2\t80\t50\t40\t20\t-1\t 
5\t1\t2\t1\t1\t1\t200\t80\t40\t20\t90.0\tnext
bad row
";

    #[test]
    fn words_group_into_lines_with_boxes() {
        let l = parse_tsv(TSV, 2);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Hello", "Rustshot"]);
        assert_eq!(l[0].words[1], Word { text: "Rustshot".into(), x: 50.0, y: 6.0, w: 60.0, h: 9.0 });
        assert_eq!(l[1].words.len(), 1, "the empty word is skipped");
        assert_eq!(l[2].words[0].text, "next");
        assert_eq!(super::super::join_lines(l), "Hello Rustshot\nsecond\nnext");
        assert!(parse_tsv("", 1).is_empty());
    }
}