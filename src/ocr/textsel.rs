//! Text mode's selection over recognized words (Copy text, "Let me
//! select"): the lines in reading order ([`super::ordered`]), a word is
//! (line, word), and a selection runs from one word to another in that
//! order like text in a document: across a line break it takes the rest
//! of the first line and the start of the last. The copied text keeps
//! single spaces between words and a line break between lines.

use super::Line;

/// A word: (line, index in the line).
pub type Pos = (usize, usize);

/// A word's box (image px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box4 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Box4 {
    pub fn x1(&self) -> f32 {
        self.x + self.w
    }
    pub fn y1(&self) -> f32 {
        self.y + self.h
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.x1() && p.1 >= self.y && p.1 < self.y1()
    }
    fn union(&self, o: &Box4) -> Box4 {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Box4 { x, y, w: self.x1().max(o.x1()) - x, h: self.y1().max(o.y1()) - y }
    }
}

/// The box of word `p` of `lines`.
pub fn word_box(lines: &[Line], p: Pos) -> Option<Box4> {
    let w = lines.get(p.0)?.words.get(p.1)?;
    Some(Box4 { x: w.x, y: w.y, w: w.w.max(0.0), h: w.h.max(0.0) })
}

/// A line's box: its words' union.
pub fn line_box(l: &Line) -> Option<Box4> {
    let mut it = l.words.iter().map(|w| Box4 { x: w.x, y: w.y, w: w.w.max(0.0), h: w.h.max(0.0) });
    let first = it.next()?;
    Some(it.fold(first, |a, b| a.union(&b)))
}

/// Reading order: `a` before or at `b`.
fn le(a: Pos, b: Pos) -> bool {
    a <= b
}

/// The selection `a`..=`b` in reading order (either way round).
pub fn span(a: Pos, b: Pos) -> (Pos, Pos) {
    if le(a, b) { (a, b) } else { (b, a) }
}

/// The word exactly under `p` (inside its box): the I-beam shows there,
/// a click there starts a selection; `None` in the gaps between words.
#[cfg_attr(not(test), allow(dead_code))]
pub fn hit(lines: &[Line], p: (f32, f32)) -> Option<Pos> {
    lines.iter().enumerate().find_map(|(li, l)| {
        l.words.iter().position(|w| (Box4 { x: w.x, y: w.y, w: w.w, h: w.h }).contains(p)).map(|wi| (li, wi))
    })
}

/// The pointer over the text block (a line's band, gaps included): the
/// I-beam and a press that starts a selection.
pub fn over_text(lines: &[Line], p: (f32, f32), slack: f32) -> bool {
    lines.iter().filter_map(line_box).any(|b| p.0 >= b.x - slack && p.0 < b.x1() + slack && p.1 >= b.y - slack && p.1 < b.y1() + slack)
}

/// The word a drag at `p` reaches: the line whose band holds `p.y` (else
/// the nearest line), and in it the word under `p.x` (in a gap: the
/// nearer of the two; before the first or past the last: that one).
pub fn nearest(lines: &[Line], p: (f32, f32)) -> Option<Pos> {
    let mut best: Option<(f32, usize)> = None;
    for (li, l) in lines.iter().enumerate() {
        let Some(b) = line_box(l) else { continue };
        let d = if p.1 < b.y {
            b.y - p.1
        } else if p.1 >= b.y1() {
            p.1 - b.y1() + 0.001
        } else {
            0.0
        };
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, li));
        }
    }
    let li = best?.1;
    let words = &lines[li].words;
    let mut wbest: Option<(f32, usize)> = None;
    for (wi, w) in words.iter().enumerate() {
        let d = if p.0 < w.x {
            w.x - p.0
        } else if p.0 >= w.x + w.w {
            p.0 - (w.x + w.w)
        } else {
            0.0
        };
        if wbest.is_none_or(|(bd, _)| d < bd) {
            wbest = Some((d, wi));
        }
    }
    wbest.map(|(_, wi)| (li, wi))
}

/// The first and last word of line `li`.
pub fn line_span(lines: &[Line], li: usize) -> Option<(Pos, Pos)> {
    let n = lines.get(li)?.words.len();
    (n > 0).then_some(((li, 0), (li, n - 1)))
}

/// Every word (Ctrl+A).
pub fn all(lines: &[Line]) -> Option<(Pos, Pos)> {
    let first = lines.iter().position(|l| !l.words.is_empty())?;
    let last = lines.iter().rposition(|l| !l.words.is_empty())?;
    Some(((first, 0), (last, lines[last].words.len() - 1)))
}

/// The words of `a`..=`b` per line: (line, first word, last word).
pub fn per_line(lines: &[Line], a: Pos, b: Pos) -> Vec<(usize, usize, usize)> {
    let (a, b) = span(a, b);
    let mut out = Vec::new();
    for li in a.0..=b.0.min(lines.len().saturating_sub(1)) {
        let n = lines[li].words.len();
        if n == 0 {
            continue;
        }
        let w0 = if li == a.0 { a.1.min(n - 1) } else { 0 };
        let w1 = if li == b.0 { b.1.min(n - 1) } else { n - 1 };
        if w0 <= w1 {
            out.push((li, w0, w1));
        }
    }
    out
}

/// The text of `a`..=`b`: words joined by single spaces, a line break
/// between lines.
pub fn text(lines: &[Line], a: Pos, b: Pos) -> String {
    per_line(lines, a, b)
        .into_iter()
        .map(|(li, w0, w1)| lines[li].words[w0..=w1].iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// How many words `a`..=`b` holds.
#[cfg_attr(not(test), allow(dead_code))]
pub fn count(lines: &[Line], a: Pos, b: Pos) -> usize {
    per_line(lines, a, b).iter().map(|(_, w0, w1)| w1 - w0 + 1).sum()
}

/// The highlight of `a`..=`b`: one box per line, from its first to its
/// last selected word (the gaps between them filled).
pub fn highlight(lines: &[Line], a: Pos, b: Pos) -> Vec<Box4> {
    per_line(lines, a, b)
        .into_iter()
        .filter_map(|(li, w0, w1)| (w0..=w1).filter_map(|wi| word_box(lines, (li, wi))).reduce(|a, b| a.union(&b)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::Word;

    /// Two lines: "Hello Rustshot world" at y 0..20, "second line" at
    /// y 30..50; words 60 px wide with 10 px gaps.
    fn doc() -> Vec<Line> {
        let l = |y: f32, ws: &[&str]| Line { words: ws.iter().enumerate().map(|(i, t)| Word { text: (*t).into(), x: i as f32 * 70.0, y, w: 60.0, h: 20.0 }).collect() };
        vec![l(0.0, &["Hello", "Rustshot", "world"]), l(30.0, &["second", "line"])]
    }

    #[test]
    fn hit_testing_leaves_the_gaps_out() {
        let d = doc();
        assert_eq!(hit(&d, (5.0, 5.0)), Some((0, 0)));
        assert_eq!(hit(&d, (75.0, 10.0)), Some((0, 1)));
        assert_eq!(hit(&d, (65.0, 10.0)), None, "the gap between two words");
        assert_eq!(hit(&d, (10.0, 25.0)), None, "between the lines");
        assert_eq!(hit(&d, (100.0, 45.0)), Some((1, 1)));
        assert!(over_text(&d, (65.0, 10.0), 0.0), "a gap is on the text");
        assert!(!over_text(&d, (300.0, 10.0), 4.0));
        // A drag in a gap reaches the nearer word; past the end, the last.
        assert_eq!(nearest(&d, (62.0, 10.0)), Some((0, 0)));
        assert_eq!(nearest(&d, (68.0, 10.0)), Some((0, 1)));
        assert_eq!(nearest(&d, (500.0, 10.0)), Some((0, 2)));
        assert_eq!(nearest(&d, (-50.0, 40.0)), Some((1, 0)));
        assert_eq!(nearest(&d, (10.0, 24.0)), Some((0, 0)), "nearer the first line");
        assert_eq!(nearest(&d, (10.0, 27.0)), Some((1, 0)));
        assert_eq!(nearest(&d, (10.0, 400.0)), Some((1, 0)), "below everything: the last line");
        assert_eq!(nearest(&[], (0.0, 0.0)), None);
    }

    #[test]
    fn a_range_follows_reading_order_across_lines() {
        let d = doc();
        assert_eq!(text(&d, (0, 1), (1, 0)), "Rustshot world\nsecond");
        assert_eq!(text(&d, (1, 0), (0, 1)), "Rustshot world\nsecond", "dragged backwards");
        assert_eq!(text(&d, (0, 2), (0, 2)), "world");
        assert_eq!(count(&d, (0, 1), (1, 0)), 3);
        let (a, b) = all(&d).unwrap();
        assert_eq!(text(&d, a, b), "Hello Rustshot world\nsecond line");
        assert_eq!(text(&d, a, b), crate::ocr::join_lines(d.clone()), "Ctrl+A copies what Copy all does");
        let (a, b) = line_span(&d, 1).unwrap();
        assert_eq!(text(&d, a, b), "second line");
        assert_eq!(line_span(&d, 7), None);
        assert_eq!(per_line(&d, (0, 2), (1, 1)), vec![(0, 2, 2), (1, 0, 1)]);
    }

    #[test]
    fn the_highlight_is_one_box_per_line() {
        let d = doc();
        let h = highlight(&d, (0, 1), (1, 0));
        assert_eq!(h, vec![Box4 { x: 70.0, y: 0.0, w: 130.0, h: 20.0 }, Box4 { x: 0.0, y: 30.0, w: 60.0, h: 20.0 }]);
    }
}
