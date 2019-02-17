//! # MPEG-2 Program Streams (PS)
//!
//! This is the container format used at the top-level of a `*.sub` file.

use common_failures::prelude::*;
use nom::{IResult, Needed};
use std::fmt;
use std::ops::Deref;

use super::clock::{Clock, clock_and_ext};
use super::pes;

/// A parsed [MPEG-2 Program Stream header][MPEG-PS] (MPEG-PS).
///
/// [MPEG-PS]: https://en.wikipedia.org/wiki/MPEG_program_stream
#[derive(Debug, PartialEq, Eq)]
pub struct Header {
    /// The System Clock Reference (SCR) and SCR extension field.
    pub scr: Clock,
    /// The bit rate, in units of 50 bytes per second.
    pub bit_rate: u32,
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PS packet @ {}, {} kbps]", self.scr, (self.bit_rate*50*8)/1024)
    }
}

/// Parse a Program Stream header.
named!(pub header<Header>,
    do_parse!(
        // Sync bytes.
        tag!(&[0x00, 0x00, 0x01, 0xba]) >>
        // 10-byte header.
        header: bits!(
            do_parse!(
                // MPEG-2 version tag.
                tag_bits!(u8, 2, 0b01) >>
                // System Clock Reference.
                scr: call!(clock_and_ext) >>
                // Bit rate.
                bit_rate: take_bits!(u32, 22) >>
                // Marker bits.
                tag_bits!(u8, 2, 0b11) >>
                // Reserved.
                take_bits!(u8, 5) >>
                // Number of bytes of stuffing.
                stuffing_length: take_bits!(usize, 3) >>
                // Stuffing bytes.  We just want to ignore these, but use a
                // large enough type to prevent overflow panics when
                // fuzzing.
                take_bits!(u64, stuffing_length * 8) >>
                (Header {
                    scr: scr,
                    bit_rate: bit_rate,
                })
            )
        ) >>
        (header)
    )
);

/// A [Packetized Elementary Stream][pes] packet with a Program Stream
/// header.
///
/// [pes]: http://dvd.sourceforge.net/dvdinfo/pes-hdr.html
#[derive(Debug, PartialEq, Eq)]
pub struct PesPacket<'a> {
    pub ps_header: Header,
    pub pes_packet: pes::Packet<'a>,
}

/// Parse a Program Stream packet and the following PES packet.
named!(pub pes_packet<PesPacket>,
    do_parse!(
        ps_header: call!(header) >>
        pes_packet: call!(pes::packet) >>
        (PesPacket {
            ps_header: ps_header,
            pes_packet: pes_packet,
        })
    )
);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NeededOpt(Option<usize>);

impl From<Needed> for NeededOpt {
    fn from(needed: Needed) -> Self {
        match needed {
            Needed::Size(needed) => NeededOpt(Some(needed)),
            Needed::Unknown => NeededOpt(None),
        }
    }
}

impl From<Option<usize>> for NeededOpt {
    fn from(needed: Option<usize>) -> Self {
        NeededOpt(needed)
    }
}

impl From<usize> for NeededOpt {
    fn from(needed: usize) -> Self {
        NeededOpt(Some(needed))
    }
}

impl From<NeededOpt> for Option<usize> {
    fn from(needed: NeededOpt) -> Self {
        needed.0
    }
}

impl Deref for NeededOpt {
    type Target = Option<usize>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Fail)]
pub enum PesPacketError {
    #[fail(display = "Incomplete PES packet: offset {}, needed {:?}", offset, needed)]
    Incomplete {
        offset: usize,
        needed: NeededOpt,
    },
    #[fail(display = "PES packet needle not found: offset {}, remaining {}", offset, remaining)]
    NeedleNotFound {
        offset: usize,
        remaining: usize,
    },
}

/// The length for the needle introducing a Program Stream packet.
pub const PES_PACKET_NEEDLE_LEN: usize = 4;

/// An iterator over all the PES packets in an MPEG-2 Program Stream.
pub struct PesPackets<'a> {
    /// The remaining input to parse.
    remaining: &'a [u8],
    /// The offset of the remaining input in the initial slice
    pub offset: usize,
}

impl<'a> Iterator for PesPackets<'a> {
    type Item = Result<PesPacket<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Search for the start of a ProgramStream packet.
            let needle = &[0x00, 0x00, 0x01, 0xba];
            let start = self.remaining.windows(needle.len())
                .position(|window| needle == window);

            if let Some(start) = start {
                // We found the start, so try to parse it.
                self.offset += start;
                self.remaining = &self.remaining[start..];
                match pes_packet(self.remaining) {
                    // We found a packet!
                    IResult::Done(remaining, packet) => {
                        self.offset += self.remaining.len() - remaining.len();
                        self.remaining = remaining;
                        trace!("Decoded packet {:?}", &packet);
                        return Some(Ok(packet));
                    }
                    // We have only a partial packet, and we hit the end of our
                    // data.
                    IResult::Incomplete(needed) => {
                        self.remaining = &[];
                        let err = PesPacketError::Incomplete {
                            offset: self.offset,
                            needed: needed.into(),
                        };
                        warn!("{:?}", err);
                        return Some(Err(Error::from(err)));
                    }
                    // We got something that looked like a packet but
                    // wasn't parseable.  Log it and keep trying.
                    IResult::Error(err) => {
                        self.offset += needle.len();
                        self.remaining = &self.remaining[needle.len()..];
                        debug!("Skipping packet {:?}", &err);
                    }
                }
            } else {
                // We didn't find the start of a packet.
                let remaining_len = self.remaining.len();
                self.remaining = &[];

                if remaining_len > 0 {
                    let err = PesPacketError::NeedleNotFound {
                        offset: self.offset,
                        remaining: remaining_len,
                    };
                    debug!("{:?}", err);
                    return Some(Err(Error::from(err)));
                } else {
                    trace!("Reached end of data");
                    return None;
                }
            }
        }
    }
}

/// Iterate over all the PES packets in an MPEG-2 Program Stream (or at
/// least those which contain subtitles).
pub fn pes_packets(input: &[u8]) -> PesPackets {
    PesPackets {
        remaining: input,
        offset: 0,
    }
}
