use crate::categorize::CATEGORIES;
use crate::db::{Book, Db, STATUSES};
use crate::lookup::{self, BookInfo};
use crate::{isbn, pie};
use eframe::egui::{self, Color32, RichText};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Scan,
    Library,
    Stats,
    Settings,
}

enum Msg {
    Isbn { scan_id: usize, isbn: String, result: Result<BookInfo, String> },
    Search(Result<Vec<BookInfo>, String>),
}

enum ScanState {
    Pending,
    Added { book_id: i64, title: String, category: String },
    Duplicate { book_id: i64, title: String, copies: i64 },
    NotFound(String),
    Invalid(String),
}

struct ScanEntry {
    id: usize,
    isbn: String,
    state: ScanState,
}

#[derive(PartialEq, Clone, Copy)]
enum Measure {
    Books,
    Pages,
}

pub struct RookshelfApp {
    db: Db,
    db_path: PathBuf,
    books: Vec<Book>,
    tab: Tab,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,

    // scan tab
    scan_input: String,
    batch_location: String,
    scans: Vec<ScanEntry>,
    next_scan: usize,
    in_flight: HashSet<String>,
    // manual add
    m_title: String,
    m_author: String,
    m_isbn: String,
    searching: bool,
    results: Vec<BookInfo>,
    search_err: String,

    // library tab
    filter_text: String,
    filter_cat: String,
    filter_status: String,
    selected: Option<Book>,

    sort: (usize, bool),
    // stats
    measure: Measure,

    // settings
    contact: String,
    status_line: String,
}

impl RookshelfApp {
    pub fn new(cc: &eframe::CreationContext<'_>, db_path: PathBuf) -> Self {
        let db = Db::open(&db_path).expect("could not open library.db");
        let (tx, rx) = channel();
        let contact = db.setting("contact");
        let batch_location = db.setting("batch_location");
        let books = db.all().unwrap_or_default();
        Self {
            db,
            db_path,
            books,
            tab: Tab::Scan,
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
            scan_input: String::new(),
            batch_location,
            scans: Vec::new(),
            next_scan: 0,
            in_flight: HashSet::new(),
            m_title: String::new(),
            m_author: String::new(),
            m_isbn: String::new(),
            searching: false,
            results: Vec::new(),
            search_err: String::new(),
            filter_text: String::new(),
            filter_cat: String::new(),
            filter_status: String::new(),
            selected: None,
            sort: (5, false), // newest first
            measure: Measure::Books,
            contact,
            status_line: String::new(),
        }
    }

    fn reload(&mut self) {
        self.books = self.db.all().unwrap_or_default();
    }

    fn flash(&mut self, s: impl Into<String>) {
        self.status_line = s.into();
    }

    // ---------- scanning ----------

    fn submit_scan(&mut self) {
        let raw = std::mem::take(&mut self.scan_input);
        if raw.trim().is_empty() {
            return;
        }
        let id = self.next_scan;
        self.next_scan += 1;
        let isbn = match isbn::normalize(&raw) {
            Ok(i) => i,
            Err(e) => {
                self.scans.insert(0, ScanEntry { id, isbn: raw.trim().into(), state: ScanState::Invalid(e) });
                return;
            }
        };
        if let Ok(Some(b)) = self.db.by_isbn(&isbn) {
            self.scans.insert(0, ScanEntry {
                id,
                isbn,
                state: ScanState::Duplicate { book_id: b.id, title: b.title, copies: b.copies },
            });
            return;
        }
        if self.in_flight.contains(&isbn) {
            return; // double-beep on the same book; first lookup is still running
        }
        self.in_flight.insert(isbn.clone());
        self.scans.insert(0, ScanEntry { id, isbn: isbn.clone(), state: ScanState::Pending });

        let (tx, ctx, contact) = (self.tx.clone(), self.ctx.clone(), self.contact.clone());
        std::thread::spawn(move || {
            let result = lookup::by_isbn(&isbn, &contact);
            let _ = tx.send(Msg::Isbn { scan_id: id, isbn, result });
            ctx.request_repaint();
        });
    }

    fn start_search(&mut self) {
        if self.m_title.trim().is_empty() {
            return;
        }
        self.searching = true;
        self.results.clear();
        self.search_err.clear();
        let (tx, ctx, contact) = (self.tx.clone(), self.ctx.clone(), self.contact.clone());
        let (t, a) = (self.m_title.clone(), self.m_author.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Search(lookup::search(&t, &a, &contact)));
            ctx.request_repaint();
        });
    }

    fn add_info(&mut self, info: &BookInfo) -> Result<i64, String> {
        if !info.isbn.is_empty() {
            if let Ok(Some(b)) = self.db.by_isbn(&info.isbn) {
                return Err(format!("Already in your library: {}", b.title));
            }
        }
        let id = self.db.insert(info, self.batch_location.trim()).map_err(|e| e.to_string())?;
        self.reload();
        Ok(id)
    }

    fn handle_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Isbn { scan_id, isbn, result } => {
                    self.in_flight.remove(&isbn);
                    let state = match result {
                        Ok(info) => match self.add_info(&info) {
                            Ok(book_id) => {
                                let category = self
                                    .books
                                    .iter()
                                    .find(|b| b.id == book_id)
                                    .map(|b| b.category.clone())
                                    .unwrap_or_default();
                                ScanState::Added { book_id, title: info.title, category }
                            }
                            Err(e) => ScanState::NotFound(e),
                        },
                        Err(e) => ScanState::NotFound(e),
                    };
                    if let Some(entry) = self.scans.iter_mut().find(|s| s.id == scan_id) {
                        entry.state = state;
                    }
                }
                Msg::Search(r) => {
                    self.searching = false;
                    match r {
                        Ok(v) if v.is_empty() => self.search_err = "No matches. Add it by hand below.".into(),
                        Ok(v) => self.results = v,
                        Err(e) => self.search_err = e,
                    }
                }
            }
        }
    }

    fn open_book(&mut self, id: i64) {
        self.selected = self.books.iter().find(|b| b.id == id).cloned();
        self.tab = Tab::Library;
    }

    // ---------- tabs ----------

    fn scan_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scan a book");
        ui.label("Point the scanner at the barcode on the back cover. Each beep adds a book.");
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("Shelf / location for this batch:");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.batch_location)
                    .hint_text("e.g. Office bookcase, top shelf")
                    .desired_width(260.0),
            );
            if r.lost_focus() {
                let _ = self.db.set_setting("batch_location", &self.batch_location);
            }
        });
        ui.add_space(4.0);

        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.scan_input)
                .hint_text("Scan or type an ISBN, then Enter")
                .font(egui::TextStyle::Heading)
                .desired_width(420.0),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter {
            self.submit_scan();
        }
        // Keep the scan box focused so the scanner's keystrokes always land here.
        if enter || !ui.ctx().egui_wants_keyboard_input() {
            resp.request_focus();
        }

        ui.add_space(8.0);
        ui.separator();

        let mut add_copy: Option<(usize, i64)> = None;
        let mut open: Option<i64> = None;
        let mut prefill: Option<String> = None;
        let mut retry: Option<String> = None;
        egui::ScrollArea::vertical()
            .id_salt("scanlog")
            .max_height(ui.available_height() * 0.45)
            .show(ui, |ui| {
                if self.scans.is_empty() {
                    ui.weak("Scans from this session show up here.");
                }
                for s in &self.scans {
                    ui.horizontal(|ui| {
                        ui.monospace(&s.isbn);
                        match &s.state {
                            ScanState::Pending => {
                                ui.spinner();
                                ui.label("Looking up…");
                            }
                            ScanState::Added { book_id, title, category } => {
                                ui.label(RichText::new("✔ Added").color(Color32::from_rgb(0, 140, 60)));
                                if ui.link(title).clicked() {
                                    open = Some(*book_id);
                                }
                                ui.weak(format!("filed under {category}"));
                            }
                            ScanState::Duplicate { book_id, title, copies } => {
                                ui.label(RichText::new("Already have it").color(Color32::from_rgb(200, 130, 0)));
                                if ui.link(title).clicked() {
                                    open = Some(*book_id);
                                }
                                ui.weak(format!("({copies} on record)"));
                                if ui.small_button("+1 copy").clicked() {
                                    add_copy = Some((s.id, *book_id));
                                }
                            }
                            ScanState::NotFound(e) => {
                                ui.label(RichText::new("✖ Not found").color(Color32::from_rgb(200, 50, 50)));
                                ui.weak(e);
                                if ui.small_button("Retry").clicked() {
                                    retry = Some(s.isbn.clone());
                                }
                                if ui.small_button("Add by hand").clicked() {
                                    prefill = Some(s.isbn.clone());
                                }
                            }
                            ScanState::Invalid(e) => {
                                ui.label(RichText::new("✖").color(Color32::from_rgb(200, 50, 50)));
                                ui.weak(e);
                            }
                        }
                    });
                }
            });
        if let Some((scan_id, book_id)) = add_copy {
            if self.db.add_copy(book_id).is_ok() {
                self.reload();
                let copies = self.books.iter().find(|b| b.id == book_id).map(|b| b.copies).unwrap_or(0);
                if let Some(e) = self.scans.iter_mut().find(|s| s.id == scan_id) {
                    if let ScanState::Duplicate { copies: c, .. } = &mut e.state {
                        *c = copies;
                    }
                }
            }
        }
        if let Some(id) = open {
            self.open_book(id);
        }
        if let Some(i) = prefill {
            self.m_isbn = i;
        }
        if let Some(i) = retry {
            self.scans.retain(|s| s.isbn != i);
            self.scan_input = i;
            self.submit_scan();
        }

        ui.separator();
        self.manual_add(ui);
    }

    fn manual_add(&mut self, ui: &mut egui::Ui) {
        ui.heading("No barcode? Add by title");
        egui::Grid::new("manual").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Title");
            let t = ui.add(egui::TextEdit::singleline(&mut self.m_title).desired_width(320.0));
            ui.end_row();
            ui.label("Author (optional)");
            let a = ui.add(egui::TextEdit::singleline(&mut self.m_author).desired_width(320.0));
            ui.end_row();
            ui.label("ISBN (optional)");
            ui.add(egui::TextEdit::singleline(&mut self.m_isbn).desired_width(320.0));
            ui.end_row();
            if (t.lost_focus() || a.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.start_search();
            }
        });
        ui.horizontal(|ui| {
            if ui.button("🔍 Search Open Library").clicked() {
                self.start_search();
            }
            if ui
                .button("Add exactly as typed")
                .on_hover_text("Skip the lookup — for self-published, family, or very old books")
                .clicked()
                && !self.m_title.trim().is_empty()
            {
                let info = BookInfo {
                    isbn: isbn::normalize(&self.m_isbn).unwrap_or_default(),
                    title: self.m_title.trim().into(),
                    authors: self.m_author.trim().into(),
                    source: "Manual".into(),
                    ..Default::default()
                };
                match self.add_info(&info) {
                    Ok(id) => {
                        self.flash(format!("Added \"{}\"", info.title));
                        self.m_title.clear();
                        self.m_author.clear();
                        self.m_isbn.clear();
                        self.open_book(id);
                    }
                    Err(e) => self.flash(e),
                }
            }
            if self.searching {
                ui.spinner();
            }
        });
        if !self.search_err.is_empty() {
            ui.colored_label(Color32::from_rgb(200, 50, 50), &self.search_err);
        }

        let mut pick: Option<BookInfo> = None;
        egui::ScrollArea::vertical().id_salt("results").show(ui, |ui| {
            for r in &self.results {
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() {
                        pick = Some(r.clone());
                    }
                    ui.vertical(|ui| {
                        ui.strong(&r.title);
                        let mut line = r.authors.clone();
                        if !r.year.is_empty() {
                            line.push_str(&format!(" · {}", r.year));
                        }
                        if !r.isbn.is_empty() {
                            line.push_str(&format!(" · ISBN {}", r.isbn));
                        }
                        ui.weak(line);
                    });
                });
            }
        });
        if let Some(mut info) = pick {
            if info.isbn.is_empty() {
                info.isbn = isbn::normalize(&self.m_isbn).unwrap_or_default();
            }
            match self.add_info(&info) {
                Ok(_) => {
                    self.flash(format!("Added \"{}\"", info.title));
                    self.results.clear();
                    self.m_title.clear();
                    self.m_author.clear();
                    self.m_isbn.clear();
                }
                Err(e) => self.flash(e),
            }
        }
    }

    fn library_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter_text)
                    .hint_text("Search title, author, ISBN, notes…")
                    .desired_width(280.0),
            );
            filter_combo(ui, "fcat", "All categories", &mut self.filter_cat, CATEGORIES);
            filter_combo(ui, "fstat", "Any status", &mut self.filter_status, STATUSES);
            if ui.button("Clear").clicked() {
                self.filter_text.clear();
                self.filter_cat.clear();
                self.filter_status.clear();
            }
        });
        ui.separator();

        let q = self.filter_text.to_lowercase();
        let mut rows: Vec<&Book> = self
            .books
            .iter()
            .filter(|b| self.filter_cat.is_empty() || b.category == self.filter_cat)
            .filter(|b| self.filter_status.is_empty() || b.status == self.filter_status)
            .filter(|b| {
                q.is_empty()
                    || b.title.to_lowercase().contains(&q)
                    || b.authors.to_lowercase().contains(&q)
                    || b.isbn.contains(&q)
                    || b.notes.to_lowercase().contains(&q)
                    || b.location.to_lowercase().contains(&q)
                    || b.loaned_to.to_lowercase().contains(&q)
            })
            .collect();
        let key = |b: &Book, col: usize| -> String {
            match col {
                0 => b.title.to_lowercase(),
                1 => b.authors.to_lowercase(),
                2 => b.category.clone(),
                3 => b.status.clone(),
                4 => b.year.clone(),
                _ => b.added_at.clone(),
            }
        };
        let (sc, asc) = self.sort;
        rows.sort_by(|x, y| {
            let o = key(x, sc).cmp(&key(y, sc));
            if asc { o } else { o.reverse() }
        });
        ui.weak(format!("{} of {} books  ·  click a column header to sort", rows.len(), self.books.len()));

        let sel_id = self.selected.as_ref().map(|b| b.id);
        let mut clicked: Option<i64> = None;
        let row_h = 22.0;
        let w = ui.available_width();
        let widths = [0.38, 0.22, 0.2, 0.1, 0.1];
        let names = ["Title", "Author", "Category", "Status", "Year"];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (i, (name, f)) in names.iter().zip(widths).enumerate() {
                let arrow = if sc == i { if asc { " ⏶" } else { " ⏷" } } else { "" };
                let btn = egui::Button::new(RichText::new(format!("{name}{arrow}")).strong()).frame(false);
                if ui.add_sized([w * f, row_h], btn).clicked() {
                    self.sort = if sc == i { (i, !asc) } else { (i, true) };
                }
            }
        });
        let font = egui::TextStyle::Body.resolve(ui.style());
        egui::ScrollArea::vertical().id_salt("lib").show_rows(ui, row_h, rows.len(), |ui, range| {
            for b in &rows[range] {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, row_h), egui::Sense::click());
                let vis = ui.visuals();
                if Some(b.id) == sel_id {
                    ui.painter().rect_filled(rect, 3.0, vis.selection.bg_fill);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 3.0, vis.widgets.hovered.weak_bg_fill);
                }
                let mut title = b.title.clone();
                if b.copies > 1 {
                    title.push_str(&format!("  x{}", b.copies));
                }
                if !b.loaned_to.is_empty() {
                    title.push_str(&format!("  (lent to {})", b.loaned_to));
                }
                let cells = [title, b.authors.clone(), b.category.clone(), b.status.clone(), b.year.clone()];
                let mut x = rect.left() + 6.0;
                for (text, f) in cells.iter().zip(widths) {
                    let cw = w * f;
                    let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(cw - 10.0, row_h));
                    ui.painter().with_clip_rect(cell).text(
                        egui::pos2(x, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        text,
                        font.clone(),
                        vis.text_color(),
                    );
                    x += cw;
                }
                if resp.clicked() {
                    clicked = Some(b.id);
                }
            }
        });
        if let Some(id) = clicked {
            self.selected = self.books.iter().find(|b| b.id == id).cloned();
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let Some(b) = self.selected.as_mut() else {
            ui.weak("Click a book to edit it.");
            return;
        };
        let mut save = false;
        let mut delete = false;
        let mut relookup = false;
        egui::ScrollArea::vertical().id_salt("editor").show(ui, |ui| {
            egui::Grid::new("edit").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                text_row(ui, "Title", &mut b.title);
                text_row(ui, "Author(s)", &mut b.authors);
                text_row(ui, "ISBN", &mut b.isbn);
                text_row(ui, "Publisher", &mut b.publisher);
                text_row(ui, "Year", &mut b.year);

                ui.label("Category");
                let before = b.category.clone();
                egui::ComboBox::from_id_salt("ecat").selected_text(&b.category).width(200.0).show_ui(ui, |ui| {
                    for c in CATEGORIES {
                        ui.selectable_value(&mut b.category, c.to_string(), *c);
                    }
                });
                if b.category != before {
                    b.category_locked = true;
                    b.category_reason = "picked by you".into();
                }
                ui.end_row();
                ui.label("");
                ui.horizontal(|ui| {
                    ui.weak(format!("Why: {}", b.category_reason));
                    if b.category_locked {
                        ui.checkbox(&mut b.category_locked, "keep my pick");
                    }
                });
                ui.end_row();

                ui.label("Status");
                egui::ComboBox::from_id_salt("estat").selected_text(&b.status).show_ui(ui, |ui| {
                    for s in STATUSES {
                        ui.selectable_value(&mut b.status, s.to_string(), *s);
                    }
                });
                ui.end_row();

                ui.label("Rating");
                ui.horizontal(|ui| {
                    for n in 1..=5 {
                        let star = if b.rating >= n { "★" } else { "☆" };
                        if ui.button(star).clicked() {
                            b.rating = if b.rating == n { 0 } else { n };
                        }
                    }
                });
                ui.end_row();

                text_row(ui, "Location", &mut b.location);
                text_row(ui, "Loaned to", &mut b.loaned_to);
                ui.label("Copies");
                ui.add(egui::DragValue::new(&mut b.copies).range(1..=99));
                ui.end_row();
                ui.label("Pages");
                let mut p = b.pages.unwrap_or(0);
                if ui.add(egui::DragValue::new(&mut p).range(0..=20000)).changed() {
                    b.pages = (p > 0).then_some(p);
                }
                ui.end_row();
            });
            ui.label("Notes");
            ui.add(egui::TextEdit::multiline(&mut b.notes).desired_rows(3).desired_width(f32::INFINITY));

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                save = ui.button("💾 Save").clicked();
                relookup = !b.isbn.is_empty()
                    && ui.button("Re-categorize").on_hover_text("Run the auto-categorizer again on this book").clicked();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    delete = ui.button(RichText::new("Delete").color(Color32::from_rgb(200, 50, 50))).clicked();
                });
            });

            ui.separator();
            ui.collapsing("Catalog details", |ui| {
                ui.weak(format!("LC class: {}", or_dash(&b.lcc)));
                ui.weak(format!("Dewey: {}", or_dash(&b.dewey)));
                ui.weak(format!("Subjects: {}", or_dash(&b.subjects)));
                ui.weak(format!("Source: {}  ·  Added {}", or_dash(&b.source), b.added_at));
                if !b.isbn.is_empty() {
                    ui.hyperlink_to("Open on Open Library", format!("https://openlibrary.org/isbn/{}", b.isbn));
                }
                if !b.cover_url.is_empty() {
                    ui.hyperlink_to("Cover image", &b.cover_url);
                }
            });
        });

        if relookup {
            let v = crate::categorize::categorize(&b.title, &b.subjects, &b.lcc, &b.dewey);
            b.category = v.category.into();
            b.category_reason = v.reason;
            b.category_locked = false;
            save = true;
        }
        if save {
            if !b.isbn.is_empty() {
                match isbn::normalize(&b.isbn) {
                    Ok(i) => b.isbn = i,
                    Err(e) => {
                        self.status_line = e;
                        return;
                    }
                }
            }
            let b = b.clone();
            self.status_line = match self.db.update(&b) {
                Ok(()) => format!("Saved \"{}\"", b.title),
                Err(e) => format!("Couldn't save: {e}"),
            };
            self.reload();
            return;
        }
        if delete {
            let (id, title) = (b.id, b.title.clone());
            if self.db.delete(id).is_ok() {
                self.flash(format!("Deleted \"{title}\""));
                self.selected = None;
                self.reload();
            }
        }
    }

    fn stats_tab(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        let total_books: i64 = self.books.iter().map(|b| b.copies.max(1)).sum::<i64>().max(0);
        let titles = self.books.len();
        let total_pages: i64 = self.books.iter().filter_map(|b| b.pages).sum();
        let read = self.books.iter().filter(|b| b.status == "Read").count();

        ui.horizontal(|ui| {
            stat(ui, &titles.to_string(), "titles");
            stat(ui, &total_books.to_string(), "books incl. copies");
            stat(ui, &format!("{total_pages}"), "pages");
            let pct = if titles > 0 { read as f64 / titles as f64 * 100.0 } else { 0.0 };
            stat(ui, &format!("{pct:.0}%"), "read");
        });
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Measure by:");
            ui.selectable_value(&mut self.measure, Measure::Books, "Number of books");
            ui.selectable_value(&mut self.measure, Measure::Pages, "Page count");
        });

        let value = |b: &Book| -> f64 {
            match self.measure {
                Measure::Books => 1.0,
                Measure::Pages => b.pages.unwrap_or(0) as f64,
            }
        };
        let by_cat = group(&self.books, |b| b.category.clone(), value);
        let by_status = group(&self.books, |b| b.status.clone(), value);
        let total: f64 = by_cat.iter().map(|(_, v)| v).sum();

        // the headline sentence
        if let Some((top, v)) = by_cat.first() {
            let unit = if self.measure == Measure::Books { "books" } else { "pages" };
            ui.label(
                RichText::new(format!(
                    "{top} is {:.1}% of your library ({} of {} {unit}).",
                    v / total * 100.0,
                    *v as i64,
                    total as i64
                ))
                .size(16.0),
            );
        }
        if self.measure == Measure::Pages {
            let missing = self.books.iter().filter(|b| b.pages.is_none()).count();
            if missing > 0 {
                ui.weak(format!("{missing} books have no page count and aren't counted here."));
            }
        }
        ui.add_space(8.0);

        let mut jump: Option<(String, String)> = None;
        egui::ScrollArea::vertical().id_salt("stats").show(ui, |ui| {
            ui.columns(2, |cols| {
                cols[0].strong("By category");
                if let Some(c) = chart_with_table(&mut cols[0], &by_cat, total, dark) {
                    jump = Some((c, String::new()));
                }
                cols[1].strong("By reading status");
                let st_total: f64 = by_status.iter().map(|(_, v)| v).sum();
                if let Some(st) = chart_with_table(&mut cols[1], &by_status, st_total, dark) {
                    jump = Some((String::new(), st));
                }

                cols[1].add_space(12.0);
                cols[1].strong("Most-collected authors");
                let authors = group(&self.books, |b| b.authors.split(';').next().unwrap_or("").trim().to_string(), |_| 1.0);
                for (a, n) in authors.iter().filter(|(a, _)| !a.is_empty()).take(8) {
                    cols[1].label(format!("{a} — {}", *n as i64));
                }

                let lent: Vec<&Book> = self.books.iter().filter(|b| !b.loaned_to.is_empty()).collect();
                if !lent.is_empty() {
                    cols[1].add_space(12.0);
                    cols[1].strong("Loaned out");
                    for b in lent {
                        cols[1].label(format!("{}  —  {}", b.title, b.loaned_to));
                    }
                }
            });
        });
        if let Some((cat, status)) = jump {
            self.filter_cat = cat;
            self.filter_status = status;
            self.filter_text.clear();
            self.tab = Tab::Library;
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.add_space(6.0);
        ui.label("Contact email sent to Open Library with each lookup (optional). Identified apps get 3 lookups/sec instead of 1.");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.contact).hint_text("you@example.com").desired_width(260.0));
            if ui.button("Save").clicked() {
                let _ = self.db.set_setting("contact", self.contact.trim());
                self.flash("Saved");
            }
        });
        ui.add_space(12.0);
        if ui
            .button("Re-categorize every book")
            .on_hover_text("Re-runs the auto-categorizer. Books you categorized by hand are left alone.")
            .clicked()
        {
            match self.db.recategorize_unlocked() {
                Ok(n) => self.flash(format!("Done — {n} books changed category")),
                Err(e) => self.flash(e.to_string()),
            }
            self.reload();
        }
        ui.add_space(6.0);
        if ui.button("Export to CSV").clicked() {
            let path = self.db_path.with_file_name("library-export.csv");
            match crate::db::export_csv(&self.books, &path) {
                Ok(()) => self.flash(format!("Exported to {}", path.display())),
                Err(e) => self.flash(format!("Export failed: {e}")),
            }
        }
        ui.add_space(12.0);
        ui.weak(format!("Your library file: {}", self.db_path.display()));
        ui.weak("Back it up by copying that file.");
    }
}

impl eframe::App for RookshelfApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("📚 Rookshelf");
                ui.add_space(12.0);
                for (t, name) in [
                    (Tab::Scan, "Scan & Add"),
                    (Tab::Library, "Library"),
                    (Tab::Stats, "Charts"),
                    (Tab::Settings, "Settings"),
                ] {
                    ui.selectable_value(&mut self.tab, t, name);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("{} books", self.books.len()));
                });
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.weak(if self.status_line.is_empty() { " " } else { &self.status_line });
        });
        if self.tab == Tab::Library {
            egui::Panel::right("editor").resizable(true).default_size(380.0).min_size(320.0).show(ui, |ui| {
                self.editor(ui);
            });
        }
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Scan => self.scan_tab(ui),
            Tab::Library => self.library_tab(ui),
            Tab::Stats => self.stats_tab(ui),
            Tab::Settings => self.settings_tab(ui),
        });
    }
}

// ---------- small helpers ----------

fn or_dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

fn text_row(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(240.0));
    ui.end_row();
}

fn filter_combo(ui: &mut egui::Ui, id: &str, all: &str, value: &mut String, options: &[&str]) {
    let shown = if value.is_empty() { all.to_string() } else { value.clone() };
    egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
        ui.selectable_value(value, String::new(), all);
        for o in options {
            ui.selectable_value(value, o.to_string(), *o);
        }
    });
}

fn stat(ui: &mut egui::Ui, big: &str, small: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(big).size(26.0).strong());
        ui.weak(small);
    });
    ui.add_space(24.0);
}

/// Sum a value per key, sorted biggest first (ties alphabetical, so colors stay put).
fn group(books: &[Book], key: impl Fn(&Book) -> String, value: impl Fn(&Book) -> f64) -> Vec<(String, f64)> {
    let mut m: BTreeMap<String, f64> = BTreeMap::new();
    for b in books {
        *m.entry(key(b)).or_default() += value(b);
    }
    let mut v: Vec<(String, f64)> = m.into_iter().filter(|(_, n)| *n > 0.0).collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v
}

/// Donut + a full percentage table beside it. Returns the row label if one
/// was clicked, so the caller can jump to the Library filtered to it.
fn chart_with_table(ui: &mut egui::Ui, data: &[(String, f64)], total: f64, dark: bool) -> Option<String> {
    let slices = pie::fold(data, dark);
    let center = format!("{}", total as i64);
    let hovered = pie::donut(ui, &slices, 240.0, &center);
    let folded = data.len() > 8;
    ui.add_space(6.0);
    let mut clicked = None;
    for (i, (label, v)) in data.iter().enumerate() {
        let in_other = folded && i >= 7;
        let swatch = if in_other { pie::other_color(dark) } else { pie::color(i, dark) };
        let hot = hovered.is_some_and(|h| h == i || (in_other && h == 7));
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2.0, swatch);
            let text = RichText::new(format!("{label}  {:.1}%  ({})", v / total * 100.0, *v as i64));
            let text = if hot { text.strong() } else { text };
            if ui.link(text).on_hover_text("Show these books").clicked() {
                clicked = Some(label.clone());
            }
        });
    }
    clicked
}
