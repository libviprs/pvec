//! Issue #219 (R1.5): the thirteen hostile tiles through the real reader. Each one is written into a file with the
//! writer's tile check off, and then: the file opens and validates and its geometry is intact, the tile is refused
//! by the rule the corpus names (and a tile over the ceiling is refused without a range being requested), the rest
//! of the file reads, and the polyglot cannot be fetched as an image from an origin that serves what the reader
//! accepts. The accepted variants are read back byte for byte.
#![allow(missing_docs)]

use pvec_container::blocks::raster::KIND_TILES;
use pvec_container::directory::BlockKind;
use pvec_container::reader::{BlockId, ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::{MemorySource, Recording};
use pvec_testkit::raster_hostile::{accepted, file, hostile};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

type Reader = ContainerReader<Recording<MemorySource<Vec<u8>>>>;

fn open(bytes: Vec<u8>) -> Reader {
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).expect("opens")
}

fn geometry(r: &Reader) -> Vec<Vec<u8>> {
    (0..r.entries().len() as u32)
        .filter(|i| {
            matches!(r.entries()[*i as usize].block_kind(), Some(BlockKind::Hot | BlockKind::Records))
        })
        .map(|i| r.read_block(BlockId(i)).unwrap().as_bytes().to_vec())
        .collect()
}

#[test]
fn there_are_thirteen_hostile_tiles_each_with_a_rule_and_a_reason() {
    let h = hostile();
    assert_eq!(h.len(), 13);
    let names: std::collections::BTreeSet<_> = h.iter().map(|t| t.name).collect();
    assert_eq!(names.len(), 13, "unique names");
    for t in &h {
        assert!(matches!(t.rule, Some("RASTER-03" | "RASTER-04" | "LIMIT-15")), "{}: {:?}", t.name, t.rule);
        assert!(!t.why.is_empty());
    }
    for rule in ["RASTER-03", "RASTER-04", "LIMIT-15"] {
        assert!(h.iter().any(|t| t.rule == Some(rule)), "no hostile tile for {rule}");
    }
}

#[test]
fn every_hostile_tile_is_refused_by_its_rule_and_the_file_and_its_geometry_are_intact() {
    let clean = {
        let mut w = pvec_container::writer::ContainerWriter::new(pvec_container::writer::WriterConfig {
            unit: pvec_container::LengthUnit::Millimetre,
            metadata: pvec_container::Metadata {
                producer_id: "pvec-hostile-tiles".into(),
                producer_version: "0".into(),
                corpus_revision: None,
            },
            bounds: None,
            capabilities: Vec::new(),
        });
        let b = pvec_container::Bounds { min: [0.0, 0.0], max: [10.0, 10.0] };
        w.push_line(
            pvec_container::writer::EntityMeta {
                id: pvec_entity::EntityId::new(0).unwrap(),
                style: 0,
                cell: 0,
                bounds: b,
            },
            [0.0, 0.0],
            [10.0, 10.0],
        )
        .unwrap();
        open(w.finish().unwrap())
    };
    for t in hostile() {
        let r = open(file(&t));
        r.validate().unwrap_or_else(|e| panic!("{}: the file must validate: {e}", t.name));
        assert_eq!(geometry(&r), geometry(&clean), "{}: the geometry is the geometry", t.name);
        let e = r.read_raster_tile(0, 0, 0, 0).expect_err(t.name);
        assert_eq!(Some(e.rule().as_str()), t.rule, "{}: {e}", t.name);
        let v = r.validate_raster(0).expect_err(t.name);
        assert_eq!(Some(v.rule().as_str()), t.rule, "{}: {v}", t.name);
        assert!(
            matches!(e, ReadError::Tile(_) | ReadError::Limit(_) | ReadError::Placement(_)),
            "{}: {e:?}",
            t.name
        );
    }
}

#[test]
fn a_tile_over_the_ceiling_is_refused_from_its_length_without_a_range_being_requested() {
    for t in hostile().into_iter().filter(|t| t.rule == Some("LIMIT-15")) {
        let bytes = file(&t);
        let r = open(bytes);
        let entry = r.entries().iter().find(|e| e.kind == KIND_TILES).cloned();
        // the tile is inline when it fits the budget, otherwise in a chunk; either way no request names it
        let before = r.source().requests().len();
        let e = r.read_raster_tile(0, 0, 0, 0).expect_err(t.name);
        assert_eq!(e.rule(), "LIMIT-15", "{}", t.name);
        let asked: Vec<_> = r.source().requests()[before..].to_vec();
        let tile_len = t.bytes.len() as u32;
        assert!(
            asked.iter().all(|q| q.len != tile_len),
            "{}: a request of the tile's length: {asked:?}",
            t.name
        );
        if let Some(c) = entry {
            assert!(
                !asked
                    .iter()
                    .any(|q| q.offset >= c.offset && q.offset < c.offset + u64::from(c.stored_length))
            );
        }
    }
}

#[test]
fn every_accepted_variant_reads_back_byte_for_byte() {
    for t in accepted() {
        let r = open(file(&t));
        let got =
            r.read_raster_tile(0, 0, 0, 0).unwrap_or_else(|e| panic!("{}: {e}", t.name)).expect("stored");
        assert_eq!(got.as_slice(), &t.bytes[..], "{}", t.name);
        r.validate_raster(0).unwrap_or_else(|e| panic!("{}: {e}", t.name));
    }
}

#[test]
fn the_writer_refuses_every_hostile_tile_it_would_not_serve() {
    use pvec_components::{RASTER_CAPABILITY, RasterId, Stream, encode_rasters};
    use pvec_container::blocks::raster::TileIn;
    for t in hostile() {
        // the writer's tile check is on: the stream of a refused pair does not even encode, and the others are
        // refused at finish with the rule of the sniffer
        let p = pvec_testkit::raster_hostile::placement(&t);
        let Ok(payload) = encode_rasters(std::slice::from_ref(&p)) else {
            assert_eq!(t.name, "jpeg-under-an-rgba-placement");
            continue;
        };
        let mut w = pvec_container::writer::ContainerWriter::new(pvec_container::writer::WriterConfig {
            unit: pvec_container::LengthUnit::Millimetre,
            metadata: pvec_container::Metadata {
                producer_id: "p".into(),
                producer_version: "0".into(),
                corpus_revision: None,
            },
            bounds: None,
            capabilities: Vec::new(),
        });
        let b = pvec_container::Bounds { min: [0.0, 0.0], max: [10.0, 10.0] };
        w.push_line(
            pvec_container::writer::EntityMeta {
                id: pvec_entity::EntityId::new(0).unwrap(),
                style: 0,
                cell: 0,
                bounds: b,
            },
            [0.0, 0.0],
            [10.0, 10.0],
        )
        .unwrap();
        w.push_stream(Stream {
            capability: RASTER_CAPABILITY,
            version: pvec_capabilities::Version::new(1, 1, 0),
            payload,
        })
        .unwrap();
        w.push_raster_tiles(
            RasterId::new(0).unwrap(),
            vec![TileIn { level: 0, col: 0, row: 0, bytes: t.bytes.clone() }],
        )
        .unwrap();
        let e = w.finish().expect_err(t.name);
        assert_eq!(e.rule(), "WRT-17", "{}: {e}", t.name);
        assert!(e.to_string().contains(t.rule.unwrap()), "{}: {e}", t.name);
    }
}

/// An origin that serves what a reader hands out as an image: `GET /<name>` answers 200 with an image content type
/// and the tile's bytes if the reader returns them, and 404 otherwise.
fn serve(files: Vec<(&'static str, Vec<u8>)>) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let files = Arc::new(files);
    let handle = std::thread::spawn(move || {
        for _ in 0..files.len() {
            let Ok((mut conn, _)) = listener.accept() else { return };
            // the whole request, so that closing the connection never leaves unread bytes behind (a reset)
            let mut request = Vec::new();
            let mut buf = [0u8; 256];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match conn.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let request = String::from_utf8_lossy(&request).to_string();
            let name = request.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
            let reply = files.iter().find(|(n, _)| *n == name).and_then(|(_, file)| {
                let r =
                    ContainerReader::open(MemorySource::new(file.clone()), ReaderConfig::default()).ok()?;
                r.read_raster_tile(0, 0, 0, 0).ok().flatten()
            });
            match reply {
                Some(tile) => {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        tile.len()
                    );
                    let _ = conn.write_all(head.as_bytes());
                    let _ = conn.write_all(&tile);
                    let _ = conn.flush();
                }
                None => {
                    let _ = conn.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        }
    });
    (addr, handle)
}

fn get(addr: std::net::SocketAddr, name: &str) -> (u16, Vec<u8>) {
    let mut c = TcpStream::connect(addr).unwrap();
    write!(c, "GET /{name} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = Vec::new();
    c.read_to_end(&mut out).unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").map_or(out.len(), |i| i + 4);
    let status: u16 =
        String::from_utf8_lossy(&out[..split]).split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out[split..].to_vec())
}

#[test]
fn no_refused_tile_can_be_fetched_as_an_image_from_an_origin_that_serves_what_the_reader_accepts() {
    let ok = accepted();
    let bad = hostile();
    let mut files: Vec<(&'static str, Vec<u8>)> = ok.iter().map(|t| (t.name, file(t))).collect();
    files.extend(bad.iter().map(|t| (t.name, file(t))));
    let (addr, server) = serve(files);
    for t in &ok {
        let (status, body) = get(addr, t.name);
        assert_eq!(status, 200, "{}", t.name);
        assert_eq!(body, t.bytes, "{}", t.name);
    }
    for t in &bad {
        let (status, body) = get(addr, t.name);
        assert_eq!(status, 404, "{}: a refused tile was served", t.name);
        assert!(body.is_empty());
    }
    server.join().unwrap();
}
