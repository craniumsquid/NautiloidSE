import sys, struct, glob, os, zlib, json
import lz4.block, zstandard
root = sys.argv[1]
for path in sorted(glob.glob(os.path.join(root, '*', '*.lsv'))):
    d = open(path, 'rb').read()
    magic, ver, off, size, flags, prio = struct.unpack_from('<4sIQIBB', d, 0)
    nparts = struct.unpack_from('<H', d, 38)[0]
    n, csize = struct.unpack_from('<II', d, off)
    tbl = lz4.block.decompress(d[off+8:off+8+csize], uncompressed_size=n*272)
    print(f'== {os.path.relpath(path, root)}  v{ver} flags={flags} prio={prio} parts={nparts} files={n} len={len(d)}')
    for i in range(n):
        e = tbl[i*272:(i+1)*272]
        name = e[:256].split(b'\0')[0].decode()
        lo, hi, part, fl, sod, us = struct.unpack_from('<IHBBII', e, 256)
        o = lo | (hi << 32)
        print(f'   {name:40s} off={o:9d} disk={sod:9d} raw={us:9d} flags=0x{fl:02x} magic={d[o:o+4].hex()}')
        if name.lower().endswith('.json'):
            raw = d[o:o+sod]
            m = fl & 0x0f
            if m == 1: raw = zlib.decompress(raw)
            elif m == 2: raw = lz4.block.decompress(raw, uncompressed_size=us)
            elif m == 3: raw = zstandard.ZstdDecompressor().decompress(raw, max_output_size=us)
            try:
                j = json.loads(raw)
                print('     JSON:', json.dumps(j)[:900])
            except Exception as ex:
                print('     json err', ex, raw[:200])
