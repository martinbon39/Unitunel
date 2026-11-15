"""Pixel-art app icon: a 16x16 grid scaled to 1024 px, written without dependencies."""
import struct, zlib, sys

G, S = 16, 64
grid = [[1] * G for _ in range(G)]
for y in (0, G - 1):
    for x in (0, G - 1):
        grid[y][x] = 0

def ring(a):
    for i in range(a, G - a):
        grid[a][i] = grid[G - 1 - a][i] = grid[i][a] = grid[i][G - 1 - a] = 2

ring(2)
ring(5)
for y in (7, 8):
    for x in (7, 8):
        grid[y][x] = 2
grid[7][G - 3] = grid[8][G - 3] = 1

colors = {0: (0, 0, 0, 0), 1: (10, 10, 10, 255), 2: (245, 245, 240, 255)}
rows = []
for y in range(G * S):
    row = bytearray([0])
    for x in range(G * S):
        row += bytes(colors[grid[y // S][x // S]])
    rows.append(bytes(row))

def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", G * S, G * S, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")
open(sys.argv[1] if len(sys.argv) > 1 else "icon.png", "wb").write(png)
