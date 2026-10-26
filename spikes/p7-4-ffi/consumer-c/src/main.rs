//! Candidate (c): a native consumer of the `pvec` crate. There is no C symbol to count; the number printed
//! is the one the C prototypes of (a), (b) and (d) are held to.

fn main() {
    let path = std::env::args().nth(1).expect("usage: p74-consumer-c FILE");
    let bytes = std::fs::read(path).expect("read the fixture");
    let reader = pvec::PvecReader::open(bytes).expect("the reader accepts the fixture");
    println!("{}", reader.entity_count().expect("the file validates"));
}
