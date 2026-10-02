//! Storage: one SQLite file (library.db) next to the .exe. No server to run,
//! back it up by copying the file. CSV export covers getting data out.

use crate::lookup::BookInfo;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;

pub const STATUSES: &[&str] = &["Unread", "Reading", "Read", "Abandoned", "Wishlist"];

#[derive(Debug, Clone, Default)]
pub struct Book {
    pub id: i64,
    pub isbn: String,
    pub title: String,
    pub authors: String,
    pub publisher: String,
    pub year: String,
    pub pages: Option<i64>,
    pub subjects: String,
    pub lcc: String,
    pub dewey: String,
    pub cover_url: String,
    pub source: String,
    pub category: String,
    pub category_reason: String,
    /// true once you've picked the category by hand; "re-categorize all" skips it
    pub category_locked: bool,
    pub status: String,
    pub rating: i64,
    pub location: String,
    pub loaned_to: String,
    pub notes: String,
    pub copies: i64,
    pub added_at: String,
}

pub struct Db {
    conn: Connection,
}

const COLS: &str = "id, isbn, title, authors, publisher, year, pages, subjects, lcc, dewey, \
    cover_url, source, category, category_reason, category_locked, status, rating, location, \
    loaned_to, notes, copies, added_at";

fn row_to_book(r: &Row) -> rusqlite::Result<Book> {
    Ok(Book {
        id: r.get(0)?,
        isbn: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
        title: r.get(2)?,
        authors: r.get(3)?,
        publisher: r.get(4)?,
        year: r.get(5)?,
        pages: r.get(6)?,
        subjects: r.get(7)?,
        lcc: r.get(8)?,
        dewey: r.get(9)?,
        cover_url: r.get(10)?,
        source: r.get(11)?,
        category: r.get(12)?,
        category_reason: r.get(13)?,
        category_locked: r.get(14)?,
        status: r.get(15)?,
        rating: r.get(16)?,
        location: r.get(17)?,
        loaned_to: r.get(18)?,
        notes: r.get(19)?,
        copies: r.get(20)?,
        added_at: r.get(21)?,
    })
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS books (
                id INTEGER PRIMARY KEY,
                isbn TEXT,
                title TEXT NOT NULL,
                authors TEXT NOT NULL DEFAULT '',
                publisher TEXT NOT NULL DEFAULT '',
                year TEXT NOT NULL DEFAULT '',
                pages INTEGER,
                subjects TEXT NOT NULL DEFAULT '',
                lcc TEXT NOT NULL DEFAULT '',
                dewey TEXT NOT NULL DEFAULT '',
                cover_url TEXT NOT NULL DEFAULT '',
                source TEXT NOT NULL DEFAULT '',
                category TEXT NOT NULL DEFAULT 'Uncategorized',
                category_reason TEXT NOT NULL DEFAULT '',
                category_locked INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'Unread',
                rating INTEGER NOT NULL DEFAULT 0,
                location TEXT NOT NULL DEFAULT '',
                loaned_to TEXT NOT NULL DEFAULT '',
                notes TEXT NOT NULL DEFAULT '',
                copies INTEGER NOT NULL DEFAULT 1,
                added_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
             );
             CREATE UNIQUE INDEX IF NOT EXISTS books_isbn ON books(isbn)
                WHERE isbn IS NOT NULL AND isbn <> '';
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Ok(Self { conn })
    }

    pub fn all(&self) -> rusqlite::Result<Vec<Book>> {
        let mut st = self
            .conn
            .prepare(&format!("SELECT {COLS} FROM books ORDER BY added_at DESC, id DESC"))?;
        let rows = st.query_map([], row_to_book)?;
        rows.collect()
    }

    pub fn by_isbn(&self, isbn: &str) -> rusqlite::Result<Option<Book>> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM books WHERE isbn = ?1"),
                [isbn],
                row_to_book,
            )
            .optional()
    }

    /// Insert a freshly looked-up (or hand-typed) book; returns its id.
    pub fn insert(&self, info: &BookInfo, location: &str) -> rusqlite::Result<i64> {
        let verdict =
            crate::categorize::categorize(&info.title, &info.subjects, &info.lcc, &info.dewey);
        let isbn = (!info.isbn.is_empty()).then_some(info.isbn.as_str());
        self.conn.execute(
            "INSERT INTO books (isbn, title, authors, publisher, year, pages, subjects, lcc, dewey,
                cover_url, source, category, category_reason, location)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                isbn,
                if info.title.is_empty() { "(untitled)" } else { &info.title },
                info.authors,
                info.publisher,
                info.year,
                info.pages,
                info.subjects,
                info.lcc,
                info.dewey,
                info.cover_url,
                info.source,
                verdict.category,
                verdict.reason,
                location
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn add_copy(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE books SET copies = copies + 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn update(&self, b: &Book) -> rusqlite::Result<()> {
        let isbn = (!b.isbn.is_empty()).then_some(b.isbn.as_str());
        self.conn.execute(
            "UPDATE books SET isbn=?2, title=?3, authors=?4, publisher=?5, year=?6, pages=?7,
                category=?8, category_reason=?9, category_locked=?10, status=?11, rating=?12,
                location=?13, loaned_to=?14, notes=?15, copies=?16
             WHERE id=?1",
            params![
                b.id,
                isbn,
                b.title,
                b.authors,
                b.publisher,
                b.year,
                b.pages,
                b.category,
                b.category_reason,
                b.category_locked,
                b.status,
                b.rating,
                b.location,
                b.loaned_to,
                b.notes,
                b.copies
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM books WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Re-run the categorizer on every book you haven't hand-categorized.
    /// Returns how many changed.
    pub fn recategorize_unlocked(&self) -> rusqlite::Result<usize> {
        let books = self.all()?;
        let mut changed = 0;
        for b in books.iter().filter(|b| !b.category_locked) {
            let v = crate::categorize::categorize(&b.title, &b.subjects, &b.lcc, &b.dewey);
            if v.category != b.category {
                changed += 1;
            }
            self.conn.execute(
                "UPDATE books SET category=?2, category_reason=?3 WHERE id=?1",
                params![b.id, v.category, v.reason],
            )?;
        }
        Ok(changed)
    }

    pub fn setting(&self, key: &str) -> String {
        self.conn
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0))
            .unwrap_or_default()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}

/// Write the whole library to CSV (opens in Excel; Goodreads-ish columns).
pub fn export_csv(books: &[Book], path: &Path) -> std::io::Result<()> {
    fn q(s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    let mut out = String::from(
        "ISBN,Title,Authors,Publisher,Year,Pages,Category,Status,Rating,Location,Loaned To,Copies,LCC,Dewey,Subjects,Notes,Added\r\n",
    );
    for b in books {
        let fields = [
            q(&b.isbn),
            q(&b.title),
            q(&b.authors),
            q(&b.publisher),
            q(&b.year),
            b.pages.map(|p| p.to_string()).unwrap_or_default(),
            q(&b.category),
            q(&b.status),
            b.rating.to_string(),
            q(&b.location),
            q(&b.loaned_to),
            b.copies.to_string(),
            q(&b.lcc),
            q(&b.dewey),
            q(&b.subjects),
            q(&b.notes),
            q(&b.added_at),
        ];
        out.push_str(&fields.join(","));
        out.push_str("\r\n");
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_duplicates() {
        let db = Db::open(Path::new(":memory:")).unwrap();
        let info = BookInfo {
            isbn: "9780140449136".into(),
            title: "The Odyssey".into(),
            lcc: "PA4025".into(),
            ..Default::default()
        };
        let id = db.insert(&info, "Office shelf 2").unwrap();
        assert!(db.insert(&info, "").is_err(), "same ISBN twice should be refused");
        db.add_copy(id).unwrap();
        let b = db.by_isbn("9780140449136").unwrap().unwrap();
        assert_eq!(b.copies, 2);
        assert_eq!(b.category, "Literature & Poetry");
        assert_eq!(b.location, "Office shelf 2");

        // manual books with no ISBN don't collide
        let manual = BookInfo { title: "Grandpa's diary".into(), ..Default::default() };
        db.insert(&manual, "").unwrap();
        db.insert(&manual, "").unwrap();
        assert_eq!(db.all().unwrap().len(), 3);

        // locked categories survive a re-run
        let mut b = db.by_isbn("9780140449136").unwrap().unwrap();
        b.category = "History".into();
        b.category_locked = true;
        db.update(&b).unwrap();
        db.recategorize_unlocked().unwrap();
        assert_eq!(db.by_isbn("9780140449136").unwrap().unwrap().category, "History");
    }
}
