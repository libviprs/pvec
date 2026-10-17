//! Prints what the pure-Rust reader makes of each dataset of an H1 file, errors included.
use hdf5_reader::Hdf5File;

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let bytes = std::fs::read(&path).expect("read");
    let f = Hdf5File::from_bytes(&bytes).expect("open");
    for name in [
        "CARTESIAN_POINT",
        "DIRECTION",
        "VECTOR",
        "LINE",
        "TRIMMED_CURVE",
        "POLYLINE",
        "B_SPLINE_CURVE_WITH_KNOTS",
        "COMPOSITE_CURVE",
        "GEOMETRIC_CURVE_SET",
    ] {
        match f.dataset(&format!("data/{name}")) {
            Err(e) => println!("{name}: dataset error: {e}"),
            Ok(ds) => {
                println!("{name}: shape {:?}, dtype {:?}", ds.shape(), ds.dtype());
                match ds.read_native_bytes() {
                    Ok(b) => println!("  read_native_bytes ok, {} bytes", b.len()),
                    Err(e) => println!("  read_native_bytes error: {e}"),
                }
                match ds.read_raw_bytes() {
                    Ok(b) => println!("  read_raw_bytes ok, {} bytes", b.len()),
                    Err(e) => println!("  read_raw_bytes error: {e}"),
                }
            }
        }
    }
}
