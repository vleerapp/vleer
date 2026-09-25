use std::time::Duration;

use anyhow::{Result, anyhow};
use gpui::{App, Global};
use serde::Deserialize;
use ureq::Agent;

const API_ROOT: &str = "https://lrclib.net/api";
const USER_AGENT: &str = concat!(
    "vleer/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/vleerapp/vleer)"
);

const CLOSE_ENOUGH_SECS: u64 = 3;
const WAY_OFF_SECS: u64 = 10;
const SYNCED_SCORE: i64 = 200;
const ALBUM_SCORE: i64 = 15;

pub struct LrclibQuery<'a> {
    pub title: &'a str,
    pub artists: &'a [String],
    pub album: Option<&'a str>,
    pub duration: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LrclibHit {
    pub synced: Option<String>,
    pub plain: Option<String>,
    pub instrumental: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Track {
    id: Option<i64>,
    track_name: Option<String>,
    artist_name: Option<String>,
    album_name: Option<String>,
    duration: Option<f64>,
    instrumental: Option<bool>,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl Track {
    fn has_lyrics(&self) -> bool {
        non_empty(&self.synced_lyrics).is_some()
            || non_empty(&self.plain_lyrics).is_some()
            || self.instrumental == Some(true)
    }

    fn is_synced(&self) -> bool {
        non_empty(&self.synced_lyrics).is_some()
    }

    fn into_hit(self) -> LrclibHit {
        LrclibHit {
            synced: non_empty(&self.synced_lyrics).map(str::to_owned),
            plain: non_empty(&self.plain_lyrics).map(str::to_owned),
            instrumental: self.instrumental == Some(true),
        }
    }
}

fn non_empty(text: &Option<String>) -> Option<&str> {
    text.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

#[derive(Clone)]
pub struct LrclibClient {
    agent: Agent,
}

impl Global for LrclibClient {}

impl LrclibClient {
    pub fn init(cx: &mut App) {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .build()
            .into();
        cx.set_global(Self { agent });
    }

    pub fn find(&self, query: &LrclibQuery) -> Result<Vec<LrclibHit>> {
        let Some(artist) = query.artists.first() else {
            return Ok(Vec::new());
        };

        let exact = self.get(query, artist)?.filter(Track::has_lyrics);
        if exact.as_ref().is_some_and(Track::is_synced) {
            return Ok(exact.into_iter().map(Track::into_hit).collect());
        }

        let mut ranked = rank(
            query,
            self.search(&[("track_name", query.title), ("artist_name", artist)])?,
        );
        if !ranked.iter().any(Track::is_synced) {
            let text = format!("{} {}", query.title, artist);
            if let Ok(extra) = self.search(&[("q", text.as_str())]) {
                ranked.extend(rank(query, extra));
            }
        }
        ranked.extend(exact);

        let mut seen = std::collections::HashSet::new();
        ranked.retain(|track| track.id.is_none_or(|id| seen.insert(id)));
        ranked.sort_by_key(|track| std::cmp::Reverse(score(query, track)));
        Ok(ranked.into_iter().map(Track::into_hit).collect())
    }

    fn get(&self, query: &LrclibQuery, artist: &str) -> Result<Option<Track>> {
        let mut request = self
            .agent
            .get(format!("{API_ROOT}/get"))
            .header("User-Agent", USER_AGENT)
            .query("track_name", query.title)
            .query("artist_name", artist);
        if let Some(album) = query.album {
            request = request.query("album_name", album);
        }
        if query.duration > 0 {
            request = request.query("duration", query.duration.to_string());
        }

        match request.call() {
            Ok(mut response) => response
                .body_mut()
                .read_json()
                .map(Some)
                .map_err(|e| anyhow!("lrclib response parse failed: {e}")),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(anyhow!("lrclib request failed: {e}")),
        }
    }

    fn search(&self, params: &[(&str, &str)]) -> Result<Vec<Track>> {
        let mut request = self
            .agent
            .get(format!("{API_ROOT}/search"))
            .header("User-Agent", USER_AGENT);
        for (key, value) in params {
            request = request.query(*key, *value);
        }
        request
            .call()
            .map_err(|e| anyhow!("lrclib request failed: {e}"))?
            .body_mut()
            .read_json()
            .map_err(|e| anyhow!("lrclib response parse failed: {e}"))
    }
}

fn rank(query: &LrclibQuery, tracks: Vec<Track>) -> Vec<Track> {
    let mut ranked: Vec<Track> = tracks
        .into_iter()
        .filter(|track| track.has_lyrics() && matches(query, track))
        .collect();
    ranked.sort_by_key(|track| std::cmp::Reverse(score(query, track)));
    ranked
}

fn drift(query: &LrclibQuery, track: &Track) -> Option<u64> {
    let duration = track.duration?;
    (query.duration > 0).then(|| (duration.round() as i64).abs_diff(query.duration as i64))
}

fn matches(query: &LrclibQuery, track: &Track) -> bool {
    let title = track.track_name.as_deref().unwrap_or_default();
    let artist = track.artist_name.as_deref().unwrap_or_default();
    alike(title, query.title)
        && query.artists.iter().any(|a| artists_alike(artist, a))
        && drift(query, track).is_none_or(|d| d <= WAY_OFF_SECS)
}

fn score(query: &LrclibQuery, track: &Track) -> i64 {
    let mut score = 0;
    if track.is_synced() {
        score += SYNCED_SCORE;
    }
    if let Some(d) = drift(query, track)
        && d <= CLOSE_ENOUGH_SECS
    {
        score += 100 - d as i64 * 10;
    }
    if let (Some(wanted), Some(named)) = (query.album, track.album_name.as_deref())
        && alike(wanted, named)
    {
        score += ALBUM_SCORE;
    }
    score
}

fn undecorated(text: &str) -> String {
    let text = text.split(" - ").next().unwrap_or(text);
    let mut depth = 0usize;
    text.chars()
        .filter(|c| match c {
            '(' | '[' => {
                depth += 1;
                false
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn alike(left: &str, right: &str) -> bool {
    let (left, right) = (undecorated(left), undecorated(right));
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    let (short, long) = if left.len() <= right.len() {
        (&left, &right)
    } else {
        (&right, &left)
    };
    long.contains(short.as_str()) && short.len() * 2 >= long.len()
}

fn artists_alike(left: &str, right: &str) -> bool {
    let names = |s: &str| {
        let mut text = s.to_lowercase();
        for joiner in [" feat. ", " feat ", " ft. ", " featuring ", " with ", " x "] {
            text = text.replace(joiner, ",");
        }
        text.split([',', '&', ';'])
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
    };
    alike(left, right)
        || names(left)
            .iter()
            .any(|l| names(right).iter().any(|r| alike(l, r)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, secs: f64, synced: bool) -> Track {
        Track {
            id: None,
            track_name: Some(title.into()),
            artist_name: Some(artist.into()),
            album_name: None,
            duration: Some(secs),
            instrumental: Some(false),
            plain_lyrics: Some("la".into()),
            synced_lyrics: synced.then(|| "[00:01.00] la".into()),
        }
    }

    fn query(artists: &[String]) -> LrclibQuery<'_> {
        LrclibQuery {
            title: "Jaded",
            artists,
            album: None,
            duration: 263,
        }
    }

    #[test]
    fn prefers_synced_and_close_duration() {
        let artists = vec!["Spiritbox".to_string()];
        let picked = rank(
            &query(&artists),
            vec![
                track("Jaded", "Spiritbox", 263.0, false),
                track("Jaded", "Spiritbox", 265.0, true),
            ],
        );
        assert!(picked[0].synced_lyrics.is_some());
    }

    #[test]
    fn rejects_wrong_artist_and_far_duration() {
        let artists = vec!["Spiritbox".to_string()];
        let picked = rank(
            &query(&artists),
            vec![
                track("Jaded", "Someone Else", 263.0, true),
                track("Jaded", "Spiritbox", 300.0, true),
            ],
        );
        assert!(picked.is_empty());
    }

    #[test]
    fn decorations_are_ignored() {
        assert!(alike("Jaded (Remastered)", "Jaded"));
        assert!(alike("Jaded - Live", "jaded"));
        assert!(!alike("Jad", "Jaded Heart"));
    }

    #[test]
    fn feat_credits_split_into_separate_artists() {
        assert!(artists_alike("Luude Feat. Colin Hay", "Colin Hay"));
        assert!(artists_alike("Luude feat. Colin Hay", "Luude"));
        assert!(!artists_alike("Luude Feat. Colin Hay", "Someone Else"));
    }

    #[test]
    fn any_artist_of_a_collab_matches() {
        assert!(artists_alike("Spiritbox & Foo", "Spiritbox"));
    }
}
