//! ISBN cleanup. Barcode scanners send the EAN-13 printed on the back cover
//! (978/979...), people type ISBN-10s with hyphens. Everything gets turned into
//! a validated ISBN-13 before it touches the database.

/// Strip a raw scan/typed string down to an ISBN-13, or explain why not.
pub fn normalize(raw: &str) -> Result<String, String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect();

    match cleaned.len() {
        13 => {
            if !(cleaned.starts_with("978") || cleaned.starts_with("979")) {
                return Err(format!(
                    "{cleaned} is a barcode but not a book ISBN (should start with 978/979)"
                ));
            }
            if !valid13(&cleaned) {
                return Err(format!("{cleaned} fails the ISBN-13 checksum — rescan?"));
            }
            Ok(cleaned)
        }
        10 => {
            if !valid10(&cleaned) {
                return Err(format!("{cleaned} fails the ISBN-10 checksum"));
            }
            Ok(to13(&cleaned))
        }
        // Some scanners append the 5-digit price add-on (EAN-5) to the ISBN.
        18 if cleaned.starts_with("978") || cleaned.starts_with("979") => normalize(&cleaned[..13]),
        0 => Err("Nothing scanned".into()),
        n => Err(format!("'{raw}' has {n} digits — an ISBN has 10 or 13")),
    }
}

fn digit(c: char) -> u32 {
    c.to_digit(10).unwrap_or(10) // 'X' == 10
}

fn valid13(s: &str) -> bool {
    if s.contains('X') {
        return false;
    }
    let sum: u32 = s
        .chars()
        .enumerate()
        .map(|(i, c)| digit(c) * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    sum.is_multiple_of(10)
}

fn valid10(s: &str) -> bool {
    // 'X' is only legal as the check digit
    if s[..9].contains('X') {
        return false;
    }
    let sum: u32 = s
        .chars()
        .enumerate()
        .map(|(i, c)| digit(c) * (10 - i as u32))
        .sum();
    sum.is_multiple_of(11)
}

fn to13(isbn10: &str) -> String {
    let body = format!("978{}", &isbn10[..9]);
    let sum: u32 = body
        .chars()
        .enumerate()
        .map(|(i, c)| digit(c) * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    let check = (10 - sum % 10) % 10;
    format!("{body}{check}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_and_typing() {
        assert_eq!(normalize("9780140449136").unwrap(), "9780140449136");
        assert_eq!(normalize("978-0-14-044913-6").unwrap(), "9780140449136");
        assert_eq!(normalize("0140449132").unwrap(), "9780140449136");
        assert_eq!(normalize("0-8044-2957-X").unwrap(), "9780804429573");
        assert_eq!(normalize("978014044913651299").unwrap(), "9780140449136");
    }

    #[test]
    fn rejects_garbage() {
        assert!(normalize("9780140449137").is_err()); // bad checksum
        assert!(normalize("0123456789012").is_err()); // UPC, not ISBN
        assert!(normalize("12345").is_err());
    }
}
