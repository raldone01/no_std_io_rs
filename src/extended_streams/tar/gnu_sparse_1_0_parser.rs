use core::marker::PhantomData;

use crate::{
  extended_streams::tar::{
    CommonParseError, CorruptField, IgnoreTarViolationHandler, SparseFileInstruction,
    TarParserError, TarViolationHandler,
  },
  BufferedRead, CopyBuffered as _, CopyUntilError, Cursor, LimitedVec, UnwrapInfallible,
  WriteAllError,
};

const fn max_string_length_from_limit(limit: usize, radix: usize) -> usize {
  if limit == 0 {
    return 1; // "0" is the only representation
  }

  limit.ilog(radix) as usize + 1
}

#[derive(Debug)]
struct StateParsingMapEntry {
  remaining_maps: usize,
  parsed_offset_before: Option<u64>,
}

#[derive(Debug)]
struct StateSkippingPadding {
  /// The amount of padding that still needs to be skipped.
  remaining_padding: usize,
}

#[derive(Debug, Default)]
enum ParserState {
  #[default]
  ParsingNumberOfMaps,
  ParsingMapEntry(StateParsingMapEntry),
  SkippingPadding(StateSkippingPadding),
  Finished,
}

const MAX_VALUE_STRING_LENGTH: usize = max_string_length_from_limit(usize::MAX, 10);

/// For version 1.0 the sparse map is stored in the data section of the file.
/// Series of decimal numbers delimited by '\n'.
/// The first number gives the number of maps in the file.
/// Each map is a pair of numbers: the offset in the file and the size of the data at that offset.
/// The map is padded to the next 512 byte block boundary.
///
/// While technically possible to recover from parsing errors and extract the data section with the corrupt sparse map,
/// this parser does not support such recovery.
#[derive(Debug)]
pub struct GnuSparse1_0Parser<VH: TarViolationHandler = IgnoreTarViolationHandler> {
  state: ParserState,
  pub(crate) bytes_read: usize,
  /// This is used by ParsingNumberOfMaps and ParsingMapEntry to buffer the decimal string representation of the number.
  value_string_cursor: Cursor<[u8; MAX_VALUE_STRING_LENGTH]>,
  _violation_handler: PhantomData<VH>,
}

impl<VH: TarViolationHandler> Default for GnuSparse1_0Parser<VH> {
  fn default() -> Self {
    Self::new()
  }
}

impl<VH: TarViolationHandler> GnuSparse1_0Parser<VH> {
  #[must_use]
  pub fn new() -> Self {
    Self {
      state: ParserState::default(),
      bytes_read: 0,
      // We just support usize max since that is only 20 bytes anyway.
      value_string_cursor: Cursor::new([0; MAX_VALUE_STRING_LENGTH]),
      _violation_handler: PhantomData,
    }
  }

  #[must_use]
  fn map_corrupt_field<'a, T: Into<CommonParseError>>(
    vh: &'a mut VH,
    field: CorruptField,
  ) -> impl FnOnce(T) -> TarParserError + 'a {
    move |error| {
      let err = TarParserError::CorruptField {
        field,
        error: error.into(),
      }
      .into();
      let _fatal_error = vh.handle(&err);
      err
    }
  }

  fn state_parsing_number_of_maps(
    &mut self,
    vh: &mut VH,
    cursor: &mut Cursor<&[u8]>,
  ) -> Result<ParserState, TarParserError> {
    // Read the length until we hit a newline
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut self.value_string_cursor,
      false,
      |&byte| byte == b'\n',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) => {},
      Err(CopyUntilError::DelimiterNotFound { .. }) => {
        // We need to read more data to find the delimiter
        return Ok(ParserState::ParsingNumberOfMaps);
      },
      Err(CopyUntilError::IoRead(..)) => unreachable!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(..)),
      ) => {
        let err = TarParserError::LimitExceeded {
          limit: MAX_VALUE_STRING_LENGTH,
          unit: "gnu 1.0 sparse maps",
          context: "Number of sparse map decimal string too long",
        };
        let _fatal_error = vh.handle(&err);
        return Err(err);
      },
    }

    // Convert the number of maps bytes to a usize
    let number_of_maps_str = core::str::from_utf8(self.value_string_cursor.before()).map_err(
      Self::map_corrupt_field(vh, CorruptField::GnuSparse1_0NumberOfMaps),
    )?;
    let number_of_maps = number_of_maps_str
      .parse::<usize>()
      .map_err(Self::map_corrupt_field(
        vh,
        CorruptField::GnuSparse1_0NumberOfMaps,
      ))?;
    if number_of_maps == 0 {
      return Ok(ParserState::Finished);
    }

    // reset the cursor for the next state
    self.value_string_cursor.set_position(0);
    Ok(ParserState::ParsingMapEntry(StateParsingMapEntry {
      remaining_maps: number_of_maps,
      parsed_offset_before: None,
    }))
  }

  fn state_parsing_map_entry(
    &mut self,
    vh: &mut VH,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateParsingMapEntry,
    sparse_file_instructions: &mut LimitedVec<SparseFileInstruction>,
    initial_cursor_position: usize,
  ) -> Result<ParserState, TarParserError> {
    // Read the offset or size until we hit a newline
    let copy_buffered_until_result = cursor.copy_buffered_until(
      &mut self.value_string_cursor,
      false,
      |&byte| byte == b'\n',
      false,
    );
    match copy_buffered_until_result {
      Ok(_) => {},
      Err(CopyUntilError::DelimiterNotFound { .. }) => {
        // We need to read more data to find the delimiter
        return Ok(ParserState::ParsingMapEntry(state));
      },
      Err(CopyUntilError::IoRead(..)) => unreachable!("BUG: Infallible error in read operation"),
      Err(
        CopyUntilError::IoWrite(WriteAllError::ZeroWrite { .. })
        | CopyUntilError::IoWrite(WriteAllError::Io(..)),
      ) => {
        let err = TarParserError::LimitExceeded {
          limit: self.value_string_cursor.len(),
          unit: "gnu 1.0 sparse map entry",
          context: "Sparse map entry decimal string too long",
        };
        // Recovering from this error would require keeping a buffer of the consumed data.
        let _fatal_error = vh.handle(&err);
        return Err(err);
      },
    }

    // Convert the offset or size bytes to a u64
    let value_str = core::str::from_utf8(self.value_string_cursor.before()).map_err(
      Self::map_corrupt_field(vh, CorruptField::GnuSparse1_0MapEntryValue),
    )?;
    let value = value_str.parse::<u64>().map_err(Self::map_corrupt_field(
      vh,
      CorruptField::GnuSparse1_0MapEntryValue,
    ))?;

    if let Some(offset_before) = state.parsed_offset_before.take() {
      // This is the size
      sparse_file_instructions
        .push(SparseFileInstruction {
          offset_before,
          data_size: value,
        })
        .map_err(|_| {
          let err = TarParserError::LimitExceeded {
            limit: sparse_file_instructions.max_len(),
            unit: "sparse file instructions",
            context: "Too many sparse file instructions",
          };
          let _fatal_error = vh.handle(&err);
          err
        })?;
      state.remaining_maps -= 1;
    } else {
      // This is the offset
      state.parsed_offset_before = Some(value);
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
    self.value_string_cursor.set_position(0);

    Ok(ParserState::ParsingMapEntry(state))
  }

  fn state_skipping_padding(
    &mut self,
    cursor: &mut Cursor<&[u8]>,
    mut state: StateSkippingPadding,
  ) -> Result<ParserState, TarParserError> {
    // Skip the remaining padding
    let skipped_bytes = cursor
      .skip_buffered(state.remaining_padding)
      .unwrap_infallible();
    state.remaining_padding -= skipped_bytes;

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
    vh: &mut VH,
    cursor: &mut Cursor<&[u8]>,
    sparse_file_instructions: &mut LimitedVec<SparseFileInstruction>,
  ) -> Result<bool, TarParserError> {
    // TODO: loop here to drive the parser until all available data is consumed.
    let parser_state = core::mem::replace(&mut self.state, ParserState::Finished);

    let initial_cursor_position = cursor.position();

    let next_state = match parser_state {
      ParserState::ParsingNumberOfMaps => self.state_parsing_number_of_maps(vh, cursor),
      ParserState::ParsingMapEntry(state) => self.state_parsing_map_entry(
        vh,
        cursor,
        state,
        sparse_file_instructions,
        initial_cursor_position,
      ),
      ParserState::SkippingPadding(state) => self.state_skipping_padding(cursor, state),
      ParserState::Finished => unreachable!("BUG: No next state set in GnuSparse1_0Parser"),
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
  ) -> Result<LimitedVec<SparseFileInstruction>, TarParserError> {
    // Pad the input to a multiple of 512 bytes
    let padding_length = (input.len() + 511) & !511;
    let mut input_padded = vec![0; padding_length];
    input_padded[..input.len()].copy_from_slice(input);
    let mut cursor = Cursor::new(input_padded.as_slice());
    let mut sparse_file_instructions = LimitedVec::new(usize::MAX);
    let mut vh = IgnoreTarViolationHandler::default();
    while !parser.parse(&mut vh, &mut cursor, &mut sparse_file_instructions)? {}
    Ok(sparse_file_instructions)
  }

  #[test]
  fn test_gnu_sparse_1_0_parser() {
    let mut parser = GnuSparse1_0Parser::default();
    let input = b"2\n0\n100\n200\n300\n".as_slice();
    let result = drive_parser(&mut parser, input).expect("Failed to parse input");
    assert_eq!(
      result.as_slice(),
      [
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
