//! Book metadata lookups. Open Library first (free, no key, gives Library of
//! Congress + Dewey class numbers, which make categorizing reliable), Google
//! Books as a fallback (free, no key, ~1000 lookups/day per IP).

use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct BookInfo {
    pub isbn: String,
    pub title: String,
    pub authors: String,
    pub publisher: String,
    pub year: String,
    pub pages: Option<i64>,
    /// Subject headings / genre tags from the source, joined with "; "
    pub subjects: String,
    /// Library of Congress class number, e.g. "D810.S7"
    pub lcc: String,
    /// Dewey decimal, e.g. "940.54"
    pub dewey: String,
    pub cover_url: String,
    pub source: String,
}

fn agent(contact: &str) -> ureq::Agent {
    // Open Library gives identified apps 3 req/s instead of 1.
    let ua = if contact.trim().is_empty() {
        "Rookshelf/0.1 (home library app)".to_string()
    } else {
        format!("Rookshelf/0.1 ({})", contact.trim())
    };
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .user_agent(ua)
        .build()
        .into()
}

fn get_json(agent: &ureq::Agent, url: &str) -> Result<Value, String> {
    let body = agent
        .get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().trim().to_string()
}

fn names(v: &Value, key: &str) -> String {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|x| if key.is_empty() { s(x) } else { s(&x[key]) })
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default()
}

fn first(v: &Value) -> String {
    v.as_array().and_then(|a| a.first()).map(s).unwrap_or_default()
}

fn year_of(date: &str) -> String {
    // "May 2003", "2003-05-01", "c1999" -> "2003"/"1999"
    date.as_bytes()
        .windows(4)
        .find(|w| w.iter().all(u8::is_ascii_digit))
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .unwrap_or_default()
}

/// Look up one ISBN-13. Tries Open Library, then Google Books, and merges so
/// a thin Open Library record still gets Google's genre tags.
pub fn by_isbn(isbn: &str, contact: &str) -> Result<BookInfo, String> {
    let agent = agent(contact);
    let ol = open_library_isbn(&agent, isbn);
    let needs_google = match &ol {
        Ok(b) => b.subjects.is_empty() && b.lcc.is_empty() && b.dewey.is_empty(),
        Err(_) => true,
    };
    let mut google_err = String::new();
    let gb = if needs_google {
        google_isbn(&agent, isbn).map_err(|e| google_err = e).ok()
    } else {
        None
    };

    match (ol, gb) {
        (Ok(mut b), Some(g)) => {
            b.subjects = g.subjects;
            if b.pages.is_none() {
                b.pages = g.pages;
            }
            b.source.push_str(" + Google Books");
            Ok(b)
        }
        (Ok(b), None) => Ok(b),
        (Err(_), Some(g)) => Ok(g),
        (Err(e), None) => Err(format!("Open Library: {e} · Google Books: {google_err}")),
    }
}

fn open_library_isbn(agent: &ureq::Agent, isbn: &str) -> Result<BookInfo, String> {
    let url = format!(
        "https://openlibrary.org/api/books?bibkeys=ISBN:{isbn}&format=json&jscmd=data"
    );
    let v = get_json(agent, &url)?;
    let d = &v[format!("ISBN:{isbn}")];
    if d.is_null() {
        return Err("no Open Library record".into());
    }
    let title = match s(&d["subtitle"]) {
        sub if sub.is_empty() => s(&d["title"]),
        sub => format!("{}: {}", s(&d["title"]), sub),
    };
    Ok(BookInfo {
        isbn: isbn.to_string(),
        title,
        authors: names(&d["authors"], "name"),
        publisher: names(&d["publishers"], "name"),
        year: year_of(&s(&d["publish_date"])),
        pages: d["number_of_pages"].as_i64(),
        subjects: names(&d["subjects"], "name"),
        lcc: first(&d["classifications"]["lc_classifications"]),
        dewey: first(&d["classifications"]["dewey_decimal_class"]),
        cover_url: s(&d["cover"]["medium"]),
        source: "Open Library".into(),
    })
}

fn google_volume(info: &Value, isbn_hint: &str) -> BookInfo {
    let isbn = info["industryIdentifiers"]
        .as_array()
        .and_then(|ids| ids.iter().find(|i| i["type"] == "ISBN_13"))
        .map(|i| s(&i["identifier"]))
        .unwrap_or_else(|| isbn_hint.to_string());
    let title = match s(&info["subtitle"]) {
        sub if sub.is_empty() => s(&info["title"]),
        sub => format!("{}: {}", s(&info["title"]), sub),
    };
    BookInfo {
        isbn,
        title,
        authors: names(&info["authors"], ""),
        publisher: s(&info["publisher"]),
        year: year_of(&s(&info["publishedDate"])),
        pages: info["pageCount"].as_i64(),
        subjects: names(&info["categories"], ""),
        cover_url: s(&info["imageLinks"]["thumbnail"]),
        source: "Google Books".into(),
        ..Default::default()
    }
}

fn google_isbn(agent: &ureq::Agent, isbn: &str) -> Result<BookInfo, String> {
    let url = format!("https://www.googleapis.com/books/v1/volumes?q=isbn:{isbn}");
    let v = get_json(agent, &url)?;
    let info = &v["items"][0]["volumeInfo"];
    if info.is_null() {
        return Err("no Google Books record".into());
    }
    Ok(google_volume(info, isbn))
}

/// Title (and optional author) search for manual adds — books with no
/// barcode, old books, gifts with the jacket gone. Returns up to 10 candidates.
pub fn search(title: &str, author: &str, contact: &str) -> Result<Vec<BookInfo>, String> {
    let agent = agent(contact);
    let mut url = format!(
        "https://openlibrary.org/search.json?title={}&limit=10&fields=title,subtitle,author_name,publisher,first_publish_year,number_of_pages_median,subject,lcc,ddc,isbn,cover_i",
        urlencoding::encode(title.trim())
    );
    if !author.trim().is_empty() {
        url.push_str(&format!("&author={}", urlencoding::encode(author.trim())));
    }
    let v = get_json(&agent, &url)?;
    let docs = v["docs"].as_array().cloned().unwrap_or_default();

    let mut out: Vec<BookInfo> = docs
        .iter()
        .map(|d| {
            let isbn = d["isbn"]
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .map(s)
                        .find(|i| i.len() == 13)
                        .or_else(|| a.first().map(s))
                })
                .and_then(|i| crate::isbn::normalize(&i).ok())
                .unwrap_or_default();
            let subjects = d["subject"]
                .as_array()
                .map(|a| a.iter().take(15).map(s).collect::<Vec<_>>().join("; "))
                .unwrap_or_default();
            BookInfo {
                isbn,
                title: s(&d["title"]),
                authors: names(&d["author_name"], ""),
                publisher: first(&d["publisher"]),
                year: d["first_publish_year"].as_i64().map(|y| y.to_string()).unwrap_or_default(),
                pages: d["number_of_pages_median"].as_i64(),
                subjects,
                // search.json pads LCC like "DF-0229.00000000.T55"; the class
                // letters at the front are all the categorizer needs.
                lcc: first(&d["lcc"]),
                dewey: first(&d["ddc"]),
                cover_url: d["cover_i"]
                    .as_i64()
                    .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-M.jpg"))
                    .unwrap_or_default(),
                source: "Open Library search".into(),
            }
        })
        .collect();

    if out.is_empty() {
        // Fall back to Google for anything Open Library doesn't know.
        let mut q = format!("intitle:{}", title.trim());
        if !author.trim().is_empty() {
            q.push_str(&format!("+inauthor:{}", author.trim()));
        }
        let url = format!(
            "https://www.googleapis.com/books/v1/volumes?maxResults=10&q={}",
            urlencoding::encode(&q)
        );
        let v = get_json(&agent, &url)?;
        if let Some(items) = v["items"].as_array() {
            out = items.iter().map(|i| google_volume(&i["volumeInfo"], "")).collect();
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn years() {
        assert_eq!(year_of("May 2003"), "2003");
        assert_eq!(year_of("2003-05-01"), "2003");
        assert_eq!(year_of("c1999."), "1999");
        assert_eq!(year_of(""), "");
    }

    #[test]
    fn parses_open_library_shape() {
        let v: Value = serde_json::from_str(
            r#"{"title":"The Odyssey","authors":[{"name":"Homer"}],"publishers":[{"name":"Penguin"}],
                "publish_date":"2003","number_of_pages":541,
                "subjects":[{"name":"Epic poetry, Greek"}],
                "classifications":{"lc_classifications":["PA4025.A5 R5"],"dewey_decimal_class":["883.01"]}}"#,
        )
        .unwrap();
        assert_eq!(names(&v["authors"], "name"), "Homer");
        assert_eq!(first(&v["classifications"]["lc_classifications"]), "PA4025.A5 R5");
    }
}
