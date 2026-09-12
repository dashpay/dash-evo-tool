#!/usr/bin/env python3
"""Verify a Dash "signed message" (as produced by DET's Key Info -> Sign Message)
against an expected address, with zero third-party dependencies.

This is the core of the key-functionality check: it recovers the signer's
address from the recoverable-ECDSA signature and confirms it matches the key's
known address. Used to prove that migrated identity keys still sign correctly
after a DET version upgrade, without needing any platform/DAPI connectivity.

Usage:
    verify_signed_message.py <address> <base64_signature> <message>

Exit code 0 and "MATCH" when the signature verifies for the address, else 1.

Notes:
- Implements secp256k1 public-key recovery and Dash's message-hash scheme
  ("\\x19DarkCoin Signed Message:\\n" || CompactSize(len) || message,
  double-SHA256).
- Address version byte defaults to 0x8c (Dash testnet/regtest/devnet, "y..."
  prefix); pass --mainnet for 0x4c ("X..." prefix).
"""
import argparse
import base64
import hashlib

P = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
B = 7
Gx = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
Gy = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8


def inv(a, m):
    return pow(a, m - 2, m)


def point_add(p, q):
    if p is None:
        return q
    if q is None:
        return p
    if p[0] == q[0] and (p[1] + q[1]) % P == 0:
        return None
    if p == q:
        lam = (3 * p[0] * p[0]) * inv(2 * p[1], P) % P
    else:
        lam = (q[1] - p[1]) * inv(q[0] - p[0], P) % P
    x = (lam * lam - p[0] - q[0]) % P
    return (x, (lam * (p[0] - x) - p[1]) % P)


def point_mul(k, p):
    r = None
    while k:
        if k & 1:
            r = point_add(r, p)
        p = point_add(p, p)
        k >>= 1
    return r


def compact_size(n: int) -> bytes:
    """Encode an unsigned integer using Dash Core's CompactSize format."""
    if n < 0xFD:
        return bytes([n])
    if n <= 0xFFFF:
        return b"\xfd" + n.to_bytes(2, "little")
    if n <= 0xFFFFFFFF:
        return b"\xfe" + n.to_bytes(4, "little")
    return b"\xff" + n.to_bytes(8, "little")


def dash_message_hash(message: str) -> bytes:
    encoded_message = message.encode("utf-8")
    data = (
        b"\x19DarkCoin Signed Message:\n"
        + compact_size(len(encoded_message))
        + encoded_message
    )
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def hash160(b: bytes) -> bytes:
    return hashlib.new("ripemd160", hashlib.sha256(b).digest()).digest()


def b58check(payload: bytes) -> str:
    d = payload + hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4]
    n = int.from_bytes(d, "big")
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    out = ""
    while n > 0:
        n, r = divmod(n, 58)
        out = alphabet[r] + out
    return "1" * (len(d) - len(d.lstrip(b"\x00"))) + out


def recover_address(sig_b64: str, message: str, version: int) -> str:
    raw = base64.b64decode(sig_b64)
    if len(raw) != 65:
        raise ValueError(f"expected 65-byte signature, got {len(raw)}")
    header = raw[0]
    r = int.from_bytes(raw[1:33], "big")
    s = int.from_bytes(raw[33:65], "big")
    recid = (header - 27) & 3
    # x coordinate of R (recid>>1 selects the second candidate when r overflowed)
    x = r + (recid >> 1) * N
    y = pow((pow(x, 3, P) + B) % P, (P + 1) // 4, P)
    if (y & 1) != (recid & 1):
        y = P - y
    e = int.from_bytes(dash_message_hash(message), "big")
    r_inv = inv(r, N)
    q = point_add(point_mul((s * r_inv) % N, (x, y)),
                  point_mul((-e * r_inv) % N, (Gx, Gy)))
    # DET signs compressed keys ("+4" in the header), so encode compressed
    prefix = b"\x02" if q[1] % 2 == 0 else b"\x03"
    return b58check(bytes([version]) + hash160(prefix + q[0].to_bytes(32, "big")))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("address")
    ap.add_argument("signature")
    ap.add_argument("message")
    ap.add_argument("--mainnet", action="store_true", help="use 0x4c version byte")
    args = ap.parse_args()
    version = 0x4C if args.mainnet else 0x8C
    recovered = recover_address(args.signature, args.message, version)
    ok = recovered == args.address
    print(f"recovered address: {recovered}")
    print(f"expected address : {args.address}")
    print("MATCH" if ok else "MISMATCH")
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
