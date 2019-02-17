//! # MPEG-2 Packetized Elementary Streams (PES)
//!
//! These packets are nested inside the MPEG-2 Program Stream packets found
//! in a `*.sub` file.

use nom::{be_u8, be_u16, IResult, rest};
use std::fmt;

use super::clock::{Clock, clock};
use util::BytesFormatter;

/// Possible combinations of PTS and DTS data which might appear inside a
/// PES header.
///
/// See the [PES header documentation][PES] for details.
///
/// [PES]: http://dvd.sourceforge.net/dvdinfo/pes-hdr.html
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtsDtsFlags {
    /// No time stamps.
    None,
    /// Presentation Time Stamp only.
    Pts,
    /// Presentation and Decode Time Stamps.
    PtsDts,
}

impl Default for PtsDtsFlags {
    fn default() -> PtsDtsFlags {
        PtsDtsFlags::None
    }
}

/// Parse PTS & DTS flags in a PES packet header.  Consumes two bits.
named!(pts_dts_flags<(&[u8], usize), PtsDtsFlags>,
   alt!(value!(PtsDtsFlags::None,   tag_bits!(u8, 2, 0b00)) |
        value!(PtsDtsFlags::Pts,    tag_bits!(u8, 2, 0b10)) |
        value!(PtsDtsFlags::PtsDts, tag_bits!(u8, 2, 0b11)))
);

#[test]
fn parse_pts_dts_flags() {
    assert_eq!(pts_dts_flags((&[0b00][..], 6)),
               IResult::Done((&[][..], 0), PtsDtsFlags::None));
    assert_eq!(pts_dts_flags((&[0b10][..], 6)),
               IResult::Done((&[][..], 0), PtsDtsFlags::Pts));
    assert_eq!(pts_dts_flags((&[0b11][..], 6)),
               IResult::Done((&[][..], 0), PtsDtsFlags::PtsDts));
}

/// Presentation and Decode Time Stamps, if available.
#[derive(Debug, PartialEq, Eq)]
pub struct PtsDts {
    /// Presentation Time Stamp.
    pub pts: Clock,
    /// Decode Time Stamp.
    pub dts: Option<Clock>,
}

/// Helper for `pts_dts`.  Parses the PTS-only case.
named!(pts_only<PtsDts>,
    bits!(
        do_parse!(
            tag_bits!(u8, 4, 0b0010) >>
            pts: call!(clock) >>
            (PtsDts { pts: pts, dts: None })
        )
    )
);

/// Helper for `pts_dts`.  Parses the PTS and DTS case.
named!(pts_and_dts<PtsDts>,
    bits!(
        do_parse!(
            tag_bits!(u8, 4, 0b0011) >>
            pts: call!(clock) >>
            tag_bits!(u8, 4, 0b0001) >>
            dts: call!(clock) >>
            (PtsDts { pts: pts, dts: Some(dts) })
        )
    )
);

/// Parse a `PtsDts` value in the format specified by `flags`.
fn pts_dts(i: &[u8], flags: PtsDtsFlags) -> IResult<&[u8], Option<PtsDts>> {
    match flags {
        PtsDtsFlags::None => IResult::Done(i, None),
        PtsDtsFlags::Pts => pts_only(i).map(Some),
        PtsDtsFlags::PtsDts => pts_and_dts(i).map(Some),
    }
}

#[test]
fn parse_pts_dts() {
    assert_eq!(pts_dts(&[][..], PtsDtsFlags::None),
               IResult::Done(&[][..], None));
    assert_eq!(pts_dts(&[0x21, 0x00, 0xab, 0xe9, 0xc1][..], PtsDtsFlags::Pts),
               IResult::Done(&[][..],
                             Some(PtsDts {
                                 pts: Clock::base(2815200),
                                 dts: None,
                             })));
}

/// Flags specifying which header data fields are present.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HeaderDataFlags {
    pub pts_dts_flags: PtsDtsFlags,
    pub escr_flag: bool,
    pub es_rate_flag: bool,
    pub dsm_trick_mode_flag: bool,
    pub additional_copy_info_flag: bool,
    pub crc_flag: bool,
    pub extension_flag: bool,
}

/// Deserialize a single Boolean flag bit.
named!(bool_flag<(&[u8], usize), bool>,
    map!(take_bits!(u8, 1), |b| b == 1)
);

named!(header_data_flags<HeaderDataFlags>,
   bits!(
       do_parse!(
           pts_dts_flags: call!(pts_dts_flags) >>
           escr_flag: call!(bool_flag) >>
           es_rate_flag: call!(bool_flag) >>
           dsm_trick_mode_flag: call!(bool_flag) >>
           additional_copy_info_flag: call!(bool_flag) >>
           crc_flag: call!(bool_flag) >>
           extension_flag: call!(bool_flag) >>
           (HeaderDataFlags {
               pts_dts_flags: pts_dts_flags,
               escr_flag: escr_flag,
               es_rate_flag: es_rate_flag,
               dsm_trick_mode_flag: dsm_trick_mode_flag,
               additional_copy_info_flag: additional_copy_info_flag,
               crc_flag: crc_flag,
               extension_flag: extension_flag,
           })
       )
   )
);

#[test]
fn parse_header_data_flags() {
    assert_eq!(header_data_flags(&[0x80][..]),
               IResult::Done(&[][..],
                             HeaderDataFlags {
                                 pts_dts_flags: PtsDtsFlags::Pts,
                                 ..HeaderDataFlags::default()
                             }));
}

/// Header data fields.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HeaderData {
    pub flags: HeaderDataFlags,
    pub pts_dts: Option<PtsDts>,
    /// There's lots of other header data we could deserialize here, but
    /// we're not interested in any of it for subtitles.  Specify a private
    /// placeholder field so we can extend this without breaking the API.
    _placeholder: (),
}

/// Parse variable length header data, ignoring any fields we don't care
/// about.  We expect to be called by `length_value!` so any extra bytes
/// will be discarded.
fn header_data_fields(i: &[u8], flags: HeaderDataFlags)
                      -> IResult<&[u8], HeaderData> {
    do_parse!(i,
        pts_dts: apply!(pts_dts, flags.pts_dts_flags) >>
        (HeaderData {
            flags: flags,
            pts_dts: pts_dts,
            _placeholder: (),
        })
    )
}

/// Parse PES header data, including the predecing flags and length bytes.
named!(header_data<HeaderData>,
    do_parse!(
        // Grab the flags from our flag byte.
        flags: call!(header_data_flags) >>
        // Grab a single length byte, read that many bytes, and recursively
        // call `header_data_fields` to do the actual parse.  This ensures
        // that if `header_data_fields` doesn't parse all the header data,
        // we discard the rest before continuing.
        data: length_value!(call!(be_u8), apply!(header_data_fields, flags)) >>
        (data)
    )
);

#[test]
fn parse_header_data() {
    assert_eq!(header_data(&[0x00, 0x00][..]),
               IResult::Done(&[][..], HeaderData::default()));
    assert_eq!(header_data(&[0x80, 0x05, 0x21, 0x00, 0xab, 0xe9, 0xc1][..]),
               IResult::Done(&[][..],
                             HeaderData {
                                 flags: HeaderDataFlags {
                                     pts_dts_flags: PtsDtsFlags::Pts,
                                     ..HeaderDataFlags::default()
                                 },
                                 pts_dts: Some(PtsDts {
                                     pts: Clock::base(2815200),
                                     dts: None,
                                 }),
                                 ..HeaderData::default()
                             }));
}

/// A [Packetized Elementary Stream][pes] header, not including the
/// `HeaderData` information (which is parsed separately).
///
/// [pes]: http://dvd.sourceforge.net/dvdinfo/pes-hdr.html
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub scrambling_control: u8,
    pub priority: bool,
    pub data_alignment_indicator: bool,
    pub copyright: bool,
    pub original: bool,
}

/// Parse the first PES header byte after the length.
named!(header<Header>,
    bits!(
        do_parse!(
            tag_bits!(u8, 2, 0b10) >>
            scrambling_control: take_bits!(u8, 2) >>
            priority: call!(bool_flag) >>
            data_alignment_indicator: call!(bool_flag) >>
            copyright: call!(bool_flag) >>
            original: call!(bool_flag) >>
            (Header {
                scrambling_control: scrambling_control,
                priority: priority,
                data_alignment_indicator: data_alignment_indicator,
                copyright: copyright,
                original: original,
            })
        )
    )
);

/// A [Packetized Elementary Stream][pes] packet.
///
/// [pes]: http://dvd.sourceforge.net/dvdinfo/pes-hdr.html
#[derive(PartialEq, Eq)]
pub struct Packet<'a> {
    pub header: Header,
    pub header_data: HeaderData,
    pub substream_id: u8,
    pub data: &'a [u8],
}

impl<'a> fmt::Debug for Packet<'a> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Packet")
            .field("header", &self.header)
            .field("header_data", &self.header_data)
            .field("substream_id", &self.substream_id)
            .field("data", &BytesFormatter(self.data))
            .finish()
    }
}

named!(packet_helper<Packet>,
    do_parse!(
        header: call!(header) >>
        header_data: call!(header_data) >>
        substream_id: call!(be_u8) >>
        data: call!(rest) >>
        (Packet {
            header: header,
            header_data: header_data,
            substream_id: substream_id,
            data: data
        })
    )
);

/// Minimum needed length to identify a new PES Packet header
/// and get its length.
pub const PES_PACKET_HEADER_MIN_LEN: usize = 4 + 2;

named!(pub packet<Packet>,
    do_parse!(
        tag!(&[0x00, 0x00, 0x01, 0xbd]) >>
        packet: length_value!(call!(be_u16), call!(packet_helper)) >>
        (packet)
    )
);

#[test]
fn parse_packet() {
    let input = &[
        0x00, 0x00, 0x01, 0xbd,
        0x00, 0x10,
        0x81,
        0x80, 0x05, 0x21, 0x00, 0xab, 0xe9, 0xc1,
        0x20,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xff,
    ][..];

    let expected = Packet {
        header: Header {
            original: true,
            ..Header::default()
        },
        header_data: HeaderData {
            flags: HeaderDataFlags {
                pts_dts_flags: PtsDtsFlags::Pts,
                ..HeaderDataFlags::default()
            },
            pts_dts: Some(PtsDts {
                pts: Clock::base(2815200),
                dts: None,
            }),
            ..HeaderData::default()
        },
        substream_id: 0x20,
        data: &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    };

    assert_eq!(packet(input), IResult::Done(&[0xff][..], expected));
}
