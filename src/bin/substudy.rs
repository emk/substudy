//! Command-line iterface to substudy.

extern crate rustc_serialize;
extern crate docopt;
extern crate env_logger;

extern crate substudy;

use docopt::Docopt;
use std::path::Path;
use std::process::exit;

use substudy::errors::Result;
use substudy::srt::SubtitleFile;
use substudy::align::combine_files;
use substudy::video;
use substudy::export;

const USAGE: &'static str = "
Subtitle processing tools for students of foreign languages

Usage: substudy clean <subs>
       substudy combine <foreign-subs> <native-subs>
       substudy export csv [--end-at=<secs>] <video> <foreign-subs> [<native-subs>]
       substudy export review [--end-at=<secs>] <video> <foreign-subs> [<native-subs>]
       substudy export tracks [--end-at=<secs>] <video> <foreign-subs>
       substudy list tracks <video>
       substudy --help
       substudy --version

For now, all subtitles must be in *.srt format. Many common encodings
will be automatically detected, but try converting to UTF-8 if you
have problems.
";

#[derive(Debug, RustcDecodable)]
struct Args {
    cmd_clean: bool,
    cmd_combine: bool,
    cmd_export: bool,
    cmd_csv: bool,
    cmd_review: bool,
    cmd_tracks: bool,
    arg_subs: String,
    arg_foreign_subs: String,
    arg_native_subs: Option<String>,
    arg_video: String,
    flag_end_at: Option<f32>,
    flag_version: bool,
}

fn export_type(args: &Args) -> &str {
    match *args {
        Args{cmd_csv: true, ..} => "csv",
        Args{cmd_review: true, ..} => "review",
        Args{cmd_tracks: true, ..} => "tracks",
        _ => panic!("Cannot determine export type: {:?}", args),
    }
}

// Choose and run the appropriate command.
fn run(args: &Args) -> Result<()> {
    match *args {
        Args{flag_version: true, ..} =>
            cmd_version(),
        Args{cmd_clean: true, arg_subs: ref path, ..} =>
            cmd_clean(&Path::new(path)),
        Args{cmd_combine: true, arg_foreign_subs: ref path1,
             arg_native_subs: Some(ref path2), ..} =>
            cmd_combine(&Path::new(path1), &Path::new(path2)),
        Args{cmd_export: true, arg_video: ref video_path,
             arg_foreign_subs: ref foreign_path,
             arg_native_subs: ref native_path, ..} =>
        {
            cmd_export(export_type(args), &Path::new(video_path),
                       &Path::new(foreign_path),
                       native_path.as_ref().map(|p| Path::new(p)),
                       args.flag_end_at)
        }
        Args{cmd_tracks: true, arg_video: ref path, ..} =>
            cmd_tracks(&Path::new(path)),
        _ => panic!("Unexpected argument combination: {:?}", args)
    }
}

fn cmd_version() -> Result<()> {
    println!("substudy {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

fn cmd_clean(path: &Path) -> Result<()> {
    let file1 = try!(SubtitleFile::cleaned_from_path(path));
    print!("{}", file1.to_string());
    Ok(())
}

fn cmd_combine(path1: &Path, path2: &Path) -> Result<()> {
    let file1 = try!(SubtitleFile::cleaned_from_path(path1));
    let file2 = try!(SubtitleFile::cleaned_from_path(path2));
    print!("{}", combine_files(&file1, &file2).to_string());
    Ok(())
}

fn cmd_tracks(path: &Path) -> Result<()> {
    let v = try!(video::Video::new(path));
    for stream in v.streams() {
        let lang = stream.language();
        let lang_str = lang.map(|l| l.as_str().to_owned())
            .unwrap_or("??".to_owned());
        println!("#{} {} {:?}", stream.index, &lang_str, stream.codec_type);
    }
    Ok(())
}

fn cmd_export(kind: &str, video_path: &Path, foreign_sub_path: &Path,
              native_sub_path: Option<&Path>,
              end_at: Option<f32>) ->
    Result<()>
{
    // Load our input files.
    let video = try!(video::Video::new(video_path));
    let foreign_subs = try!(SubtitleFile::cleaned_from_path(foreign_sub_path));
    let native_subs = match native_sub_path {
        None => None,
        Some(p) => Some(try!(SubtitleFile::cleaned_from_path(p))),
    };

    let mut exporter =
        try!(export::Exporter::new(video, foreign_subs, native_subs, kind, end_at));
    match kind {
        "csv" => try!(export::export_csv(&mut exporter)),
        "review" => try!(export::export_review(&mut exporter)),
        "tracks" => try!(export::export_tracks(&mut exporter)),
        _ => panic!("Uknown export type: {}", kind),
    }

    Ok(())
}

fn main() {
    env_logger::init().unwrap();

    // Parse our command-line arguments using docopt (very shiny).
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.decode())
        .unwrap_or_else(|e| e.exit());

    // Decide which command to run, and run it.
    if let Err(ref err) = run(&args) {
        // Print any error and exit with an error status.
        println!("{}", err);
        exit(1);
    }
}
