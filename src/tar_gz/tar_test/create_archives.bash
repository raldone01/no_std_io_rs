#!/bin/bash
# Pack the folder 'test-archive' into a tarball using multiple tar formats and GNU sparse variants

# Change directory to the location of this script
cd "$(dirname "$0")"

# Check if the test directory exists
if [ ! -d "test-archive" ]; then
  echo "Directory 'test-archive' does not exist."
  exit 1
fi

# Define sparse file path
sparse_output_file="test-archive/sparse_test_file.txt"

# Create a 2 MB sparse file
truncate -s 2M "$sparse_output_file"

# Add "Test" at offset 0
printf 'Test\n' | dd of="$sparse_output_file" bs=1 seek=0 conv=notrunc

# Add "Test" at 1 KB offset
printf 'Test\n' | dd of="$sparse_output_file" bs=1 seek=$((1 * 1024)) conv=notrunc

# Add "Test" at 1 MB offset
printf 'Test\n' | dd of="$sparse_output_file" bs=1 seek=$((1 * 1024 * 1024)) conv=notrunc

# Create tar archives using different formats

# POSIX.1-2001 pax format (recommended)
tar --format=pax -cf test-pax.tar test-archive

# GNU tar formats

# GNU tar without sparse support
tar --format=gnu -cf test-gnu-nosparse.tar test-archive

# GNU sparse format version 0.0
tar --format=gnu --sparse-version=0.0 -cf test-gnu-sparse-0.0.tar test-archive

# GNU sparse format version 0.1
tar --format=gnu --sparse-version=0.1 -cf test-gnu-sparse-0.1.tar test-archive

# GNU sparse format version 1.0
tar --format=gnu --sparse-version=1.0 -cf test-gnu-sparse-1.0.tar test-archive

# POSIX ustar format
tar --format=ustar -cf test-ustar.tar test-archive

# V7 tar format (limited)
tar --format=v7 -cf test-v7.tar test-archive

# Create gzip-compressed versions
gzip -k -f test-pax.tar

gzip -k -f test-gnu-nosparse.tar
gzip -k -f test-gnu-sparse-0.0.tar
gzip -k -f test-gnu-sparse-0.1.tar
gzip -k -f test-gnu-sparse-1.0.tar

gzip -k -f test-ustar.tar
gzip -k -f test-v7.tar

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
