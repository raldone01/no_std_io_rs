use core::marker::PhantomData;

use alloc::vec::Vec;

use thiserror::Error;

use crate::{
  extended_streams::tar::{
    CommonParseError, IgnoreTarViolationHandler, SparseFileInstruction, TarParserError,
    TarViolationHandler,
  },
  BufferedRead, CopyBuffered as _, CopyUntilError, Cursor, LimitedBackingBuffer, WriteAllError,
};

// TODO: use violation handler

fn max_string_length_from_limit(limit: usize, radix: usize) -> usize {
  if limit == 0 {
    return 1; // "0" is the only representation
  }

  limit.ilog(radix) as usize + 1
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum GnuSparse1_0ParserError {
  #[error("Parsing field {field} failed: {error}")]
  CorruptField {
    field: &'static str,
    error: CommonParseError,
  },
}

#[derive(Debug)]
struct StateParsingNumberOfMaps {
  number_string_cursor: Cursor<LimitedBackingBuffer<Vec<u8>>>,
}

impl StateParsingNumberOfMaps {
  #[must_use]
  pub fn new(limits: &GnuSparse1_0ParserLimits) -> Self {
    Self {
      number_string_cursor: Cursor::new(LimitedBackingBuffer::new(
        Vec::new(),
        max_string_length_from_limit(limits.max_number_of_maps, 10),
      )),
    }
  }
}

#[derive(Debug)]
struct StateParsingMapEntry {
  remaining_maps: usize,
  value_string_cursor: Cursor<LimitedBackingBuffer<Vec<u8>>>,
  parsed_offset: Option<u64>,
}

#[derive(Debug)]
struct StateSkippingPadding {
  /// The amount of padding that still needs to be skipped.
  remaining_padding: usize,
}

#[derive(Debug)]
enum ParserState {
  ParsingNumberOfMaps(StateParsingNumberOfMaps),
  ParsingMapEntry(StateParsingMapEntry),
  SkippingPadding(StateSkippingPadding),
  Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GnuSparse1_0ParserLimits {
  pub max_number_of_maps: usize,
  pub max_map_entry_value: usize,
}

impl Default for GnuSparse1_0ParserLimits {
  fn default() -> Self {
    Self {
      max_number_of_maps: 256,
      max_map_entry_value: usize::MAX,
    }
  }
}

/// For version 1.0 the sparse map is stored in the data section of the file.
/// Series of decimal numbers delimited by '\n'.
/// The first number gives the number of maps in the file.
/// Each map is a pair of numbers: the offset in the file and the size of the data at that offset.
/// The map is padded to the next 512 byte block boundary.
#[derive(Debug)]
pub struct GnuSparse1_0Parser<VH: TarViolationHandler = IgnoreTarViolationHandler> {
  state: ParserState,
  pub(crate) bytes_read: usize,
  limits: GnuSparse1_0ParserLimits,
  _violation_handler: PhantomData<VH>,
}

impl<VH: TarViolationHandler> Default for GnuSparse1_0Parser<VH> {
  fn default() -> Self {
    let limits = GnuSparse1_0ParserLimits::default();
    Self {
      state: ParserState::ParsingNumberOfMaps(StateParsingNumberOfMaps::new(&limits)),
      bytes_read: 0,
      limits,
      _violation_handler: PhantomData,
    }
  }
}

impl<VH: TarViolationHandler> GnuSparse1_0Parser<VH> {
  #[must_use]
  fn map_corrupt_field<T: Into<CommonParseError>>(
    field: &'static str,
  ) -> impl FnOnce(T) -> GnuSparse1_0ParserError {
    move |error| GnuSparse1_0ParserError::CorruptField {
      field,
      error: error.into(),
    }
  }

  fn state_parsing_number_of_maps(
    &mut self,
    //vh: &mut VH,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateParsingNumberOfMaps,
  ) -> Result<ParserState, TarParserError> {
    // Read the length until we hit a newline
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut &mut state.number_string_cursor,
      false,
      |byte: &u8| *byte == b'\n',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) => {},
      Err(CopyUntilError::DelimiterNotFound { .. }) => {
        // We need to read more data to find the delimiter
        return Ok(ParserState::ParsingNumberOfMaps(state));
      },
      Err(CopyUntilError::IoRead(..)) => panic!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(..)),
      ) => {
        return Err(TarParserError::LimitExceeded {
          limit: self.limits.max_number_of_maps,
          unit: "gnu 1.0 sparse maps",
          context: "Number of sparse map decimal string too long",
        })
      },
    }

    // Convert the number of maps bytes to a usize
    let number_of_maps_str =
      core::str::from_utf8(state.number_string_cursor.before()).unwrap_or("0");
    let number_of_maps = number_of_maps_str
      .parse::<usize>()
      .map_err(Self::map_corrupt_field("number of maps"))?;
    if number_of_maps == 0 {
      return Ok(ParserState::Finished);
    }

    Ok(ParserState::ParsingMapEntry(StateParsingMapEntry {
      remaining_maps: number_of_maps,
      value_string_cursor: Cursor::new(LimitedBackingBuffer::new(
        Vec::new(),
        max_string_length_from_limit(self.limits.max_map_entry_value, 10),
      )),
      parsed_offset: None,
    }))
  }

  fn state_parsing_map_entry(
    &mut self,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateParsingMapEntry,
    sparse_file_instructions: &mut Vec<SparseFileInstruction>,
    initial_cursor_position: usize,
  ) -> Result<ParserState, TarParserError> {
    // Read the offset or size until we hit a newline
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut &mut state.value_string_cursor,
      false,
      |byte: &u8| *byte == b'\n',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) => {},
      Err(CopyUntilError::DelimiterNotFound { .. }) => {
        // We need to read more data to find the delimiter
        return Ok(ParserState::ParsingMapEntry(state));
      },
      Err(CopyUntilError::IoRead(..)) => panic!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(..)),
      ) => {
        return Err(TarParserError::LimitExceeded {
          limit: self.limits.max_map_entry_value,
          unit: "gnu 1.0 sparse map entry",
          context: "Sparse map entry decimal string too long",
        });
      },
    }

    // Convert the offset or size bytes to a u64
    let value_str = core::str::from_utf8(state.value_string_cursor.before()).unwrap_or("0");
    let value = value_str
      .parse::<u64>()
      .map_err(Self::map_corrupt_field("sparse map entry value"))?;

    if state.parsed_offset.is_none() {
      // This is the offset
      state.parsed_offset = Some(value);
    } else {
      // This is the size
      let offset_before = state.parsed_offset.take().unwrap();
      sparse_file_instructions.push(SparseFileInstruction {
        offset_before,
        data_size: value,
      });
      state.remaining_maps -= 1;
    }

    if state.remaining_maps == 0 {
      // All maps have been parsed. We still need to skip padding.
      let bytes_read = self.bytes_read + cursor.position() - initial_cursor_position;
      let remaining_padding = ((bytes_read + 511) & !511) - bytes_read;
      return Ok(ParserState::SkippingPadding(StateSkippingPadding {
        remaining_padding,
      }));
    }

    // Reset the cursor for the next map entry
    state.value_string_cursor.set_position(0);

    Ok(ParserState::ParsingMapEntry(state))
  }

  fn state_skipping_padding(
    &mut self,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateSkippingPadding,
  ) -> Result<ParserState, TarParserError> {
    // Skip the remaining padding
    let bytes_to_skip = state.remaining_padding.min(cursor.remaining());
    cursor
      .skip(bytes_to_skip)
      .expect("BUG: Incremental padding skipping failed");
    state.remaining_padding -= bytes_to_skip;

    if state.remaining_padding == 0 {
      Ok(ParserState::Finished)
    } else {
      Ok(ParserState::SkippingPadding(state))
    }
  }

  /// Returns true if the parser has finished parsing the sparse map.
  /// Returns false if it needs more data to continue parsing.
  pub fn parse(
    &mut self,
    cursor: &mut Cursor<&[u8]>,
    sparse_file_instructions: &mut Vec<SparseFileInstruction>,
  ) -> Result<bool, TarParserError> {
    let parser_state = core::mem::replace(&mut self.state, ParserState::Finished);

    let initial_cursor_position = cursor.position();

    let next_state = match parser_state {
      ParserState::ParsingNumberOfMaps(state) => self.state_parsing_number_of_maps(cursor, state),
      ParserState::ParsingMapEntry(state) => self.state_parsing_map_entry(
        cursor,
        state,
        sparse_file_instructions,
        initial_cursor_position,
      ),
      ParserState::SkippingPadding(state) => self.state_skipping_padding(cursor, state),
      ParserState::Finished => panic!("BUG: No next state set in GnuSparse1_0Parser"),
    };

    let bytes_read_this_parse = cursor.position() - initial_cursor_position;
    self.bytes_read += bytes_read_this_parse;

    self.state = next_state?;

    match self.state {
      ParserState::Finished => Ok(true),
      _ => Ok(false),
    }
  }
}

#[cfg(test)]
mod tests {
  use alloc::vec;

  use crate::extended_streams::tar::{tar_constants::BLOCK_SIZE, IgnoreTarViolationHandler};

  use super::*;

  fn drive_parser(
    parser: &mut GnuSparse1_0Parser<IgnoreTarViolationHandler>,
    input: &[u8],
  ) -> Result<Vec<SparseFileInstruction>, TarParserError> {
    // Pad the input to a multiple of 512 bytes
    let padding_length = (input.len() + 511) & !511;
    let mut input_padded = vec![0; padding_length];
    input_padded[..input.len()].copy_from_slice(input);
    let mut cursor = Cursor::new(input_padded.as_slice());
    let mut sparse_file_instructions = Vec::new();
    while !parser.parse(&mut cursor, &mut sparse_file_instructions)? {}
    Ok(sparse_file_instructions)
  }

  #[test]
  fn test_gnu_sparse_1_0_parser() {
    let mut parser = GnuSparse1_0Parser::default();
    let input = b"2\n0\n100\n200\n300\n".as_slice();
    let result = drive_parser(&mut parser, input).unwrap();
    assert_eq!(
      result,
      vec![
        SparseFileInstruction {
          offset_before: 0,
          data_size: 100,
        },
        SparseFileInstruction {
          offset_before: 200,
          data_size: 300,
        },
      ]
    );
    assert_eq!(parser.bytes_read, BLOCK_SIZE);
  }
}
