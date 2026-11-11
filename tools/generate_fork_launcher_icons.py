#!/usr/bin/env python3
"""Generate the deliberately distinct FORK launcher badge without external dependencies."""
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / 'packaging/icons/fork'
GLYPHS = ['111101101111101101101', '111101101101101101111',
          '111101101111110101101', '101101110100110101101']

def png(size):
    scale = size / 26
    left, top = (size - 23 * scale) // 2, (size - 7 * scale) // 2
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            gx, gy = int((x - left) // scale), int((y - top) // scale)
            on = (0 <= gx < 23 and 0 <= gy < 7 and gx % 6 < 3
                  and GLYPHS[gx // 6][gy * 3 + gx % 6] == '1')
            rows.extend((255, 255, 255, 255) if on else (76, 35, 131, 255))
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(bytes(rows))) + chunk(b'IEND', b''))

ROOT.mkdir(parents=True, exist_ok=True)
for size in (16, 24, 32, 48, 64, 128, 256, 512):
    (ROOT / f'{size}.png').write_bytes(png(size))
image = png(256)
(ROOT / 'launcher.ico').write_bytes(struct.pack('<HHH', 0, 1, 1) + struct.pack('<BBBBHHII', 0, 0, 0, 0, 1, 32, len(image), 22) + image)
image = png(512)
body = b'ic09' + struct.pack('>I', len(image) + 8) + image
(ROOT / 'launcher.icns').write_bytes(b'icns' + struct.pack('>I', len(body) + 8) + body)
(ROOT / 'launcher.svg').write_text('<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><rect width="512" height="512" fill="#4c2383"/><text x="256" y="280" text-anchor="middle" font-family="sans-serif" font-size="130" fill="white">FORK</text></svg>\n')
