#!/bin/bash
# Pack the folder 'test-archive' into a tarball using different tar technologies

# change dir to the script's directory
cd "$(dirname "$0")"

# Check if the test directory exists
if [ ! -d "test-archive" ]; then
  echo "Directory 'test-archive' does not exist."
  exit 1
fi

# Create tar archives using common formats

# POSIX.1-2001 pax format (recommended)
tar --format=pax -cf test-pax.tar test-archive

# GNU tar format
tar --format=gnu -cf test-gnu.tar test-archive

# POSIX ustar format
tar --format=ustar -cf test-ustar.tar test-archive

# V7 tar format (limited)
tar --format=v7 -cf test-v7.tar test-archive

# Create gzip-compressed versions (zlib)
gzip -k -f test-pax.tar   # -> test-pax.tar.gz
gzip -k -f test-gnu.tar   # -> test-gnu.tar.gz
gzip -k -f test-ustar.tar # -> test-ustar.tar.gz
gzip -k -f test-v7.tar    # -> test-v7.tar.gz

echo "Archives created:"
echo "  Uncompressed: test-pax.tar, test-gnu.tar, test-ustar.tar, test-v7.tar"
echo "  Compressed  : test-pax.tar.gz, test-gnu.tar.gz, test-ustar.tar.gz, test-v7.tar.gz"
