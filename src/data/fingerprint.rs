use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use xxhash_rust::xxh3::Xxh3;

const HEAD_WINDOW: usize = 64 * 1024;
const OGG_HEADER_SCAN_LIMIT: u64 = 64 * 1024 * 1024;
const OGG_MAX_PAGE_HEADER: usize = 27 + 255;

pub fn audio_fingerprint<R: Read + Seek>(reader: &mut R) -> io::Result<String> {
    let len = reader.seek(SeekFrom::End(0))?;
    let hash = if let Some(hash) = flac_streaminfo_hash(reader, len)? {
        hash
    } else if let Some(start) = ogg_audio_start(reader, len)? {
        hash_ogg(reader, len, start)?
    } else {
        let span = audio_span(reader, len)?;
        hash_span(reader, span)?
    };
    Ok(format!("{hash:032x}"))
}

fn read_at<R: Read + Seek>(reader: &mut R, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn header<R: Read + Seek, const N: usize>(
    reader: &mut R,
    offset: u64,
    limit: u64,
) -> io::Result<Option<[u8; N]>> {
    if offset.saturating_add(N as u64) > limit {
        return Ok(None);
    }
    let mut buf = [0u8; N];
    Ok((read_at(reader, offset, &mut buf)? == N).then_some(buf))
}

fn audio_span<R: Read + Seek>(reader: &mut R, len: u64) -> io::Result<Range<u64>> {
    let start = skip_id3v2(reader, len)?;
    let magic: [u8; 12] = header(reader, start, len)?.unwrap_or_default();
    let span = if magic.starts_with(b"fLaC") {
        flac_span(reader, start, len)?
    } else if &magic[..4] == b"RIFF" && &magic[8..12] == b"WAVE" {
        riff_span(reader, start, len)?
    } else if &magic[..4] == b"FORM" && matches!(&magic[8..12], b"AIFF" | b"AIFC") {
        aiff_span(reader, start, len)?
    } else if &magic[4..8] == b"ftyp" {
        mp4_span(reader, start, len)?
    } else {
        None
    };
    match span {
        Some(span) => Ok(span),
        None => {
            let start = start + mp3_info_frame_len(reader, start, len)?;
            Ok(start..trim_trailing_tags(reader, start, len)?)
        }
    }
}

fn mp3_info_frame_len<R: Read + Seek>(reader: &mut R, start: u64, len: u64) -> io::Result<u64> {
    const MPEG1_KBPS: [u64; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const MPEG2_KBPS: [u64; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    const SAMPLE_RATES: [u64; 3] = [44100, 48000, 32000];

    let Some(frame) = header::<_, 64>(reader, start, len)? else {
        return Ok(0);
    };
    let version = (frame[1] >> 3) & 3;
    let layer = (frame[1] >> 1) & 3;
    let bitrate = usize::from(frame[2] >> 4);
    let rate = usize::from((frame[2] >> 2) & 3);
    if frame[0] != 0xff
        || frame[1] & 0xe0 != 0xe0
        || version == 1
        || layer != 1
        || bitrate == 0
        || bitrate == 15
        || rate == 3
    {
        return Ok(0);
    }
    let is_info = frame[4..]
        .windows(4)
        .any(|w| matches!(w, b"Xing" | b"Info" | b"VBRI"));
    if !is_info {
        return Ok(0);
    }
    let (kbps, samples_factor, rate_divisor) = match version {
        3 => (MPEG1_KBPS[bitrate], 144_000, 1),
        2 => (MPEG2_KBPS[bitrate], 72_000, 2),
        _ => (MPEG2_KBPS[bitrate], 72_000, 4),
    };
    let sample_rate = SAMPLE_RATES[rate] / rate_divisor;
    let padding = u64::from((frame[2] >> 1) & 1);
    Ok(samples_factor * kbps / sample_rate + padding)
}

fn flac_streaminfo_hash<R: Read + Seek>(reader: &mut R, len: u64) -> io::Result<Option<u128>> {
    let start = skip_id3v2(reader, len)?;
    let Some(block) = header::<_, 42>(reader, start, len)? else {
        return Ok(None);
    };
    let streaminfo = &block[8..];
    if &block[..4] != b"fLaC" || block[4] & 0x7f != 0 || streaminfo[18..].iter().all(|&b| b == 0) {
        return Ok(None);
    }
    let mut hasher = Xxh3::new();
    hasher.update(b"flac-md5");
    hasher.update(&streaminfo[10..]);
    Ok(Some(hasher.digest128()))
}

fn skip_id3v2<R: Read + Seek>(reader: &mut R, len: u64) -> io::Result<u64> {
    let mut pos = 0;
    while let Some(h) = header::<_, 10>(reader, pos, len)? {
        if &h[..3] != b"ID3" || h[3] == 0xff || h[6..10].iter().any(|b| b & 0x80 != 0) {
            break;
        }
        let size = h[6..10]
            .iter()
            .fold(0u64, |acc, &b| (acc << 7) | u64::from(b));
        let footer = if h[5] & 0x10 != 0 { 10 } else { 0 };
        let next = pos + 10 + size + footer;
        if next > len {
            break;
        }
        pos = next;
    }
    Ok(pos)
}

fn trim_trailing_tags<R: Read + Seek>(reader: &mut R, start: u64, mut end: u64) -> io::Result<u64> {
    loop {
        if end - start >= 128 && header::<_, 3>(reader, end - 128, end)? == Some(*b"TAG") {
            end -= 128;
            continue;
        }
        if end - start >= 32
            && let Some(footer) = header::<_, 32>(reader, end - 32, end)?
            && &footer[..8] == b"APETAGEX"
        {
            let size = u64::from(u32::from_le_bytes(footer[12..16].try_into().unwrap()));
            let flags = u32::from_le_bytes(footer[20..24].try_into().unwrap());
            let total = size + if flags & 0x8000_0000 != 0 { 32 } else { 0 };
            if total >= 32 && total <= end - start {
                end -= total;
                continue;
            }
        }
        return Ok(end);
    }
}

fn flac_span<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    len: u64,
) -> io::Result<Option<Range<u64>>> {
    let mut pos = start + 4;
    loop {
        let Some(h) = header::<_, 4>(reader, pos, len)? else {
            return Ok(None);
        };
        pos += 4 + u64::from(u32::from_be_bytes([0, h[1], h[2], h[3]]));
        if pos > len {
            return Ok(None);
        }
        if h[0] & 0x80 != 0 {
            break;
        }
    }
    Ok(Some(pos..trim_trailing_tags(reader, pos, len)?))
}

fn riff_span<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    len: u64,
) -> io::Result<Option<Range<u64>>> {
    let mut pos = start + 12;
    while let Some(h) = header::<_, 8>(reader, pos, len)? {
        let size = u32::from_le_bytes(h[4..8].try_into().unwrap());
        let body = pos + 8;
        if &h[..4] == b"data" {
            let end = if size == u32::MAX {
                len
            } else {
                (body + u64::from(size)).min(len)
            };
            return Ok(Some(body..end));
        }
        pos = body + u64::from(size) + u64::from(size & 1);
    }
    Ok(None)
}

fn aiff_span<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    len: u64,
) -> io::Result<Option<Range<u64>>> {
    let mut pos = start + 12;
    while let Some(h) = header::<_, 8>(reader, pos, len)? {
        let size = u64::from(u32::from_be_bytes(h[4..8].try_into().unwrap()));
        let body = pos + 8;
        if &h[..4] == b"SSND" {
            let Some(offset) = header::<_, 4>(reader, body, len)? else {
                return Ok(None);
            };
            let audio = body + 8 + u64::from(u32::from_be_bytes(offset));
            let end = (body + size).min(len);
            return Ok((audio <= end).then_some(audio..end));
        }
        pos = body + size + (size & 1);
    }
    Ok(None)
}

fn mp4_span<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    len: u64,
) -> io::Result<Option<Range<u64>>> {
    let mut pos = start;
    while let Some(h) = header::<_, 8>(reader, pos, len)? {
        let mut size = u64::from(u32::from_be_bytes(h[..4].try_into().unwrap()));
        let mut head = 8;
        if size == 1 {
            let Some(large) = header::<_, 8>(reader, pos + 8, len)? else {
                break;
            };
            size = u64::from_be_bytes(large);
            head = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < head {
            break;
        }
        if &h[4..8] == b"mdat" {
            return Ok(Some(pos + head..pos.saturating_add(size).min(len)));
        }
        pos = pos.saturating_add(size);
    }
    Ok(None)
}

fn hash_span<R: Read + Seek>(reader: &mut R, span: Range<u64>) -> io::Result<u128> {
    let span_len = span.end - span.start;
    let mut hasher = Xxh3::new();
    hasher.update(b"raw");
    hasher.update(&span_len.to_le_bytes());
    let mut buf = vec![0u8; span_len.min(HEAD_WINDOW as u64) as usize];
    let n = read_at(reader, span.start, &mut buf)?;
    hasher.update(&buf[..n]);
    Ok(hasher.digest128())
}

struct OggPage {
    granule: u64,
    header_len: usize,
    body_len: usize,
}

fn parse_ogg_page(buf: &[u8]) -> Option<OggPage> {
    if buf.len() < 27 || &buf[..4] != b"OggS" || buf[4] != 0 {
        return None;
    }
    let segments = usize::from(buf[26]);
    let table = buf.get(27..27 + segments)?;
    Some(OggPage {
        granule: u64::from_le_bytes(buf[6..14].try_into().unwrap()),
        header_len: 27 + segments,
        body_len: table.iter().map(|&b| usize::from(b)).sum(),
    })
}

fn read_ogg_page<R: Read + Seek>(
    reader: &mut R,
    pos: u64,
    len: u64,
) -> io::Result<Option<OggPage>> {
    let mut buf = [0u8; OGG_MAX_PAGE_HEADER];
    let n = read_at(reader, pos, &mut buf)?;
    Ok(parse_ogg_page(&buf[..n])
        .filter(|page| pos + (page.header_len + page.body_len) as u64 <= len))
}

fn ogg_audio_start<R: Read + Seek>(reader: &mut R, len: u64) -> io::Result<Option<u64>> {
    let mut pos = 0;
    let mut headers_end = None;
    while pos < len.min(OGG_HEADER_SCAN_LIMIT) {
        let Some(page) = read_ogg_page(reader, pos, len)? else {
            break;
        };
        let end = pos + (page.header_len + page.body_len) as u64;
        match page.granule {
            0 => headers_end = Some(end),
            u64::MAX => {}
            _ => return Ok(headers_end),
        }
        pos = end;
    }
    Ok(headers_end)
}

fn hash_ogg<R: Read + Seek>(reader: &mut R, len: u64, audio_start: u64) -> io::Result<u128> {
    let mut hasher = Xxh3::new();
    hasher.update(b"ogg");

    let mut buf = vec![0u8; HEAD_WINDOW];
    let mut pos = audio_start;
    let mut fed = 0;
    while fed < HEAD_WINDOW
        && let Some(page) = read_ogg_page(reader, pos, len)?
    {
        let body = pos + page.header_len as u64;
        let take = page.body_len.min(HEAD_WINDOW - fed);
        let n = read_at(reader, body, &mut buf[..take])?;
        hasher.update(&buf[..n]);
        fed += take;
        pos = body + page.body_len as u64;
    }

    Ok(hasher.digest128())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn fingerprint(bytes: Vec<u8>) -> String {
        audio_fingerprint(&mut Cursor::new(bytes)).unwrap()
    }

    fn audio(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[1] ^ seed)
            .collect()
    }

    fn id3v2(body: usize) -> Vec<u8> {
        let mut tag = b"ID3\x04\x00\x00".to_vec();
        tag.extend(
            (0..4)
                .rev()
                .map(|shift| ((body >> (7 * shift)) & 0x7f) as u8),
        );
        tag.extend(vec![0x55; body]);
        tag
    }

    fn assert_tag_independent(build: impl Fn(usize, u8) -> Vec<u8>) {
        let base = fingerprint(build(10, 0));
        assert_eq!(base, fingerprint(build(5_000, 0)), "retag changed the id");
        assert_ne!(base, fingerprint(build(10, 1)), "different audio collided");
    }

    #[test]
    fn mp3_ignores_id3v2_id3v1_and_ape_tags() {
        assert_tag_independent(|tag, seed| {
            let mut file = id3v2(tag);
            file.extend(audio(200_000, seed));
            let mut ape = vec![0x33; tag];
            let mut footer = b"APETAGEX".to_vec();
            footer.extend(2000u32.to_le_bytes());
            footer.extend((tag as u32 + 32).to_le_bytes());
            footer.extend(0u32.to_le_bytes());
            footer.extend(0u32.to_le_bytes());
            footer.extend([0u8; 8]);
            ape.extend(footer);
            file.extend(ape);
            let mut v1 = b"TAG".to_vec();
            v1.extend(vec![tag as u8; 125]);
            file.extend(v1);
            file
        });
    }

    fn flac(streaminfo_md5: [u8; 16], tag: usize, seed: u8) -> Vec<u8> {
        let mut file = id3v2(tag);
        file.extend(b"fLaC");
        file.extend([0x00, 0, 0, 34]);
        file.extend([7u8; 18]);
        file.extend(streaminfo_md5);
        file.push(0x84);
        file.extend(&(tag as u32).to_be_bytes()[1..]);
        file.extend(vec![0x44; tag]);
        file.extend(audio(100_000, seed));
        file
    }

    fn mp3_with_info_frame(bitrate_byte: u8, frame_len: usize, seed: u8) -> Vec<u8> {
        let mut info = vec![0xff, 0xfb, bitrate_byte, 0x00];
        info.extend([0u8; 32]);
        info.extend(b"Info");
        info.resize(frame_len, 0);
        let mut file = id3v2(20);
        file.extend(info);
        file.extend(audio(100_000, seed));
        file
    }

    #[test]
    fn mp3_ignores_a_rewritten_info_frame() {
        let at_128 = |seed| fingerprint(mp3_with_info_frame(0x90, 417, seed));
        let at_192 = |seed| fingerprint(mp3_with_info_frame(0xb0, 626, seed));
        assert_eq!(at_128(0), at_192(0));
        assert_ne!(at_128(0), at_128(1));
    }

    #[test]
    fn flac_uses_the_streaminfo_md5_when_present() {
        let md5 = [0xab; 16];
        assert_eq!(
            fingerprint(flac(md5, 10, 0)),
            fingerprint(flac(md5, 5_000, 9))
        );
        assert_ne!(
            fingerprint(flac(md5, 10, 0)),
            fingerprint(flac([0xcd; 16], 10, 0))
        );
    }

    #[test]
    fn flac_without_md5_ignores_metadata_blocks() {
        assert_tag_independent(|tag, seed| {
            let mut file = b"fLaC".to_vec();
            file.extend([0x00, 0, 0, 34]);
            file.extend([0u8; 34]);
            file.push(0x84);
            file.extend(&(tag as u32).to_be_bytes()[1..]);
            file.extend(vec![0x44; tag]);
            file.extend(audio(100_000, seed));
            file
        });
    }

    #[test]
    fn mp4_hashes_mdat_wherever_moov_sits() {
        let boxed = |kind: &[u8], body: Vec<u8>| {
            let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            out.extend(kind);
            out.extend(body);
            out
        };
        let front = |tag: usize, seed: u8| {
            let mut file = boxed(b"ftyp", b"M4A \0\0\0\0".to_vec());
            file.extend(boxed(b"moov", vec![1; tag]));
            file.extend(boxed(b"mdat", audio(90_000, seed)));
            file
        };
        assert_tag_independent(front);
        let mut back = boxed(b"ftyp", b"M4A \0\0\0\0".to_vec());
        back.extend(boxed(b"mdat", audio(90_000, 0)));
        back.extend(boxed(b"moov", vec![1; 777]));
        assert_eq!(fingerprint(front(10, 0)), fingerprint(back));
    }

    #[test]
    fn wav_hashes_only_the_data_chunk() {
        assert_tag_independent(|tag, seed| {
            let data = audio(50_001, seed);
            let mut file = b"RIFF\0\0\0\0WAVE".to_vec();
            file.extend(b"LIST");
            file.extend((tag as u32).to_le_bytes());
            file.extend(vec![2; tag + tag % 2]);
            file.extend(b"data");
            file.extend((data.len() as u32).to_le_bytes());
            file.extend(data);
            file.push(0);
            file
        });
    }

    #[test]
    fn aiff_hashes_only_sound_data() {
        assert_tag_independent(|tag, seed| {
            let data = audio(40_000, seed);
            let mut file = b"FORM\0\0\0\0AIFF".to_vec();
            file.extend(b"ANNO");
            file.extend((tag as u32).to_be_bytes());
            file.extend(vec![3; tag + tag % 2]);
            file.extend(b"SSND");
            file.extend((data.len() as u32 + 8).to_be_bytes());
            file.extend([0u8; 8]);
            file.extend(data);
            file
        });
    }

    fn ogg_page(granule: u64, seq: u32, body: &[u8]) -> Vec<u8> {
        let mut segments = vec![255u8; body.len() / 255];
        segments.push((body.len() % 255) as u8);
        let mut page = b"OggS\0\0".to_vec();
        page.extend(granule.to_le_bytes());
        page.extend(1u32.to_le_bytes());
        page.extend(seq.to_le_bytes());
        page.extend(seq.wrapping_mul(0x9e37_79b9).to_le_bytes());
        page.push(segments.len() as u8);
        page.extend(segments);
        page.extend(body);
        page
    }

    #[test]
    fn ogg_ignores_comment_pages_and_page_renumbering() {
        assert_tag_independent(|tag, seed| {
            let mut file = ogg_page(0, 0, b"OpusHead........");
            let mut seq = 1;
            for chunk in vec![9u8; tag].chunks(4_000) {
                file.extend(ogg_page(u64::MAX, seq, chunk));
                seq += 1;
            }
            file.extend(ogg_page(0, seq, b"end of tags"));
            seq += 1;
            let samples = audio(4_000 * 40, seed);
            for (i, chunk) in samples.chunks(4_000).enumerate() {
                file.extend(ogg_page(960 * (i as u64 + 1), seq, chunk));
                seq += 1;
            }
            file
        });
    }

    #[test]
    #[ignore]
    fn bench_fingerprint_directory() {
        let dir = std::env::var("VLEER_BENCH_DIR").expect("set VLEER_BENCH_DIR");
        let files: Vec<_> = walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .collect();
        let started = std::time::Instant::now();
        let mut seen = std::collections::HashMap::new();
        for path in &files {
            let hash = audio_fingerprint(&mut std::fs::File::open(path).unwrap()).unwrap();
            println!("{hash} {}", path.display());
            if let Some(other) = seen.insert(hash, path.clone()) {
                println!("duplicate: {} == {}", path.display(), other.display());
            }
        }
        println!(
            "{} files in {:?} ({:?}/file)",
            files.len(),
            started.elapsed(),
            started.elapsed() / files.len().max(1) as u32
        );
    }

    #[test]
    fn tiny_and_empty_files_do_not_fail() {
        assert_ne!(fingerprint(Vec::new()), fingerprint(vec![1]));
        assert_eq!(fingerprint(b"TAG".to_vec()), fingerprint(b"TAG".to_vec()));
        fingerprint(b"fLaC\x80\xff\xff\xff".to_vec());
        fingerprint(b"OggS".to_vec());
    }
}
