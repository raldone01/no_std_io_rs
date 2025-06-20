#!/bin/bash
# Pack the folder 'test-archive' into a tarball using multiple tar formats and GNU sparse variants

# Change directory to the location of this script
cd "$(dirname "$0")"

# Check if the test directory exists
if [ ! -d "test-archive" ]; then
  echo "Directory 'test-archive' does not exist."
  echo "Please create it first: mkdir test-archive"
  exit 1
fi

# --- Generate special files for testing ---

SPECIAL_FILES_DIR="test-archive/special_files"
echo "Preparing special files in $SPECIAL_FILES_DIR..."

# Clean up previous special files to ensure a fresh start
rm -rf "$SPECIAL_FILES_DIR"
mkdir -p "$SPECIAL_FILES_DIR"

# 1. Long file names (to test ustar limitations and pax/gnu extensions)
# Ustar has a 100-char name limit and a 155-char prefix limit. We'll create a path longer than that.
LONG_DIR_NAME=$(printf 'long_directory_name_%.0s' {1..20})
LONG_FILE_NAME=$(printf 'long_file_name_%.0s' {1..20})
mkdir -p "$SPECIAL_FILES_DIR/$LONG_DIR_NAME"
echo "This file has a very long path." >"$SPECIAL_FILES_DIR/$LONG_DIR_NAME/$LONG_FILE_NAME"

# 2. Long link name (for symbolic links)
LONG_LINK_TARGET=$(printf 'a/b/c/this_is_a_very_long_symlink_target_path_that_exceeds_the_standard_ustar_limit/%.0s' {1..10})
ln -s "$LONG_LINK_TARGET" "$SPECIAL_FILES_DIR/symlink_with_long_target"

# 3. Hardlinks
echo "This is the source file for the hardlink." >"$SPECIAL_FILES_DIR/hardlink_source"
ln "$SPECIAL_FILES_DIR/hardlink_source" "$SPECIAL_FILES_DIR/hardlink_to_source"

# 4. Symlinks
echo "This is the target file for the symlink." >"$SPECIAL_FILES_DIR/symlink_target"
ln -s "symlink_target" "$SPECIAL_FILES_DIR/symlink_to_target"

# 5. FIFOs (named pipes)
mkfifo "$SPECIAL_FILES_DIR/my_fifo"

# 6. Block and Character Devices (requires root privileges)
if [ "$(id -u)" -eq 0 ]; then
  echo "Running as root, creating device files."
  # Use standard major/minor numbers for common devices (e.g., /dev/null, /dev/loop0)
  mknod "$SPECIAL_FILES_DIR/my_char_device" c 1 3
  mknod "$SPECIAL_FILES_DIR/my_block_device" b 7 0
else
  echo "WARNING: Not running as root. Skipping creation of block and character device files."
  echo "         Run this script with 'sudo' to include device files in the test archives."
fi
echo "Special file generation complete."
echo

# --- End of special file generation ---

# Define sparse file path
sparse_output_file="test-archive/sparse_test_file.txt"

# Create a 2 MB sparse file
truncate -s 2M "$sparse_output_file"

# Write "Test\n"
for ((j = 0; j < 128; j++)); do
  offset=$((j * 16 * 1024))
  printf 'Test\n' | dd of="$sparse_output_file" bs=1 seek=$((offset)) conv=notrunc status=none
done

# Create tar archives using different formats

# POSIX.1-2001 pax format (recommended, handles long names and special files)
tar --format=pax -cf test-pax.tar test-archive

# GNU tar formats

# GNU tar without sparse support (handles long names and special files)
tar --format=gnu -cf test-gnu-nosparse.tar test-archive

# GNU sparse format version 0.0
tar --format=gnu --sparse-version=0.0 -cf test-gnu-sparse-0.0.tar test-archive

# GNU sparse format version 0.1
tar --format=gnu --sparse-version=0.1 -cf test-gnu-sparse-0.1.tar test-archive

# GNU sparse format version 1.0
tar --format=gnu --sparse-version=1.0 -cf test-gnu-sparse-1.0.tar test-archive

# POSIX ustar format (will warn about long file names)
echo "--- Creating ustar archive (expect warnings about long names) ---"
tar --format=ustar -cf test-ustar.tar test-archive
echo "-----------------------------------------------------------------"

# V7 tar format (very limited, will warn about many things)
echo "--- Creating v7 archive (expect warnings) ---"
tar --format=v7 -cf test-v7.tar test-archive
echo "-------------------------------------------"

# Create gzip-compressed versions
gzip -k -f test-pax.tar

gzip -k -f test-gnu-nosparse.tar
gzip -k -f test-gnu-sparse-0.0.tar
gzip -k -f test-gnu-sparse-0.1.tar
gzip -k -f test-gnu-sparse-1.0.tar

gzip -k -f test-ustar.tar
gzip -k -f test-v7.tar

echo
echo "Archives created:"
echo "  Uncompressed:"
echo "    test-pax.tar"
echo "    test-gnu-nosparse.tar"
echo "    test-gnu-sparse-0.0.tar"
echo "    test-gnu-sparse-0.1.tar"
echo "    test-gnu-sparse-1.0.tar"
echo "    test-ustar.tar"
echo "    test-v7.tar"
echo "  Compressed:"
echo "    test-pax.tar.gz"
echo "    test-gnu-nosparse.tar.gz"
echo "    test-gnu-sparse-0.0.tar.gz"
echo "    test-gnu-sparse-0.1.tar.gz"
echo "    test-gnu-sparse-1.0.tar.gz"
echo "    test-ustar.tar.gz"
echo "    test-v7.tar.gz"

# Optional: Uncomment the following lines to clean up all generated files
# echo
# echo "Cleaning up..."
# rm -f test-*.tar test-*.tar.gz
# rm -rf "test-archive/special_files"
# rm -f "test-archive/sparse_test_file.txt"
# rm -f "test-archive/.gitignore"
# echo "Cleanup complete."
