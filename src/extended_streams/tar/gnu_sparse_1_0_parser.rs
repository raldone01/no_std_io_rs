use alloc::vec::Vec;

use crate::{
  extended_streams::tar::{SparseFileInstruction, TarParserError},
  BufferedRead, CopyBuffered as _, CopyUntilError, Cursor, FixedSizeBufferError, WriteAllError,
};

#[derive(Debug, PartialEq, Eq)]
struct StateParsingNumberOfMaps {
  number_string_cursor: Cursor<[u8; 20]>,
}

#[derive(Debug, PartialEq, Eq)]
struct StateParsingMapEntry {
  remaining_maps: usize,
  value_string_cursor: Cursor<[u8; 20]>,
  parsed_offset: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
struct StateSkippingPadding {
  /// The amount of padding that still needs to be skipped.
  remaining_padding: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum ParserState {
  ParsingNumberOfMaps(StateParsingNumberOfMaps),
  ParsingMapEntry(StateParsingMapEntry),
  SkippingPadding(StateSkippingPadding),
  Finished,
}

impl Default for ParserState {
  fn default() -> Self {
    ParserState::ParsingNumberOfMaps(StateParsingNumberOfMaps {
      number_string_cursor: Cursor::new([0; 20]),
    })
  }
}

/// For version 1.0 the sparse map is stored in the data section of the file.
/// Series of decimal numbers delimited by '\n'.
/// The first number gives the number of maps in the file.
/// Each map is a pair of numbers: the offset in the file and the size of the data at that offset.
/// The map is padded to the next 512 byte block boundary.
#[derive(Default)]
pub struct GnuSparse1_0Parser {
  state: ParserState,
  pub(crate) bytes_read: usize,
}

impl GnuSparse1_0Parser {
  fn state_parsing_number_of_maps(
    &mut self,
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
      Ok(_) | Err(CopyUntilError::DelimiterNotFound { .. }) => {},
      Err(CopyUntilError::IoRead(..)) => panic!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(FixedSizeBufferError { .. })),
      ) => {
        return Err(TarParserError::CorruptGnuSparse1_0NumberOfMaps {
          max_number_of_maps_field_length: state.number_string_cursor.full_buffer().len(),
        })
      },
    }

    // Convert the number of maps bytes to a usize
    let number_of_maps_str =
      core::str::from_utf8(state.number_string_cursor.before()).unwrap_or("0");
    let number_of_maps = match number_of_maps_str.parse::<usize>() {
      Ok(value) => value,
      Err(e) => return Err(TarParserError::CorruptGnuSparse1_0NumberOfMapsInteger(e)),
    };
    if number_of_maps == 0 {
      return Ok(ParserState::Finished);
    }

    Ok(ParserState::ParsingMapEntry(StateParsingMapEntry {
      remaining_maps: number_of_maps,
      value_string_cursor: Cursor::new([0; 20]),
      parsed_offset: None,
    }))
  }

  fn state_parsing_map_entry(
    &mut self,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateParsingMapEntry,
    sparse_file_instructions: &mut Vec<SparseFileInstruction>,
  ) -> Result<ParserState, TarParserError> {
    // Read the offset or size until we hit a newline
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut &mut state.value_string_cursor,
      false,
      |byte: &u8| *byte == b'\n',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) | Err(CopyUntilError::DelimiterNotFound { .. }) => {},
      Err(CopyUntilError::IoRead(..)) => panic!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(FixedSizeBufferError { .. })),
      ) => {
        return Err(TarParserError::CorruptGnuSparse1_0MapEntryLength {
          max_value_field_length: state.value_string_cursor.full_buffer().len(),
        })
      },
    }

    // Convert the offset or size bytes to a u64
    let value_str = core::str::from_utf8(state.value_string_cursor.before()).unwrap_or("0");
    let value = match value_str.parse::<u64>() {
      Ok(value) => value,
      Err(e) => return Err(TarParserError::CorruptGnuSparse1_0MapEntryInteger(e)),
    };

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
      let remaining_padding = (cursor.position() as usize + 511) & !511;
      return Ok(ParserState::SkippingPadding(StateSkippingPadding {
        remaining_padding: remaining_padding - cursor.position() as usize,
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
      ParserState::ParsingMapEntry(state) => {
        self.state_parsing_map_entry(cursor, state, sparse_file_instructions)
      },
      ParserState::SkippingPadding(state) => self.state_skipping_padding(cursor, state),
      ParserState::Finished => panic!("BUG: No next state set in GnuSparse1_0Parser"),
    };

    self.bytes_read += cursor.position() - initial_cursor_position;

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

  use crate::extended_streams::tar::tar_constants::BLOCK_SIZE;

  use super::*;

  fn drive_parser(
    parser: &mut GnuSparse1_0Parser,
    input: &[u8],
  ) -> Result<Vec<SparseFileInstruction>, TarParserError> {
    // Pad the input to a multiple of 512 bytes
    let padding_length = (512 - (input.len() % 512)) % 512;
    let mut input_padded = vec![0; input.len() + padding_length];
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
