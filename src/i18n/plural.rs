//! CLDR plural rules for whole numbers, one per language family (a
//! catalog's `@plural`). Fractions never reach the UI's counts, so the
//! rules' `v`/`f` operands are left out.

/// A CLDR plural category (the order plural entries store their forms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cat {
    Zero = 0,
    One = 1,
    Two = 2,
    Few = 3,
    Many = 4,
    Other = 5,
}

/// The rule families of the shipped languages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// No plural forms: Chinese, Japanese, Korean, Thai, Vietnamese, Indonesian.
    None,
    /// 1 is one: English, German, Dutch, the Nordic languages, Italian,
    /// Spanish, Portuguese (Portugal), Greek, Bulgarian, Hungarian, Turkish, Estonian.
    OneOther,
    /// 0 and 1 are one: French, Portuguese (Brazil), Hindi.
    ZeroOneOther,
    /// Russian, Ukrainian: 1, 21 one; 2-4, 22-24 few; the rest many.
    EastSlavic,
    /// Polish: 1 one; 2-4, 22-24 few; the rest many.
    Polish,
    /// Czech, Slovak: 1 one; 2-4 few.
    CzechSlovak,
    /// Serbian, Croatian, Bosnian: 1, 21 one; 2-4, 22-24 few.
    Bcs,
    /// Slovenian: 1, 101 one; 2 two; 3-4 few.
    Slovenian,
    /// Romanian: 1 one; 0, 2-19, 101-119 few.
    Romanian,
    /// Latvian: 0, 10-20, 30 zero; 1, 21 one.
    Latvian,
    /// Lithuanian: 1, 21 one; 2-9, 22-29 few.
    Lithuanian,
    /// Arabic: zero, one, two, few (3-10), many (11-99).
    Arabic,
    /// Hebrew: one, two.
    Hebrew,
}

impl Rule {
    /// The rule a catalog's `@plural` names (unknown: one/other).
    pub fn from_name(s: &str) -> Rule {
        match s {
            "none" => Rule::None,
            "zero_one_other" => Rule::ZeroOneOther,
            "east_slavic" => Rule::EastSlavic,
            "polish" => Rule::Polish,
            "czech_slovak" => Rule::CzechSlovak,
            "bcs" => Rule::Bcs,
            "slovenian" => Rule::Slovenian,
            "romanian" => Rule::Romanian,
            "latvian" => Rule::Latvian,
            "lithuanian" => Rule::Lithuanian,
            "arabic" => Rule::Arabic,
            "hebrew" => Rule::Hebrew,
            _ => Rule::OneOther,
        }
    }

    /// The category of the whole number `n`.
    pub fn category(self, n: u64) -> Cat {
        let (m10, m100) = (n % 10, n % 100);
        let teen = (11..=19).contains(&m100);
        let few24 = (2..=4).contains(&m10) && !(12..=14).contains(&m100);
        match self {
            Rule::None => Cat::Other,
            Rule::OneOther => if n == 1 { Cat::One } else { Cat::Other },
            Rule::ZeroOneOther => if n <= 1 { Cat::One } else { Cat::Other },
            Rule::EastSlavic | Rule::Polish | Rule::Bcs => {
                let one = if self == Rule::Polish { n == 1 } else { m10 == 1 && m100 != 11 };
                if one {
                    Cat::One
                } else if few24 {
                    Cat::Few
                } else if self == Rule::Bcs {
                    Cat::Other
                } else {
                    Cat::Many
                }
            }
            Rule::CzechSlovak => match n {
                1 => Cat::One,
                2..=4 => Cat::Few,
                _ => Cat::Other,
            },
            Rule::Slovenian => match m100 {
                1 => Cat::One,
                2 => Cat::Two,
                3 | 4 => Cat::Few,
                _ => Cat::Other,
            },
            Rule::Romanian => {
                if n == 1 {
                    Cat::One
                } else if n == 0 || (1..=19).contains(&m100) {
                    Cat::Few
                } else {
                    Cat::Other
                }
            }
            Rule::Latvian => {
                if m10 == 0 || teen {
                    Cat::Zero
                } else if m10 == 1 && m100 != 11 {
                    Cat::One
                } else {
                    Cat::Other
                }
            }
            Rule::Lithuanian => {
                if m10 == 1 && !teen {
                    Cat::One
                } else if (2..=9).contains(&m10) && !teen {
                    Cat::Few
                } else {
                    Cat::Other
                }
            }
            Rule::Arabic => match (n, m100) {
                (0, _) => Cat::Zero,
                (1, _) => Cat::One,
                (2, _) => Cat::Two,
                (_, 3..=10) => Cat::Few,
                (_, 11..=99) => Cat::Many,
                _ => Cat::Other,
            },
            Rule::Hebrew => match n {
                1 => Cat::One,
                2 => Cat::Two,
                _ => Cat::Other,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cats(r: Rule, ns: &[u64]) -> Vec<Cat> {
        ns.iter().map(|&n| r.category(n)).collect()
    }

    #[test]
    fn rules_match_cldr_samples() {
        use Cat::*;
        assert_eq!(cats(Rule::OneOther, &[0, 1, 2, 11]), [Other, One, Other, Other]);
        assert_eq!(cats(Rule::ZeroOneOther, &[0, 1, 2]), [One, One, Other]);
        assert_eq!(cats(Rule::None, &[0, 1, 2]), [Other, Other, Other]);
        assert_eq!(cats(Rule::EastSlavic, &[1, 21, 2, 24, 5, 11, 12, 111, 0]), [One, One, Few, Few, Many, Many, Many, Many, Many]);
        assert_eq!(cats(Rule::Polish, &[1, 21, 2, 22, 5, 12, 0]), [One, Many, Few, Few, Many, Many, Many]);
        assert_eq!(cats(Rule::CzechSlovak, &[1, 2, 4, 5, 0, 22]), [One, Few, Few, Other, Other, Other]);
        assert_eq!(cats(Rule::Bcs, &[1, 21, 11, 2, 23, 12, 5]), [One, One, Other, Few, Few, Other, Other]);
        assert_eq!(cats(Rule::Slovenian, &[1, 101, 2, 102, 3, 4, 5, 0]), [One, One, Two, Two, Few, Few, Other, Other]);
        assert_eq!(cats(Rule::Romanian, &[1, 0, 2, 19, 101, 20, 100]), [One, Few, Few, Few, Few, Other, Other]);
        assert_eq!(cats(Rule::Latvian, &[0, 10, 11, 19, 30, 1, 21, 2, 111]), [Zero, Zero, Zero, Zero, Zero, One, One, Other, Zero]);
        assert_eq!(cats(Rule::Lithuanian, &[1, 21, 11, 2, 9, 22, 12, 10, 0]), [One, One, Other, Few, Few, Few, Other, Other, Other]);
        assert_eq!(cats(Rule::Arabic, &[0, 1, 2, 3, 10, 11, 99, 100, 102, 103]), [Zero, One, Two, Few, Few, Many, Many, Other, Other, Few]);
        assert_eq!(cats(Rule::Hebrew, &[1, 2, 3, 20]), [One, Two, Other, Other]);
    }

    #[test]
    fn names_round_trip() {
        for (n, r) in [("none", Rule::None), ("east_slavic", Rule::EastSlavic), ("arabic", Rule::Arabic), ("whatever", Rule::OneOther)] {
            assert_eq!(Rule::from_name(n), r);
        }
    }
}
