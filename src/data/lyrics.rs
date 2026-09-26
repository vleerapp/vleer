use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use tracing::debug;

use crate::data::db::repo::Database;
use crate::data::models::{Song, StoredLyrics};
use crate::services::lrclib::{LrclibClient, LrclibHit, LrclibQuery};

const RETRY_DAYS: u32 = 7;
pub const MIN_GAP: f32 = 4.0;
const SUNG_BASE: f32 = 2.0;
const SUNG_PER_CHAR: f32 = 0.12;
const INFERRED_GAP: f32 = 5.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub at: f32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub at: f32,
    pub text: String,
    pub secondary: Option<String>,
    pub words: Vec<Word>,
    pub end: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lyrics {
    Synced(Vec<Line>),
    Plain(Vec<String>),
    Instrumental,
}

impl Lyrics {
    fn build(content: &str, synced: bool, instrumental: bool) -> Self {
        if instrumental {
            return Self::Instrumental;
        }
        if synced {
            return Self::Synced(parse_lrc(content));
        }
        Self::Plain(content.lines().map(|l| l.trim_end().to_owned()).collect())
    }

    fn from_stored(stored: &StoredLyrics) -> Option<Self> {
        (stored.source != "none")
            .then(|| Self::build(&stored.content, stored.synced, stored.instrumental))
    }
}

pub fn is_gap(lines: &[Line], index: usize) -> bool {
    lines[index].text.is_empty()
        && lines
            .get(index + 1)
            .is_some_and(|next| next.at - lines[index].at >= MIN_GAP)
}

pub fn active_line(lines: &[Line], at: f32) -> Option<usize> {
    active_line_by(lines, at, |i| lines[i].at)
}

pub fn active_line_by(lines: &[Line], at: f32, start: impl Fn(usize) -> f32) -> Option<usize> {
    let index = (0..lines.len()).rev().find(|&i| start(i) <= at)?;
    if lines[index].text.is_empty() && !is_gap(lines, index) {
        index.checked_sub(1)
    } else {
        Some(index)
    }
}

fn stamp(tag: &str) -> Option<f32> {
    let (minutes, rest) = tag.split_once(':')?;
    if minutes.is_empty() || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (seconds, fraction) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let minutes: f32 = minutes.parse().ok()?;
    let seconds: f32 = seconds.parse().ok()?;
    let fraction: f32 = if fraction.is_empty() {
        0.0
    } else {
        format!("0.{fraction}").parse().ok()?
    };
    Some(minutes * 60.0 + seconds + fraction)
}

struct Timed {
    text: String,
    words: Vec<Word>,
    end: Option<f32>,
}

fn parse_words(rest: &str, line_at: f32) -> Timed {
    let mut pieces: Vec<(Option<f32>, String)> = vec![(None, String::new())];
    let mut cursor = rest;
    while let Some(start) = cursor.find('<') {
        let (before, after) = cursor.split_at(start);
        let stamped = after
            .find('>')
            .and_then(|close| stamp(&after[1..close]).map(|at| (at, close)));
        let Some(current) = pieces.last_mut() else {
            break;
        };
        current.1.push_str(before);
        match stamped {
            Some((at, close)) => {
                pieces.push((Some(at), String::new()));
                cursor = &after[close + 1..];
            }
            None => {
                current.1.push('<');
                cursor = &after[1..];
            }
        }
    }
    if let Some(current) = pieces.last_mut() {
        current.1.push_str(cursor);
    }

    if pieces.len() == 1 {
        return Timed {
            text: pieces.remove(0).1.trim().to_owned(),
            words: Vec::new(),
            end: None,
        };
    }

    let last = pieces.len() - 1;
    let mut words: Vec<Word> = Vec::new();
    let mut end = None;
    for (index, (at, text)) in pieces.into_iter().enumerate() {
        let at = at.unwrap_or(line_at);
        if text.trim().is_empty() {
            if let Some(previous) = words.last_mut() {
                previous.text.push_str(&text);
            }
            if index == last {
                end = Some(at);
            }
            continue;
        }
        words.push(Word { at, text });
    }
    if let Some(first) = words.first_mut() {
        first.text = first.text.trim_start().to_owned();
    }
    if let Some(final_word) = words.last_mut() {
        final_word.text = final_word.text.trim_end().to_owned();
    }
    let text = words.iter().map(|w| w.text.as_str()).collect::<String>();
    Timed { text, words, end }
}

pub fn parse_lrc(text: &str) -> Vec<Line> {
    let mut offset = 0.0f32;
    let mut lines: Vec<Line> = Vec::new();

    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(body) = rest.strip_prefix('[') {
            let Some((tag, tail)) = body.split_once(']') else {
                break;
            };
            if let Some(at) = stamp(tag) {
                stamps.push(at);
            } else if let Some(value) = tag.strip_prefix("offset:") {
                offset = value.trim().parse::<f32>().unwrap_or(0.0) / 1000.0;
            } else if !tag.contains(':') {
                break;
            }
            rest = tail.trim_start();
        }
        if stamps.is_empty() {
            continue;
        }
        let single = stamps.len() == 1;
        let timed = parse_words(rest, stamps[0]);
        lines.extend(stamps.into_iter().map(|at| Line {
            at,
            text: timed.text.clone(),
            secondary: None,
            words: if single {
                timed.words.clone()
            } else {
                Vec::new()
            },
            end: if single { timed.end } else { None },
        }));
    }

    let shift = |at: f32| (at - offset).max(0.0);
    for line in &mut lines {
        line.at = shift(line.at);
        line.end = line.end.map(shift);
        for word in &mut line.words {
            word.at = shift(word.at);
        }
    }
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));

    let mut merged: Vec<Line> = Vec::with_capacity(lines.len());
    for line in lines {
        match merged.last_mut() {
            Some(last)
                if (last.at - line.at).abs() < 0.001
                    && !last.text.is_empty()
                    && !line.text.is_empty() =>
            {
                match &mut last.secondary {
                    Some(secondary) => {
                        secondary.push('\n');
                        secondary.push_str(&line.text);
                    }
                    None => last.secondary = Some(line.text),
                }
            }
            _ => merged.push(line),
        }
    }

    let mut cleaned: Vec<Line> = Vec::with_capacity(merged.len());
    for line in merged {
        let blank = line.text.is_empty();
        let prev_blank = cleaned.last().is_none_or(|l| l.text.is_empty());
        if blank && prev_blank {
            continue;
        }
        cleaned.push(line);
    }
    let mut cleaned = insert_inferred_gaps(cleaned);
    if cleaned.first().is_some_and(|l| l.at >= MIN_GAP) {
        cleaned.insert(
            0,
            Line {
                at: 0.0,
                text: String::new(),
                secondary: None,
                words: Vec::new(),
                end: None,
            },
        );
    }
    cleaned
}

fn insert_inferred_gaps(lines: Vec<Line>) -> Vec<Line> {
    let mut out = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        out.push(line.clone());
        let Some(next) = lines.get(i + 1) else {
            continue;
        };
        if line.text.is_empty() || next.text.is_empty() {
            continue;
        }
        let ends = line
            .end
            .unwrap_or(line.at + SUNG_BASE + line.text.chars().count() as f32 * SUNG_PER_CHAR);
        if next.at - ends >= INFERRED_GAP {
            out.push(Line {
                at: ends,
                text: String::new(),
                secondary: None,
                words: Vec::new(),
                end: None,
            });
        }
    }
    out
}

pub fn read_embedded(path: &Path) -> Option<String> {
    let tagged = Probe::open(path)
        .ok()?
        .guess_file_type()
        .ok()?
        .options(
            ParseOptions::new()
                .read_cover_art(false)
                .read_properties(false),
        )
        .read()
        .ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    tag.get_string(ItemKey::Lyrics)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

struct Found {
    source: &'static str,
    content: String,
    synced: bool,
    instrumental: bool,
}

impl Found {
    fn embedded(content: String) -> Self {
        let synced = !parse_lrc(&content).is_empty();
        Self {
            source: "embedded",
            content,
            synced,
            instrumental: false,
        }
    }

    fn online(hits: Vec<LrclibHit>) -> Option<Self> {
        let source = "lrclib";
        let synced = hits.iter().find_map(|hit| {
            let content = hit.synced.as_ref()?;
            (!parse_lrc(content).is_empty()).then(|| content.clone())
        });
        if let Some(content) = synced {
            return Some(Self {
                source,
                content,
                synced: true,
                instrumental: false,
            });
        }
        if let Some(content) = hits.iter().find_map(|hit| hit.plain.clone()) {
            return Some(Self {
                source,
                content,
                synced: false,
                instrumental: false,
            });
        }
        hits.iter().any(|hit| hit.instrumental).then(|| Self {
            source,
            content: String::new(),
            synced: false,
            instrumental: true,
        })
    }

    fn lyrics(&self) -> Lyrics {
        Lyrics::build(&self.content, self.synced, self.instrumental)
    }
}

fn store(db: &Database, song: &Song, found: Option<Found>) -> Option<Lyrics> {
    let result = match &found {
        Some(f) => db.upsert_lyrics(&song.id, f.source, f.synced, f.instrumental, &f.content),
        None => db.upsert_lyrics(&song.id, "none", false, false, ""),
    };
    if let Err(e) = result {
        debug!("failed to cache lyrics: {e}");
    }
    found.map(|f| f.lyrics())
}

pub fn resolve(db: &Database, client: &LrclibClient, song: &Song) -> Option<Lyrics> {
    if let Ok(Some(stored)) = db.get_lyrics(&song.id, RETRY_DAYS)
        && (stored.synced || stored.instrumental || !stored.stale)
    {
        return Lyrics::from_stored(&stored);
    }

    let embedded = read_embedded(Path::new(&song.file_path)).map(Found::embedded);
    if embedded.as_ref().is_some_and(|f| f.synced) {
        return store(db, song, embedded);
    }

    let album = song
        .album_id
        .as_ref()
        .and_then(|id| db.get_album(id).ok().flatten())
        .map(|a| a.title);
    let query = LrclibQuery {
        title: &song.title,
        artists: &song.artists,
        album: album.as_deref(),
        duration: song.duration.max(0) as u32,
    };

    let online = match client.find(&query) {
        Ok(hits) => Found::online(hits),
        Err(e) => {
            debug!("lrclib lookup failed: {e}");
            return embedded.map(|f| f.lyrics());
        }
    };

    let chosen = match (embedded, online) {
        (_, Some(online)) if online.synced => Some(online),
        (Some(embedded), _) => Some(embedded),
        (None, online) => online,
    };
    store(db, song, chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stamps_and_ignores_metadata() {
        let lines = parse_lrc("[ar:Someone]\n[00:01.50] one\n[01:02.5]two\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].at, 1.5);
        assert!(lines[1].text.is_empty());
        assert_eq!(lines[2].at, 62.5);
        assert_eq!(lines[2].text, "two");
    }

    #[test]
    fn repeated_stamps_expand_and_sort() {
        let lines = parse_lrc("[00:10.00][00:02.00]chorus\n[00:05.00]verse");
        let ats: Vec<f32> = lines.iter().map(|l| l.at).collect();
        assert_eq!(ats, vec![2.0, 5.0, 10.0]);
    }

    #[test]
    fn applies_offset_and_strips_word_stamps() {
        let lines = parse_lrc("[offset:+500]\n[00:02.00]<00:02.00>hel<00:02.40>lo");
        assert_eq!(lines[0].at, 1.5);
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[0].words.len(), 2);
        assert!((lines[0].words[1].at - 1.9).abs() < 0.001);
    }

    #[test]
    fn enhanced_lrc_keeps_word_timing_and_end() {
        let lines = parse_lrc(
            "[00:01.00]<00:01.00>Hel<00:01.40>lo <00:01.80>world<00:02.50>\n[00:05.00]next",
        );
        assert_eq!(lines[0].text, "Hello world");
        let words: Vec<(&str, f32)> = lines[0]
            .words
            .iter()
            .map(|w| (w.text.as_str(), w.at))
            .collect();
        assert_eq!(words, [("Hel", 1.0), ("lo ", 1.4), ("world", 1.8)]);
        assert_eq!(lines[0].end, Some(2.5));
        assert!(lines[1].words.is_empty());
    }

    #[test]
    fn collapses_blank_lines() {
        let lines = parse_lrc("[00:01.00]\n[00:02.00]a\n[00:03.00]\n[00:04.00]\n[00:05.00]b");
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "", "b"]);
    }

    #[test]
    fn plain_text_is_not_synced() {
        assert!(parse_lrc("just some\nplain lyrics").is_empty());
    }

    #[test]
    fn finds_active_line() {
        let lines = parse_lrc("[00:01.00]a\n[00:05.00]b");
        assert_eq!(active_line(&lines, 0.5), None);
        assert_eq!(active_line(&lines, 3.0), Some(0));
        assert_eq!(active_line(&lines, 9.0), Some(1));
    }

    #[test]
    fn long_intro_gets_a_gap_line() {
        let lines = parse_lrc("[00:10.00]a");
        assert_eq!(lines.len(), 2);
        assert!(is_gap(&lines, 0));
        assert_eq!(active_line(&lines, 2.0), Some(0));
        assert_eq!(active_line(&lines, 10.5), Some(1));
    }

    #[test]
    fn short_blank_keeps_previous_line_active() {
        let lines = parse_lrc("[00:01.00]a\n[00:03.00]\n[00:05.00]b");
        assert!(!is_gap(&lines, 1));
        assert_eq!(active_line(&lines, 3.5), Some(0));
    }

    #[test]
    fn long_silence_without_a_marker_becomes_a_gap() {
        let lines = parse_lrc("[00:02.00]hello\n[00:40.00]world");
        assert_eq!(lines.len(), 3);
        assert!(lines[1].text.is_empty() && is_gap(&lines, 1));
        assert_eq!(active_line(&lines, 20.0), Some(1));
        assert_eq!(active_line(&lines, 3.0), Some(0));
    }

    #[test]
    fn close_lines_get_no_inferred_gap() {
        let lines = parse_lrc("[00:02.00]hello there friend\n[00:09.00]world");
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn same_stamp_lines_merge_into_a_secondary() {
        let lines = parse_lrc(
            "[00:01.00]いつか\n[00:01.00]itsuka\n[00:03.50]愛で\n[00:03.50]ai de\n[00:05.00]end",
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "いつか");
        assert_eq!(lines[0].secondary.as_deref(), Some("itsuka"));
        assert_eq!(lines[1].secondary.as_deref(), Some("ai de"));
        assert_eq!(lines[2].secondary, None);
        assert_eq!(active_line(&lines, 2.0), Some(0));
    }

    #[test]
    fn long_blank_is_a_gap() {
        let lines = parse_lrc("[00:01.00]a\n[00:03.00]\n[00:12.00]b");
        assert!(is_gap(&lines, 1));
        assert_eq!(active_line(&lines, 5.0), Some(1));
    }
}
