#!/bin/bash

echo "SPV Hash Diagnosis Script"
echo "========================"
echo ""
echo "This script will help diagnose why SPV is requesting headers with an incorrect hash"
echo ""

# The mysterious hash that appears in GetHeaders after syncing 2000 headers
MYSTERY_HASH="00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749"

# The actual hash at height 2000 from the logs
EXPECTED_HASH="0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06"

echo "Issue Summary:"
echo "- After syncing headers 0-2000, the SPV client should request headers starting from height 2000"
echo "- Expected hash (height 2000): $EXPECTED_HASH"
echo "- Actual hash in GetHeaders:   $MYSTERY_HASH"
echo ""

echo "Possible causes:"
echo "1. Bug in dash-spv's locator hash generation"
echo "2. Incorrect block being used as locator (maybe using wrong height?)"
echo "3. Memory corruption or uninitialized data"
echo "4. Hash from a different chain/network"
echo ""

echo "Diagnostic steps:"
echo "1. Check if this hash exists in any SPV storage files"
echo "2. Look for this hash in dash-spv source code"
echo "3. Monitor when this hash first appears in the logs"
echo "4. Check if this is a valid Dash block hash at all"
echo ""

echo "Searching for the mystery hash in SPV storage..."
SPV_DIR="$HOME/Library/Application Support/Dash-Evo-Tool/spv"
if [ -d "$SPV_DIR" ]; then
    echo "Checking SPV directory: $SPV_DIR"
    find "$SPV_DIR" -type f -exec grep -l "$MYSTERY_HASH" {} \; 2>/dev/null
else
    echo "SPV directory not found at: $SPV_DIR"
fi

echo ""
echo "Recommendation:"
echo "This appears to be a bug in dash-spv where it's generating an incorrect locator hash"
echo "after syncing the first batch of headers. The hash doesn't correspond to any"
echo "block we've seen in the sync process."
echo ""
echo "Next steps:"
echo "1. Report this issue to the dash-spv repository"
echo "2. Add detailed logging in dash-spv to track where this hash is generated"
echo "3. Consider implementing a workaround in the SPV manager to detect and correct this"