//! Integration tests for our command-line interface.  We actually run the
//! binary and make sure it produces the expected output.

extern crate cli_test_dir;

use cli_test_dir::TestDir;
use std::str::from_utf8;

#[test]
fn show_help() {
    let testdir = TestDir::new("substudy", "show_help");
    let output = testdir.cmd()
        .arg("--help")
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    assert!(from_utf8(&output.stdout).unwrap().find("substudy --help").is_some());
}

#[test]
fn show_version() {
    let testdir = TestDir::new("substudy", "show_version");
    let output = testdir.cmd()
        .arg("--version")
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    assert!(from_utf8(&output.stdout).unwrap().find("substudy ").is_some());
}

#[test]
fn cmd_clean() {
    let testdir = TestDir::new("substudy", "cmd_clean");
    let output = testdir.cmd()
        .arg("clean")
        .arg(testdir.src_path("fixtures/sample.en.srt"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    assert!(from_utf8(&output.stdout).unwrap().find("Yay!").is_some());
}

#[test]
fn cmd_combine() {
    let testdir = TestDir::new("substudy", "cmd_combine");
    let output = testdir.cmd()
        .arg("combine")
        .arg(testdir.src_path("fixtures/sample.es.srt"))
        .arg(testdir.src_path("fixtures/sample.en.srt"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    assert!(from_utf8(&output.stdout).unwrap().find("Yay!").is_some());
    assert!(from_utf8(&output.stdout).unwrap().find("¡Si!").is_some());
}

static FIRST_LINE: &'static str = "¡Si! ¡Aang ha vuelto!,Yay! Yay! Aang\'s back!,,,¡Lo sabía!,I knew it!";

static LAST_LINE: &'static str = "\"El no ha hecho nada, Sokka, fué un accidente.\",Aang didn\'t do anything.,harás que vengan a nosotros.,\"You\'re leading them straight to us, aren\'t you?\"";

#[test]
fn cmd_export_csv() {
    let testdir = TestDir::new("substudy", "cmd_export_csv");
    let output = testdir.cmd()
        .args(&["export", "csv"])
        .arg(testdir.src_path("fixtures/empty.mp4"))
        .arg(testdir.src_path("fixtures/sample.es.srt"))
        .arg(testdir.src_path("fixtures/sample.en.srt"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    testdir.expect_path("empty_csv/cards.csv");
    testdir.expect_path("empty_csv/empty_00063_496.jpg");
    testdir.expect_path("empty_csv/empty_00060_828-00066_164.es.mp3");
    testdir.expect_contains("empty_csv/cards.csv", FIRST_LINE);
    testdir.expect_contains("empty_csv/cards.csv", LAST_LINE);
}

#[test]
fn cmd_export_csv_end_at() {
    let testdir = TestDir::new("substudy", "cmd_export_csv_end_at");
    let output = testdir.cmd()
        .args(&["export", "csv", "--end-at=65"])
        .arg(testdir.src_path("fixtures/empty.mp4"))
        .arg(testdir.src_path("fixtures/sample.es.srt"))
        .arg(testdir.src_path("fixtures/sample.en.srt"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    testdir.expect_path("empty_csv/cards.csv");
    testdir.expect_path("empty_csv/empty_00063_496.jpg");
    testdir.expect_path("empty_csv/empty_00060_828-00066_164.es.mp3");
    testdir.expect_contains("empty_csv/cards.csv", FIRST_LINE);
}

#[test]
fn cmd_export_review() {
    let testdir = TestDir::new("substudy", "cmd_export_review");
    let output = testdir.cmd()
        .args(&["export", "review"])
        .arg(testdir.src_path("fixtures/empty.mp4"))
        .arg(testdir.src_path("fixtures/sample.es.srt"))
        .arg(testdir.src_path("fixtures/sample.en.srt"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    testdir.expect_path("empty_review/index.html");
    testdir.expect_path("empty_review/empty_00063_496.jpg");
    testdir.expect_path("empty_review/empty_00061_828-00065_164.es.mp3");
}

#[test]
fn cmd_export_review_end_at() {
    let testdir = TestDir::new("substudy", "cmd_export_review_end_at");
    let output = testdir.cmd()
        .args(&["export", "review", "--end-at=65"])
        .arg(testdir.src_path("fixtures/empty.mp4"))
        .arg(testdir.src_path("fixtures/sample.es.srt"))
        .arg(testdir.src_path("fixtures/sample.en.srt"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    testdir.expect_path("empty_review/index.html");
    testdir.expect_path("empty_review/empty_00063_496.jpg");
    testdir.expect_path("empty_review/empty_00061_828-00065_164.es.mp3");
}

#[test]
fn cmd_export_tracks() {
    let testdir = TestDir::new("substudy", "cmd_export_tracks");
    let output = testdir.cmd()
        .args(&["export", "tracks"])
        .arg(testdir.src_path("fixtures/empty.mp4"))
        .arg(testdir.src_path("fixtures/sample.es.srt"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    testdir.expect_path("empty_tracks/playlist.m3u8");
    testdir.expect_path("empty_tracks/empty_00059_828-00067_164.es.mp3");
}

#[test]
fn cmd_export_tracks_end_at() {
    let testdir = TestDir::new("substudy", "cmd_export_tracks_end_at");
    let output = testdir.cmd()
        .args(&["export", "tracks", "--end-at=65"])
        .arg(testdir.src_path("fixtures/empty.mp4"))
        .arg(testdir.src_path("fixtures/sample.es.srt"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    testdir.expect_path("empty_tracks/playlist.m3u8");
    testdir.expect_path("empty_tracks/empty_00059_828-00067_164.es.mp3");
}

#[test]
fn cmd_list_tracks() {
    let testdir = TestDir::new("substudy", "cmd_export_tracks");
    let output = testdir.cmd()
        .args(&["list", "tracks"])
        .arg(testdir.src_path("fixtures/empty.mp4"))
        .output()
        .expect("could not run substudy");
    assert!(output.status.success());
    assert!(from_utf8(&output.stdout).unwrap().find("#1 es Audio").is_some());
}
