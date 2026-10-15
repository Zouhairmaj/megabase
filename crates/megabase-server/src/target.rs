//! Percent-encode the request-target bytes `http::Uri` rejects.
//!
//! Kong (nginx) accepts `"`, `<` and `>` raw in a request-target and passes
//! them on, so `GET /rest/v1/events?select=payload->user` works through the
//! reference gateway. hyper parses the target with `http::Uri`, which rejects
//! those bytes (and `` ` ``, `{`, `}` in the path), and answers a `400` with
//! an empty body before any router runs. [`Encoded`] wraps an accepted
//! stream and percent-encodes those bytes in each HTTP/1 request line, which
//! is what a browser sends for the same URL. Components percent-decode the
//! target, so the request means the same thing.
//!
//! Only the request line changes. The headers and a `Content-Length` body
//! pass through unchanged; after a body framing this module does not track
//! (chunked, an upgrade, `CONNECT`, HTTP/2) the rest of the connection passes
//! through unchanged too.

use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A request or header line longer than this passes through unchanged, and
/// so does the rest of the connection.
const MAX_LINE: usize = 64 * 1024;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Before a request line. Empty lines before it are kept as they are.
    Line,
    /// The header lines of a request.
    Head(Framing),
    /// Bytes left in a `Content-Length` body.
    Body(u64),
    /// Untracked framing: bytes pass through for the rest of the connection.
    Raw,
}

/// What the headers say about the body that follows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Framing {
    length: Option<u64>,
    untracked: bool,
}

impl Framing {
    fn header(&mut self, line: &[u8]) {
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            self.untracked = true;
            return;
        };
        let name = &line[..colon];
        if name.first().is_some_and(u8::is_ascii_whitespace) {
            // An obs-fold continuation line.
            self.untracked = true;
            return;
        }
        let name = name.trim_ascii();
        if name.eq_ignore_ascii_case(b"transfer-encoding") || name.eq_ignore_ascii_case(b"upgrade")
        {
            self.untracked = true;
        } else if name.eq_ignore_ascii_case(b"content-length") {
            let value = line[colon + 1..].trim_ascii();
            let parsed = std::str::from_utf8(value)
                .ok()
                .filter(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|text| text.parse::<u64>().ok());
            match (parsed, self.length) {
                (Some(length), None) => self.length = Some(length),
                (Some(length), Some(seen)) if length == seen => {}
                _ => self.untracked = true,
            }
        }
    }

    fn next(self) -> State {
        if self.untracked {
            return State::Raw;
        }
        match self.length {
            Some(length) if length > 0 => State::Body(length),
            _ => State::Line,
        }
    }
}

/// The byte-level rewriter, without I/O.
#[derive(Debug)]
struct Rewriter {
    state: State,
    /// Bytes read but not yet processed (an incomplete line).
    pending: Vec<u8>,
    /// Processed bytes not yet handed to the reader, from `ready_at`.
    ready: Vec<u8>,
    ready_at: usize,
}

impl Rewriter {
    fn new() -> Self {
        Self {
            state: State::Line,
            pending: Vec::new(),
            ready: Vec::new(),
            ready_at: 0,
        }
    }

    fn has_ready(&self) -> bool {
        self.ready_at < self.ready.len()
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        self.process();
    }

    /// The peer closed its side: an incomplete line is handed on as it is.
    fn finish(&mut self) {
        self.ready.append(&mut self.pending);
    }

    fn take_into(&mut self, buf: &mut ReadBuf<'_>) {
        let available = &self.ready[self.ready_at..];
        let count = available.len().min(buf.remaining());
        buf.put_slice(&available[..count]);
        self.ready_at += count;
        if self.ready_at == self.ready.len() {
            self.ready.clear();
            self.ready_at = 0;
        }
    }

    fn process(&mut self) {
        loop {
            match self.state {
                State::Raw => {
                    self.ready.append(&mut self.pending);
                    return;
                }
                State::Body(left) => {
                    if self.pending.is_empty() {
                        return;
                    }
                    let take = usize::try_from(left)
                        .map_or(self.pending.len(), |left| left.min(self.pending.len()));
                    self.ready.extend(self.pending.drain(..take));
                    let left = left.saturating_sub(u64::try_from(take).unwrap_or(u64::MAX));
                    self.state = if left == 0 {
                        State::Line
                    } else {
                        State::Body(left)
                    };
                }
                State::Line | State::Head(_) => {
                    let Some(end) = self.pending.iter().position(|&b| b == b'\n') else {
                        if self.pending.len() > MAX_LINE {
                            self.state = State::Raw;
                            continue;
                        }
                        return;
                    };
                    let line: Vec<u8> = self.pending.drain(..=end).collect();
                    self.state = advance(self.state, &line, &mut self.ready);
                }
            }
        }
    }
}

fn is_blank(line: &[u8]) -> bool {
    matches!(line, b"\n" | b"\r\n")
}

/// Handle one complete line in the `Line` or `Head` state.
fn advance(state: State, line: &[u8], out: &mut Vec<u8>) -> State {
    match state {
        State::Line if is_blank(line) => {
            out.extend_from_slice(line);
            State::Line
        }
        State::Line if line.starts_with(b"PRI * HTTP/2.0") => {
            out.extend_from_slice(line);
            State::Raw
        }
        State::Line => {
            encode_request_line(line, out);
            State::Head(Framing {
                length: None,
                // A tunnel: what follows the head is not HTTP.
                untracked: line.starts_with(b"CONNECT "),
            })
        }
        State::Head(framing) if is_blank(line) => {
            out.extend_from_slice(line);
            framing.next()
        }
        State::Head(mut framing) => {
            out.extend_from_slice(line);
            framing.header(line);
            State::Head(framing)
        }
        State::Body(_) | State::Raw => {
            out.extend_from_slice(line);
            State::Raw
        }
    }
}

/// `METHOD SP target SP version`: percent-encode `"`, `<`, `>` in the
/// target, and `` ` ``, `{`, `}` before its `?`. A line without that shape
/// is left for hyper to reject.
fn encode_request_line(line: &[u8], out: &mut Vec<u8>) {
    let first = line.iter().position(|&b| b == b' ');
    let last = line.iter().rposition(|&b| b == b' ');
    let (Some(first), Some(last)) = (first, last) else {
        out.extend_from_slice(line);
        return;
    };
    if last <= first {
        out.extend_from_slice(line);
        return;
    }
    out.extend_from_slice(&line[..=first]);
    let mut query = false;
    for &byte in &line[first + 1..last] {
        query |= byte == b'?';
        let encode =
            matches!(byte, b'"' | b'<' | b'>') || (!query && matches!(byte, b'`' | b'{' | b'}'));
        if encode {
            out.push(b'%');
            out.push(HEX[usize::from(byte >> 4)]);
            out.push(HEX[usize::from(byte & 0x0f)]);
        } else {
            out.push(byte);
        }
    }
    out.extend_from_slice(&line[last..]);
}

pin_project_lite::pin_project! {
    /// An accepted stream whose HTTP/1 request lines are percent-encoded
    /// where `http::Uri` would reject them. Writes pass through.
    pub(crate) struct Encoded<IO> {
        #[pin]
        inner: IO,
        rewriter: Rewriter,
        eof: bool,
    }
}

impl<IO> Encoded<IO> {
    pub(crate) fn new(inner: IO) -> Self {
        Self {
            inner,
            rewriter: Rewriter::new(),
            eof: false,
        }
    }
}

impl<IO: AsyncRead> AsyncRead for Encoded<IO> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut this = self.project();
        loop {
            if this.rewriter.has_ready() {
                this.rewriter.take_into(buf);
                return Poll::Ready(Ok(()));
            }
            if *this.eof || buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }
            let mut chunk = [0_u8; 8192];
            let mut read = ReadBuf::new(&mut chunk);
            ready!(this.inner.as_mut().poll_read(cx, &mut read))?;
            if read.filled().is_empty() {
                *this.eof = true;
                this.rewriter.finish();
            } else {
                this.rewriter.feed(read.filled());
            }
        }
    }
}

impl<IO: AsyncWrite> AsyncWrite for Encoded<IO> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.project().inner.poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.project().inner.poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().inner.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().inner.poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite_in(chunks: &[&[u8]]) -> Vec<u8> {
        let mut rewriter = Rewriter::new();
        for chunk in chunks {
            rewriter.feed(chunk);
        }
        rewriter.finish();
        rewriter.ready.split_off(rewriter.ready_at)
    }

    fn rewrite(input: &[u8]) -> Vec<u8> {
        rewrite_in(&[input])
    }

    #[test]
    fn encodes_json_arrows_and_quotes_in_the_query() {
        assert_eq!(
            rewrite(b"GET /rest/v1/events?select=payload->user->>name&a=eq.\"x<y\" HTTP/1.1\r\nHost: h\r\n\r\n"),
            b"GET /rest/v1/events?select=payload-%3Euser-%3E%3Ename&a=eq.%22x%3Cy%22 HTTP/1.1\r\nHost: h\r\n\r\n"
        );
    }

    #[test]
    fn braces_stay_in_the_query_and_are_encoded_in_the_path() {
        assert_eq!(
            rewrite(b"GET /a{b}`c?tags=cs.{a,b} HTTP/1.1\r\n\r\n"),
            b"GET /a%7Bb%7D%60c?tags=cs.{a,b} HTTP/1.1\r\n\r\n"
        );
    }

    #[test]
    fn a_valid_request_is_unchanged() {
        let input = b"\r\nGET /rest/v1/todos?id=eq.1 HTTP/1.1\r\nHost: h\r\nX-A: <b>\r\n\r\n";
        assert_eq!(rewrite(input), input);
    }

    #[test]
    fn a_line_split_across_reads_is_encoded_once_complete() {
        assert_eq!(
            rewrite_in(&[b"GET /x?a", b"->b HT", b"TP/1.1\r", b"\n\r\n"]),
            b"GET /x?a-%3Eb HTTP/1.1\r\n\r\n"
        );
    }

    #[test]
    fn keep_alive_requests_after_a_sized_body_are_encoded() {
        let input: &[u8] = b"POST /x HTTP/1.1\r\nContent-Length: 9\r\n\r\nGET /a->b\
            GET /y?a->b HTTP/1.1\r\n\r\n";
        assert_eq!(
            rewrite(input),
            b"POST /x HTTP/1.1\r\nContent-Length: 9\r\n\r\nGET /a->b\
            GET /y?a-%3Eb HTTP/1.1\r\n\r\n"
        );
        // The same body split across reads.
        assert_eq!(
            rewrite_in(&[
                b"POST /x HTTP/1.1\r\ncontent-length:  9 \r\n\r\nGET",
                b" /a->b",
                b"GET /y?a->b HTTP/1.1\r\n\r\n"
            ]),
            b"POST /x HTTP/1.1\r\ncontent-length:  9 \r\n\r\nGET /a->b\
            GET /y?a-%3Eb HTTP/1.1\r\n\r\n"
        );
    }

    #[test]
    fn untracked_framing_passes_the_rest_through() {
        for head in [
            &b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"[..],
            b"GET /realtime HTTP/1.1\r\nUpgrade: websocket\r\n\r\n",
            b"POST /x HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n",
            b"POST /x HTTP/1.1\r\nContent-Length: -1\r\n\r\n",
            b"POST /x HTTP/1.1\r\nX: a\r\n folded\r\n\r\n",
            b"CONNECT h:443 HTTP/1.1\r\n\r\n",
            b"PRI * HTTP/2.0\r\n\r\n",
        ] {
            let mut input = head.to_vec();
            input.extend_from_slice(b"GET /y?a->b HTTP/1.1\r\n\r\n");
            assert_eq!(rewrite(&input), input, "{}", String::from_utf8_lossy(head));
        }
    }

    #[test]
    fn an_overlong_line_passes_through() {
        let mut input = b"GET /x?a->".to_vec();
        input.resize(MAX_LINE + 10, b'a');
        input.extend_from_slice(b" HTTP/1.1\r\n\r\nGET /y?a->b HTTP/1.1\r\n\r\n");
        assert_eq!(
            rewrite_in(&[&input[..MAX_LINE + 2], &input[MAX_LINE + 2..]]),
            input
        );
    }

    #[test]
    fn a_malformed_request_line_is_left_for_hyper() {
        for input in [
            &b"GARBAGE->\r\n\r\n"[..],
            b"GET /a->b\r\n\r\n",
            b"GET /a->b",
        ] {
            assert_eq!(rewrite(input), input);
        }
    }

    #[test]
    fn a_bare_lf_ends_the_line() {
        assert_eq!(
            rewrite(b"GET /?a->b HTTP/1.1\nHost: h\n\nGET /?c->d HTTP/1.1\n\n"),
            b"GET /?a-%3Eb HTTP/1.1\nHost: h\n\nGET /?c-%3Ed HTTP/1.1\n\n"
        );
    }

    #[tokio::test]
    async fn reads_through_the_stream_in_small_buffers() {
        use tokio::io::AsyncReadExt;
        let (mut client, server) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            tokio::io::AsyncWriteExt::write_all(&mut client, b"GET /?a->b HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
        });
        let mut encoded = Encoded::new(server);
        let mut out = Vec::new();
        let mut small = [0_u8; 3];
        loop {
            let read = encoded.read(&mut small).await.unwrap();
            if read == 0 {
                break;
            }
            out.extend_from_slice(&small[..read]);
        }
        writer.await.unwrap();
        assert_eq!(out, b"GET /?a-%3Eb HTTP/1.1\r\n\r\n");
    }
}
