extern crate env_logger;
extern crate glob;
#[macro_use]
extern crate log;
extern crate vobsub;

use std::fs;
use std::io::prelude::*;
use std::path::Path;

// To run this test, use `cargo test -- --ignored`.  This tests against a
// larger selection of *.sub files in our private corpus, which is
// unfortunately not open source.
#[test]
#[ignore]
fn private_corpus() {
    let _ = env_logger::init();

    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };
    for entry in glob::glob_with("../private/**/*.sub", &options).unwrap() {
        let entry = entry.unwrap();
        process_file(&entry, false);
    }
}

// To run this test, use `cargo test -- --ignored`.  This tests against a
// larger selection of *.sub files in our private corpus, which is
// unfortunately not open source.
#[test]
#[ignore]
fn private_corpus_by_chunks() {
    let _ = env_logger::init();

    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };
    for entry in glob::glob_with("../private/**/*.sub", &options).unwrap() {
        let entry = entry.unwrap();
        process_file_by_chunks(&entry);
    }
}

// This corpus was generated using `cargo fuzz`, and it represents all the
// crashes that we've found so far.
#[test]
fn error_corpus() {
    let _ = env_logger::init();

    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };
    for entry in glob::glob_with("../fixtures/invalid/*", &options).unwrap() {
        let entry = entry.unwrap();
        process_file(&entry, true);
    }
}

fn process_file(path: &Path, expect_err: bool) {
    debug!("Processing {}", path.display());
    let mut f = fs::File::open(path).unwrap();
    let mut buffer = vec![];
    f.read_to_end(&mut buffer).unwrap();
    let count = vobsub::subtitles(&buffer)
        .map(|s| {
            assert_eq!(s.is_err(), expect_err);
        })
        .count();
    debug!("Found {} subtitles", count);
}

fn process_file_by_chunks(path: &Path) {
    use std::mem::swap;

    debug!("Processing {}", path.display());
    let mut f = fs::File::open(path).unwrap();

    // Use 2 buffers in order to simulate a cheap ring buffer.
    let capacity = 4096;
    let mut buffer = vec![0; capacity];
    let mut temp_buffer = vec![0; capacity];

    let mut upper = 0;

    let mut subs = vobsub::SubtitlesFromChunks::new();
    let mut count = 0;
    loop {
        let offset = subs.offset();
        let remaining = upper - offset;
        if remaining > 0 {
            // Copy bytes that are left to process at the beggining of `temp_buffer`
            // and swap buffers so that `buffer` is always the working buffer.
            temp_buffer[..remaining].copy_from_slice(&buffer[offset..offset + remaining]);
            swap(&mut temp_buffer, &mut buffer);
        }

        let read_len = f.read(&mut buffer[remaining..]).expect("Failed to read file");
        if read_len == 0 {
            // No more data to read.
            break;
        }
        upper = remaining + read_len;

        let mut sub_iter = subs.iter(&buffer[0..upper]);
        for sub in sub_iter.next() {
            match sub {
                Ok(_) => count += 1,
                Err(err) => panic!("Unexpected error while parsing: {:?}", err),
            }
        }
    }

    debug!("Found {} subtitles", count);
}