//! Issue #235: the files under the spill writer. A directory that goes with its last holder, a log that reads back in
//! order and by offset, an external sort that agrees with a stable sort at every budget, runs merged in passes,
//! and a truncated run that is an error and not a short result.

mod support;

use pvec_container::spill::{ExternalSorter, Log, SpillStore, Store};
use std::path::PathBuf;
use std::sync::Arc;

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pvec235-files-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lcg(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *seed >> 33
}

#[test]
fn the_directory_is_made_in_an_existing_root_only_and_goes_with_its_last_holder() {
    let r = root("dir");
    let missing = r.join("nothing-here");
    assert_eq!(
        support::spill_fs::FsStore::create(&missing).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(!missing.exists());
    let store = support::spill_fs::FsStore::create(&r).unwrap();
    let path = store.path().to_path_buf();
    assert!(path.starts_with(&r) && path.is_dir());
    let shared: Store = Arc::new(store);
    let other = Arc::clone(&shared);
    drop(shared);
    assert!(path.is_dir(), "a second holder keeps it");
    drop(other);
    assert!(!path.exists());
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn a_log_reads_back_in_order_and_by_offset() {
    let r = root("log");
    let store = support::store(&r);
    let mut log = Log::create(&store).unwrap();
    let mut at = Vec::new();
    for i in 0..1000u32 {
        at.push(log.append(&vec![i as u8; (i % 37) as usize]).unwrap());
    }
    assert_eq!(log.count(), 1000);
    assert_eq!(log.read_at(at[500]).unwrap(), vec![500u32 as u8; 500 % 37]);
    let all: Vec<Vec<u8>> = log.records().unwrap().map(Result::unwrap).collect();
    assert_eq!(all.len(), 1000);
    assert_eq!(all[999], vec![999u32 as u8; 999 % 37]);
    // a record bigger than the buffers
    let big = vec![7u8; 100_000];
    let off = log.append(&big).unwrap();
    assert_eq!(log.read_at(off).unwrap(), big);
    drop((log, store));
    assert_eq!(std::fs::read_dir(&r).unwrap().count(), 0);
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn the_sorter_agrees_with_a_stable_sort_at_every_budget_and_pushes_equal_keys_in_arrival_order() {
    let r = root("sort");
    for budget in [0usize, 1, 100, 5000, 1 << 20] {
        for n in [0usize, 1, 7, 500, 3000] {
            let store = support::store(&r);
            let mut seed = (n as u64) << 8 | budget as u64 & 0xff;
            let mut sorter = ExternalSorter::new(&store, budget);
            let mut model = Vec::new();
            for i in 0..n {
                let key = u128::from(lcg(&mut seed) % 50) << 70 | u128::from(lcg(&mut seed) % 3);
                let payload = (i as u32).to_le_bytes().repeat(1 + (lcg(&mut seed) % 5) as usize);
                model.push((key, payload.clone()));
                sorter.push(key, payload).unwrap();
            }
            model.sort_by_key(|m| m.0);
            let got: Vec<(u128, Vec<u8>)> = sorter.finish().unwrap().map(Result::unwrap).collect();
            assert_eq!(got, model, "budget {budget}, {n} records");
        }
    }
    assert_eq!(std::fs::read_dir(&r).unwrap().count(), 0, "every spill directory is gone");
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn more_runs_than_the_fan_in_are_merged_in_passes_and_stay_stable() {
    let r = root("passes");
    let store = support::store(&r);
    let mut sorter = ExternalSorter::new(&store, 1);
    let mut model = Vec::new();
    for i in 0..(64 * 3 + 5) as u32 {
        let key = u128::from(i % 4);
        model.push((key, i.to_le_bytes().to_vec()));
        sorter.push(key, i.to_le_bytes().to_vec()).unwrap();
    }
    model.sort_by_key(|m| m.0);
    let got: Vec<_> = sorter.finish().unwrap().map(Result::unwrap).collect();
    assert_eq!(got, model);
    drop(store);
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn a_truncated_run_is_an_error_and_not_a_short_result() {
    let r = root("trunc");
    let fs_store = support::spill_fs::FsStore::create(&r).unwrap();
    let dir = fs_store.path().to_path_buf();
    let store: Store = Arc::new(fs_store);
    let mut sorter = ExternalSorter::new(&store, 1);
    for i in 0..10u32 {
        sorter.push(u128::from(i), vec![7; 40]).unwrap();
    }
    let path = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    let len = std::fs::metadata(&path).unwrap().len();
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(len - 5).unwrap();
    let outcome: Result<Vec<_>, _> = sorter.finish().and_then(|s| s.collect());
    assert!(outcome.is_err());
    drop(store);
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn a_store_that_fails_is_an_error_at_the_call_and_not_a_panic() {
    #[derive(Debug)]
    struct Broken;
    impl SpillStore for Broken {
        fn create(&self) -> pvec_container::spill::SpillResult<pvec_container::spill::FileId> {
            Ok(pvec_container::spill::FileId(0))
        }
        fn append(
            &self,
            _: pvec_container::spill::FileId,
            _: &[u8],
        ) -> pvec_container::spill::SpillResult<()> {
            Err(pvec_container::spill::SpillError::corrupt("the disk is full"))
        }
        fn read_at(
            &self,
            _: pvec_container::spill::FileId,
            _: u64,
            _: &mut [u8],
        ) -> pvec_container::spill::SpillResult<usize> {
            Err(pvec_container::spill::SpillError::corrupt("the disk is gone"))
        }
        fn remove(&self, _: pvec_container::spill::FileId) -> pvec_container::spill::SpillResult<()> {
            Ok(())
        }
    }
    let store: Store = Arc::new(Broken);
    let mut sorter = ExternalSorter::new(&store, 1);
    assert!(sorter.push(1, vec![0; 10]).is_err(), "the run cannot be written");
    let mut log = Log::create(&store).unwrap();
    log.append(&[1, 2, 3]).unwrap();
    assert!(log.read_at(0).is_err());
}
