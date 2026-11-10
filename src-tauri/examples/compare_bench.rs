//! Speed check: `cargo run --release --example compare_bench -- <left> <right>`
//! Only reads folders; never changes anything.

use relax_lib::engine::compare::{compare, CompareOptions, FolderPairInput};
use relax_lib::engine::filter::Filter;
use relax_lib::engine::CancelToken;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = [FolderPairInput {
        left: args[0].clone(),
        right: args[1].clone(),
    }];
    let start = Instant::now();
    let result = compare(
        &input,
        &CompareOptions::default(),
        &Filter::default(),
        &CancelToken::new(),
        &|_| {},
    );
    let elapsed = start.elapsed();
    match result {
        Ok(r) => {
            let s = &r[0].stats;
            println!(
                "{:.2}s  left: {} files / {} folders / {:.1} GB   right: {} files   pairs: {}   equal: {}   errors: {}   links skipped: {}",
                elapsed.as_secs_f64(),
                s.left_files,
                s.left_folders,
                s.left_bytes as f64 / 1e9,
                s.right_files,
                r[0].pairs.len(),
                s.equal,
                r[0].errors.len(),
                r[0].skipped_links.len()
            );
        }
        Err(e) => println!("error: {e}"),
    }
}
