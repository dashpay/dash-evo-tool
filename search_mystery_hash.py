#!/usr/bin/env python3
import os
import hashlib

mystery_hash = "00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749"
expected_hash = "0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06"

print("Searching for Mystery Hash in SPV Storage")
print("=========================================\n")

home = os.environ.get('HOME', '.')
segment_path = os.path.join(home, "Library/Application Support/Dash-Evo-Tool/spv/dash/headers/segment_0000.dat")

if os.path.exists(segment_path):
    with open(segment_path, 'rb') as f:
        data = f.read()
        
    header_size = 80
    num_headers = len(data) // header_size
    
    print(f"Total headers in file: {num_headers}")
    print(f"Searching for mystery hash...\n")
    
    found_mystery = False
    found_expected = False
    
    for i in range(num_headers):
        header_data = data[i * header_size:(i + 1) * header_size]
        # Calculate hash (double SHA256)
        hash1 = hashlib.sha256(header_data).digest()
        hash2 = hashlib.sha256(hash1).digest()
        hash_hex = hash2[::-1].hex()
        
        if hash_hex == mystery_hash:
            print(f"🎯 FOUND MYSTERY HASH at height {i}!")
            found_mystery = True
            # Print surrounding headers
            print("\nSurrounding headers:")
            for j in range(max(0, i-2), min(num_headers, i+3)):
                h_data = data[j * header_size:(j + 1) * header_size]
                h1 = hashlib.sha256(h_data).digest()
                h2 = hashlib.sha256(h1).digest()
                h_hex = h2[::-1].hex()
                marker = " <-- MYSTERY" if j == i else ""
                print(f"  Height {j}: {h_hex}{marker}")
            print()
            
        if hash_hex == expected_hash:
            print(f"✓ Found expected hash at height {i}")
            found_expected = True
    
    if not found_mystery:
        print("❌ Mystery hash NOT found in the segment file")
        print("   This suggests it's being generated incorrectly, not read from storage")
        
    if not found_expected:
        print("❌ Expected hash for height 2000 NOT found")
        print("   The file may not contain headers up to 2000 yet")
        
    # Check if the file might be corrupted or have padding
    print(f"\nFile analysis:")
    print(f"- File size: {len(data)} bytes")
    print(f"- Complete headers: {num_headers}")
    print(f"- Leftover bytes: {len(data) % header_size}")
    
    # Check the last few headers
    print("\nLast 5 headers in file:")
    for i in range(max(0, num_headers - 5), num_headers):
        header_data = data[i * header_size:(i + 1) * header_size]
        hash1 = hashlib.sha256(header_data).digest()
        hash2 = hashlib.sha256(hash1).digest()
        hash_hex = hash2[::-1].hex()
        print(f"  Height {i}: {hash_hex}")
else:
    print(f"Segment file not found: {segment_path}")