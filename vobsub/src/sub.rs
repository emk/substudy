//! # Subtitle data parsing.
//!
//! For background, see [this documentation on the DVD subtitle format][subs].
//!
//! [subs]: http://sam.zoy.org/writings/dvd/subtitles/

use cast;
use common_failures::prelude::*;
use nom::{be_u16, IResult};
use std::fmt;
use std::rc::Rc;
use std::cell::RefCell;

use errors::VobsubError;
use idx;
use image::{ImageBuffer, Rgba, RgbaImage};
use img::{decompress, Size};
use mpeg2::pes::PES_PACKET_HEADER_MIN_LEN;
use mpeg2::ps;
use mpeg2::ps::{NeededOpt, PES_PACKET_NEEDLE_LEN, PesPacketError};
use util::BytesFormatter;

/// The default time between two adjacent subtitles if no end time is
/// provided.  This is chosen to be a value that's usually representable in
/// SRT format, barring rounding errors.
const DEFAULT_SUBTITLE_SPACING: f64 = 0.001;

/// The default length of a subtitle if no end time is provided and no
/// subtitle follows immediately after.
const DEFAULT_SUBTITLE_LENGTH: f64 = 5.0;

/// Parse four 4-bit palette entries.
named!(palette_entries<[u8; 4]>, bits!(count_fixed!(u8, take_bits!(u8, 4), 4)));

#[test]
fn parse_palette_entries() {
    assert_eq!(palette_entries(&[0x03, 0x10][..]),
               IResult::Done(&[][..], [0x00, 0x03, 0x01, 0x00]));
}

/// Location at which to display the subtitle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coordinates {
    x1: u16,
    y1: u16,
    x2: u16,
    y2: u16,
}

impl Coordinates {
    /// The leftmost edge of the subtitle.
    pub fn left(&self) -> u16 {
        self.x1
    }

    /// The rightmost edge of the subtitle.
    pub fn top(&self) -> u16 {
        self.y1
    }

    /// The width of the subtitle.
    pub fn width(&self) -> u16 {
        self.x2 + 1 - self.x1
    }

    /// The height of the subtitle.
    pub fn height(&self) -> u16 {
        self.y2 + 1 - self.y1
    }

    /// The size of the subtitle.
    fn size(&self) -> Size {
        Size {
            w: cast::usize(self.width()),
            h: cast::usize(self.height()),
        }
    }
}

/// Parse a 12-bit coordinate value.
named!(coordinate<(&[u8], usize), u16>, take_bits!(u16, 12));

/// Parse four 12-bit coordinate values as a rectangle (with right and
/// bottom coordinates inclusive).
named!(coordinates<Coordinates>,
    bits!(
        do_parse!(
            x1: call!(coordinate) >>
            x2: call!(coordinate) >>
            y1: call!(coordinate) >>
            y2: call!(coordinate) >>
            (Coordinates {
                x1: x1,
                y1: y1,
                x2: x2,
                y2: y2,
            })
        )
    )
);

/// Parse a pair of 16-bit RLE offsets.
named!(rle_offsets<[u16; 2]>, bits!(count_fixed!(u16, take_bits!(u16, 16), 2)));

/// Individual commands which may appear in a control sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ControlCommand<'a> {
    /// Should this subtitle be displayed even if subtitles are turned off?
    Force,
    /// We should start displaying the subtitle at the `date` for this
    /// `ControlSequence`.
    StartDate,
    /// We should stop displaying the subtitle at the `date` for this
    /// `ControlSequence`.
    StopDate,
    /// Map each of the 4 colors in this subtitle to a 4-bit palette.
    Palette([u8; 4]),
    /// Map each of the 4 colors in this subtitle to 4 bits of alpha
    /// channel data.
    Alpha([u8; 4]),
    /// Coordinates at which to display the subtitle.
    Coordinates(Coordinates),
    /// Offsets of first and second scan line in our data buffer.  Note
    /// that the data buffer stores alternating scan lines separately, so
    /// these are the first line in each of the two chunks.
    RleOffsets([u16; 2]),
    /// Unsupported trailing data that we don't know how to parse.
    Unsupported(&'a [u8]),
}

/// Parse a single command in a control sequence.
named!(control_command<ControlCommand>,
    alt!(
        value!(ControlCommand::Force, tag!(&[0x00])) |
        value!(ControlCommand::StartDate, tag!(&[0x01])) |
        value!(ControlCommand::StopDate, tag!(&[0x02])) |
        map!(preceded!(tag!(&[0x03]), call!(palette_entries)),
             ControlCommand::Palette) |
        map!(preceded!(tag!(&[0x04]), call!(palette_entries)),
             ControlCommand::Alpha) |
        map!(preceded!(tag!(&[0x05]), call!(coordinates)),
             ControlCommand::Coordinates) |
        map!(preceded!(tag!(&[0x06]), call!(rle_offsets)),
             ControlCommand::RleOffsets) |
        // We only capture this so we have something to log.  Note that we
        // may not find the _true_ `ControlCommand::End` in this case, but
        // that doesn't matter, because we'll use the `next` field of
        // `ControlSequence` to find the next `ControlSequence`.
        map!(take_until!(&[0xff][..]), ControlCommand::Unsupported)
    )
);

/// The end of a control sequence.
named!(control_command_end, tag!(&[0xff]));

/// The control packet for a subtitle.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlSequence<'a> {
    /// The time associated with this control sequence, specified in
    /// 1/100th of a second after the Presentation Time Stamp for this
    /// subtitle's packet.
    date: u16,
    /// The offset of the next control sequence, relative to ???.  If this
    /// equals the offset of the current control sequence, this is the last
    /// control packet.
    next: u16,
    /// Individual commands in this sequence.
    commands: Vec<ControlCommand<'a>>,
}

/// Parse a single control sequence.
named!(control_sequence<ControlSequence>,
    do_parse!(
        date: call!(be_u16) >>
        next: call!(be_u16) >>
        commands: many_till!(call!(control_command),
                             call!(control_command_end)) >>
        (ControlSequence {
            date: date,
            next: next,
            commands: commands.0,
        })
    )
);

#[test]
fn parse_control_sequence() {
    let input_1 = &[
        0x00, 0x00, 0x0f, 0x41,
        0x01,
        0x03, 0x03, 0x10,
        0x04, 0xff, 0xf0,
        0x05, 0x29, 0xb4, 0xe6, 0x3c, 0x54, 0x00,
        0x06, 0x00, 0x04, 0x07, 0x7b,
        0xff
    ][..];
    let expected_1 = ControlSequence {
        date: 0x0000,
        next: 0x0f41,
        commands: vec![
            ControlCommand::StartDate,
            ControlCommand::Palette([0x0, 0x3, 0x1, 0x0]),
            ControlCommand::Alpha([0xf, 0xf, 0xf, 0x0]),
            ControlCommand::Coordinates(Coordinates {
                x1: 0x29b,
                x2: 0x4e6,
                y1: 0x3c5,
                y2: 0x400,
            }),
            ControlCommand::RleOffsets([0x0004, 0x077b]),
        ]
    };
    assert_eq!(control_sequence(input_1),
               IResult::Done(&[][..], expected_1));

    let input_2 = &[
        0x00, 0x77, 0x0f, 0x41,
        0x02,
        0xff
    ][..];
    let expected_2 = ControlSequence {
        date: 0x0077,
        next: 0x0f41,
        commands: vec![ControlCommand::StopDate],
    };
    assert_eq!(control_sequence(input_2),
               IResult::Done(&[][..], expected_2));

    // An out of order example.
    let input_3 = &[
        0x00, 0x00, 0x0b, 0x30,
        0x01,
        0x00,
        // ...other commands would appear here...
        0xff,
    ][..];
    let expected_3 = ControlSequence {
        date: 0x0000,
        next: 0x0b30,
        commands: vec![
            ControlCommand::StartDate,
            ControlCommand::Force,
        ],
    };
    assert_eq!(control_sequence(input_3),
               IResult::Done(&[][..], expected_3));
}

/// A single subtitle.
#[derive(Clone, PartialEq)]
pub struct Subtitle {
    /// Start time of subtitle, in seconds.
    start_time: f64,
    /// End time of subtitle, in seconds.  This may be missing from certain
    /// subtitles.
    end_time: Option<f64>,
    /// Should this subtitle be shown even when subtitles are off?
    force: bool,
    /// Coordinates at which to display the subtitle.
    coordinates: Coordinates,
    /// Map each of the 4 colors in this subtitle to a 4-bit palette.
    palette: [u8; 4],
    /// Map each of the 4 colors in this subtitle to 4 bits of alpha
    /// channel data.
    alpha: [u8; 4],
    /// Our decompressed image, stored with 2 bits per byte in row-major
    /// order, that can be used as indices into `palette` and `alpha`.
    raw_image: Vec<u8>,
}

impl Subtitle {
    /// Start time of subtitle, in seconds.
    pub fn start_time(&self) -> f64 {
        self.start_time
    }

    /// End time of subtitle, in seconds.  This may be missing from certain
    /// subtitles.
    pub fn end_time(&self) -> f64 {
        self.end_time
            .expect("end time should have been set before returning subtitle")
    }

    /// Should this subtitle be shown even when subtitles are off?
    pub fn force(&self) -> bool {
        self.force
    }

    /// Coordinates at which to display the subtitle.
    pub fn coordinates(&self) -> &Coordinates {
        &self.coordinates
    }

    /// Map each of the 4 colors in this subtitle to a 4-bit palette.
    pub fn palette(&self) -> &[u8; 4] {
        &self.palette
    }

    /// Map each of the 4 colors in this subtitle to 4 bits of alpha
    /// channel data.
    pub fn alpha(&self) -> &[u8; 4] {
        &self.alpha
    }

    /// Our decompressed image, stored with 2 bits per byte in row-major
    /// order, that can be used as indices into `palette` and `alpha`.
    pub fn raw_image(&self) -> &[u8] {
        &self.raw_image
    }

    /// Decompress to subtitle to an RBGA image.
    pub fn to_image(&self, palette: &idx::Palette) -> RgbaImage {
        let width = cast::u32(self.coordinates.width());
        let height = cast::u32(self.coordinates.height());
        ImageBuffer::from_fn(width, height, |x, y| {
            let offset = cast::usize(y*width + x);
            // We need to subtract the raw index from 3 to get the same
            // results as everybody else.  I found this by inspecting the
            // Handbrake subtitle decoding routines.
            let px = cast::usize(3-self.raw_image[offset]);
            let rgb = palette[cast::usize(self.palette[px])].data;
            let a = self.alpha[px];
            let aa = a << 4 | a;
            Rgba { data: [rgb[0], rgb[1], rgb[2], aa] }
        })
    }
}

impl<'a> fmt::Debug for Subtitle {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Subtitle")
            .field("start_time", &self.start_time)
            .field("end_time", &self.end_time)
            .field("force", &self.force)
            .field("coordinates", &self.coordinates)
            .field("palette", &self.palette)
            .field("alpha", &self.alpha)
            .finish()
    }
}

/// Parse a single `u16` value from a buffer.  We don't use `nom` for this
/// because it has an inconvenient error type.
fn parse_be_u16_as_usize(buff: &[u8]) -> Result<(&[u8], usize)> {
    if buff.len() < 2 {
        Err(format_err!("unexpected end of buffer while parsing 16-bit size"))
    } else {
        Ok((&buff[2..], usize::from(buff[0]) << 8 | usize::from(buff[1])))
    }
}

/// Parse a subtitle.
fn subtitle(raw_data: &[u8], base_time: f64) -> Result<Subtitle> {
    // This parser is somewhat non-standard, because we need to work with
    // explicit offsets into `packet` in several places.

    // Figure out where our control data starts.
    if raw_data.len() < 2 {
        return Err(format_err!("unexpected end of subtitle data"));
    }
    let (_, initial_control_offset) = parse_be_u16_as_usize(&raw_data[2..])?;

    // Declare data we want to collect from our control packets.
    let mut start_time = None;
    let mut end_time = None;
    let mut force = false;
    let mut coordinates = None;
    let mut palette = None;
    let mut alpha = None;
    let mut rle_offsets = None;

    // Loop over the individual control sequences.
    let mut control_offset = initial_control_offset;
    loop {
        trace!("looking for control sequence at: 0x{:x}", control_offset);
        if control_offset >= raw_data.len() {
            return Err(format_err!("control offset is 0x{:x}, but packet is only 0x{:x} \
                                   bytes",
                                   control_offset,
                                   raw_data.len()));
        }

        let control_data = &raw_data[control_offset..];
        match control_sequence(control_data) {
            IResult::Done(_, control) => {
                trace!("parsed control sequence: {:?}", &control);

                // Extract as much data as we can from this control sequence.
                let time = base_time + f64::from(control.date) / 100.0;
                for command in &control.commands {
                    match *command {
                        ControlCommand::Force => {
                            force = true;
                        }
                        ControlCommand::StartDate => {
                            start_time = start_time.or(Some(time));
                        }
                        ControlCommand::StopDate => {
                            end_time = end_time.or(Some(time));
                        }
                        ControlCommand::Palette(p) => {
                            palette = palette.or(Some(p));
                        }
                        ControlCommand::Alpha(a) => {
                            alpha = alpha.or(Some(a));
                        }
                        ControlCommand::Coordinates(ref c) => {
                            // Check for weird bounding boxes.  Ideally we
                            // would do this while parsing, but I can't
                            // figure out how to get nom to do what I want.
                            // Later on, we assume that all bounding boxes
                            // have non-negative width and height and we'll
                            // crash if they don't.
                            if c.x2 <= c.x1 || c.y2 <= c.y1 {
                                return Err(format_err!("invalid bounding box"));
                            }
                            coordinates = coordinates.or(Some(c.clone()));
                        }
                        ControlCommand::RleOffsets(r) => {
                            rle_offsets = Some(r);
                        }
                        ControlCommand::Unsupported(b) => {
                            warn!("unsupported control sequence: {:?}",
                                  BytesFormatter(b));
                        }
                    }
                }

                // Figure out where to look for the next control sequence,
                // if any.
                let next_control_offset = cast::usize(control.next);
                if next_control_offset == control_offset {
                    // This points back at us, so we're the last packet.
                    break;
                } else if next_control_offset < control_offset {
                    return Err(format_err!("control offset went backwards"));
                } else {
                    control_offset = next_control_offset;
                }
            }
            IResult::Incomplete(_) => {
                return Err(format_err!("incomplete control packet"));
            }
            IResult::Error(err) => {
                return Err(format_err!("error parsing subtitle: {:?}", err));
            }
        }
    }

    // Make sure we found all the control commands that we expect.
    let start_time = start_time.ok_or_else(|| {
        format_err!("no start time for subtitle")
    })?;
    let coordinates = coordinates.ok_or_else(|| {
        format_err!("no coordinates for subtitle")
    })?;
    let palette = palette.ok_or_else(|| -> Error {
        format_err!("no palette for subtitle")
    })?;
    let alpha = alpha.ok_or_else(|| -> Error {
        format_err!("no alpha for subtitle")
    })?;
    let rle_offsets = rle_offsets.ok_or_else(|| -> Error {
        format_err!("no RLE offsets for subtitle")
    })?;

    // Decompress our image.
    //
    // We know the starting points of each set of scan lines, but we don't
    // really know where they end, because encoders like to reuse bytes
    // that they're already using for something else.  For example, the
    // last few bytes of the first set of scan lines may overlap with the
    // first bytes of the second set of scanlines, and the last bytes of
    // the second set of scan lines may overlap with the start of the
    // control sequence.  For now, we limit it to the first two bytes of
    // the control packet, which are usually `[0x00, 0x00]`.  (We might
    // actually want to remove `end` entirely here and allow the scan lines
    // to go to the end of the packet, but I've never seen that in
    // practice.)
    let start_0 = cast::usize(rle_offsets[0]);
    let start_1 = cast::usize(rle_offsets[1]);
    let end = cast::usize(initial_control_offset+2);
    if start_0 > start_1 || start_1 > end {
        return Err(format_err!("invalid scan line offsets"));
    }
    let image = decompress(coordinates.size(),
                           [&raw_data[start_0..end],
                            &raw_data[start_1..end]])?;

    // Return our parsed subtitle.
    let result = Subtitle {
        start_time: start_time,
        end_time: end_time,
        force: force,
        coordinates: coordinates,
        palette: palette,
        alpha: alpha,
        raw_image: image,
    };
    trace!("Parsed subtitle: {:?}", &result);
    Ok(result)
}

const DATA_LENGTH_LEN: usize = 2;

struct PartialSubtitle {
    packets_data: Vec<u8>,
    base_time: f64,
    substream_id: u8,
    wanted: usize,
}

impl PartialSubtitle {
    fn new(packet: ps::PesPacket) -> Result<Self> {
        let pts_dts = match packet.pes_packet.header_data.pts_dts {
            Some(v) => v,
            None => return Err(format_err!("Found subtitle without timing info")),
        };

        // Figure out how many total bytes we'll need to collect from one
        // or more PES packets, and collect the first chunk into a buffer.
        if packet.pes_packet.data.len() < DATA_LENGTH_LEN {
            return Err(Error::from(VobsubError::IncompleteInput{
                needed: DATA_LENGTH_LEN.into(),
            }));
        }

        Ok(PartialSubtitle {
            packets_data: packet.pes_packet.data.to_owned(),
            base_time: pts_dts.pts.to_seconds(),
            substream_id: packet.pes_packet.substream_id,
            wanted: usize::from(packet.pes_packet.data[0]) << 8
                | usize::from(packet.pes_packet.data[1]),
        })
    }

    fn have_packet(&mut self, packet: ps::PesPacket) -> bool {
        // Make sure this is part of the same subtitle stream.  This is
        // mostly just paranoia; I don't expect this to happen.
        if packet.pes_packet.substream_id != self.substream_id {
            warn!("Found subtitle for stream 0x{:x} while looking for 0x{:x}",
                  packet.pes_packet.substream_id, self.substream_id);
            return true;
        }

        // Add the extra bytes to our buffer.
        self.packets_data.extend_from_slice(packet.pes_packet.data);

        if !self.more_data_wanted() {
            // Expected data retrieved.

            // Check to make sure we didn't get too _many_ bytes.  Again, this
            // is paranoia.
            if self.packets_data.len() > self.wanted {
                warn!("Found 0x{:x} bytes of data in subtitle packet, wanted 0x{:x}",
                      self.packets_data.len(), self.wanted);
                self.packets_data.truncate(self.wanted);
            }

            false
        } else {
            true
        }
    }

    fn more_data_wanted(&self) -> bool {
        self.packets_data.len() < self.wanted
    }
}

/// A subtitle parsing context that keeps track of the parsing between chunks.
#[derive(Default)]
struct SubtitlesContext {
    is_chunked: bool,
    offset: usize,
    needed: NeededOpt,
    partial_subtitle: Option<PartialSubtitle>,
}

impl SubtitlesContext {
    /// Handle this new packet.
    /// Return `Ok(Some(partial_subtitle))` if the subtitle data is complete,
    /// and `Ok(None)` if other packets are needed.
    fn have_packet(
        &mut self,
        packet: ps::PesPacket,
        new_offset: usize,
    ) -> Result<Option<PartialSubtitle>> {
        self.pepare_for_new_packet();

        if self.partial_subtitle.is_none() {
            // First packet.
            match PartialSubtitle::new(packet) {
                Ok(partial_subtitle) => {
                    self.offset = new_offset;
                    if partial_subtitle.more_data_wanted() {
                        self.update_needed(&partial_subtitle);
                        self.partial_subtitle = Some(partial_subtitle);
                        Ok(None)
                    } else {
                        Ok(Some(partial_subtitle))
                    }
                }
                Err(err) => {
                    if let Some(ref vobsub_err) = err.downcast_ref::<VobsubError>() {
                        if let VobsubError::IncompleteInput { ref needed } = **vobsub_err {
                            self.needed = *needed;
                        }
                    }
                    return Err(err);
                }
            }
        } else {
            // Additional packet.
            let mut partial_subtitle = self.partial_subtitle.take().unwrap();
            let more_data_wanted = partial_subtitle.have_packet(packet);
            self.offset = new_offset;
            if more_data_wanted {
                self.update_needed(&partial_subtitle);
                self.partial_subtitle = Some(partial_subtitle);
                Ok(None)
            } else {
                Ok(Some(partial_subtitle))
            }
        }
    }

    fn update_needed(&mut self, partial_subtitle: &PartialSubtitle) {
        // Guestimate the needed length making sure we get
        // a minimum length to grab a meaningfull header.
        // Note: this doesn't include PS header size, but
        // that should be sufficient to iteratively progress
        // in a stream.
        self.needed = (
            PES_PACKET_NEEDLE_LEN + PES_PACKET_HEADER_MIN_LEN + DATA_LENGTH_LEN
            + partial_subtitle.wanted - partial_subtitle.packets_data.len()
        ).into();
    }

    fn pepare_for_new_chunk(&mut self) {
        self.offset = 0;
        self.pepare_for_new_packet();
    }

    fn pepare_for_new_packet(&mut self) {
        self.needed = None.into();
    }

    fn set_unrecoverable_error(&mut self) {
        self.needed = None.into();
        self.partial_subtitle.take();
    }

    fn expecting_chunk(&self) -> bool {
        self.needed.is_some() || self.partial_subtitle.is_some()
    }
}

/// An internal iterator over subtitles.  These subtitles may not have a
/// valid `end_time`, so we'll try to fix them up before letting the user
/// see them.
struct SubtitlesInternal<'a> {
    pes_packets: ps::PesPackets<'a>,
    // We use `Rc<RefCell>>` for the `SubtitlesContext` because we have 2 use cases:
    // 1. Full stream processing. The `SubtitlesContext` is valid
    //    until the `Subtitles` instance is destroyed.
    // 2. Chunk processing. The `Subtitles` instances liftetime is
    //    tied to the input chunk's life time. The `SubtitlesContext`
    //    outlives the `Subtitles` instances.
    context: Rc<RefCell<SubtitlesContext>>,
}

impl<'a> SubtitlesInternal<'a> {
    fn new(input: &'a [u8], context: &Rc<RefCell<SubtitlesContext>>) -> Self {
        SubtitlesInternal {
            pes_packets: ps::pes_packets(input),
            context: context.clone(),
        }
    }
}

impl<'a> Iterator for SubtitlesInternal<'a> {
    type Item = Result<Subtitle>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let packet: ps::PesPacket = match self.pes_packets.next() {
                Some(Ok(value)) => value,
                None => return None,
                Some(Err(err)) => {
                    if self.context.borrow().partial_subtitle.is_none() {
                        // Update the offset in case packets were skipped.
                        self.context.borrow_mut().offset = self.pes_packets.offset;
                    }

                    match err.downcast::<PesPacketError>() {
                        Ok(pes_pack_err) => {
                            if !self.context.borrow().is_chunked {
                                // Working with full stream.
                                match pes_pack_err {
                                    PesPacketError::NeedleNotFound { .. } => {
                                        // No more packets could be found.
                                        // Consider this as the end of stream.
                                        return None;
                                    }
                                    PesPacketError::Incomplete { needed, .. } => {
                                        self.context.borrow_mut().needed = needed;
                                        return Some(Err(Error::from(
                                            VobsubError::IncompleteInput { needed  }
                                        )));
                                    }
                                }
                            } else {
                                // Incomplete packets are expected in chunked mode.
                                self.context.borrow_mut().needed = match pes_pack_err {
                                    PesPacketError::NeedleNotFound { .. } => (
                                        PES_PACKET_NEEDLE_LEN + PES_PACKET_HEADER_MIN_LEN
                                    ).into(),
                                    PesPacketError::Incomplete { needed, .. } => needed,
                                };
                                return None;
                            }
                        }
                        Err(err) => {
                            trace!("Unrecoverable error");
                            self.context.borrow_mut().set_unrecoverable_error();
                            return Some(Err(err));
                        }
                    }
                }
            };

            match self.context.borrow_mut().have_packet(packet, self.pes_packets.offset) {
                Ok(None) => (), // Expecting more data.
                Ok(Some(subtitle_data)) => {
                    // No more packet wanted. Parse the subtitle data retrieved.
                    return Some(subtitle(&subtitle_data.packets_data, subtitle_data.base_time));
                }
                Err(err) => return Some(Err(err)),
            }
        }
    }
}

/// An iterator over subtitles.
pub struct Subtitles<'a> {
    internal: SubtitlesInternal<'a>,
    prev: Option<Subtitle>,
    prev_error: Option<Error>,
}

impl<'a> Iterator for Subtitles<'a> {
    type Item = Result<Subtitle>;

    // This whole routine exists to make sure that `end_time` is set to a
    // useful value even if the subtitles themselves didn't supply one.
    // I'm not even sure this is valid, but it has been observed in the
    // wild.
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(err) = self.prev_error.take() {
            // Previous iter generated an error which propagation was delayed
            // in order to send previous subtitle.
            return Some(Err(err));
        }

        // If we don't currently have a previous subtitle, attempt to fetch
        // one.
        if self.prev.is_none() {
            match self.internal.next() {
                Some(Ok(sub)) => { self.prev = Some(sub); }
                other => return other,
            }
        }
        debug_assert!(self.prev.is_some());

        match self.internal.next() {
            // We have a another subtitle!  We want to return `self.prev`
            // and store the new subtitle as `self.prev`.
            Some(Ok(curr)) => {
                // `unwrap` is safe because of the invariant above.
                let mut prev = self.prev.take().unwrap();
                if prev.end_time.is_none() {
                    // Our subtitle has no end time, so end it just before
                    // the next subtitle.
                    let new_end = curr.start_time - DEFAULT_SUBTITLE_SPACING;
                    let alt_end = prev.start_time + DEFAULT_SUBTITLE_LENGTH;
                    prev.end_time = Some(new_end.min(alt_end));
                }
                self.prev = Some(curr);
                Some(Ok(prev))
            }
            // We encountered an error.
            Some(Err(err)) => {
                // Save it for next iteration and return previous subtitle for now.
                self.prev_error = Some(err);
                Some(Ok(self.prev.take().unwrap()))
            }
            // The only subtitle left to return is `self.prev`.
            None => {
                self.prev.take().map(|mut sub| {
                    if sub.end_time.is_none() {
                        // Our subtitle has no end time, and it's the last
                        // subtitle, so just pick something.
                        sub.end_time =
                            Some(sub.start_time + DEFAULT_SUBTITLE_LENGTH);
                    }
                    Ok(sub)
                })
            }
        }
    }
}

/// Return an iterator over the subtitles in this data stream.
pub fn subtitles(input: &[u8]) -> Subtitles {
    Subtitles {
        internal: SubtitlesInternal::new(
            input,
            &Rc::new(RefCell::new(SubtitlesContext::default())),
        ),
        prev: None,
        prev_error: None,
    }
}

/// Subtitles iter manager which can process a stream by chunks.
///
/// # Example
///
/// Process subtitles from a file containing a Program Stream.
///
/// ```
/// use vobsub::SubtitlesFromChunks;
/// use std::fs;
/// use std::io::prelude::*;
///
/// let mut f = fs::File::open("../fixtures/example.sub").unwrap();
/// let mut buffer = vec![];
/// f.read_to_end(&mut buffer).unwrap();
/// let mut chunk_size = 2048;
/// let mut lower_offset = 0;
/// let mut upper_offset = 0;
///
/// let mut subs = SubtitlesFromChunks::new();
/// while upper_offset < buffer.len() {
///     lower_offset += subs.offset();
///     upper_offset = (lower_offset + chunk_size).min(buffer.len());
///
///     let mut sub_iter = subs.iter(&buffer[lower_offset..upper_offset]);
///     for sub in sub_iter.next() {
///         let subtitle = sub.unwrap();
///         // ... process `subtitle`
///     }
///
///     if let Some(needed) = subs.needed() {
///         chunk_size = chunk_size.max(needed);
///     }
/// }
/// ```
pub struct SubtitlesFromChunks {
    context: Rc<RefCell<SubtitlesContext>>,
    had_chunk: bool,
}

impl SubtitlesFromChunks {
    /// Build a new `SubtitlesFromChunks`, a [`Subtitles`] iterator manager
    /// capable of handling streams by chunks.
    ///
    /// [`Subtitles`]: struct.Subtitles.html
    pub fn new() -> Self {
        SubtitlesFromChunks {
            context: Rc::new(RefCell::new(SubtitlesContext {
                is_chunked: true,
                .. SubtitlesContext::default()
            })),
            had_chunk: false,
        }
    }

    /// Get a [`Subtitles`] iterator from this chunk.
    /// The parsing context is preserved for subsequent chunks.
    ///
    /// It is up to the caller to provide a slice that includes the remaining bytes
    /// from the [`offset`] reached after previous iteration.
    ///
    /// [`Subtitles`]: struct.Subtitles.html
    /// [`offset`]: struct.SubtitlesFromChunks.html#method.offset
    pub fn iter<'a>(&mut self, input: &'a [u8]) -> Subtitles<'a> {
        self.context.borrow_mut().pepare_for_new_chunk();
        self.had_chunk = true;

        Subtitles {
            internal: SubtitlesInternal::new(input, &self.context),
            prev: None,
            prev_error: None,
        }
    }

    /// Get current `offset` in the input chunk.
    ///
    /// The next chunk should start from the bytes in input chunk starting at `offset`.
    pub fn offset(&self) -> usize {
        self.context.borrow().offset
    }

    /// Whether this [`SubtitlesFromChunks`] is expecting other chunks.
    ///
    /// This is `true` when the [`SubtitlesFromChunks`] is initialized,
    /// and then everytime an incomplete packet is being processed.
    /// The next chunk must include remaining bytes not processed yet
    /// (use [`offset`] to determine the first offset to include).
    ///
    /// [`SubtitlesFromChunks`]: struct.SubtitlesFromChunks.html
    /// [`offset`]: struct.SubtitlesFromChunks.html#method.offset
    pub fn expecting_chunk(&self) -> bool {
        self.context.borrow().expecting_chunk() || !self.had_chunk
    }

    /// Get the minimal needed bytes required to continue parsing the stream.
    ///
    /// The next chunk should contain at least the needed bytes otherwise
    /// you might loop infinitely.
    pub fn needed(&self) -> Option<usize> {
        self.context.borrow().needed.into()
    }
}

#[test]
fn parse_subtitles() {
    //use env_logger;
    use std::fs;
    use std::io::prelude::*;

    //let _ = env_logger::init();

    let mut f = fs::File::open("../fixtures/example.sub").unwrap();
    let mut buffer = vec![];
    f.read_to_end(&mut buffer).unwrap();
    let mut subs = subtitles(&buffer);
    let sub1 = subs.next().expect("missing sub 1").unwrap();
    assert!(sub1.start_time - 49.4 < 0.1);
    assert!(sub1.end_time.unwrap() - 50.9 < 0.1);
    assert_eq!(sub1.force, false);
    assert_eq!(sub1.coordinates,
               Coordinates { x1: 750, y1: 916, x2: 1172, y2: 966 });
    assert_eq!(sub1.palette, [0,3,1,0]);
    assert_eq!(sub1.alpha, [15,15,15,0]);
    assert_eq!(sub1.raw_image.len(), 21573);
    subs.next().expect("missing sub 2").unwrap();
    assert!(subs.next().is_none());
}

#[test]
fn parse_subtitles_from_subtitle_edit() {
    //use env_logger;
    use idx::Index;
    //let _ = env_logger::init();
    let idx = Index::open("../fixtures/tiny.idx").unwrap();
    let mut subs = idx.subtitles();
    subs.next().expect("missing sub").unwrap();
    assert!(subs.next().is_none());
}

#[test]
fn parse_subtitles_by_chunks() {
    use env_logger;
    use std::fs;
    use std::io::prelude::*;

    let _ = env_logger::init();
    let mut f = fs::File::open("../fixtures/example.sub").unwrap();
    let mut buffer = vec![];
    f.read_to_end(&mut buffer).unwrap();

    // 1. Attempt to read the first subtitle
    // with a chunk too short to retrieve it completely.
    let mut subs = SubtitlesFromChunks::new();
    {
        let mut sub_iter = subs.iter(&buffer[..1024]);
        assert!(sub_iter.next().is_none());
    }
    assert!(subs.expecting_chunk());
    let mut offset = subs.offset();
    assert_eq!(0, offset);
    let needed = subs.needed().expect("expecting needed value");

    // Add a chunk large enough for the missing length for sub1
    // and additional bytes for part of sub2.
    {
        let mut sub_iter = subs.iter(&buffer[offset..offset + needed + 1024]);
        let sub1 = sub_iter.next();
        let sub1 = sub1
            .expect("missing sub 1")
            .expect("unexpected error for sub 1");
        assert!(sub1.start_time - 49.4 < 0.1);
        assert!(sub1.end_time.unwrap() - 50.9 < 0.1);
        assert_eq!(sub1.force, false);
        assert_eq!(sub1.coordinates,
                   Coordinates { x1: 750, y1: 916, x2: 1172, y2: 966 });
        assert_eq!(sub1.palette, [0,3,1,0]);
        assert_eq!(sub1.alpha, [15,15,15,0]);
        assert_eq!(sub1.raw_image.len(), 21573);

        // Missing chunk for sub 2
        assert!(sub_iter.next().is_none());
    }
    assert!(subs.expecting_chunk());
    assert!(subs.needed().is_some());

    // Add the rest of the buffer
    offset += subs.offset();
    {
        let mut sub_iter = subs.iter(&buffer[offset..]);
        let _sub2 = sub_iter.next()
            .expect("missing sub 2")
            .expect("unexpected error for sub 2");
    }

    // 2. Read the subtitles with a chunk large enough for the first packet
    // and too short for the second packet's needle.
    let mut subs = SubtitlesFromChunks::new();
    {
        let mut sub_iter = subs.iter(&buffer[..2048 + 2]);
        assert!(sub_iter.next().is_none());
    }
    assert!(subs.expecting_chunk());
}

#[test]
fn parse_fuzz_corpus_seeds() {
    //use env_logger;
    use idx::Index;
    //let _ = env_logger::init();

    // Make sure these two fuzz corpus inputs still work, and that they
    // return the same subtitle data.
    let tiny = Index::open("../fixtures/tiny.idx").unwrap()
        .subtitles().next().unwrap().unwrap();
    let split = Index::open("../fixtures/tiny-split.idx").unwrap()
        .subtitles().next().unwrap().unwrap();
    assert_eq!(tiny, split);
}
