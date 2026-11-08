//! Issue #264 (slice 6 of #263): a ranged reader over HTTP sends `If-Range` with the validator it captured
//! from the first response, and an object that changed under it is refused as `SourceError::Changed`,
//! never decoded.

mod support;

use pvec_container::reader::{AsyncContainerReader, BlockId, ReadError, ReaderConfig};
use pvec_container::source::{
    AsyncBlockSource, HttpRange, HttpRangeSource, HttpResponse, HttpTransport, SourceError,
};
use std::future::Future;
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use support::art;

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

/// A server holding one object that can be replaced under the reader. It logs the `If-Range` of every
/// request, answers an `If-Range` that no longer matches with `200` when it honours the header, and sends
/// its `ETag` (or none) with every answer.
struct Server {
    object: Mutex<(Vec<u8>, Option<String>)>,
    honours_if_range: bool,
    sent: Mutex<Vec<Option<String>>>,
}

impl Server {
    fn new(bytes: Vec<u8>, etag: Option<&str>, honours_if_range: bool) -> Server {
        Server {
            object: Mutex::new((bytes, etag.map(str::to_string))),
            honours_if_range,
            sent: Mutex::new(Vec::new()),
        }
    }

    fn replace(&self, bytes: Vec<u8>, etag: Option<&str>) {
        *self.object.lock().unwrap() = (bytes, etag.map(str::to_string));
    }

    fn sent(&self) -> Vec<Option<String>> {
        self.sent.lock().unwrap().clone()
    }
}

impl HttpTransport for &Server {
    fn fetch(&self, range: HttpRange) -> impl Future<Output = Result<HttpResponse, String>> {
        let answer = self.fetch_if_range(range, None);
        async move { answer.await.map(|(r, _)| r) }
    }

    fn fetch_if_range(
        &self,
        range: HttpRange,
        if_range: Option<&str>,
    ) -> impl Future<Output = Result<(HttpResponse, Option<String>), String>> {
        self.sent.lock().unwrap().push(if_range.map(str::to_string));
        let (bytes, etag) = self.object.lock().unwrap().clone();
        let stale = if_range.is_some() && if_range != etag.as_deref();
        let response = if stale && self.honours_if_range {
            HttpResponse { status: 200, content_range: None, body: Vec::new() }
        } else {
            let end = (range.end as usize).min(bytes.len() - 1);
            HttpResponse {
                status: 206,
                content_range: Some((range.start, end as u64, Some(bytes.len() as u64))),
                body: bytes[range.start as usize..=end].to_vec(),
            }
        };
        std::future::ready(Ok((response, etag)))
    }
}

fn open(server: &Server, len: usize) -> AsyncContainerReader<HttpRangeSource<&Server>> {
    block_on(AsyncContainerReader::open(HttpRangeSource::new(server, len as u64), ReaderConfig::default()))
        .expect("the unchanged object opens")
}

fn changed(e: &ReadError) -> bool {
    matches!(e, ReadError::Source(SourceError::Changed { .. }))
}

#[test]
fn every_range_after_the_first_carries_the_validator_the_first_one_returned() {
    let a = art::lines(8, 1);
    let server = Server::new(a.bytes.clone(), Some("\"v1\""), true);
    let reader = open(&server, a.len());
    block_on(reader.read_block(BlockId(0))).expect("an unchanged object reads");
    let sent = server.sent();
    assert!(sent.len() >= 2, "opening and one block are at least two ranges: {sent:?}");
    assert_eq!(sent[0], None, "the first range has nothing to compare against");
    assert!(sent[1..].iter().all(|v| v.as_deref() == Some("\"v1\"")), "{sent:?}");
}

#[test]
fn an_object_rewritten_at_the_same_length_is_refused_as_changed_and_not_decoded() {
    // the shape of a re-calibration that replaces the last block: same length, other bytes
    let a = art::lines(8, 1);
    let server = Server::new(a.bytes.clone(), Some("\"v1\""), true);
    let reader = open(&server, a.len());
    let mut rewritten = a.bytes.clone();
    let at = a.entries[0].offset as usize + 8;
    rewritten[at] ^= 0xff;
    server.replace(rewritten, Some("\"v2\""));
    let e = block_on(reader.read_block(BlockId(0))).unwrap_err();
    assert!(changed(&e), "{e:?}");
    assert_eq!(e.rule(), "SOURCE");
}

#[test]
fn a_server_that_ignores_if_range_but_sends_the_new_etag_is_refused_as_changed() {
    let a = art::lines(8, 1);
    let server = Server::new(a.bytes.clone(), Some("\"v1\""), false);
    let reader = open(&server, a.len());
    server.replace(a.bytes.clone(), Some("\"v2\""));
    let e = block_on(reader.read_block(BlockId(0))).unwrap_err();
    assert!(changed(&e), "a 206 under another validator is another object: {e:?}");
}

#[test]
fn a_content_range_total_other_than_the_opened_length_is_refused_as_changed() {
    // no validator at all: the total in Content-Range is the only evidence, and it is enough
    let a = art::lines(8, 1);
    let server = Server::new(a.bytes.clone(), None, true);
    let reader = open(&server, a.len());
    server.replace(art::lines(64, 1).bytes, None);
    let e = block_on(reader.read_block(BlockId(0))).unwrap_err();
    assert!(changed(&e), "{e:?}");
}

#[test]
fn a_weak_etag_is_never_sent_as_if_range() {
    // RFC 9110 13.1.5: If-Range takes a strong validator only
    let a = art::lines(8, 1);
    let server = Server::new(a.bytes.clone(), Some("W/\"v1\""), true);
    let reader = open(&server, a.len());
    block_on(reader.read_block(BlockId(0))).expect("reads");
    assert!(server.sent().iter().all(Option::is_none), "{:?}", server.sent());
}

#[test]
fn a_200_to_an_if_range_is_changed_and_not_range_not_honoured() {
    let a = art::lines(8, 1);
    let server = Server::new(a.bytes.clone(), Some("\"v1\""), true);
    let source = HttpRangeSource::new(&server, a.len() as u64);
    block_on(AsyncBlockSource::read_range(&source, 0, 64)).expect("the first range");
    server.replace(a.bytes.clone(), Some("\"v2\""));
    let e = block_on(AsyncBlockSource::read_range(&source, 0, 64)).unwrap_err();
    assert!(matches!(e, SourceError::Changed { .. }), "{e:?}");
    assert!(e.to_string().contains("changed"), "{e}");
}
