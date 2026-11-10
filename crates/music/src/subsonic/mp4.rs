use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use reqwest::header::{CONTENT_RANGE, DATE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE};
use symphonia::core::io::MediaSourceStreamOptions;

use crate::stream::{Reader, Stream};

/// The largest metadata suffix retained separately from the sequential download.
const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
const RANGE_PATIENCE: Duration = Duration::from_secs(5);
/// Allows for second-resolution modification times and clock skew between HTTP dates.
const VALIDATOR_AGE: time::Duration = time::Duration::seconds(60);

/// A validated suffix in the original file's byte coordinates.
pub(super) struct Tail {
    start: u64,
    bytes: Vec<u8>,
}

/// How to open a track without making the audio thread wait for its metadata.
#[derive(Clone, Default)]
pub(super) struct Prepared {
    progressive: bool,
    tail: Option<Arc<Tail>>,
}

/// The original stream response's address and validator, reused for metadata ranges.
pub(super) struct Remote {
    url: reqwest::Url,
    validator: Option<reqwest::header::HeaderValue>,
    total: u64,
}

impl Remote {
    pub(super) fn new(response: &reqwest::Response) -> Option<Self> {
        Some(Self {
            url: response.url().clone(),
            validator: range_validator(response.headers()),
            total: response.content_length()?,
        })
    }

    /// Accepts only an exact partial response and bounds both its size and its lifetime.
    async fn tail(&self, http: &reqwest::Client, start: u64) -> Option<Tail> {
        let validator = self.validator.as_ref()?;
        let length = self.total.checked_sub(start)?;
        if length == 0 || length > METADATA_LIMIT {
            return None;
        }
        let request = http
            .get(self.url.clone())
            .header(RANGE, format!("bytes={start}-{}", self.total - 1))
            .header(IF_RANGE, validator);
        tokio::time::timeout(RANGE_PATIENCE, async {
            let mut response = request.send().await.ok()?;
            let expected = format!("bytes {start}-{}/{}", self.total - 1, self.total);
            if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                || response.headers().get(CONTENT_RANGE)?.to_str().ok()? != expected
                || response.content_length().is_some_and(|size| size != length)
            {
                return None;
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.ok()? {
                if bytes.len() as u64 + chunk.len() as u64 > length {
                    return None;
                }
                bytes.extend_from_slice(&chunk);
            }
            (bytes.len() as u64 == length).then_some(Tail { start, bytes })
        })
        .await
        .ok()
        .flatten()
    }
}

impl Prepared {
    /// Waits for MP4 metadata on the cancellable load task. A server that refuses ranges falls
    /// back to the original sequential download without holding the audio thread.
    pub(super) async fn load(
        stream: &mut Stream,
        remote: Option<(&reqwest::Client, &Remote)>,
    ) -> Result<Self> {
        let mut at = 0usize;
        loop {
            let wanted = at
                .checked_add(16)
                .ok_or_else(|| anyhow::anyhow!("mp4 atom offset overflows"))?;
            stream.wait_for(wanted).await?;
            let header = stream
                .with(|_, spool| {
                    (spool.len() >= wanted)
                        .then(|| spool.bytes(at..wanted))
                        .transpose()
                })
                .transpose()?
                .flatten();
            let Some(header) = header else {
                return Ok(Self::default());
            };
            let kind = &header[4..8];
            if at == 0 && kind != b"ftyp" {
                return Ok(Self::default());
            }
            let length = match u32::from_be_bytes(header[..4].try_into()?) {
                1 => u64::from_be_bytes(header[8..16].try_into()?),
                0 => {
                    stream.wait_for(usize::MAX).await?;
                    return Ok(Self::default());
                }
                length if length >= 8 => u64::from(length),
                _ => bail!("mp4 atom is shorter than its header"),
            };
            if length < 8 || (header[..4] == 1u32.to_be_bytes() && length < 16) {
                bail!("mp4 atom is shorter than its header");
            }
            let end = usize::try_from(length)
                .ok()
                .and_then(|length| at.checked_add(length))
                .ok_or_else(|| anyhow::anyhow!("mp4 atom offset overflows"))?;
            if kind == b"mdat" || kind == b"moof" {
                if !stream.done()
                    && kind == b"mdat"
                    && let Some((http, remote)) = remote
                {
                    // Symphonia consumes one ring buffer before a skipped atom's end.
                    let overlap = MediaSourceStreamOptions::default().buffer_len as u64;
                    let start = (end as u64).saturating_sub(overlap);
                    if let Some(tail) = remote.tail(http, start).await {
                        return Ok(Self {
                            progressive: false,
                            tail: Some(Arc::new(tail)),
                        });
                    }
                }
                stream.wait_for(usize::MAX).await?;
                return Ok(Self::default());
            }
            if kind == b"moov" {
                stream.wait_for(end).await?;
                return Ok(Self {
                    progressive: !stream.done(),
                    tail: None,
                });
            }
            at = end;
        }
    }

    pub(super) fn seekable(&self, stream: &Stream) -> bool {
        stream.total().is_some() && (!self.progressive || stream.done())
    }

    pub(super) fn reader(&self, stream: &Stream) -> Input {
        Input {
            reader: stream.reader(),
            tail: self.tail.clone(),
            total: stream.total(),
            at: 0,
        }
    }
}

/// Reads fetched metadata at its original offsets while media keeps arriving sequentially.
/// The vault always receives the unmodified original file.
pub(super) struct Input {
    reader: Reader,
    tail: Option<Arc<Tail>>,
    total: Option<u64>,
    at: u64,
}

impl Read for Input {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.total.is_some_and(|total| self.at >= total) {
            return Ok(0);
        }
        let mut wanted = out.len();
        if let Some(tail) = &self.tail {
            if self.at >= tail.start {
                let offset = usize::try_from(self.at - tail.start).map_err(io::Error::other)?;
                let available = tail.bytes.get(offset..).unwrap_or_default();
                let count = wanted.min(available.len());
                out[..count].copy_from_slice(&available[..count]);
                self.at += count as u64;
                return Ok(count);
            }
            wanted = wanted.min(usize::try_from(tail.start - self.at).unwrap_or(usize::MAX));
        }
        self.reader.seek(SeekFrom::Start(self.at))?;
        let count = self.reader.read(&mut out[..wanted])?;
        self.at += count as u64;
        Ok(count)
    }
}

impl Seek for Input {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.reader.seek(SeekFrom::Start(self.at))?;
        self.at = self.reader.seek(pos)?;
        Ok(self.at)
    }
}

/// Selects a strong If-Range validator. A date is usable only without an ETag and with
/// enough separation from Date to account for timestamp resolution and clock skew.
fn range_validator(headers: &reqwest::header::HeaderMap) -> Option<reqwest::header::HeaderValue> {
    if let Some(etag) = headers.get(ETAG) {
        let bytes = etag.as_bytes();
        let strong = bytes.len() >= 2
            && bytes.first() == Some(&b'"')
            && bytes.last() == Some(&b'"')
            && bytes[1..bytes.len() - 1]
                .iter()
                .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte) || *byte >= 0x80);
        return strong.then(|| etag.clone());
    }
    let modified = headers.get(LAST_MODIFIED)?;
    let modified_at = time::OffsetDateTime::parse(
        modified.to_str().ok()?,
        &time::format_description::well_known::Rfc2822,
    )
    .ok()?;
    let dated_at = time::OffsetDateTime::parse(
        headers.get(DATE)?.to_str().ok()?,
        &time::format_description::well_known::Rfc2822,
    )
    .ok()?;
    (dated_at - modified_at >= VALIDATOR_AGE).then(|| modified.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::sync::mpsc;

    const FRONT: &[u8] = include_bytes!("fixtures/front.m4a");
    const TAIL: &[u8] = include_bytes!("fixtures/tail.m4a");
    const PREFIX: usize = 8192;

    /// Releases only chunks admitted by the test, leaving the rest of the media unavailable.
    struct Gated(mpsc::UnboundedReceiver<Bytes>);

    impl crate::stream::Source for Gated {
        async fn chunk(&mut self) -> Result<Option<Bytes>> {
            Ok(self.0.recv().await)
        }
    }

    /// Starts a download with only its prefix available to the decoder.
    async fn partial(
        bytes: &'static [u8],
        total: Option<u64>,
    ) -> (Stream, mpsc::UnboundedSender<Bytes>) {
        let (send, recv) = mpsc::unbounded_channel();
        let mut stream = Stream::pulling(Gated(recv), total, crate::stream::Plain);
        let length = PREFIX.min(bytes.len());
        send.send(Bytes::from_static(&bytes[..length])).unwrap();
        stream.wait_for(length).await.unwrap();
        (stream, send)
    }

    /// Opens the actual playback decoder while the sequential download remains gated.
    async fn decoder(stream: Stream, prepared: Prepared, at: Duration) -> rodio::Decoder<Input> {
        let task = tokio::task::spawn_blocking(move || {
            let total = stream.total();
            let mut builder = rodio::Decoder::builder()
                .with_symphonia_decoder::<crate::aac::Aac>()
                .with_data(prepared.reader(&stream));
            if let Some(total) = total {
                builder = builder.with_byte_len(total);
            }
            let mut decoder = builder
                .with_seekable(prepared.seekable(&stream))
                .build()
                .unwrap();
            if !at.is_zero() {
                rodio::Source::try_seek(&mut decoder, at).unwrap();
            }
            decoder
        });
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("decoder waited for withheld media")
            .unwrap()
    }

    /// Returns valid ranges or a deliberately invalid response for fallback checks.
    async fn server(
        mode: &'static str,
    ) -> (
        reqwest::Client,
        Remote,
        mpsc::UnboundedReceiver<String>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let (send, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let send = send.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        if socket.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        request.push(byte[0]);
                        assert!(request.len() < 8192);
                    }
                    let request = String::from_utf8(request).unwrap();
                    send.send(request.clone()).unwrap();
                    let range = request
                        .lines()
                        .find_map(|line| line.strip_prefix("range: bytes="))
                        .unwrap();
                    let (start, end) = range.split_once('-').unwrap();
                    let start = start.parse::<usize>().unwrap();
                    assert_eq!(end.parse::<usize>().unwrap(), TAIL.len() - 1);
                    let body = &TAIL[start..];
                    let header = match mode {
                        "ignore" => format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            TAIL.len()
                        ),
                        "wrong" => format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            TAIL.len() - 1,
                            TAIL.len(),
                            body.len()
                        ),
                        _ => format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            TAIL.len() - 1,
                            TAIL.len(),
                            body.len()
                        ),
                    };
                    if socket.write_all(header.as_bytes()).await.is_err() {
                        return;
                    }
                    let body = match mode {
                        "short" => &body[..body.len() / 2],
                        _ => body,
                    };
                    socket.write_all(body).await.ok();
                });
            }
        });
        let remote = Remote {
            url,
            total: TAIL.len() as u64,
            validator: Some("\"test-version\"".parse().unwrap()),
        };
        (reqwest::Client::new(), remote, requests, task)
    }

    #[test]
    fn date_validators_require_a_conservative_age_without_an_etag() {
        use reqwest::header::HeaderMap;
        let mut headers = HeaderMap::new();
        headers.insert(
            LAST_MODIFIED,
            "Tue, 15 Nov 1994 12:45:26 GMT".parse().unwrap(),
        );
        assert!(range_validator(&headers).is_none());
        for (date, accepted) in [
            ("Tue, 15 Nov 1994 12:45:26 GMT", false),
            ("Tue, 15 Nov 1994 12:46:25 GMT", false),
            ("Tue, 15 Nov 1994 12:46:26 GMT", true),
            ("Tue, 15 Nov 1994 12:44:26 GMT", false),
            ("invalid date", false),
        ] {
            headers.insert(DATE, date.parse().unwrap());
            assert_eq!(range_validator(&headers).is_some(), accepted, "{date}");
        }
        headers.insert(DATE, "Tue, 15 Nov 1994 12:46:26 GMT".parse().unwrap());
        headers.insert(ETAG, "W/\"version\"".parse().unwrap());
        assert!(range_validator(&headers).is_none());
        headers.remove(ETAG);
        headers.insert(LAST_MODIFIED, "invalid date".parse().unwrap());
        assert!(range_validator(&headers).is_none());
    }

    #[test]
    fn only_strong_well_formed_etags_can_be_used_for_ranges() {
        use reqwest::header::HeaderMap;
        let mut headers = HeaderMap::new();
        headers.insert(
            LAST_MODIFIED,
            "Tue, 15 Nov 1994 12:45:26 GMT".parse().unwrap(),
        );
        headers.insert(DATE, "Tue, 15 Nov 1994 12:46:26 GMT".parse().unwrap());
        for (etag, accepted) in [
            ("\"version\"", true),
            ("\"\"", true),
            ("W/\"version\"", false),
            ("unquoted", false),
            ("\"has space\"", false),
            ("\"has\"quote\"", false),
        ] {
            headers.insert(ETAG, etag.parse().unwrap());
            assert_eq!(range_validator(&headers).is_some(), accepted, "{etag}");
        }
    }

    #[tokio::test]
    async fn tail_metadata_starts_before_media_downloads() {
        let (http, remote, mut requests, server) = server("valid").await;
        let (mut stream, _send) = partial(TAIL, Some(TAIL.len() as u64)).await;
        let prepared = Prepared::load(&mut stream, Some((&http, &remote)))
            .await
            .unwrap();
        let tail = prepared.tail.as_ref().unwrap();
        assert!(tail.start > PREFIX as u64);
        assert!(tail.bytes.len() < TAIL.len() / 2);
        let request = requests.recv().await.unwrap();
        assert!(request.contains("if-range: \"test-version\""));
        let mut decoder = decoder(stream.clone(), prepared, Duration::ZERO).await;
        assert!(decoder.next().is_some());
        assert_eq!(stream.arrived(), PREFIX);
        assert!(!stream.done());
        server.abort();
    }

    #[tokio::test]
    async fn bad_range_responses_fall_back_to_a_cancellable_download() {
        for mode in ["ignore", "wrong", "short"] {
            let (http, remote, mut requests, server) = server(mode).await;
            let (mut stream, send) = partial(TAIL, Some(TAIL.len() as u64)).await;
            let mut waiting = tokio::spawn(async move {
                let prepared = Prepared::load(&mut stream, Some((&http, &remote)))
                    .await
                    .unwrap();
                (stream, prepared)
            });
            requests.recv().await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                    .await
                    .is_err()
            );
            send.send(Bytes::from_static(&TAIL[PREFIX..])).unwrap();
            drop(send);
            let (stream, prepared) = waiting.await.unwrap();
            assert!(prepared.tail.is_none());
            assert!(
                decoder(stream, prepared, Duration::ZERO)
                    .await
                    .next()
                    .is_some()
            );
            server.abort();
        }
    }

    #[tokio::test]
    async fn missing_validator_keeps_the_original_sequential_response() {
        let (http, mut remote, mut requests, server) = server("valid").await;
        remote.validator = None;
        let (mut stream, send) = partial(TAIL, Some(TAIL.len() as u64)).await;
        let mut waiting = tokio::spawn(async move {
            let prepared = Prepared::load(&mut stream, Some((&http, &remote)))
                .await
                .unwrap();
            (stream, prepared)
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                .await
                .is_err()
        );
        send.send(Bytes::from_static(&TAIL[PREFIX..])).unwrap();
        drop(send);
        let (stream, prepared) = waiting.await.unwrap();
        assert!(matches!(
            requests.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(prepared.tail.is_none());
        assert!(
            decoder(stream, prepared, Duration::ZERO)
                .await
                .next()
                .is_some()
        );
        server.abort();
    }

    #[tokio::test]
    async fn tail_without_ranges_can_be_cancelled() {
        let (mut stream, _send) = partial(TAIL, None).await;
        let mut waiting = tokio::spawn(async move { Prepared::load(&mut stream, None).await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                .await
                .is_err()
        );
        waiting.abort();
        assert!(waiting.await.err().unwrap().is_cancelled());
        let (mut next, _send) = partial(FRONT, None).await;
        let prepared = Prepared::load(&mut next, None).await.unwrap();
        assert!(
            decoder(next, prepared, Duration::ZERO)
                .await
                .next()
                .is_some()
        );
    }

    #[tokio::test]
    async fn front_metadata_starts_without_ranges_or_a_known_length() {
        for total in [Some(FRONT.len() as u64), None] {
            let (mut stream, _send) = partial(FRONT, total).await;
            let prepared = Prepared::load(&mut stream, None).await.unwrap();
            assert!(prepared.progressive);
            assert!(
                decoder(stream.clone(), prepared, Duration::from_millis(100))
                    .await
                    .next()
                    .is_some()
            );
            assert_eq!(stream.arrived(), PREFIX);
        }
    }

    #[tokio::test]
    async fn front_metadata_spans_chunks_and_restores_seeking_after_download() {
        use rodio::Source as _;
        let (send, recv) = mpsc::unbounded_channel();
        let mut stream =
            Stream::pulling(Gated(recv), Some(FRONT.len() as u64), crate::stream::Plain);
        send.send(Bytes::from_static(&FRONT[..64])).unwrap();
        stream.wait_for(64).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), Prepared::load(&mut stream, None))
                .await
                .is_err()
        );
        send.send(Bytes::from_static(&FRONT[64..PREFIX])).unwrap();
        let prepared = Prepared::load(&mut stream, None).await.unwrap();
        assert!(!prepared.seekable(&stream));
        let progressive = decoder(stream.clone(), prepared.clone(), Duration::ZERO).await;
        send.send(Bytes::from_static(&FRONT[PREFIX..])).unwrap();
        drop(send);
        stream.finished().await;
        assert!(prepared.seekable(&stream));
        let expected = decoder(stream.clone(), Prepared::default(), Duration::ZERO)
            .await
            .collect::<Vec<_>>();
        assert_eq!(progressive.collect::<Vec<_>>(), expected);
        let mut seeked = decoder(stream, prepared, Duration::from_secs(1)).await;
        seeked.try_seek(Duration::ZERO).unwrap();
        assert!(seeked.next().is_some());
    }

    #[tokio::test]
    async fn tail_overlay_preserves_every_byte_and_sample_after_completion() {
        use rodio::Source as _;
        let (http, remote, _requests, server) = server("valid").await;
        let (mut stream, send) = partial(TAIL, Some(TAIL.len() as u64)).await;
        let prepared = Prepared::load(&mut stream, Some((&http, &remote)))
            .await
            .unwrap();
        let progressive = decoder(stream.clone(), prepared.clone(), Duration::ZERO).await;
        send.send(Bytes::from_static(&TAIL[PREFIX..])).unwrap();
        drop(send);
        stream.finished().await;
        let mut reader = prepared.reader(&stream);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, TAIL);
        let expected = decoder(stream.clone(), Prepared::default(), Duration::ZERO)
            .await
            .collect::<Vec<_>>();
        assert_eq!(progressive.collect::<Vec<_>>(), expected);
        let mut seeked = decoder(stream, prepared, Duration::from_secs(10)).await;
        assert!(seeked.next().is_some());
        seeked.try_seek(Duration::ZERO).unwrap();
        assert!(seeked.next().is_some());
        server.abort();
    }

    #[tokio::test]
    async fn other_formats_keep_their_decoder_mode() {
        let formats: &[&'static [u8]] = &[
            include_bytes!("fixtures/control.wav"),
            include_bytes!("fixtures/control.flac"),
            include_bytes!("fixtures/control.mp3"),
            include_bytes!("fixtures/control.ogg"),
        ];
        for &bytes in formats {
            let (mut stream, send) = partial(bytes, Some(bytes.len() as u64)).await;
            let prepared = Prepared::load(&mut stream, None).await.unwrap();
            assert!(!prepared.progressive);
            assert!(prepared.tail.is_none());
            send.send(Bytes::from_static(&bytes[PREFIX.min(bytes.len())..]))
                .unwrap();
            drop(send);
            stream.finished().await;
            assert!(
                decoder(stream, prepared, Duration::ZERO)
                    .await
                    .next()
                    .is_some()
            );
        }
    }
}
