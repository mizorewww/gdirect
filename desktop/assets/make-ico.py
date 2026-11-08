#!/usr/bin/env python3
"""Writes gdirect.ico, PNG entries from the iconset make-icon.swift renders.

python3 make-ico.py <dir>/gdirect.iconset gdirect.ico
"""
import pathlib
import struct
import sys

src = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = [(16, 'icon_16x16.png'), (32, 'icon_32x32.png'), (64, 'icon_32x32@2x.png'),
         (128, 'icon_128x128.png'), (256, 'icon_256x256.png')]
entries = [(size, (src / name).read_bytes()) for size, name in files]
table = b''
data = b''
offset = 6 + 16 * len(entries)
for size, png in entries:
    dim = 0 if size >= 256 else size
    table += struct.pack('<BBBBHHII', dim, dim, 0, 0, 1, 32, len(png), offset + len(data))
    data += png
out.write_bytes(struct.pack('<HHH', 0, 1, len(entries)) + table + data)
