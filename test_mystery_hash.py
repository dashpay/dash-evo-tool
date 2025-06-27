#!/usr/bin/env python3
import os
import struct

mystery_hash = "00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749"
expected_hash = "0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06"

print("Analyzing Mystery Hash in SPV Storage")
print("====================================\n")

print(f"Expected hash at height 2000: {expected_hash}")
print(f"Mystery hash in GetHeaders:   {mystery_hash}")
print()

# Check if segment file exists
home = os.environ.get('HOME', '.')
segment_path = os.path.join(home, "Library/Application Support/Dash-Evo-Tool/spv/dash/headers/segment_0000.dat")

if os.path.exists(segment_path):
    print(f"Found segment file: {segment_path}")
    print(f"File size: {os.path.getsize(segment_path)} bytes")
    
    # Calculate expected size for 2001 headers (0-2000)
    header_size = 80  # Bitcoin/Dash header is 80 bytes
    expected_size = 2001 * header_size
    print(f"Expected size for 2001 headers: {expected_size} bytes")
    
    # Read and analyze the file
    with open(segment_path, 'rb') as f:
        data = f.read()
        
        # Check headers around position 2000
        print("\nChecking headers around position 2000:")
        for i in range(1998, 2003):
            if i * header_size + header_size <= len(data):
                header_data = data[i * header_size:(i + 1) * header_size]
                # Calculate hash (double SHA256)
                import hashlib
                hash1 = hashlib.sha256(header_data).digest()
                hash2 = hashlib.sha256(hash1).digest()
                hash_hex = hash2[::-1].hex()  # Reverse for display
                
                print(f"  Height {i}: {hash_hex}")
                
                if hash_hex == mystery_hash:
                    print(f"    ^^^ FOUND MYSTERY HASH at height {i}!")
                elif hash_hex == expected_hash:
                    print(f"    ^^^ This is the expected hash for height 2000")
    
    print("\nRecommendations:")
    print("1. The mystery hash might be from:")
    print("   - Reading the wrong height due to off-by-one error")
    print("   - Block locator using wrong index")
    print("   - Storage returning header from wrong position")
    print("2. Add debug logging in dash-spv at:")
    print("   - block_locator.rs when getting tip header")
    print("   - storage get_header() method")
    print("   - Before sending GetHeaders message")
else:
    print(f"Segment file not found: {segment_path}")
    print("SPV has not synced any headers yet")