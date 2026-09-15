use crate::data::models::Cuid;
use xxhash_rust::xxh3::Xxh3;

const SEPARATOR: [u8; 1] = [0x1f];
const TAIL_LEN: usize = 23;

pub fn fold_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub fn hash_parts(kind: &str, parts: &[&[u8]]) -> u128 {
    let mut hasher = Xxh3::new();
    hasher.update(kind.as_bytes());
    hasher.update(&SEPARATOR);
    for part in parts {
        hasher.update(part);
        hasher.update(&SEPARATOR);
    }
    hasher.digest128()
}

fn encode(mut hash: u128) -> String {
    const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = [0u8; TAIL_LEN + 1];
    out[0] = b'a' + (hash % 26) as u8;
    hash /= 26;
    for slot in out[1..].iter_mut().rev() {
        *slot = ALPHABET[(hash % 36) as usize];
        hash /= 36;
    }
    String::from_utf8(out.to_vec()).expect("id alphabet is ascii")
}

impl Cuid {
    pub fn derive(kind: &str, parts: &[&[u8]]) -> Self {
        Cuid::from(encode(hash_parts(kind, parts)))
    }

    pub fn for_song(audio_hash: &str, album_id: Option<&Cuid>) -> Self {
        Self::derive(
            "song",
            &[
                audio_hash.as_bytes(),
                album_id.map_or(b"" as &[u8], |id| id.as_str().as_bytes()),
            ],
        )
    }

    pub fn for_album(title: &str, artist: &str) -> Self {
        Self::derive(
            "album",
            &[fold_key(title).as_bytes(), fold_key(artist).as_bytes()],
        )
    }

    pub fn for_artist(name: &str) -> Self {
        Self::derive("artist", &[fold_key(name).as_bytes()])
    }

    pub fn for_genre(name: &str) -> Self {
        Self::derive("genre", &[fold_key(name).as_bytes()])
    }

    pub fn for_playlist_song(playlist_id: &Cuid, song_id: &Cuid) -> Self {
        Self::derive(
            "playlist_song",
            &[playlist_id.as_str().as_bytes(), song_id.as_str().as_bytes()],
        )
    }

    pub fn for_image(bytes: &[u8]) -> Self {
        Self::derive("image", &[bytes])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_cuid_shaped(id: &str) -> bool {
        id.len() == 24
            && id.as_bytes()[0].is_ascii_lowercase()
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
    }

    #[test]
    fn derived_ids_match_the_random_id_format() {
        assert!(is_cuid_shaped(Cuid::new().as_str()));
        for id in [
            Cuid::for_artist("Ellis"),
            Cuid::for_album("", ""),
            Cuid::for_image(&[0xff; 4096]),
            Cuid::derive("x", &[]),
        ] {
            assert!(is_cuid_shaped(id.as_str()), "{id}");
        }
        assert!(is_cuid_shaped(&encode(u128::MAX)));
        assert!(is_cuid_shaped(&encode(0)));
    }

    #[test]
    fn derived_ids_are_stable() {
        assert_eq!(Cuid::for_artist("Ellis"), Cuid::for_artist("Ellis"));
        assert_eq!(encode(0), "a00000000000000000000000");
        assert_eq!(
            hash_parts("artist", &[b"ellis"]),
            hash_parts("artist", &[b"ellis"])
        );
    }

    #[test]
    fn names_fold_case_and_surrounding_whitespace() {
        assert_eq!(Cuid::for_artist("Ellis"), Cuid::for_artist("  ellis "));
        assert_eq!(Cuid::for_genre("POP"), Cuid::for_genre("pop"));
        assert_eq!(
            Cuid::for_album("Night", "Ellis"),
            Cuid::for_album("night", "ELLIS")
        );
    }

    #[test]
    fn kinds_and_part_boundaries_are_separated() {
        assert_ne!(Cuid::for_artist("Pop"), Cuid::for_genre("Pop"));
        assert_ne!(Cuid::for_album("ab", "c"), Cuid::for_album("a", "bc"));
        assert_ne!(
            Cuid::for_album("Greatest Hits", "Queen"),
            Cuid::for_album("Greatest Hits", "ABBA")
        );
        assert_ne!(Cuid::for_song("00ff", None), Cuid::for_song("00aa", None));
        assert_ne!(
            Cuid::for_song("00ff", None),
            Cuid::for_song("00ff", Some(&Cuid::for_album("A", "B")))
        );
    }
}
