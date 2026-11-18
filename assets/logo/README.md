# Logo

`second-brain.svg` (1024×1024) is the only source. Every other file here is generated from it and must be regenerated, never edited by hand.

| File | Made with |
| --- | --- |
| `second-brain-{16,32,48,64,128,256,512}.png` | `resvg` 0.50.0 (`cargo install resvg --locked`) |
| `second-brain.ico` | Python 3 standard library, from the 16–256 px PNGs |

PNGs (RGBA, transparent corners), run in this folder:

```sh
for s in 16 32 48 64 128 256 512; do resvg -w $s -h $s second-brain.svg second-brain-$s.png; done
```

ICO (6 PNG-compressed images: 16, 32, 48, 64, 128, 256), run in this folder:

```python
import struct
sizes = [16, 32, 48, 64, 128, 256]
data = [open(f"second-brain-{s}.png", "rb").read() for s in sizes]
out = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
offset = 6 + 16 * len(sizes)
for s, d in zip(sizes, data):
    out += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(d), offset)
    offset += len(d)
for d in data:
    out += d
open("second-brain.ico", "wb").write(out)
```

Uses: the 256 px PNG is embedded in `secondbrain` (window icon and welcome logo); the `.ico` is the Windows resource of `secondbrain.exe` (`crates/sb-app/build.rs`).
