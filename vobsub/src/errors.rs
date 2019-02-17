//! Custom error types.

use common_failures::prelude::*;
use nom::IResult;
use std::default::Default;
use std::fmt;

use mpeg2::ps::NeededOpt;

/// A type representing errors that are specific to `vobsub`. Note that we may
/// normally return `Error`, not `VobsubError`, which allows to return other
/// kinds of errors from third-party libraries.
#[derive(Debug, Fail, PartialEq)]
pub enum VobsubError {

    /// Our input data ended sooner than we expected.
    #[fail(display = "Input ended unexpectedly. {:?} bytes needed", needed)]
    IncompleteInput {
        needed: NeededOpt,
    },

    /// We were unable to find a required key in an `*.idx` file.
    #[fail(display = "Could not find required key '{}'", key)]
    MissingKey {
        key: &'static str,
    },

    /// We could not parse a value.
    #[fail(display = "Could not parse: {}", message)]
    Parse {
        message: String,
    },

    /// We have leftover input that we didn't expect.
    #[fail(display = "Unexpected extra input")]
    UnexpectedInput,
}

pub trait IResultExt<I, O, E> {
    fn ignore_trailing_data(self) -> IResult<I, O, E>;
    fn to_vobsub_result(self) -> Result<O>;
}

impl<I: Default+Eq, O, E: fmt::Debug> IResultExt<I, O, E> for IResult<I, O, E> {
    fn ignore_trailing_data(self) -> IResult<I, O, E> {
        match self {
            IResult::Done(_, val) => IResult::Done(I::default(), val),
            other => other,
        }
    }

    fn to_vobsub_result(self) -> Result<O> {
        match self {
            IResult::Done(rest, val) => {
                if rest == I::default() {
                    Ok(val)
                } else {
                    Err(VobsubError::UnexpectedInput.into())
                }
            }
            IResult::Incomplete(needed) => {
                Err(VobsubError::IncompleteInput{
                    needed: needed.into(),
                }.into())
            }
            IResult::Error(err) => {
                Err(VobsubError::Parse {
                    message: format!("{:?}", err),
                }.into())
            }
        }
    }
}
