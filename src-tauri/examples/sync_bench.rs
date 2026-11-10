//! Speed check for the executor: creates test data in a temporary folder, mirrors it, then deletes it.
//! `cargo run --release --example sync_bench -- <scratch dir>`

use relax_lib::engine::compare::{compare, CompareOptions, FolderPairInput};
use relax_lib::engine::execute::{execute, DeletionPolicy, ExecOptions, PauseToken};
use relax_lib::engine::filter::Filter;
use relax_lib::engine::plan::{plan_pair, SyncConfig};
use relax_lib::engine::{winfs, CancelToken};
use std::fs;
use std::path::Path;
use std::time::Instant;

fn main() {
    let root = Path::new(&std::env::args().nth(1).expect("scratch dir")).join("relax-sync-bench");
    let (left, right) = (root.join("left"), root.join("right"));
    let _ = fs::remove_dir_all(&root);
    for i in 0..2000 {
        let dir = left.join(format!("folder{:02}", i % 40));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("small{i}.txt")), vec![b'x'; 8 * 1024]).unwrap();
    }
    fs::write(left.join("big.bin"), vec![7u8; 512 * 1024 * 1024]).unwrap();

    let input = [FolderPairInput {
        left: left.display().to_string(),
        right: right.display().to_string(),
    }];
    for verify in [false, true] {
        let _ = fs::remove_dir_all(&right);
        let started = Instant::now();
        let r = compare(
            &input,
            &CompareOptions::default(),
            &Filter::none(),
            &CancelToken::new(),
            &|_| {},
        )
        .unwrap()
        .remove(0);
        let plan = plan_pair(&r, &SyncConfig::default(), None);
        let opts = ExecOptions {
            deletion: DeletionPolicy::Permanent,
            verify_copies: verify,
        };
        let report = execute(
            &plan,
            &opts,
            &CancelToken::new(),
            &PauseToken::new(),
            &winfs::free_space,
            &|_| {},
            &|f| panic!("{f:?}"),
        )
        .unwrap();
        let secs = started.elapsed().as_secs_f64();
        println!(
            "verify={verify}: {} items, {:.0} MB in {secs:.2}s = {:.0} MB/s, failures: {}",
            report.done.create_right,
            report.bytes_copied as f64 / 1e6,
            report.bytes_copied as f64 / 1e6 / secs,
            report.failures.len()
        );
    }
    fs::remove_dir_all(&root).unwrap();
}
