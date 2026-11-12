import sys, struct, glob, os, zlib, json
import lz4.block, zstandard
root = sys.argv[1]
def get(d, tbl, n, want):
    for i in range(n):
        e = tbl[i*272:(i+1)*272]
        name = e[:256].split(b'\0')[0].decode()
        lo, hi, part, fl, sod, us = struct.unpack_from('<IHBBII', e, 256)
        if name == want:
            raw = d[lo|(hi<<32):][:sod]; m = fl & 0xf
            if m == 1: return zlib.decompress(raw)
            if m == 2: return lz4.block.decompress(raw, uncompressed_size=us)
            if m == 3: return zstandard.ZstdDecompressor().decompress(raw, max_output_size=us)
            return raw
for path in sorted(glob.glob(os.path.join(root, '*', '*.lsv'))):
    d = open(path, 'rb').read()
    off = struct.unpack_from('<Q', d, 8)[0]
    n, csize = struct.unpack_from('<II', d, off)
    tbl = lz4.block.decompress(d[off+8:off+8+csize], uncompressed_size=n*272)
    j = json.loads(get(d, tbl, n, 'SaveInfo.json'))
    chars = j['Active Party']['Characters']
    print(os.path.basename(os.path.dirname(path)))
    print('   ', {k: v for k, v in j.items() if k != 'Active Party'})
    print('    party:', [(c.get('Origin'), c.get('Level'), '/'.join(x['Main'] for x in c.get('Classes', []))) for c in chars])
    m = get(d, tbl, n, 'meta.lsf')
    print('    meta.lsf:', m[:4], 'ver', struct.unpack_from('<I', m, 4)[0], 'engine', hex(struct.unpack_from('<Q', m, 8)[0]))
    g = get(d, tbl, n, 'Globals.lsf')
    print('    Globals.lsf:', g[:4], 'ver', struct.unpack_from('<I', g, 4)[0], 'hdr', g[8:64].hex())
