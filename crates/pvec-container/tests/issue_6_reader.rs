//! The safe range-readable reader (issue P4.5): what it costs to open, how it plans reads, that the
//! blocking and the async reader are one body of checks, and that a source that answers wrong is an error.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
mod support;

use pvec_container::limits::Limits;
use pvec_container::reader::{
    AsyncContainerReader, BlockId, ContainerReader, RangeRequest, ReadError, ReaderConfig, Validated,
};
use pvec_container::source::{
    AsyncBlockSource, BlockSource, Bytes, HttpRange, HttpRangeSource, HttpResponse, HttpTransport,
    MemorySource, ReadAtSource, Recording, Request, SourceError,
};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use support::art::{self, Art, with_gaps};
use support::fixtures::{Rng, fixtures};

#[global_allocator]
static A: counting_alloc::Counting = counting_alloc::Counting;

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn open(art: &Art) -> ContainerReader<MemorySource<Vec<u8>>> {
    ContainerReader::open(MemorySource::new(art.bytes.clone()), ReaderConfig::default()).expect("opens")
}

// ---------------------------------------------------------------- opening a 1 GiB artifact

/// An artifact of `total` bytes that is the real one with a gap of zeros between its blocks and its
/// directory, so a reader that fetched the file would need a gigabyte. Nothing is allocated for the gap.
struct Sparse {
    prefix: Vec<u8>,
    tail_at: u64,
    tail: Vec<u8>,
    total: u64,
}

impl Sparse {
    fn new(art: &Art, total: u64) -> Sparse {
        let dir_len = art.manifest.directories[0].length;
        let tail_at = total - dir_len;
        let mut prefix = art.bytes[..art.directory()].to_vec();
        let at = art.pointer_offset_field();
        prefix[at..at + 8].copy_from_slice(&tail_at.to_le_bytes());
        let tail = art.bytes[art.directory()..].to_vec();
        Sparse { prefix, tail_at, tail, total }
    }
}

impl BlockSource for Sparse {
    fn len(&self) -> u64 {
        self.total
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        let end = offset + u64::from(len);
        if end > self.total {
            return Err(SourceError::OutOfRange { offset, len: u64::from(len), source_len: self.total });
        }
        let mut out = vec![0u8; len as usize];
        for (base, data) in [(0u64, &self.prefix), (self.tail_at, &self.tail)] {
            let (lo, hi) = (offset.max(base), end.min(base + data.len() as u64));
            if lo < hi {
                out[(lo - offset) as usize..(hi - offset) as usize]
                    .copy_from_slice(&data[(lo - base) as usize..(hi - base) as usize]);
            }
        }
        Ok(Bytes::new(out))
    }
}

#[test]
fn opening_a_1gib_artifact_to_read_one_block_issues_at_most_three_requests_and_transfers_under_1mib() {
    let art = art::lines(40, 6);
    let total = 1u64 << 30;
    let source = Recording::new(Sparse::new(&art, total));
    let reader = ContainerReader::open(source, ReaderConfig::default()).expect("opens a 1 GiB artifact");
    let block = reader.read_block(BlockId(3)).expect("one block");
    assert_eq!(block.as_bytes().len(), art.entries[3].uncompressed_length as usize);
    let stats = reader.stats();
    assert!(stats.requests <= 3, "{} requests: header prefix, directory, block", stats.requests);
    assert!(stats.bytes < 1 << 20, "{} bytes transferred", stats.bytes);
}

#[test]
fn the_requests_of_an_open_are_the_header_prefix_the_directory_and_the_block_and_nothing_else() {
    let art = art::lines(40, 6);
    let total = 1u64 << 30;
    let sparse = Recording::new(Sparse::new(&art, total));
    let reader = ContainerReader::open(sparse, ReaderConfig::default()).unwrap();
    reader.read_block(BlockId(0)).unwrap();
    // the reader owns the recording source; the counters it keeps are the same numbers
    let s = reader.stats();
    assert_eq!(s.requests, 3);
    let dir_len = art.manifest.directories[0].length;
    assert_eq!(s.bytes, 4096 + dir_len + u64::from(art.entries[0].stored_length));
}

// ---------------------------------------------------------------- the read plan

fn gapped(gaps: [usize; 2]) -> Art {
    with_gaps(&art::lines(4, 2), &gaps)
}

#[test]
fn the_planner_coalesces_two_blocks_512_bytes_apart_into_one_request() {
    let art = gapped([0, 512]);
    let reader = open(&art);
    let plan = reader.plan(&[BlockId(0), BlockId(1)]).unwrap();
    assert_eq!(plan.len(), 1, "{plan:?}");
    let (a, b) = (&art.entries[0], &art.entries[1]);
    assert_eq!(plan[0].offset, a.offset);
    assert_eq!(u64::from(plan[0].len), b.offset + u64::from(b.stored_length) - a.offset);
    assert_eq!(plan[0].blocks, vec![BlockId(0), BlockId(1)]);
}

#[test]
fn the_planner_keeps_two_blocks_8_mib_apart_separate() {
    let art = gapped([0, 8 << 20]);
    let reader = open(&art);
    let plan = reader.plan(&[BlockId(0), BlockId(1)]).unwrap();
    assert_eq!(plan.len(), 2, "{plan:?}");
    assert_eq!(plan[0].blocks, vec![BlockId(0)]);
    assert_eq!(plan[1].blocks, vec![BlockId(1)]);
    assert_eq!(u64::from(plan[1].len), u64::from(art.entries[1].stored_length));
}

#[test]
fn reading_two_close_blocks_costs_one_request_after_the_open() {
    let art = gapped([0, 512]);
    let reader = open(&art);
    let before = reader.stats().requests;
    let blocks = reader.read_blocks(&[BlockId(1), BlockId(0)]).unwrap();
    assert_eq!(reader.stats().requests - before, 1);
    assert_eq!(blocks[0].id(), BlockId(1), "the result is in the order asked for");
    assert_eq!(blocks[1].id(), BlockId(0));
    let alone = open(&art).read_block(BlockId(1)).unwrap();
    assert_eq!(blocks[0].as_bytes(), alone.as_bytes());
}

#[test]
fn the_gap_threshold_and_the_request_size_are_configuration() {
    let art = gapped([0, 8 << 20]);
    let wide =
        ReaderConfig { gap_threshold: 16 << 20, max_request_bytes: 64 << 20, ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), wide).unwrap();
    assert_eq!(reader.plan(&[BlockId(0), BlockId(1)]).unwrap().len(), 1);
    let capped =
        ReaderConfig { gap_threshold: 16 << 20, max_request_bytes: 1 << 20, ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), capped).unwrap();
    assert_eq!(
        reader.plan(&[BlockId(0), BlockId(1)]).unwrap().len(),
        2,
        "a request never spans more than the cap"
    );
}

#[test]
fn a_plan_for_a_block_that_is_not_in_the_directory_is_an_error_and_not_a_panic() {
    let reader = open(&art::lines(4, 2));
    let e = reader.plan(&[BlockId(0), BlockId(99)]).unwrap_err();
    assert!(matches!(e, ReadError::NoSuchBlock { id: 99, blocks: 2 }), "{e:?}");
    assert!(reader.read_block(BlockId(u32::MAX)).is_err());
}

fn request_invariants(plan: &[RangeRequest], art: &Art, wanted: &[u32], gap: u32, max: u32) {
    let mut covered: Vec<u32> = plan.iter().flat_map(|r| r.blocks.iter().map(|b| b.0)).collect();
    let mut want: Vec<u32> = wanted.to_vec();
    want.sort_unstable();
    want.dedup();
    covered.sort_unstable();
    assert_eq!(covered, want, "every block is in exactly one request");
    for r in plan {
        for b in &r.blocks {
            let e = &art.entries[b.0 as usize];
            assert!(
                e.offset >= r.offset && e.offset + u64::from(e.stored_length) <= r.offset + u64::from(r.len)
            );
        }
        let first = &art.entries[r.blocks[0].0 as usize];
        assert_eq!(r.offset, first.offset, "a request starts where its first block does");
        if r.blocks.len() > 1 {
            assert!(r.len <= max, "a request over the cap holds more than one block");
        }
    }
    for pair in plan.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(a.offset + u64::from(a.len) <= b.offset, "requests are ordered and disjoint");
        let between = b.offset - (a.offset + u64::from(a.len));
        let merged = u64::from(b.len) + between + u64::from(a.len);
        assert!(between > u64::from(gap) || merged > u64::from(max), "two requests that could have been one");
    }
}

#[test]
fn a_plan_covers_each_wanted_block_once_in_ordered_disjoint_requests_whatever_the_gaps() {
    let mut rng = Rng(0x5eed_1234);
    for round in 0..300 {
        let n = 2 + (rng.next() % 12) as u32;
        let base = art::lines(3, n);
        let gaps: Vec<usize> =
            (0..n).map(|_| [0usize, 8, 512, 4096, 65536, 70000 & !7][(rng.next() % 6) as usize]).collect();
        let art = with_gaps(&base, &gaps);
        let (gap, max) =
            [(0, 1 << 20), (512, 1 << 20), (64 << 10, 1 << 20), (64 << 10, 700), (1 << 20, 4096)]
                [(rng.next() % 5) as usize];
        let config = ReaderConfig { gap_threshold: gap, max_request_bytes: max, ..ReaderConfig::default() };
        let reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), config).unwrap();
        let mut wanted: Vec<u32> = (0..n).filter(|_| rng.next() % 3 != 0).collect();
        if wanted.is_empty() {
            wanted.push(0);
        }
        // in any order, with repeats
        rng.shuffle(&mut wanted);
        let again = wanted[0];
        wanted.push(again);
        let ids: Vec<BlockId> = wanted.iter().map(|b| BlockId(*b)).collect();
        let plan = reader.plan(&ids).unwrap();
        request_invariants(&plan, &art, &wanted, gap, max);
        assert!(round < 300);
    }
}

// ---------------------------------------------------------------- one body of checks, two readers

/// What either reader gives back for a fixture.
#[derive(Debug, PartialEq)]
struct Outcome {
    validated: Result<Validated, String>,
    blocks: Vec<Result<Vec<u8>, String>>,
    batch: Result<Vec<Vec<u8>>, String>,
    requests: Vec<Request>,
}

trait Drive {
    fn block_count(&self) -> usize;
    fn block(&self, i: u32) -> Result<Vec<u8>, ReadError>;
    fn batch(&self, ids: &[BlockId]) -> Result<Vec<Vec<u8>>, ReadError>;
    fn validate(&self) -> Result<Validated, ReadError>;
    fn requests(&self) -> Vec<Request>;
}

struct Blocking(ContainerReader<Recording<MemorySource<Arc<[u8]>>>>);
struct Async(AsyncContainerReader<Recording<MemorySource<Arc<[u8]>>>>);

impl Drive for Blocking {
    fn block_count(&self) -> usize {
        self.0.entries().len()
    }
    fn block(&self, i: u32) -> Result<Vec<u8>, ReadError> {
        self.0.read_block(BlockId(i)).map(|b| b.as_bytes().to_vec())
    }
    fn batch(&self, ids: &[BlockId]) -> Result<Vec<Vec<u8>>, ReadError> {
        self.0.read_blocks(ids).map(|v| v.iter().map(|b| b.as_bytes().to_vec()).collect())
    }
    fn validate(&self) -> Result<Validated, ReadError> {
        self.0.validate()
    }
    fn requests(&self) -> Vec<Request> {
        self.0.source().requests()
    }
}

impl Drive for Async {
    fn block_count(&self) -> usize {
        self.0.entries().len()
    }
    fn block(&self, i: u32) -> Result<Vec<u8>, ReadError> {
        block_on(self.0.read_block(BlockId(i))).map(|b| b.as_bytes().to_vec())
    }
    fn batch(&self, ids: &[BlockId]) -> Result<Vec<Vec<u8>>, ReadError> {
        block_on(self.0.read_blocks(ids)).map(|v| v.iter().map(|b| b.as_bytes().to_vec()).collect())
    }
    fn validate(&self) -> Result<Validated, ReadError> {
        block_on(self.0.validate())
    }
    fn requests(&self) -> Vec<Request> {
        self.0.source().requests()
    }
}

fn blocking(bytes: &[u8]) -> Result<Blocking, ReadError> {
    let source = Recording::new(MemorySource::new(Arc::<[u8]>::from(bytes)));
    ContainerReader::open(source, ReaderConfig::default()).map(Blocking)
}

fn asynchronous(bytes: &[u8]) -> Result<Async, ReadError> {
    let source = Recording::new(MemorySource::new(Arc::<[u8]>::from(bytes)));
    block_on(AsyncContainerReader::open(source, ReaderConfig::default())).map(Async)
}

/// The one body: everything a caller can ask of a reader, whichever reader it is.
fn exercise(open: Result<impl Drive, ReadError>) -> Result<Outcome, String> {
    let d = open.map_err(|e| e.to_string())?;
    let n = d.block_count() as u32;
    let validated = d.validate().map_err(|e| e.to_string());
    let blocks = (0..n).map(|i| d.block(i).map_err(|e| e.to_string())).collect();
    let all: Vec<BlockId> = (0..n).rev().map(BlockId).collect();
    let batch = d.batch(&all).map_err(|e| e.to_string());
    Ok(Outcome { validated, blocks, batch, requests: d.requests() })
}

fn same_outcome(name: &str, bytes: &[u8]) {
    let (sync, asynchronous) = (exercise(blocking(bytes)), exercise(asynchronous(bytes)));
    assert_eq!(sync, asynchronous, "{name}: the blocking and the async reader differ");
    if !name.starts_with("hostile") {
        let o = sync.as_ref().unwrap_or_else(|e| panic!("{name}: a valid file did not open: {e}"));
        assert!(o.validated.is_ok(), "{name}: {:?}", o.validated);
        assert!(o.blocks.iter().all(Result::is_ok) && o.batch.is_ok(), "{name}: a block did not read");
    }
}

#[test]
fn sync_and_async_produce_identical_output_on_every_fixture_through_one_shared_body() {
    for input in fixtures() {
        let art = art::from_input(&input);
        same_outcome(input.name, &art.bytes);
    }
    same_outcome("lines", &art::lines(30, 3).bytes);
    same_outcome("splines", &art::splines().bytes);
    same_outcome("composites", &art::composites(4).bytes);
    same_outcome("gapped", &gapped([0, 512]).bytes);
}

#[test]
fn sync_and_async_refuse_hostile_files_in_the_same_way() {
    let art = art::splines();
    let knots = art.entries[art.index_of(3, 1)].offset as usize + 32;
    same_outcome("hostile-knots", &art.clone().with_u32(knots + 4, u32::MAX).bytes);
    same_outcome(
        "hostile-count",
        &art::lines(8, 1).with_u32(art::lines(8, 1).entry_field(0, "count"), u32::MAX).bytes,
    );
    let mut cut = art.bytes.clone();
    cut.truncate(cut.len() - 5);
    same_outcome("hostile-truncated", &cut);
}

// ---------------------------------------------------------------- sources that answer wrong

struct Fake {
    data: Vec<u8>,
    mode: Mode,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Honest,
    OneByteShort,
    WholeObject,
    OtherRange,
    ShortAt(u64),
}

impl HttpTransport for Fake {
    fn fetch(&self, range: HttpRange) -> impl Future<Output = Result<HttpResponse, String>> {
        let body = self.data[range.start as usize..=range.end as usize].to_vec();
        let response = match self.mode {
            Mode::Honest => HttpResponse {
                status: 206,
                content_range: Some((range.start, range.end, Some(self.data.len() as u64))),
                body,
            },
            Mode::OneByteShort => {
                HttpResponse { status: 206, content_range: None, body: body[..body.len() - 1].to_vec() }
            }
            Mode::WholeObject => HttpResponse { status: 200, content_range: None, body: self.data.clone() },
            Mode::OtherRange => HttpResponse {
                status: 206,
                content_range: Some((range.start + 8, range.end + 8, None)),
                body,
            },
            Mode::ShortAt(at) if range.start == at => {
                HttpResponse { status: 206, content_range: None, body: body[..body.len() / 2].to_vec() }
            }
            Mode::ShortAt(_) => HttpResponse { status: 206, content_range: None, body },
        };
        std::future::ready(Ok(response))
    }
}

fn http(art: &Art, mode: Mode) -> HttpRangeSource<Fake> {
    HttpRangeSource::new(Fake { data: art.bytes.clone(), mode }, art.len() as u64)
}

#[test]
fn an_http_range_response_shorter_than_requested_is_an_error_and_not_a_short_decode() {
    let art = art::lines(8, 1);
    let source = http(&art, Mode::OneByteShort);
    let e = block_on(AsyncBlockSource::read_range(&source, 0, 64)).unwrap_err();
    assert_eq!(e, SourceError::Short { wanted: 64, got: 63 });
    let opened = block_on(AsyncContainerReader::open(source, ReaderConfig::default()));
    assert!(matches!(opened.err(), Some(ReadError::Source(SourceError::Short { .. }))));
}

#[test]
fn a_short_response_for_a_block_is_an_error_and_nothing_is_decoded_or_held() {
    let art = art::lines(8, 1);
    let from = art.entries[0].offset;
    let reader =
        block_on(AsyncContainerReader::open(http(&art, Mode::ShortAt(from)), ReaderConfig::default()))
            .expect("the prefix and the directory arrive whole");
    let held_by_the_layout = reader.accountant().live();
    let e = block_on(reader.read_block(BlockId(0))).unwrap_err();
    assert!(matches!(e, ReadError::Source(SourceError::Short { .. })), "{e:?}");
    assert_eq!(reader.accountant().live(), held_by_the_layout, "a short block holds no memory");
}

#[test]
fn a_server_that_ignores_the_range_or_answers_for_another_is_an_error() {
    let art = art::lines(8, 1);
    for mode in [Mode::WholeObject, Mode::OtherRange] {
        let e = block_on(AsyncBlockSource::read_range(&http(&art, mode), 0, 64)).unwrap_err();
        assert!(matches!(e, SourceError::RangeNotHonoured { .. }), "{e:?}");
    }
    // and an honest server is read
    let ok = block_on(AsyncBlockSource::read_range(&http(&art, Mode::Honest), 0, 64)).unwrap();
    assert_eq!(&ok[..], &art.bytes[..64]);
}

/// A source whose contract is broken: it says Ok and returns the wrong number of bytes.
struct Lying(Vec<u8>);

impl BlockSource for Lying {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        let end = (offset + u64::from(len)).min(self.0.len() as u64) as usize;
        Ok(Bytes::new(self.0[offset as usize..end.saturating_sub(1).max(offset as usize)].to_vec()))
    }
}

#[test]
fn the_reader_does_not_trust_a_source_to_honour_its_own_contract() {
    let art = art::lines(8, 1);
    let e = ContainerReader::open(Lying(art.bytes.clone()), ReaderConfig::default()).err().expect("refused");
    assert!(matches!(e, ReadError::Source(SourceError::Short { .. })), "{e:?}");
}

// ---------------------------------------------------------------- the sources

#[test]
fn a_positional_read_and_memory_give_the_same_artifact() {
    let art = art::from_input(&fixtures()[0]);
    let want = blocking(&art.bytes).unwrap().validate().unwrap();
    let positional = ReadAtSource::new(Arc::<[u8]>::from(&art.bytes[..]), art.len() as u64);
    let r = ContainerReader::open(positional, ReaderConfig::default()).unwrap();
    assert_eq!(r.validate().unwrap(), want);
    let slice = ContainerReader::open(MemorySource::new(&art.bytes[..]), ReaderConfig::default()).unwrap();
    assert_eq!(
        slice.validate().unwrap(),
        want,
        "a caller's own bytes, a memory map for one, are read the same way"
    );
}

#[test]
fn a_range_past_the_end_of_a_source_is_refused_by_every_source() {
    let art = art::lines(4, 1);
    let m = MemorySource::new(art.bytes.clone());
    assert!(matches!(
        BlockSource::read_range(&m, art.len() as u64 - 4, 8),
        Err(SourceError::OutOfRange { .. })
    ));
    assert!(matches!(BlockSource::read_range(&m, u64::MAX, 8), Err(SourceError::OutOfRange { .. })));
}

// ---------------------------------------------------------------- the cache

#[test]
fn the_cache_keeps_the_least_recently_used_block_out_last_and_counts_what_it_does() {
    let art = art::lines(100, 4);
    let one = u64::from(art.entries[0].uncompressed_length);
    let cfg = ReaderConfig { cache_bytes: 2 * one, ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), cfg).unwrap();
    let read = |i: u32| drop(reader.read_block(BlockId(i)).unwrap());
    read(0);
    read(1);
    read(0); // a hit: 0 is now the most recently used
    read(2); // evicts 1, the least recently used
    let s = reader.stats();
    assert_eq!((s.cache_hits, s.cache_misses, s.evictions), (1, 3, 1));
    read(0); // still cached
    read(1); // was evicted
    let s = reader.stats();
    assert_eq!((s.cache_hits, s.cache_misses), (2, 4));
}

#[test]
fn a_cache_budget_of_zero_caches_nothing_and_a_block_larger_than_the_budget_is_not_kept() {
    let art = art::lines(100, 2);
    let none = ReaderConfig { cache_bytes: 0, ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), none).unwrap();
    reader.read_block(BlockId(0)).unwrap();
    reader.read_block(BlockId(0)).unwrap();
    assert_eq!(reader.stats().cache_hits, 0);
    let small = ReaderConfig { cache_bytes: 100, ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), small).unwrap();
    reader.read_block(BlockId(0)).unwrap();
    reader.read_block(BlockId(0)).unwrap();
    assert_eq!(reader.stats().cache_hits, 0);
}

#[test]
fn a_cached_block_is_the_same_bytes_and_costs_no_request() {
    let art = art::lines(20, 2);
    let reader = open(&art);
    let first = reader.read_block(BlockId(1)).unwrap();
    let requests = reader.stats().requests;
    let second = reader.read_block(BlockId(1)).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(reader.stats().requests, requests);
}

// ---------------------------------------------------------------- the whole file

#[test]
fn the_decoded_curves_of_a_file_come_out_through_the_reader() {
    let art = art::splines();
    let reader = open(&art);
    let set = reader.advanced(0).unwrap().expect("a file with splines has the advanced set");
    let view = set.view().unwrap();
    assert_eq!(view.len(), 3);
    match view.curve(0).unwrap() {
        pvec_container::blocks::advanced::CurveView::Spline(s) => {
            assert_eq!((s.degree, s.x.len(), s.knots.len()), (3, 10, 14))
        }
        other => panic!("{other:?}"),
    }
    assert!(open(&art::lines(4, 1)).advanced(0).unwrap().is_none());
    let hot = open(&art::lines(4, 1)).read_block(BlockId(0)).unwrap();
    assert_eq!(hot.hot().unwrap().len(), 4);
}

#[test]
fn validation_checks_the_whole_file_and_reports_what_it_found() {
    let art = art::from_input(&fixtures()[2]);
    let v = open(&art).validate().expect("a canonical artifact validates");
    assert_eq!(v.blocks, art.entries.len());
    assert!(v.entities >= 140);
    assert!(!v.stream_capabilities.is_empty());
}

// ---------------------------------------------------------------- never a panic

fn mutate(rng: &mut Rng, bytes: &[u8]) -> Vec<u8> {
    let mut v = bytes.to_vec();
    match rng.next() % 6 {
        0 => {
            for _ in 0..1 + rng.next() % 4 {
                let at = (rng.next() as usize) % v.len();
                v[at] ^= 1 << (rng.next() % 8);
            }
        }
        1 => {
            let at = (rng.next() as usize) % v.len().saturating_sub(4).max(1);
            let word = [0u32, 1, u32::MAX, 0x7fff_ffff, 1 << 28, 0x0fff_ffff][(rng.next() % 6) as usize];
            v[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        2 => {
            let at = (rng.next() as usize) % v.len().saturating_sub(8).max(1);
            let word = [0u64, 1, u64::MAX, u64::MAX - 7, 1 << 40, 1 << 63][(rng.next() % 6) as usize];
            v[at..at + 8].copy_from_slice(&word.to_le_bytes());
        }
        3 => v.truncate((rng.next() as usize) % v.len()),
        4 => {
            let at = (rng.next() as usize) % v.len();
            v.remove(at);
        }
        _ => {
            let at = (rng.next() as usize) % v.len();
            let n = 1 + (rng.next() as usize) % 64;
            let chunk: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            v.splice(at..at, chunk);
        }
    }
    v
}

#[test]
fn no_mutation_of_a_valid_artifact_panics_the_reader_or_makes_it_allocate_what_the_file_claims() {
    let seeds: Vec<Vec<u8>> = vec![
        art::lines(20, 3).bytes,
        art::splines().bytes,
        art::composites(4).bytes,
        art::from_input(&fixtures()[2]).bytes,
        art::from_input(&fixtures()[4]).bytes,
    ];
    for (i, seed) in seeds.iter().enumerate() {
        let valid = ContainerReader::open(MemorySource::new(seed.clone()), ReaderConfig::default())
            .and_then(|r| r.validate());
        assert!(valid.is_ok(), "seed {i} is a valid artifact and must validate: {valid:?}");
    }
    let mut rng = Rng(0xfeed_f00d_dead_beef);
    let mut accepted = 0usize;
    for round in 0..6000 {
        let base = &seeds[round % seeds.len()];
        let bytes = mutate(&mut rng, base);
        let source = MemorySource::new(bytes.clone());
        let before = counting_alloc::requested();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            ContainerReader::open(source, ReaderConfig::default()).and_then(|r| r.validate())
        }));
        let used = counting_alloc::requested() - before;
        let result = outcome.unwrap_or_else(|_| panic!("the reader panicked on mutation {round}"));
        if result.is_ok() {
            accepted += 1;
        }
        let allowed = 64 << 10;
        assert!(
            used <= allowed + 16 * bytes.len(),
            "mutation {round}: {used} bytes requested for {} bytes",
            bytes.len()
        );
    }
    assert!(accepted < 6000, "some mutations are refused");
}

#[test]
fn no_prefix_of_a_valid_artifact_opens_and_validates() {
    let art = art::lines(6, 2);
    let whole = ContainerReader::open(MemorySource::new(art.bytes.clone()), ReaderConfig::default())
        .and_then(|r| r.validate());
    assert!(whole.is_ok(), "the whole file validates: {whole:?}");
    for cut in 0..art.len() {
        let source = MemorySource::new(art.bytes[..cut].to_vec());
        let r = catch_unwind(AssertUnwindSafe(|| {
            ContainerReader::open(source, ReaderConfig::default()).and_then(|r| r.validate())
        }));
        assert!(r.expect("no panic").is_err(), "a file cut at {cut} of {} validated", art.len());
    }
}

#[test]
fn limits_are_not_a_way_to_make_the_reader_panic() {
    let art = art::splines();
    let ok = ContainerReader::open(MemorySource::new(art.bytes.clone()), ReaderConfig::default())
        .and_then(|r| r.validate());
    assert!(ok.is_ok(), "{ok:?}");
    let mut rng = Rng(7);
    for _ in 0..200 {
        let l = Limits {
            max_artifact_bytes: rng.next() % 4096,
            max_block_bytes: (rng.next() % 2048) as u32,
            max_blocks: (rng.next() % 8) as u32,
            max_expansion_ratio: (rng.next() % 3) as u32,
            max_entities_per_artifact: rng.next() % 8,
            max_entities_per_block: (rng.next() % 8) as u32,
            max_control_points: (rng.next() % 12) as u32,
            max_knots: (rng.next() % 16) as u32,
            max_nesting_depth: (rng.next() % 5) as u32,
            max_alloc_bytes: rng.next() % 8192,
            max_decode_work: rng.next() % 8192,
            max_rasters: (rng.next() % 8) as u32,
            max_raster_tiles: (rng.next() % 64) as u32,
            max_tile_bytes: (rng.next() % 4096) as u32,
            max_plan_decoded_bytes: rng.next() % 8192,
        };
        let cfg = ReaderConfig { limits: l, ..ReaderConfig::default() };
        let source = MemorySource::new(art.bytes.clone());
        let r =
            catch_unwind(AssertUnwindSafe(|| ContainerReader::open(source, cfg).and_then(|r| r.validate())));
        assert!(r.is_ok(), "the reader panicked under {l:?}");
    }
}
