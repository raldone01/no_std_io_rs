use core::convert::Infallible;

use alloc::{
  format,
  string::{String, ToString as _},
  vec::Vec,
};

use hashbrown::HashMap;
use zerocopy::FromBytes as _;

use crate::{
  core_streams::Cursor,
  extended_streams::tar::{
    confident_value::ConfidentValue,
    gnu_sparse_1_0_parser::GnuSparse1_0Parser,
    pax_parser::{PaxConfidence, PaxConfidentValue, PaxParser},
    tar_constants::{
      find_null_terminator_index, CommonHeaderAdditions, GnuHeaderAdditions, GnuHeaderExtSparse,
      GnuSparseInstruction, TarTypeFlag, UstarHeaderAdditions, V7Header, BLOCK_SIZE,
      TAR_ZERO_HEADER,
    },
    BlockDeviceEntry, CharacterDeviceEntry, CorruptFieldContext, FileData, FileEntry,
    FilePermissions, GeneralParseError, HardLinkEntry, IgnoreTarViolationHandler, RegularFileEntry,
    SparseFileInstruction, SparseFormat, SymbolicLinkEntry, TarHeaderParserError, TarInode,
    TarParserError, TarParserLimits, TarParserOptions, TarViolationHandler, TimeStamp, VHW,
  },
  BufferedRead as _, LimitedVec, UnwrapInfallible, Write, WriteAll as _,
};

pub(crate) fn align_to_block_size(size: usize) -> usize {
  (size + BLOCK_SIZE - 1) & !(BLOCK_SIZE - 1)
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum TarConfidence {
  V7 = 1,
  Ustar,
  Gnu,
  PaxGlobal,
  PaxLocal,
}

impl From<PaxConfidence> for TarConfidence {
  fn from(value: PaxConfidence) -> Self {
    match value {
      PaxConfidence::LOCAL => TarConfidence::PaxLocal,
      PaxConfidence::GLOBAL => TarConfidence::PaxGlobal,
    }
  }
}

enum GnuLongNameType {
  FileName,
  LinkName,
}

pub struct StateReadingOldGnuSparseExtendedHeader {
  /// The size of the data section following the old gnu sparse extended headers.
  data_after_header: usize,
  /// The amount of padding after the data section.
  padding_after_data: usize,
}

pub struct StateSkippingData {
  /// The amount of data that must be skipped.
  remaining_data: usize,
  /// The context for the skipped data, used for error messages and debugging.
  context: &'static str,
}

pub struct StateParsingGnuLongName {
  /// The amount of data that is still remaining to be read.
  remaining_data: usize,
  /// The amount of padding after the long name data.
  padding_after_data: usize,
  /// The type of the long name (file name or link name).
  long_name_type: GnuLongNameType,
  /// The collected long name bytes.
  collected_name: Vec<u8>,
}

struct StateReadingFileData {
  /// The amount of data that is still remaining to be read.
  remaining_data: usize,
  /// The amount of padding after the file data.
  padding_after: usize,
}

struct StateParsingPaxData {
  /// The amount of data that is still remaining to be read.
  remaining_data: usize,
  /// The amount of padding after the PAX data.
  padding_after: usize,
  pax_mode: PaxConfidence,
}

struct StateParsingGnuSparse1_0 {
  /// The amount of data that is still remaining to be read.
  data_after_header: usize,
  /// The amount of padding after the file data.
  padding_after: usize,
}

#[derive(Default)]
enum TarParserState {
  #[default]
  ReadingTarHeader,
  ReadingOldGnuSparseExtendedHeader(StateReadingOldGnuSparseExtendedHeader),
  SkippingData(StateSkippingData),
  ParsingGnuLongName(StateParsingGnuLongName),
  ReadingFileData(StateReadingFileData),
  ParsingPaxData(StateParsingPaxData),
  ParsingGnuSparse1_0(StateParsingGnuSparse1_0),
  NoNextStateSet,
}

pub struct TarParser<VH: TarViolationHandler = IgnoreTarViolationHandler> {
  /// The extracted files.
  extracted_files: Vec<TarInode>,

  /// The number of files found with each type flag.
  found_type_flags: HashMap<TarTypeFlag, usize>,
  violation_handler: VH,
  /// Stores all the file metadata that has been parsed so far.
  /// Must be reset after each file.
  inode_state: InodeBuilder,

  /// Stores the index of each file in `extracted_files`.
  /// Used for keeping only the last version of each file.
  /// Only used if `keep_only_last` is true.
  seen_files: HashMap<String, usize>,
  keep_only_last: bool,

  parser_state: TarParserState,
  /// Contains both the global and local extended attributes.
  pax_parser: PaxParser<VH>,

  /// The temporary buffer used for reading the tar header.
  /// Used by the `ReadingTarHeader` and `ReadingOldGnuSparseExtendedHeader` states.
  header_buffer: Cursor<[u8; BLOCK_SIZE]>,
  /// Used by the `ParsingGnuSparse1_0` state.
  sparse_parser: GnuSparse1_0Parser<VH>,

  limits: TarParserLimits,
}

pub(crate) fn buffer_array<'a, const BUFFER_SIZE: usize>(
  reader: &'a mut Cursor<&[u8]>,
  temp_buffer: &'a mut Cursor<[u8; BUFFER_SIZE]>,
) -> Result<Option<&'a [u8]>, TarParserError> {
  // perform an incremental read into the tar header buffer
  let read_bytes = reader
    .read_buffered(temp_buffer.remaining())
    .unwrap_infallible();

  if read_bytes.len() == BUFFER_SIZE {
    // We can directly pass through the buffer so we don't have to copy it to the intermediate buffer.
    return Ok(Some(read_bytes));
  }

  // read bytes into the tar header buffer
  temp_buffer
    .write_all(read_bytes, false)
    .expect("BUG: buffer_array incremental write failed");
  if temp_buffer.remaining() == 0 {
    // We have a complete tar header block, so we can return it.
    temp_buffer.set_position(0); // reset the cursor for the next read
    let header_buffer = temp_buffer.after();
    Ok(Some(header_buffer))
  } else {
    // We don't have a complete tar header block yet, so we return None.
    Ok(None)
  }
}

pub(crate) type InodeConfidentValue<T> = ConfidentValue<TarConfidence, T>;

impl<T: Clone> From<PaxConfidentValue<T>> for InodeConfidentValue<T> {
  fn from(value: PaxConfidentValue<T>) -> Self {
    let mut confident_value = ConfidentValue::default();
    if let Some((pax_confidence, value)) = value.get_with_confidence() {
      confident_value.set(TarConfidence::from(pax_confidence), value.clone());
    }
    confident_value
  }
}

pub(crate) struct InodeBuilder {
  pub(crate) file_path: InodeConfidentValue<String>,
  pub(crate) mode: InodeConfidentValue<FilePermissions>,
  pub(crate) uid: InodeConfidentValue<u32>,
  pub(crate) gid: InodeConfidentValue<u32>,
  pub(crate) mtime: InodeConfidentValue<TimeStamp>,
  pub(crate) atime: InodeConfidentValue<TimeStamp>,
  pub(crate) ctime: InodeConfidentValue<TimeStamp>,
  pub(crate) uname: InodeConfidentValue<String>,
  pub(crate) gname: InodeConfidentValue<String>,
  pub(crate) link_target: InodeConfidentValue<String>,
  pub(crate) sparse_file_instructions: LimitedVec<SparseFileInstruction>,
  /// The realsize if it is a sparse file.
  pub(crate) sparse_real_size: InodeConfidentValue<usize>,
  pub(crate) sparse_format: Option<SparseFormat>,
  pub(crate) dev_major: u32,
  pub(crate) dev_minor: u32,
  pub(crate) data_after_header_size: InodeConfidentValue<usize>,
  pub(crate) contiguous_file: bool,
  pub(crate) data: Vec<u8>,
}

impl InodeBuilder {
  #[must_use]
  pub fn new(max_sparse_file_instructions: usize) -> Self {
    Self {
      file_path: Default::default(),
      mode: Default::default(),
      uid: Default::default(),
      gid: Default::default(),
      mtime: Default::default(),
      atime: Default::default(),
      ctime: Default::default(),
      uname: Default::default(),
      gname: Default::default(),
      link_target: Default::default(),
      sparse_file_instructions: LimitedVec::new(max_sparse_file_instructions),
      sparse_real_size: Default::default(),
      sparse_format: None,
      dev_major: 0,
      dev_minor: 0,
      data_after_header_size: Default::default(),
      contiguous_file: false,
      data: Vec::new(),
    }
  }
}

impl From<InodeBuilder> for RegularFileEntry {
  fn from(inode_builder: InodeBuilder) -> Self {
    let contiguous = inode_builder.contiguous_file;
    let data = if inode_builder.sparse_file_instructions.is_empty() {
      FileData::Regular(inode_builder.data)
    } else {
      FileData::Sparse {
        instructions: inode_builder.sparse_file_instructions.to_vec(),
        data: inode_builder.data,
      }
    };

    Self { contiguous, data }
  }
}

impl<VH: TarViolationHandler + Default> Default for TarParser<VH> {
  fn default() -> Self {
    Self::try_new(TarParserOptions::default(), VH::default())
      .expect("BUG: Default TarParser should always be creatable")
  }
}

impl<VH: TarViolationHandler> TarParser<VH> {
  pub fn try_new(
    options: TarParserOptions,
    mut violation_handler: VH,
  ) -> Result<Self, TarParserError> {
    let mut violation_handler_wrapped = VHW(&mut violation_handler);
    Ok(Self {
      extracted_files: Default::default(),

      found_type_flags: Default::default(),
      seen_files: Default::default(),
      keep_only_last: options.keep_only_last,

      parser_state: Default::default(),
      pax_parser: PaxParser::try_new(
        &mut violation_handler_wrapped,
        options.initial_global_extended_attributes,
        options.tar_parser_limits.max_global_attributes,
        options.tar_parser_limits.max_unparsed_global_attributes,
        options.tar_parser_limits.max_unparsed_local_attributes,
        options.tar_parser_limits.max_pax_key_value_length,
        options.tar_parser_limits.max_sparse_file_instructions,
      )?,
      inode_state: InodeBuilder::new(options.tar_parser_limits.max_sparse_file_instructions),
      header_buffer: Cursor::new([0; BLOCK_SIZE]),
      sparse_parser: GnuSparse1_0Parser::new(),

      limits: options.tar_parser_limits,
      violation_handler,
    })
  }

  fn recover_internal(&mut self) -> InodeBuilder {
    self.pax_parser.recover();
    self.parser_state = Default::default();
    core::mem::replace(
      &mut self.inode_state,
      InodeBuilder::new(self.limits.max_sparse_file_instructions),
    )
  }

  pub fn recover(&mut self) {
    self.recover_internal();
  }

  /// Returns the currently active global extended pax attributes.
  pub fn get_global_extended_attributes(&self) -> &HashMap<String, String> {
    &self.pax_parser.global_extended_attributes()
  }

  /// Returns the files that have been extracted so far.
  pub fn get_extracted_files(&self) -> &[TarInode] {
    &self.extracted_files
  }

  /// Returns the number of files found with each type flag.
  pub fn get_found_type_flags(&self) -> &HashMap<TarTypeFlag, usize> {
    &self.found_type_flags
  }

  fn parse_old_gnu_sparse_instructions(
    inode_state: &mut InodeBuilder,
    sparse_headers: &[GnuSparseInstruction],
  ) {
    debug_assert_eq!(inode_state.sparse_format, Some(SparseFormat::GnuOld));
    for sparse_header in sparse_headers {
      if sparse_header.is_empty() {
        continue;
      }
      if let Ok(instruction) = sparse_header.convert_to_sparse_instruction() {
        inode_state.sparse_file_instructions.push(instruction);
      } else {
        // If we can't parse the sparse header, we just ignore it.
        // This is a best-effort approach.
      }
    }
  }

  fn finish_inode(&mut self, file_entry: impl FnOnce(&mut Self, InodeBuilder) -> FileEntry) {
    self
      .pax_parser
      .load_pax_attributes_into_inode_builder(&mut self.inode_state);
    let inode_builder = self.recover_internal();

    // TODO: These clones can definitely be optimized.
    // Splitting the Inode builder into two parts would be a good start.
    let tar_inode = TarInode {
      path: inode_builder
        .file_path
        .get()
        .cloned()
        .unwrap_or_else(|| "".to_string()),
      entry: FileEntry::Fifo,
      mode: inode_builder
        .mode
        .get()
        .map(Clone::clone)
        .unwrap_or_else(|| FilePermissions::default()),
      uid: inode_builder.uid.get().cloned().unwrap_or(0),
      gid: inode_builder.gid.get().cloned().unwrap_or(0),
      mtime: inode_builder.mtime.get().cloned().unwrap_or_default(),
      atime: inode_builder.atime.get().cloned().unwrap_or_default(),
      ctime: inode_builder.ctime.get().cloned().unwrap_or_default(),
      uname: inode_builder.uname.get().cloned().unwrap_or_default(),
      gname: inode_builder.gname.get().cloned().unwrap_or_default(),
      unparsed_extended_attributes: self.pax_parser.drain_local_unparsed_attributes(),
    };

    let file_entry = file_entry(self, inode_builder);

    // If we are keeping only the last version of each file, we check if we have seen this file before.
    if self.keep_only_last {
      if let Some(index) = self.seen_files.get(&tar_inode.path) {
        // We have seen this file before, so we replace the old entry.
        self.extracted_files[*index] = TarInode {
          entry: file_entry,
          ..tar_inode
        };
      } else {
        // We haven't seen this file before, so we add it to the list.
        self
          .seen_files
          .insert(tar_inode.path.clone(), self.extracted_files.len());
        self.extracted_files.push(TarInode {
          entry: file_entry,
          ..tar_inode
        });
      }
    } else {
      // We just add the new file to the list.
      self.extracted_files.push(TarInode {
        entry: file_entry,
        ..tar_inode
      });
    }
  }

  fn compute_file_parsing_state(
    &mut self,
    data_after_header: usize,
    padding_after_data: usize,
  ) -> TarParserState {
    if self.pax_parser.get_sparse_format() == Some(SparseFormat::Gnu1_0) {
      self.sparse_parser.reset();
      TarParserState::ParsingGnuSparse1_0(StateParsingGnuSparse1_0 {
        data_after_header,
        padding_after: padding_after_data,
      })
    } else {
      TarParserState::ReadingFileData(StateReadingFileData {
        remaining_data: data_after_header,
        padding_after: padding_after_data,
      })
    }
  }

  fn compute_opt_skip_state(
    &mut self,
    data_after_header: usize,
    context: &'static str,
  ) -> TarParserState {
    if data_after_header > 0 {
      TarParserState::SkippingData(StateSkippingData {
        remaining_data: data_after_header,
        context,
      })
    } else {
      TarParserState::default()
    }
  }

  #[must_use]
  fn map_corrupt_header_field<T: Into<GeneralParseError>>(
    field: CorruptFieldContext,
  ) -> impl FnOnce(T) -> TarParserError {
    move |error| TarParserError::CorruptField {
      field,
      error: error.into(),
    }
  }

  fn parse_v7_header(
    vh: &mut VHW<'_, VH>,
    found_type_flags: &mut HashMap<TarTypeFlag, usize>,
    inode_state: &mut InodeBuilder,
    old_header: &V7Header,
  ) -> Result<TarTypeFlag, TarParserError> {
    // verify checksum
    vh.hpvr(
      old_header
        .verify_checksum()
        .map_err(TarHeaderParserError::CorruptHeaderChecksum),
    )?;

    let typeflag = old_header.parse_typeflag();
    if let Some(count) = found_type_flags.get_mut(&typeflag) {
      *count += 1;
    } else {
      found_type_flags.insert(typeflag.clone(), 1);
    }

    // parse the information from the old header
    vh.hpvr(
      inode_state
        .data_after_header_size
        .try_get_or_set_with(TarConfidence::V7, || old_header.parse_size())
        .map_err(Self::map_corrupt_header_field(
          CorruptFieldContext::HeaderSize,
        )),
    )?;

    if typeflag.is_file_like() {
      vh.hpvr(
        inode_state
          .file_path
          .try_get_or_set_with(TarConfidence::V7, || old_header.parse_name())
          .map_err(Self::map_corrupt_header_field(
            CorruptFieldContext::HeaderName,
          )),
      )?;
      vh.hpvr(
        inode_state
          .mode
          .try_get_or_set_with(TarConfidence::V7, || old_header.parse_mode())
          .map_err(Self::map_corrupt_header_field(
            CorruptFieldContext::HeaderMode,
          )),
      )?;
      vh.hpvr(
        inode_state
          .uid
          .try_get_or_set_with(TarConfidence::V7, || old_header.parse_uid())
          .map_err(Self::map_corrupt_header_field(
            CorruptFieldContext::HeaderUid,
          )),
      )?;
      vh.hpvr(
        inode_state
          .gid
          .try_get_or_set_with(TarConfidence::V7, || old_header.parse_gid())
          .map_err(Self::map_corrupt_header_field(
            CorruptFieldContext::HeaderGid,
          )),
      )?;

      vh.hpvr(
        inode_state
          .mtime
          .try_get_or_set_with(TarConfidence::V7, || old_header.parse_mtime())
          .map_err(Self::map_corrupt_header_field(
            CorruptFieldContext::HeaderMtime,
          )),
      )?;
    }

    if typeflag.is_link_like() {
      vh.hpvr(
        inode_state
          .link_target
          .try_get_or_set_with(TarConfidence::V7, || {
            old_header.parse_linkname().map(String::from)
          })
          .map_err(Self::map_corrupt_header_field(
            CorruptFieldContext::HeaderLinkname,
          )),
      )?;
    }

    Ok(typeflag)
  }

  fn parse_common_header_additions(
    vh: &mut VHW<'_, VH>,
    inode_state: &mut InodeBuilder,
    common_header_additions: &CommonHeaderAdditions,
  ) -> Result<(), TarParserError> {
    vh.hpvr(
      inode_state
        .uname
        .try_get_or_set_with(TarConfidence::Ustar, || {
          common_header_additions.parse_uname().map(String::from)
        })
        .map_err(Self::map_corrupt_header_field(
          CorruptFieldContext::HeaderUname,
        )),
    )?;
    vh.hpvr(
      inode_state
        .gname
        .try_get_or_set_with(TarConfidence::Ustar, || {
          common_header_additions.parse_gname().map(String::from)
        })
        .map_err(Self::map_corrupt_header_field(
          CorruptFieldContext::HeaderGname,
        )),
    )?;
    if let Some(dev_major) = vh.hpvr(common_header_additions.parse_dev_major().map_err(
      Self::map_corrupt_header_field(CorruptFieldContext::HeaderDevMajor),
    ))? {
      inode_state.dev_major = dev_major;
    }
    if let Some(dev_minor) = vh.hpvr(common_header_additions.parse_dev_minor().map_err(
      Self::map_corrupt_header_field(CorruptFieldContext::HeaderDevMinor),
    ))? {
      inode_state.dev_minor = dev_minor;
    }
    Ok(())
  }

  fn state_reading_tar_header(
    &mut self,
    reader: &mut Cursor<&[u8]>,
  ) -> Result<TarParserState, TarParserError> {
    // header parsing variables
    let mut typeflag = TarTypeFlag::UnknownTypeFlag(255);
    let mut old_gnu_sparse_is_extended = false;

    // TODO: fix strict mode recovery is not possible because we consume the buffer here.
    // We should wait to consume the buffer until we have fully parsed the header.
    let header_buffer = match buffer_array(reader, &mut self.header_buffer)? {
      Some(buffer) => buffer,
      None => {
        // We don't have a complete buffer yet, so we need to wait for more data.
        return Ok(TarParserState::ReadingTarHeader);
      },
    };

    if header_buffer == TAR_ZERO_HEADER {
      // We have reached the end of the tar archive.
      // However we remain ready to read the next header.
      return Ok(TarParserState::default());
    }

    let old_header =
      V7Header::ref_from_bytes(&header_buffer).expect("BUG: Not enough bytes for OldHeader");

    let vh = &mut VHW(&mut self.violation_handler);

    // This parses all fields in a header block regardless of the typeflag.
    // There is some room for improving allocations/parsing based on the typeflag.
    match &old_header.magic_version {
      V7Header::MAGIC_VERSION_V7 => {
        typeflag = Self::parse_v7_header(
          vh,
          &mut self.found_type_flags,
          &mut self.inode_state,
          old_header,
        )?;
        // Done v7 header parsing.
      },
      V7Header::MAGIC_VERSION_USTAR => {
        typeflag = Self::parse_v7_header(
          vh,
          &mut self.found_type_flags,
          &mut self.inode_state,
          old_header,
        )?;

        if typeflag.is_file_like() {
          let common_header_additions = CommonHeaderAdditions::ref_from_bytes(&old_header.padding)
            .expect("BUG: Not enough bytes for CommonHeaderAdditions in USTAR");
          Self::parse_common_header_additions(vh, &mut self.inode_state, common_header_additions)?;
          let ustar_additions =
            UstarHeaderAdditions::ref_from_bytes(&common_header_additions.padding)
              .expect("BUG: Not enough bytes for UstarHeaderAdditions");

          // If there is already a path with a confidence of USTAR or less, we want to prefix the path with the ustar prefix.
          // If there is no path, we want to use the ustar prefix as the path.
          if let Some(potential_path) = self
            .inode_state
            .file_path
            .extract_if_confidence_le(&TarConfidence::Ustar)
          {
            let prefix = ustar_additions.parse_prefix();
            // prefix.join(potential_path)
            let joined = match prefix {
              Ok(prefix) => {
                if prefix.is_empty() {
                  potential_path
                } else {
                  format!("{}/{}", prefix, potential_path)
                }
              },
              Err(parse_error) => {
                vh.hpve(Self::map_corrupt_header_field(
                  CorruptFieldContext::HeaderPrefix,
                )(parse_error))?;
                potential_path
              },
            };
            self.inode_state.file_path.set(TarConfidence::Ustar, joined);
          } else {
            vh.hpvr(
              self
                .inode_state
                .file_path
                .try_get_or_set_with(TarConfidence::Ustar, || {
                  ustar_additions.parse_prefix().map(String::from)
                })
                .map_err(Self::map_corrupt_header_field(
                  CorruptFieldContext::HeaderPrefix,
                )),
            )?;
          }
        }

        // Done ustar header parsing.
      },
      V7Header::MAGIC_VERSION_GNU => {
        typeflag = Self::parse_v7_header(
          vh,
          &mut self.found_type_flags,
          &mut self.inode_state,
          old_header,
        )?;

        let common_header_additions = CommonHeaderAdditions::ref_from_bytes(&old_header.padding)
          .expect("BUG: Not enough bytes for CommonHeaderAdditions in GNU");
        Self::parse_common_header_additions(vh, &mut self.inode_state, common_header_additions)?;
        let gnu_additions = GnuHeaderAdditions::ref_from_bytes(&common_header_additions.padding)
          .expect("BUG: Not enough bytes for GnuHeaderAdditions");

        if typeflag.is_file_like() {
          vh.hpvr(
            self
              .inode_state
              .atime
              .try_get_or_set_with(TarConfidence::Gnu, || gnu_additions.parse_atime())
              .map_err(Self::map_corrupt_header_field(
                CorruptFieldContext::HeaderAtime,
              )),
          )?;
          vh.hpvr(
            self
              .inode_state
              .ctime
              .try_get_or_set_with(TarConfidence::Gnu, || gnu_additions.parse_ctime())
              .map_err(Self::map_corrupt_header_field(
                CorruptFieldContext::HeaderCtime,
              )),
          )?;
        }

        // Handle sparse entries (Old GNU Format)
        if typeflag == TarTypeFlag::SparseOldGnu {
          self.inode_state.sparse_format = Some(SparseFormat::GnuOld);
          Self::parse_old_gnu_sparse_instructions(&mut self.inode_state, &gnu_additions.sparse);
          old_gnu_sparse_is_extended = gnu_additions.parse_is_extended();
        }

        vh.hpvr(
          self
            .inode_state
            .sparse_real_size
            .try_get_or_set_with(TarConfidence::Gnu, || {
              gnu_additions.parse_real_size().map(|s| s as usize)
            })
            .map_err(Self::map_corrupt_header_field(
              CorruptFieldContext::HeaderRealSize,
            )),
        )?;

        // Done GNU header parsing.
      },
      unknown_version_magic => {
        return Err(
          TarHeaderParserError::UnknownHeaderMagicVersion {
            magic: unknown_version_magic[..6].try_into().unwrap(),
            version: unknown_version_magic[6..].try_into().unwrap(),
          }
          .into(),
        );
      },
    }
    // We parsed everything from the header block and released the buffer.

    let data_after_header = *self.inode_state.data_after_header_size.get().unwrap_or(&0);
    let data_after_header_block_aligned = align_to_block_size(data_after_header); // align to next 512 byte block
    let padding_after_data = data_after_header_block_aligned - data_after_header; // padding after header block

    // now we match on the typeflag
    Ok(match typeflag {
      TarTypeFlag::RegularFile => {
        self.inode_state.contiguous_file = false;
        self.compute_file_parsing_state(data_after_header, padding_after_data)
      },
      TarTypeFlag::HardLink => {
        self.finish_inode(|selv, inode_state| {
          FileEntry::HardLink(HardLinkEntry {
            link_target: inode_state
              .link_target
              .get()
              .map(|v| v.clone())
              .unwrap_or_default(),
          })
        });
        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after HardLink")
      },
      TarTypeFlag::SymbolicLink => {
        self.finish_inode(|selv, inode_state| {
          FileEntry::SymbolicLink(SymbolicLinkEntry {
            link_target: inode_state
              .link_target
              .get()
              .map(|v| v.clone())
              .unwrap_or_default(),
          })
        });

        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after SymbolicLink")
      },
      TarTypeFlag::CharacterDevice => {
        self.finish_inode(|selv, inode_state| {
          FileEntry::CharacterDevice(CharacterDeviceEntry {
            major: inode_state.dev_major,
            minor: inode_state.dev_minor,
          })
        });

        self.compute_opt_skip_state(
          data_after_header_block_aligned,
          "Data after CharacterDevice",
        )
      },
      TarTypeFlag::BlockDevice => {
        self.finish_inode(|selv, inode_state| {
          FileEntry::BlockDevice(BlockDeviceEntry {
            major: inode_state.dev_major,
            minor: inode_state.dev_minor,
          })
        });
        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after BlockDevice")
      },
      TarTypeFlag::Directory => {
        self.finish_inode(|_, _| FileEntry::Directory);
        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after Directory")
      },
      TarTypeFlag::Fifo => {
        self.finish_inode(|_, _| FileEntry::Fifo);
        self.compute_opt_skip_state(data_after_header_block_aligned, "Data after Fifo")
      },
      TarTypeFlag::ContiguousFile => {
        self.inode_state.contiguous_file = true;
        self.compute_file_parsing_state(data_after_header, padding_after_data)
      },
      TarTypeFlag::PaxExtendedHeader => {
        self.pax_parser.set_current_pax_mode(PaxConfidence::LOCAL);
        TarParserState::ParsingPaxData(StateParsingPaxData {
          remaining_data: data_after_header,
          padding_after: padding_after_data,
          pax_mode: PaxConfidence::LOCAL, // We are parsing a local PAX header.
        })
      },
      TarTypeFlag::PaxGlobalExtendedHeader => {
        self.pax_parser.set_current_pax_mode(PaxConfidence::GLOBAL);
        TarParserState::ParsingPaxData(StateParsingPaxData {
          remaining_data: data_after_header,
          padding_after: padding_after_data,
          pax_mode: PaxConfidence::GLOBAL, // We are parsing a local PAX header.
        })
      },
      TarTypeFlag::LongNameGnu => {
        TarParserState::ParsingGnuLongName(StateParsingGnuLongName {
          remaining_data: data_after_header,
          padding_after_data,
          long_name_type: GnuLongNameType::FileName,
          collected_name: Vec::new(), // We don't use with_capacity here since this is a user controlled value and we don't want to exhaust resources.
        })
      },
      TarTypeFlag::LongLinkNameGnu => {
        TarParserState::ParsingGnuLongName(StateParsingGnuLongName {
          remaining_data: data_after_header,
          padding_after_data,
          long_name_type: GnuLongNameType::LinkName,
          collected_name: Vec::new(), // We don't use with_capacity here since this is a user controlled value and we don't want to exhaust resources.
        })
      },
      TarTypeFlag::SparseOldGnu => {
        if old_gnu_sparse_is_extended {
          TarParserState::ReadingOldGnuSparseExtendedHeader(
            StateReadingOldGnuSparseExtendedHeader {
              data_after_header,
              padding_after_data,
            },
          )
        } else {
          TarParserState::ReadingFileData(StateReadingFileData {
            remaining_data: data_after_header,
            padding_after: padding_after_data,
          })
        }
      },
      TarTypeFlag::UnknownTypeFlag(_) => {
        // we just skip the data_after_header bytes if we don't know the typeflag
        self.compute_opt_skip_state(data_after_header_block_aligned, "Unknown typeflag")
      },
    })
  }

  fn state_skipping_data(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateSkippingData,
  ) -> Result<TarParserState, TarParserError> {
    // incrementally skip the data
    let skipped_bytes = reader
      .read_buffered(state.remaining_data)
      .unwrap_infallible()
      .len();
    state.remaining_data -= skipped_bytes;
    Ok(if state.remaining_data == 0 {
      // We are done skipping unknown data, so we reset the parser state.
      TarParserState::default()
    } else {
      // We still have some data to skip, so we keep the parser state.
      TarParserState::SkippingData(state)
    })
  }

  fn state_parsing_gnu_long_name(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateParsingGnuLongName,
  ) -> Result<TarParserState, TarParserError> {
    // incrementally read the long name
    let long_name_bytes = reader
      .read_buffered(state.remaining_data)
      .unwrap_infallible();

    state.collected_name.extend_from_slice(long_name_bytes);
    state.remaining_data -= long_name_bytes.len();
    Ok(if state.remaining_data == 0 {
      // We are done reading the long name, so we parse it.
      let null_term = find_null_terminator_index(&state.collected_name);
      state.collected_name.truncate(null_term);
      let long_name = String::from_utf8(state.collected_name);

      if let Ok(long_name) = long_name {
        // Now we can insert the long name into the inode state.
        match state.long_name_type {
          GnuLongNameType::FileName => {
            self
              .inode_state
              .file_path
              .get_or_set_with(TarConfidence::Gnu, || Some(long_name));
          },
          GnuLongNameType::LinkName => {
            self
              .inode_state
              .link_target
              .get_or_set_with(TarConfidence::Gnu, || Some(long_name));
          },
        }
      } else {
        // TODO: log this
      }

      if state.padding_after_data > 0 {
        // We have some padding after the long name, so we skip it.
        TarParserState::SkippingData(StateSkippingData {
          remaining_data: state.padding_after_data,
          context: "Padding after long name",
        })
      } else {
        // We are done with the long name and there is no padding, so we reset the parser state.
        TarParserState::default()
      }
    } else {
      // We still have some data to read, so we keep the parser state.
      TarParserState::ParsingGnuLongName(state)
    })
  }

  fn state_reading_old_gnu_sparse_extended_header(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    state: StateReadingOldGnuSparseExtendedHeader,
  ) -> Result<TarParserState, TarParserError> {
    // We must read the next block to get more sparse headers.

    // TODO: possible bug in the future we should only advance after we have fully parsed the extended header.
    let extended_header_buffer = match buffer_array(reader, &mut self.header_buffer)? {
      Some(buffer) => buffer,
      None => {
        // We don't have a complete buffer yet, so we need to wait for more data.
        return Ok(TarParserState::ReadingOldGnuSparseExtendedHeader(state));
      },
    };

    let extended_header = GnuHeaderExtSparse::ref_from_bytes(&extended_header_buffer)
      .expect("BUG: Not enough bytes for GnuHeaderExtSparse");
    Self::parse_old_gnu_sparse_instructions(&mut self.inode_state, &extended_header.sparse);
    Ok(if extended_header.parse_is_extended() {
      // If the extended header is still extended, we need to read the next block.
      TarParserState::ReadingOldGnuSparseExtendedHeader(state)
    } else {
      TarParserState::ReadingFileData(StateReadingFileData {
        remaining_data: state.data_after_header,
        padding_after: state.padding_after_data,
      })
    })
  }

  fn state_parsing_pax_data(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateParsingPaxData,
  ) -> Result<TarParserState, TarParserError> {
    // incrementally read the PAX data
    let pax_bytes = reader
      .peek_buffered(state.remaining_data)
      .unwrap_infallible();

    let vh = &mut VHW(&mut self.violation_handler);

    let bytes_read = self.pax_parser.parse(vh, pax_bytes)?;
    reader.skip_buffered(bytes_read).unwrap_infallible();

    state.remaining_data -= bytes_read;
    Ok(if state.remaining_data == 0 {
      // We are done reading the PAX data, so we reset the parser state.
      if state.padding_after > 0 {
        // We have some padding after the PAX data, so we skip it.
        TarParserState::SkippingData(StateSkippingData {
          remaining_data: state.padding_after,
          context: "Padding after PAX data",
        })
      } else {
        TarParserState::default()
      }
    } else {
      // We still have some data to read, so we keep the parser state.
      TarParserState::ParsingPaxData(state)
    })
  }

  fn state_parsing_gnu_sparse_1_0(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    state: StateParsingGnuSparse1_0,
  ) -> Result<TarParserState, TarParserError> {
    let vh = &mut VHW(&mut self.violation_handler);

    let done =
      self
        .sparse_parser
        .parse(vh, reader, &mut self.inode_state.sparse_file_instructions)?;

    if !done {
      // We still have some data to read, so we keep the parser state.
      return Ok(TarParserState::ParsingGnuSparse1_0(state));
    }

    // We are done reading the sparse data
    let remaining_data = state.data_after_header - self.sparse_parser.bytes_read;
    Ok(TarParserState::ReadingFileData(StateReadingFileData {
      remaining_data: remaining_data,
      padding_after: state.padding_after,
    }))
  }

  fn state_reading_file_data(
    &mut self,
    reader: &mut Cursor<&[u8]>,
    mut state: StateReadingFileData,
  ) -> Result<TarParserState, TarParserError> {
    // incrementally read the file data
    let file_data_bytes = reader
      .read_buffered(state.remaining_data)
      .unwrap_infallible();

    self.inode_state.data.extend_from_slice(file_data_bytes);
    state.remaining_data -= file_data_bytes.len();

    if state.remaining_data != 0 {
      // We still have some data to read, so we keep the parser state.
      return Ok(TarParserState::ReadingFileData(state));
    }

    // We are done reading the file data, so we can finish the inode.
    self.finish_inode(|selv, inode_state| FileEntry::RegularFile(inode_state.into()));

    Ok(self.compute_opt_skip_state(state.padding_after, "Padding after file data"))
  }
}

impl<VH: TarViolationHandler> Write for TarParser<VH> {
  type WriteError = TarParserError;
  type FlushError = Infallible;

  fn write(&mut self, input_buffer: &[u8], _sync_hint: bool) -> Result<usize, Self::WriteError> {
    let mut cursor = Cursor::new(input_buffer);
    loop {
      let parser_state = core::mem::replace(&mut self.parser_state, TarParserState::NoNextStateSet);

      let initial_cursor_position = cursor.position();

      let next_state = match parser_state {
        TarParserState::ReadingTarHeader => self.state_reading_tar_header(&mut cursor),
        TarParserState::SkippingData(state) => self.state_skipping_data(&mut cursor, state),
        TarParserState::ParsingGnuLongName(state) => {
          self.state_parsing_gnu_long_name(&mut cursor, state)
        },
        TarParserState::ReadingOldGnuSparseExtendedHeader(state) => {
          self.state_reading_old_gnu_sparse_extended_header(&mut cursor, state)
        },
        TarParserState::ParsingPaxData(state) => self.state_parsing_pax_data(&mut cursor, state),
        TarParserState::ParsingGnuSparse1_0(state) => {
          self.state_parsing_gnu_sparse_1_0(&mut cursor, state)
        },
        TarParserState::ReadingFileData(state) => self.state_reading_file_data(&mut cursor, state),
        TarParserState::NoNextStateSet => {
          unreachable!("BUG: No next state set in TarParser");
        },
      };
      let bytes_read_this_parse = cursor.position() - initial_cursor_position;

      self.parser_state = next_state?;

      if bytes_read_this_parse == 0 {
        return Ok(cursor.position());
      }
    }
  }

  fn flush(&mut self) -> Result<(), Self::FlushError> {
    Ok(())
  }
}
