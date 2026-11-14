//! Reading JSONL files from the end without loading them whole.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const CHUNK: usize = 64 * 1024;

/// Yields complete lines of a file from the end towards the start, with the byte offset
/// at which each line starts. A trailing line without a newline (still being written) is
/// skipped.
pub struct ReverseLines {
    file: File,
    /// File offset of `tail[0]`.
    pos: u64,
    /// Unconsumed bytes `[pos, pos + tail.len())`, never containing the newline that
    /// terminates the region.
    tail: Vec<u8>,
    /// Bytes read from disk so far.
    pub bytes_read: u64,
    /// Offset just past the last complete line at or before the requested end.
    pub end: u64,
}

impl ReverseLines {
    /// Start reading backwards from `before` (a line-start offset), or from the end of the
    /// file when `None`.
    pub fn open(path: &Path, before: Option<u64>) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut end = before.unwrap_or(len).min(len);
        let mut bytes_read = 0;
        if before.is_none() && end > 0 {
            // Drop a trailing partial line: find the last newline.
            let mut probe_end = end;
            loop {
                let start = probe_end.saturating_sub(CHUNK as u64);
                let mut buf = vec![0u8; (probe_end - start) as usize];
                file.seek(SeekFrom::Start(start))?;
                file.read_exact(&mut buf)?;
                bytes_read += buf.len() as u64;
                if let Some(i) = buf.iter().rposition(|&b| b == b'\n') {
                    end = start + i as u64 + 1;
                    break;
                }
                if start == 0 {
                    end = 0;
                    break;
                }
                probe_end = start;
            }
        }
        // The region ends with the newline of its last line (or at 0); exclude it.
        let region_end = if end > 0 { end - 1 } else { 0 };
        Ok(ReverseLines { file, pos: region_end, tail: Vec::new(), bytes_read, end })
    }

    /// The next line going backwards: `(start_offset, bytes)` without the newline.
    pub fn next_line(&mut self) -> io::Result<Option<OffsetLine>> {
        loop {
            if let Some(i) = self.tail.iter().rposition(|&b| b == b'\n') {
                let line = self.tail.split_off(i + 1);
                self.tail.truncate(i);
                let start = self.pos + i as u64 + 1;
                if line.is_empty() {
                    continue;
                }
                return Ok(Some((start, line)));
            }
            if self.pos == 0 {
                if self.tail.is_empty() {
                    return Ok(None);
                }
                let line = std::mem::take(&mut self.tail);
                return Ok(Some((0, line)));
            }
            let new_pos = self.pos.saturating_sub(CHUNK as u64);
            let mut buf = vec![0u8; (self.pos - new_pos) as usize];
            self.file.seek(SeekFrom::Start(new_pos))?;
            self.file.read_exact(&mut buf)?;
            self.bytes_read += buf.len() as u64;
            buf.extend_from_slice(&self.tail);
            self.tail = buf;
            self.pos = new_pos;
        }
    }
}

/// A line with the byte offset where it starts.
pub type OffsetLine = (u64, Vec<u8>);

/// Complete lines appended after byte offset `after`, reading at most `max_bytes`.
/// Returns the lines with their start offsets and the offset just past the last complete
/// line read (the next `after`).
pub fn lines_after(path: &Path, after: u64, max_bytes: u64) -> io::Result<(Vec<OffsetLine>, u64)> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if after >= len {
        return Ok((Vec::new(), after.min(len)));
    }
    let want = (len - after).min(max_bytes);
    let mut buf = vec![0u8; want as usize];
    file.seek(SeekFrom::Start(after))?;
    file.read_exact(&mut buf)?;
    let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
        return Ok((Vec::new(), after));
    };
    buf.truncate(last_nl);
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, &b) in buf.iter().enumerate() {
        if b == b'\n' {
            if i > start {
                out.push((after + start as u64, buf[start..i].to_vec()));
            }
            start = i + 1;
        }
    }
    if buf.len() > start {
        out.push((after + start as u64, buf[start..].to_vec()));
    }
    Ok((out, after + last_nl as u64 + 1))
}

/// Cheap pre-filter so huge lines that can't matter are not JSON-parsed.
pub(crate) fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(name: &str, content: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("muster-agents-tail-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.jsonl");
        std::fs::File::create(&p).unwrap().write_all(content).unwrap();
        p
    }

    #[test]
    fn reverse_lines_and_offsets() {
        let big = "x".repeat(200_000);
        let content = format!("a\n{big}\nccc\npartial");
        let p = tmpfile("rev", content.as_bytes());
        let mut r = ReverseLines::open(&p, None).unwrap();
        assert_eq!(r.end, (content.len() - "partial".len()) as u64);
        let (o, l) = r.next_line().unwrap().unwrap();
        assert_eq!((o, l.as_slice()), ((2 + big.len() + 1) as u64, b"ccc".as_slice()));
        let (o, l) = r.next_line().unwrap().unwrap();
        assert_eq!(o, 2);
        assert_eq!(l.len(), big.len());
        let (o, l) = r.next_line().unwrap().unwrap();
        assert_eq!((o, l.as_slice()), (0, b"a".as_slice()));
        assert!(r.next_line().unwrap().is_none());

        // resume before the big line
        let mut r = ReverseLines::open(&p, Some(2)).unwrap();
        assert_eq!(r.next_line().unwrap().unwrap().1, b"a");
        assert!(r.next_line().unwrap().is_none());
    }

    #[test]
    fn lines_after_reads_only_complete_lines() {
        let p = tmpfile("after", b"one\ntwo\nthr");
        let (lines, next) = lines_after(&p, 0, 1 << 20).unwrap();
        assert_eq!(lines.iter().map(|l| l.1.clone()).collect::<Vec<_>>(), vec![b"one".to_vec(), b"two".to_vec()]);
        assert_eq!(next, 8);
        let (lines, next2) = lines_after(&p, next, 1 << 20).unwrap();
        assert!(lines.is_empty());
        assert_eq!(next2, 8);
    }

    #[test]
    fn empty_and_newline_only() {
        let p = tmpfile("empty", b"");
        assert!(ReverseLines::open(&p, None).unwrap().next_line().unwrap().is_none());
        let p = tmpfile("nl", b"\n\nx\n");
        let mut r = ReverseLines::open(&p, None).unwrap();
        assert_eq!(r.next_line().unwrap().unwrap(), (2, b"x".to_vec()));
        assert!(r.next_line().unwrap().is_none());
    }
}
