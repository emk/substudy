//! Code shared between multiple exporters.

use std::convert::AsRef;
use std::default::Default;
use std::io::Write;
use std::ffi::OsStr;
use std::fmt::Write as fmt_Write;
use std::fs;
use std::path::{Path, PathBuf};

use align::align_available_files;
use errors::*;
use lang::Lang;
use srt::{Subtitle, SubtitleFile};
use time::{Period, ToTimestamp};
use video::{Extraction, ExtractionSpec, Id3Metadata, Video};

/// Take a platform-specific pathname fragment and turn it into a regular
/// Unicode string.
pub fn os_str_to_string(os_str: &OsStr) -> String {
    os_str.to_string_lossy().into_owned()
}

/// Information about a specific language.
pub struct LanguageResources {
    /// The subtitles associated with this language.
    pub subtitles: SubtitleFile,

    /// The language used in our subtitles, if we can figure it out.
    pub language: Option<Lang>,
}

impl LanguageResources {
    /// Create a list of per-language resources.
    fn new(subtitles: SubtitleFile) -> LanguageResources {
        let language = subtitles.detect_language();
        LanguageResources {
            subtitles: subtitles,
            language: language,
        }
    }
}

/// Information about media file and associated subtitles that the user
/// wants to export.
pub struct Exporter {
    /// The video file from which to extract images and audio clips.
    video: Video,

    /// Resources related to the foreign language.
    foreign: LanguageResources,

    /// Resources related to the native language, if any.
    native: Option<LanguageResources>,

    /// The base name to use when constructing other filenames.
    file_stem: String,

    /// The directory into which we want to output files.
    dir: PathBuf,

    /// A list of media files we want to extract from our video as
    /// efficiently as possible.
    extractions: Vec<Extraction>,

    /// The number of seconds to end processing at. If not set,
    /// process the entire video.
    end_at: Option<f32>,
}

impl Exporter {
    /// Create a new exporter for the specified video and subtitles.  The
    /// `label` parameter will be used to construct an output directory
    /// name.
    pub fn new(video: Video,
               foreign_subtitles: SubtitleFile,
               native_subtitles: Option<SubtitleFile>,
               label: &str,
               end_at: Option<f32>) ->
        Result<Exporter>
    {
        let foreign = LanguageResources::new(foreign_subtitles);
        let native = native_subtitles.map(|subs| {
            LanguageResources::new(subs)
        });

        // Construct a path `dir` which we'll use to store our output
        // files.  This is much uglier than it ought to be because paths
        // are not necessarily valid Unicode strings on all OSes, so we
        // need to jump through extra hoops.  We test for a directory's
        // existence using the `metadata` call, which is the only way to do
        // it in stable Rust.
        let file_stem = os_str_to_string(video.file_stem());
        let dir = Path::new("./").join(format!("{}_{}", &file_stem, label));
        if fs::metadata(&dir).is_ok() {
            return Err(err_str(format!("Directory already exists: {}",
                                       &dir.to_string_lossy())));
        }
        try!(fs::create_dir_all(&dir));

        Ok(Exporter {
            video: video,
            foreign: foreign,
            native: native,
            file_stem: file_stem,
            dir: dir,
            extractions: vec!(),
            end_at: end_at,
        })
    }

    /// The base name of this file, with the directory and file extension
    /// removed.
    pub fn file_stem(&self) -> &str {
        &self.file_stem
    }

    /// Return a title for this video.
    pub fn title(&self) -> &str {
        &self.file_stem
    }

    /// Get the video we're exporting.
    pub fn video(&self) -> &Video {
        &self.video
    }

    /// Get data related to the foreign language.
    pub fn foreign(&self) -> &LanguageResources {
        &self.foreign
    }

    /// Get data related to the native language.
    pub fn native(&self) -> Option<&LanguageResources> {
        self.native.as_ref()
    }

    /// Get the number of seconds to end processing at.
    pub fn end_at(&self) -> Option<f32> {
        self.end_at
    }

    /// Align our two sets of subtitles.
    pub fn align(&self) -> Vec<(Option<Subtitle>, Option<Subtitle>)> {
        align_available_files(&self.foreign.subtitles,
                              self.native.as_ref().map(|n| &n.subtitles))
    }

    /// Construct a path to an extracted media file, including timestamps
    /// and language information as appropriate.
    fn media_path<T: ToTimestamp>(&self, timestamp: T,
                                  lang: Option<Lang>, extension: &str) ->
        PathBuf
    {
        let mut file_name =
            format!("{}_{}", &self.file_stem, timestamp.to_file_timestamp());
        if let Some(l) = lang {
            write!(&mut file_name, ".{}", l.as_str()).unwrap();
        }
        write!(&mut file_name, ".{}", extension).unwrap();
        self.dir.join(file_name)
    }

    /// Schedule an export of the image at the specified time code.
    /// Returns the path to which the image will be written.
    pub fn schedule_image_export(&mut self, time: f32) -> String {
        let path = self.media_path(time, None, "jpg");
        self.extractions.push(Extraction {
            path: path.clone(),
            spec: ExtractionSpec::Image(time),
        });
        os_str_to_string(path.file_name().unwrap())
    }

    /// Schedule an export of the audio at the specified time period.
    /// Returns the path to which the audio will be written.
    pub fn schedule_audio_export(&mut self, lang: Option<Lang>,
                                 period: Period) ->
        String
    {
        self.schedule_audio_export_ext(lang, period, Default::default())
    }

    /// Schedule an export of the audio at the specified time period, using
    /// the specified metadata.  Returns the path to which the audio will
    /// be written.
    pub fn schedule_audio_export_ext(&mut self, lang: Option<Lang>,
                                     period: Period, metadata: Id3Metadata) ->
        String
    {
        let path = self.media_path(period, lang, "mp3");
        let stream = lang.and_then(|l| self.video.audio_for(l));
        self.extractions.push(Extraction {
            path: path.clone(),
            spec: ExtractionSpec::Audio(stream, period, metadata),
        });
        os_str_to_string(path.file_name().unwrap())
    }

    /// Write a raw chunk of bytes to a file in our export directory.
    pub fn export_data_file<P>(&self, rel_path: P, data: &[u8]) ->
        Result<()>
        where P: AsRef<Path>
    {
        let mut f = try!(fs::File::create(self.dir.join(rel_path.as_ref())));
        try!(f.write_all(data));
        Ok(())
    }

    /// Finish all scheduled exports.
    pub fn finish_exports(&mut self) -> Result<()> {
        try!(self.video.extract(&self.extractions));
        Ok(())
    }
}
