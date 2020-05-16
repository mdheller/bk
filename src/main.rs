use std::io::{stdout, Write};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    queue,
    style::{Attribute, Print},
    terminal,
};

mod epub;
use epub::Epub;

fn wrap(text: &str, width: usize) -> Vec<(usize, String)> {
    // XXX assumes a char is 1 unit wide
    let mut lines = Vec::new();

    let mut start = 0;
    let mut end = 0;
    let mut len = 0;
    let mut word = 0;
    let mut skip = 0;

    for (i, c) in text.char_indices() {
        len += 1;
        match c {
            ' ' => {
                end = i;
                skip = 1;
                word = 0;
            }
            '-' | '—' => {
                if len > width {
                    // `end = i + 1` will extend over the margin
                    word += 1;
                } else {
                    end = i + c.len_utf8();
                    skip = 0;
                    word = 0;
                }
            }
            _ => {
                word += 1;
            }
        }
        if c == '\n' {
            lines.push((start, String::from(&text[start..i])));
            start = i + 1;
            len = 0;
        } else if len > width {
            let line = if word == len {
                &text[start..i]
            } else {
                &text[start..end]
            };
            lines.push((start, String::from(line)));
            start = end + skip;
            len = word;
        }
    }

    lines
}

struct Position(String, usize, usize);

enum Direction {
    Forward,
    Backward,
}

trait View {
    fn run(&self, bk: &mut Bk, kc: KeyCode);
    fn render(&self, bk: &Bk) -> Vec<String>;
}

struct Help;
impl View for Help {
    fn run(&self, bk: &mut Bk, _: KeyCode) {
        bk.view = Some(&Page);
    }
    fn render(&self, _: &Bk) -> Vec<String> {
        let text = r#"
                   Esc q  Quit
                    F1 ?  Help
                       /  Search
                     Tab  Table of Contents

PageDown Right Space f l  Page Down
         PageUp Left b h  Page Up
                       d  Half Page Down
                       u  Half Page Up
                  Down j  Line Down
                    Up k  Line Up
                  Home g  Chapter Start
                   End G  Chapter End
                       [  Previous Chapter
                       ]  Next Chapter
                       n  Search Forward
                       N  Search Backward
                       '  Jump to previous position
                   "#;

        text.lines().map(String::from).collect()
    }
}

struct Nav;
impl View for Nav {
    fn run(&self, bk: &mut Bk, kc: KeyCode) {
        match kc {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('q') => {
                bk.view = Some(&Page);
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                bk.jump = (bk.chapter, bk.line);
                bk.chapter = bk.nav_idx;
                bk.line = 0;
                bk.view = Some(&Page);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if bk.nav_idx < bk.toc.len() - 1 {
                    bk.nav_idx += 1;
                    if bk.nav_idx == bk.nav_top + bk.rows {
                        bk.nav_top += 1;
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if bk.nav_idx > 0 {
                    if bk.nav_idx == bk.nav_top {
                        bk.nav_top -= 1;
                    }
                    bk.nav_idx -= 1;
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                bk.nav_idx = 0;
                bk.nav_top = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                bk.nav_idx = bk.toc.len() - 1;
                bk.nav_top = bk.toc.len().saturating_sub(bk.rows);
            }
            _ => (),
        }
    }
    fn render(&self, bk: &Bk) -> Vec<String> {
        let end = std::cmp::min(bk.nav_top + bk.rows, bk.toc.len());

        bk.toc[bk.nav_top..end]
            .iter()
            .enumerate()
            .map(|(i, label)| {
                if bk.nav_idx == bk.nav_top + i {
                    format!("{}{}{}", Attribute::Reverse, label, Attribute::Reset)
                } else {
                    label.to_string()
                }
            })
            .collect()
    }
}

struct Page;
impl View for Page {
    fn run(&self, bk: &mut Bk, kc: KeyCode) {
        match kc {
            KeyCode::Esc | KeyCode::Char('q') => bk.view = None,
            KeyCode::Tab => {
                bk.nav_idx = bk.chapter;
                bk.nav_top = bk.nav_idx.saturating_sub(bk.rows - 1);
                bk.view = Some(&Nav);
            }
            KeyCode::F(1) | KeyCode::Char('?') => bk.view = Some(&Help),
            KeyCode::Char('/') => {
                bk.search = String::new();
                bk.jump = (bk.chapter, bk.line);
                bk.view = Some(&Search);
            }
            KeyCode::Char('\'') => {
                let jump = (bk.chapter, bk.line);
                bk.jump();
                bk.jump = jump;
            }
            KeyCode::Char('N') => {
                bk.search(Direction::Backward);
            }
            KeyCode::Char('n') => {
                // FIXME
                bk.scroll_down(1);
                bk.search(Direction::Forward);
            }
            KeyCode::End | KeyCode::Char('G') => {
                bk.line = bk.lines().len().saturating_sub(bk.rows);
            }
            KeyCode::Home | KeyCode::Char('g') => bk.line = 0,
            KeyCode::Char('d') => {
                bk.scroll_down(bk.rows / 2);
            }
            KeyCode::Char('u') => {
                bk.scroll_up(bk.rows / 2);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                bk.scroll_up(1);
            }
            KeyCode::Left | KeyCode::PageUp | KeyCode::Char('b') | KeyCode::Char('h') => {
                bk.scroll_up(bk.rows);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                bk.scroll_down(1);
            }
            KeyCode::Right
            | KeyCode::PageDown
            | KeyCode::Char('f')
            | KeyCode::Char('l')
            | KeyCode::Char(' ') => {
                bk.scroll_down(bk.rows);
            }
            KeyCode::Char('[') => bk.prev_chapter(),
            KeyCode::Char(']') => bk.next_chapter(),
            _ => (),
        }
    }
    fn render(&self, bk: &Bk) -> Vec<String> {
        let end = std::cmp::min(bk.line + bk.rows, bk.lines().len());
        bk.lines()[bk.line..end].iter().map(String::from).collect()
    }
}

struct Search;
impl View for Search {
    fn run(&self, bk: &mut Bk, kc: KeyCode) {
        match kc {
            KeyCode::Esc => {
                bk.jump();
                bk.view = Some(&Page);
            }
            KeyCode::Enter => {
                bk.view = Some(&Page);
            }
            KeyCode::Backspace => {
                bk.search.pop();
                bk.jump();
                bk.search(Direction::Forward);
            }
            KeyCode::Char(c) => {
                bk.search.push(c);
                bk.search(Direction::Forward);
            }
            _ => (),
        }
    }
    fn render(&self, bk: &Bk) -> Vec<String> {
        let end = std::cmp::min(bk.line + bk.rows - 1, bk.lines().len());
        let mut buf = Vec::with_capacity(bk.rows);

        for line in bk.lines()[bk.line..end].iter() {
            if let Some(i) = line.find(&bk.search) {
                buf.push(format!(
                    "{}{}{}{}{}",
                    &line[..i],
                    Attribute::Reverse,
                    &bk.search,
                    Attribute::Reset,
                    &line[i + bk.search.len()..],
                ));
            } else {
                buf.push(String::from(line));
            }
        }

        for _ in buf.len()..bk.rows - 1 {
            buf.push(String::new());
        }
        buf.push(format!("/{}", bk.search));
        buf
    }
}

struct Chapter {
    text: String,
    lines: Vec<String>,
    bytes: Vec<usize>,
}

struct Bk<'a> {
    view: Option<&'a dyn View>,
    chapter: usize,
    cols: u16,
    // ideally we could use string slices as pointers, but self referential structs are hard
    chapters: Vec<Chapter>,
    nav_idx: usize,
    nav_top: usize,
    line: usize,
    jump: (usize, usize),
    rows: usize,
    toc: Vec<String>,
    max_width: u16,
    search: String,
}

impl Bk<'_> {
    fn new(epub: Epub, line: &Position, max_width: u16) -> Self {
        let (cols, rows) = terminal::size().unwrap();
        let width = std::cmp::min(cols, max_width) as usize;
        let mut chapters = Vec::with_capacity(epub.chapters.len());
        for text in epub.chapters {
            let wrap = wrap(&text, width);
            let mut lines = Vec::with_capacity(wrap.len());
            let mut bytes = Vec::with_capacity(wrap.len());

            for (byte, line) in wrap {
                lines.push(line);
                bytes.push(byte);
            }
            chapters.push(Chapter { text, lines, bytes });
        }

        Bk {
            jump: (0, 0),
            view: Some(&Page),
            chapter: line.1,
            nav_idx: 0,
            nav_top: 0,
            toc: epub.nav,
            chapters,
            line: line.2,
            max_width,
            cols,
            rows: rows as usize,
            search: String::new(),
        }
    }
    fn jump(&mut self) {
        let (c, l) = self.jump;
        self.chapter = c;
        self.line = l;
    }
    fn lines(&self) -> &Vec<String> {
        &self.chapters[self.chapter].lines
    }
    fn run(&mut self) -> crossterm::Result<()> {
        let mut stdout = stdout();
        queue!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;
        terminal::enable_raw_mode()?;

        while let Some(view) = self.view {
            let pad = self.cols.saturating_sub(self.max_width) / 2;

            queue!(stdout, terminal::Clear(terminal::ClearType::All))?;
            for (i, line) in view.render(self).iter().enumerate() {
                queue!(stdout, cursor::MoveTo(pad, i as u16), Print(line))?;
            }
            stdout.flush().unwrap();

            match event::read()? {
                Event::Key(e) => view.run(self, e.code),
                // TODO
                Event::Resize(_, _) => (),
                Event::Mouse(_) => (),
            }
        }

        queue!(stdout, terminal::LeaveAlternateScreen, cursor::Show)?;
        terminal::disable_raw_mode()
    }
    fn next_chapter(&mut self) {
        if self.chapter < self.toc.len() - 1 {
            self.chapter += 1;
            self.line = 0;
        }
    }
    fn prev_chapter(&mut self) {
        if self.chapter > 0 {
            self.chapter -= 1;
            self.line = 0;
        }
    }
    fn scroll_down(&mut self, n: usize) {
        if self.line + self.rows < self.lines().len() {
            self.line += n;
        } else {
            self.next_chapter();
        }
    }
    fn scroll_up(&mut self, n: usize) {
        if self.line > 0 {
            self.line = self.line.saturating_sub(n);
        } else {
            self.prev_chapter();
            self.line = self.lines().len().saturating_sub(self.rows);
        }
    }
    fn search(&mut self, dir: Direction) {
        // https://doc.rust-lang.org/std/vec/struct.Vec.html#method.binary_search
        // If the value is not found then Result::Err is returned, containing the index where a matching element
        // could be inserted while maintaining sorted order.
        let head = (self.chapter, self.chapters[self.chapter].bytes[self.line]);
        match dir {
            Direction::Forward => {
                let rest = (self.chapter + 1..self.chapters.len() - 1).map(|n| (n, 0));
                for (c, byte) in std::iter::once(head).chain(rest) {
                    if let Some(index) = self.chapters[c].text[byte..].find(&self.search) {
                        self.line = match self.chapters[c].bytes.binary_search(&(byte + index)) {
                            Ok(n) => n,
                            Err(n) => n - 1,
                        };
                        self.chapter = c;
                        return;
                    }
                }
                self.jump();
            }
            Direction::Backward => {
                let rest = (0..self.chapter - 1)
                    .rev()
                    .map(|c| (c, self.chapters[c].text.len()));
                for (c, byte) in std::iter::once(head).chain(rest) {
                    if let Some(index) = self.chapters[c].text[..byte].rfind(&self.search) {
                        self.line = match self.chapters[c].bytes.binary_search(&index) {
                            Ok(n) => n,
                            Err(n) => n - 1,
                        };
                        self.chapter = c;
                        return;
                    }
                }
                self.jump();
            }
        }
    }
}

fn restore() -> Option<Position> {
    let path = std::env::args().nth(1);
    let save_path = format!("{}/.local/share/bk", std::env::var("HOME").unwrap());
    let save = std::fs::read_to_string(save_path);

    let get_save = |s: String| {
        let mut lines = s.lines();
        Position(
            lines.next().unwrap().to_string(),
            lines.next().unwrap().parse::<usize>().unwrap(),
            lines.next().unwrap().parse::<usize>().unwrap(),
        )
    };

    match (save, path) {
        (Err(_), None) => None,
        (Err(_), Some(path)) => Some(Position(path, 0, 0)),
        (Ok(save), None) => Some(get_save(save)),
        (Ok(save), Some(path)) => {
            let save = get_save(save);
            if save.0 == path {
                Some(save)
            } else {
                Some(Position(path, 0, 0))
            }
        }
    }
}

fn main() {
    let line = restore().unwrap_or_else(|| {
        println!("usage: bk path");
        std::process::exit(1);
    });

    let epub = Epub::new(&line.0).unwrap_or_else(|e| {
        println!("error reading epub: {}", e);
        std::process::exit(1);
    });

    let mut bk = Bk::new(epub, &line, 75);
    // crossterm really shouldn't error
    bk.run().unwrap();

    std::fs::write(
        format!("{}/.local/share/bk", std::env::var("HOME").unwrap()),
        format!("{}\n{}\n{}", line.0, bk.chapter, bk.line),
    )
    .unwrap_or_else(|e| {
        println!("error saving position: {}", e);
        std::process::exit(1);
    });
}
