//! Auto-categorizing. The most reliable signal is the Library of Congress class
//! number (librarians already assigned the subject), then Dewey, then keyword
//! matching on subject headings and the title. Every result carries a reason so
//! the app can show *why* a book landed where it did.

pub const CATEGORIES: &[&str] = &[
    "History",
    "Military & War",
    "Biography & Memoir",
    "Politics & Government",
    "Economics & Business",
    "Law",
    "Social Sciences",
    "Psychology",
    "Philosophy",
    "Religion",
    "Science",
    "Technology & Computing",
    "Medicine & Health",
    "Fiction",
    "Literature & Poetry",
    "Art & Music",
    "Geography & Travel",
    "Sports & Recreation",
    "Cooking & Home",
    "Education",
    "Reference",
    "Uncategorized",
];

pub struct Verdict {
    pub category: &'static str,
    pub reason: String,
}

fn v(category: &'static str, reason: impl Into<String>) -> Verdict {
    Verdict { category, reason: reason.into() }
}

pub fn categorize(title: &str, subjects: &str, lcc: &str, dewey: &str) -> Verdict {
    let subj = subjects.to_lowercase();
    let ttl = title.to_lowercase();

    // Novels and biographies get filed by *topic* in LC/Dewey (a WWII novel
    // sits in PS, a Patton biography in E745), so check these headings first.
    let fictionish = subj.split(';').any(|t| {
        let t = t.trim();
        (t.contains("fiction") && !t.contains("nonfiction") && !t.contains("non-fiction"))
            || t.contains("novel")
            || t == "fantasy"
            || t == "thrillers"
    });
    if fictionish {
        return v("Fiction", "subject heading says fiction");
    }
    if ["biography", "autobiography", "memoir", "biographies"]
        .iter()
        .any(|k| subj.contains(k))
    {
        return v("Biography & Memoir", "subject heading says biography/memoir");
    }

    if let Some(c) = from_lcc(lcc) {
        return v(c, format!("Library of Congress class {lcc}"));
    }
    if let Some(c) = from_dewey(dewey) {
        return v(c, format!("Dewey {dewey}"));
    }

    // Whole-word matching so "art" doesn't fire on "start" or "law" on "flaw".
    let hay = words(&format!("{subj} {ttl}"));
    for (cat, kws) in KEYWORDS {
        if let Some(w) = kws.iter().find(|w| {
            let k = words(w);
            hay.contains(&k) || hay.contains(&format!("{}s ", k.trim_end()))
        }) {
            return v(cat, format!("keyword \"{w}\""));
        }
    }
    v("Uncategorized", "no classification or keywords found")
}

/// Lowercase, punctuation to spaces, single-spaced, padded: " like this "
fn words(s: &str) -> String {
    let cleaned: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    format!(" {} ", cleaned.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn from_lcc(lcc: &str) -> Option<&'static str> {
    let lcc = lcc.trim().to_uppercase();
    let letters: String = lcc.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if letters.is_empty() {
        return None;
    }
    // Class number: digits with at most one decimal point ("76.73.R87" -> 76.73)
    let mut seen_dot = false;
    let num: f64 = lcc[letters.len()..]
        .trim_start_matches(['-', ' ', '0'])
        .chars()
        .take_while(|c| {
            if *c == '.' && !seen_dot {
                seen_dot = true;
                true
            } else {
                c.is_ascii_digit()
            }
        })
        .collect::<String>()
        .trim_end_matches('.')
        .parse()
        .unwrap_or(0.0);
    let l = letters.as_str();
    let c = match l.chars().next()? {
        'A' => "Reference",
        'B' => match l {
            "BF" => "Psychology",
            _ if l >= "BL" => "Religion",
            _ => "Philosophy",
        },
        'C' | 'D' | 'E' | 'F' => "History",
        'G' => match l {
            "GV" => "Sports & Recreation",
            "GN" | "GR" | "GT" => "Social Sciences",
            _ => "Geography & Travel",
        },
        'H' => {
            if l <= "HJ" {
                "Economics & Business"
            } else {
                "Social Sciences"
            }
        }
        'J' => "Politics & Government",
        'K' => "Law",
        'L' => "Education",
        'M' | 'N' => "Art & Music",
        'P' => match l {
            "PZ" => "Fiction",
            _ => "Literature & Poetry",
        },
        'Q' => {
            if l == "QA" && (75.0..77.0).contains(&num) {
                "Technology & Computing"
            } else {
                "Science"
            }
        }
        'R' => "Medicine & Health",
        'S' => "Science",
        'T' => match l {
            "TX" => "Cooking & Home",
            _ => "Technology & Computing",
        },
        'U' | 'V' => "Military & War",
        'Z' => "Reference",
        _ => return None,
    };
    Some(c)
}

fn from_dewey(dewey: &str) -> Option<&'static str> {
    let n: f64 = dewey
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .parse()
        .ok()?;
    let n = n as u32;
    Some(match n {
        4..=6 => "Technology & Computing",
        0..=99 => "Reference",
        150..=159 => "Psychology",
        100..=199 => "Philosophy",
        200..=299 => "Religion",
        320..=329 => "Politics & Government",
        330..=339 | 650..=659 => "Economics & Business",
        340..=349 => "Law",
        355..=359 => "Military & War",
        370..=379 => "Education",
        300..=399 => "Social Sciences",
        400..=499 | 800..=899 => "Literature & Poetry",
        500..=599 => "Science",
        610..=619 => "Medicine & Health",
        640..=649 => "Cooking & Home",
        600..=699 => "Technology & Computing",
        790..=799 => "Sports & Recreation",
        700..=799 => "Art & Music",
        910..=919 => "Geography & Travel",
        920..=929 => "Biography & Memoir",
        900..=999 => "History",
        _ => return None,
    })
}

/// Fallback keyword rules, checked in order (first hit wins). Includes Google
/// Books' genre names, which is what most fallback records carry.
const KEYWORDS: &[(&str, &[&str])] = &[
    ("Military & War", &["military", "warfare", "army", "navy", "battles", "world war", "civil war", "tank", "strategy, military"]),
    ("History", &["history", "historical", "ancient", "medieval", "empire", "revolution", "century"]),
    ("Politics & Government", &["political science", "politics", "government", "foreign relations", "democracy", "election"]),
    ("Economics & Business", &["business & economics", "economics", "finance", "management", "investing", "accounting", "marketing", "entrepreneur"]),
    ("Technology & Computing", &["computers", "programming", "software", "rust", "python", "javascript", "engineering", "technology", "artificial intelligence", "data"]),
    ("Science", &["science", "physics", "chemistry", "biology", "mathematics", "astronomy", "evolution", "nature"]),
    ("Medicine & Health", &["medical", "medicine", "health", "fitness", "nutrition"]),
    ("Psychology", &["psychology", "self-help", "habits", "mind"]),
    ("Philosophy", &["philosophy", "ethics", "stoic"]),
    ("Religion", &["religion", "bible", "christian", "theology", "spirituality", "buddhism"]),
    ("Law", &["law", "legal"]),
    ("Social Sciences", &["social science", "sociology", "anthropology", "true crime", "society"]),
    ("Literature & Poetry", &["poetry", "literary criticism", "drama", "essays", "literary collections"]),
    ("Art & Music", &["art", "music", "photography", "design", "film", "performing arts", "architecture"]),
    ("Geography & Travel", &["travel", "geography", "maps", "guidebook"]),
    ("Sports & Recreation", &["sports", "games", "recreation", "baseball", "football"]),
    ("Cooking & Home", &["cooking", "cookbook", "recipes", "gardening", "house & home", "crafts"]),
    ("Education", &["education", "teaching", "study aids"]),
    ("Reference", &["reference", "dictionaries", "encyclopedias", "language arts"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcc_classes() {
        assert_eq!(categorize("", "", "D810.S7 H3", "").category, "History");
        assert_eq!(categorize("", "", "QA76.73.R87 K53", "").category, "Technology & Computing");
        assert_eq!(categorize("", "", "QC21.3", "").category, "Science");
        assert_eq!(categorize("", "", "BF637.S4", "").category, "Psychology");
        assert_eq!(categorize("", "", "BR115", "").category, "Religion");
        assert_eq!(categorize("", "", "U162", "").category, "Military & War");
        assert_eq!(categorize("", "", "TX714", "").category, "Cooking & Home");
        assert_eq!(categorize("", "", "DF-0229.00000000.T55", "").category, "History");
        assert_eq!(categorize("", "", "HG4521", "").category, "Economics & Business");
        assert_eq!(categorize("", "", "HV6250", "").category, "Social Sciences");
    }

    #[test]
    fn fiction_and_bio_beat_classification() {
        let novel = categorize("Slaughterhouse-Five", "World War, 1939-1945 -- Fiction", "PS3572.O5", "813.54");
        assert_eq!(novel.category, "Fiction");
        let bio = categorize("Patton", "Generals -- United States -- Biography", "E745.P3", "");
        assert_eq!(bio.category, "Biography & Memoir");
        assert_eq!(categorize("", "Nonfiction; History", "", "").category, "History");
    }

    #[test]
    fn dewey_and_keywords() {
        assert_eq!(categorize("", "", "", "940.54").category, "History");
        assert_eq!(categorize("", "", "", "005.133").category, "Technology & Computing");
        assert_eq!(categorize("", "Business & Economics", "", "").category, "Economics & Business");
        assert_eq!(categorize("A History of Rome", "", "", "").category, "History");
        assert_eq!(categorize("Untitled", "", "", "").category, "Uncategorized");
        // whole words only
        assert_eq!(categorize("Start With Why", "", "", "").category, "Uncategorized");
        assert_eq!(categorize("", "Cookbooks", "", "").category, "Cooking & Home");
    }
}
